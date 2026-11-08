//! `/api/v1/scopes*`: scopes and their members. Every handler resolves the caller's
//! role through `scopes::access` (404 for what it can't see, 403 for a role too low).
//!
//! Device tokens (`AnyUser`) may list and read scopes and members and create scopes;
//! renaming, the trash and member management need a web session (`SessionUser`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    AddMemberRequest, ChangeKind, CreateScopeRequest, RestoreRequest, ScopeInfo, ScopeMember,
    ScopeRole, ScopeUser, UpdateMemberRequest, UpdateScopeRequest,
};
use uuid::Uuid;

use super::extract::{ApiJson, optional_json, parse_id};
use crate::accounts::users;
use crate::auth::extractors::{AnyUser, SessionUser};
use crate::auth::limits;
use crate::db::entities::{scopes as scope_entity, users as user_entity};
use crate::errors::ApiError;
use crate::events::notify;
use crate::scopes::access::ScopeAccess;
use crate::scopes::{members, names, store};
use crate::state::AppState;
use crate::time::rfc3339;
use crate::validate;

pub fn scope_user(user: &user_entity::Model) -> ScopeUser {
    ScopeUser {
        id: user.id.to_string(),
        email: user.email.clone(),
        display_name: user.display_name.clone(),
    }
}

fn scope_info(
    scope: &scope_entity::Model,
    role: ScopeRole,
    owner: &user_entity::Model,
    folder_count: u64,
) -> ScopeInfo {
    ScopeInfo {
        id: scope.id.to_string(),
        name: scope.name.clone(),
        role,
        owner: scope_user(owner),
        folder_count,
        created_at: rfc3339(scope.created_at),
        updated_at: rfc3339(scope.updated_at),
    }
}

/// The API view of one scope (loads its owner and folder count).
async fn view(
    state: &AppState,
    scope: &scope_entity::Model,
    role: ScopeRole,
) -> Result<ScopeInfo, ApiError> {
    let owner = users::find(&state.db, scope.owner_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("scope owner missing"))?;
    let counts = store::folder_counts(&state.db, &[scope.id]).await?;
    Ok(scope_info(
        scope,
        role,
        &owner,
        counts.get(&scope.id).copied().unwrap_or(0),
    ))
}

/// Reloads a scope after a change.
async fn reload(state: &AppState, id: Uuid) -> Result<scope_entity::Model, ApiError> {
    Ok(store::find(&state.db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("scope vanished"))?)
}

/// `GET /scopes`: own and shared scopes outside the trash, by name.
pub async fn list(
    State(state): State<AppState>,
    me: AnyUser,
) -> Result<Json<Vec<ScopeInfo>>, ApiError> {
    let scopes = store::list_visible(&state.db, me.user.id).await?;
    let ids: Vec<Uuid> = scopes.iter().map(|(s, _)| s.id).collect();
    let owner_ids: Vec<Uuid> = scopes.iter().map(|(s, _)| s.owner_id).collect();
    let counts = store::folder_counts(&state.db, &ids).await?;
    let owners = users::find_many(&state.db, &owner_ids).await?;
    scopes
        .iter()
        .map(|(scope, role)| {
            let owner = owners
                .get(&scope.owner_id)
                .ok_or_else(|| anyhow::anyhow!("scope owner missing"))?;
            Ok(scope_info(
                scope,
                *role,
                owner,
                counts.get(&scope.id).copied().unwrap_or(0),
            ))
        })
        .collect::<Result<_, ApiError>>()
        .map(Json)
}

/// `POST /scopes {name}` → 201; the caller owns it.
pub async fn create(
    State(state): State<AppState>,
    me: AnyUser,
    ApiJson(body): ApiJson<CreateScopeRequest>,
) -> Result<(StatusCode, Json<ScopeInfo>), ApiError> {
    let name = names::parse(&body.name, "name")?;
    let scope = store::create(&state.db, me.user.id, name).await?;
    tracing::info!(user_id = %me.user.id, scope_id = %scope.id, "scope created");
    notify::scope_changed(&state, scope.id, ChangeKind::Created).await;
    Ok((
        StatusCode::CREATED,
        Json(scope_info(&scope, ScopeRole::Owner, &me.user, 0)),
    ))
}

/// `GET /scopes/{id}`.
pub async fn get(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<ScopeInfo>, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    Ok(Json(view(&state, &access.scope, access.role).await?))
}

/// `PATCH /scopes/{id} {name}` (owner).
pub async fn update(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateScopeRequest>,
) -> Result<Json<ScopeInfo>, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Owner)?;
    let name = names::parse(&body.name, "name")?;
    store::rename(&state.db, access.scope.id, name).await?;
    notify::scope_changed(&state, access.scope.id, ChangeKind::Updated).await;
    let scope = reload(&state, access.scope.id).await?;
    Ok(Json(view(&state, &scope, access.role).await?))
}

/// `DELETE /scopes/{id}` (owner) → 204: moves the scope to the trash.
pub async fn delete(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Owner)?;
    if !store::trash(&state.db, access.scope.id, me.user.id).await? {
        return Err(ApiError::NotFound);
    }
    tracing::info!(user_id = %me.user.id, scope_id = %access.scope.id, "scope moved to the trash");
    notify::scope_changed(&state, access.scope.id, ChangeKind::Trashed).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /scopes/{id}/restore {name?}` (owner of a trashed scope). `name_taken` if
/// its name is in use: retry with another `name`.
pub async fn restore(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<ScopeInfo>, ApiError> {
    let access = ScopeAccess::load_trashed(&state.db, me.user.id, parse_id(&id)?).await?;
    let body: RestoreRequest = optional_json(&body)?;
    let name = body
        .name
        .as_deref()
        .map(|n| names::parse(n, "name"))
        .transpose()?;
    store::restore(&state.db, &access.scope, name).await?;
    tracing::info!(user_id = %me.user.id, scope_id = %access.scope.id, "scope restored");
    notify::scope_changed(&state, access.scope.id, ChangeKind::Restored).await;
    let scope = reload(&state, access.scope.id).await?;
    Ok(Json(view(&state, &scope, ScopeRole::Owner).await?))
}

/// `GET /scopes/{id}/members` (any member): the owner first, then the members by
/// date added.
pub async fn list_members(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<ScopeMember>>, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let list = members::list(&state.db, access.scope.id).await?;
    let mut ids: Vec<Uuid> = list.iter().map(|m| m.user_id).collect();
    ids.push(access.scope.owner_id);
    let people = users::find_many(&state.db, &ids).await?;
    let person = |id: &Uuid| {
        people
            .get(id)
            .ok_or_else(|| ApiError::from(anyhow::anyhow!("scope member account missing")))
    };
    let mut out = vec![ScopeMember {
        user: scope_user(person(&access.scope.owner_id)?),
        role: ScopeRole::Owner,
        added_by: None,
        created_at: rfc3339(access.scope.created_at),
    }];
    for member in &list {
        out.push(member_view(member, person(&member.user_id)?));
    }
    Ok(Json(out))
}

fn member_view(
    member: &crate::db::entities::scope_members::Model,
    user: &user_entity::Model,
) -> ScopeMember {
    ScopeMember {
        user: scope_user(user),
        role: members::role_of(member),
        added_by: Some(member.added_by.to_string()),
        created_at: rfc3339(member.created_at),
    }
}

/// `POST /scopes/{id}/members {email, role?}` (owner; rate limited) → 201 for a new
/// member, 200 when the user already was one (its role changes). `user_not_found`
/// unless an approved, enabled account has exactly this email.
pub async fn add_member(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AddMemberRequest>,
) -> Result<(StatusCode, Json<ScopeMember>), ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Owner)?;
    limits::enforce(
        &state,
        &format!("member_add:{}", me.user.id),
        limits::MEMBER_ADD_PER_USER,
    )
    .await?;
    let email = validate::email(&body.email, "email")?;
    let user = users::find_by_email(&state.db, &email)
        .await?
        .filter(user_entity::Model::is_active)
        .ok_or(ApiError::UserNotFound)?;
    if user.id == access.scope.owner_id {
        return Err(ApiError::validation("email", "this is the scope's owner"));
    }
    let role = body.role.unwrap_or_default();
    let (member, created) =
        members::upsert(&state.db, access.scope.id, user.id, role, me.user.id).await?;
    tracing::info!(
        user_id = %me.user.id,
        scope_id = %access.scope.id,
        member_id = %user.id,
        role = role.as_str(),
        created,
        "scope shared"
    );
    notify::membership_changed(&state, access.scope.id, user.id).await;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(member_view(&member, &user))))
}

/// `PATCH /scopes/{id}/members/{user_id} {role}` (owner).
pub async fn update_member(
    State(state): State<AppState>,
    me: SessionUser,
    Path((id, user_id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateMemberRequest>,
) -> Result<Json<ScopeMember>, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Owner)?;
    let target = parse_id(&user_id)?;
    let member = members::update_role(&state.db, access.scope.id, target, body.role)
        .await?
        .ok_or(ApiError::NotFound)?;
    let user = users::find(&state.db, target)
        .await?
        .ok_or_else(|| anyhow::anyhow!("scope member account missing"))?;
    tracing::info!(
        user_id = %me.user.id,
        scope_id = %access.scope.id,
        member_id = %target,
        role = body.role.as_str(),
        "scope member role changed"
    );
    notify::membership_changed(&state, access.scope.id, target).await;
    Ok(Json(member_view(&member, &user)))
}

/// `DELETE /scopes/{id}/members/{user_id}` → 204: the owner removes a member, or a
/// member leaves (its own id). The owner can't leave (`forbidden`): it moves the
/// scope to the trash instead.
pub async fn remove_member(
    State(state): State<AppState>,
    me: SessionUser,
    Path((id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let target = parse_id(&user_id)?;
    if target != me.user.id {
        access.require(ScopeRole::Owner)?;
    } else if access.role == ScopeRole::Owner {
        return Err(ApiError::Forbidden);
    }
    if !members::remove(&state.db, access.scope.id, target).await? {
        return Err(ApiError::NotFound);
    }
    tracing::info!(
        user_id = %me.user.id,
        scope_id = %access.scope.id,
        member_id = %target,
        "scope member removed"
    );
    notify::membership_changed(&state, access.scope.id, target).await;
    Ok(StatusCode::NO_CONTENT)
}
