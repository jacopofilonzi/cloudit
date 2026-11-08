//! Publishing the events of each change, with its recipients (see the [`super`] docs).
//! Call these **after** the change committed. They never fail: an error computing the
//! recipients is logged and the event dropped (clients resync through `/sync/heads`).

use anyhow::Context as _;
use cloudit_core::protocol::{
    ChangeKind, ConflictEvent, FolderChangedEvent, FolderHeadEvent, HistoryChangedEvent,
    MembershipChangedEvent, ScopeChangedEvent, ServerEvent, UsageChangedEvent,
};
use sea_orm::{
    ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _, QuerySelect as _,
};
use uuid::Uuid;

use crate::db::entities::{scope_members, scopes};
use crate::state::AppState;

/// Who may see `scope` now: its owner and members (empty if the scope is gone).
pub async fn scope_recipients(db: &impl ConnectionTrait, scope: Uuid) -> anyhow::Result<Vec<Uuid>> {
    let Some(owner) = scopes::Entity::find_by_id(scope)
        .select_only()
        .column(scopes::Column::OwnerId)
        .into_tuple::<Uuid>()
        .one(db)
        .await
        .context("loading a scope owner")?
    else {
        return Ok(Vec::new());
    };
    let mut users: Vec<Uuid> = scope_members::Entity::find()
        .select_only()
        .column(scope_members::Column::UserId)
        .filter(scope_members::Column::ScopeId.eq(scope))
        .into_tuple()
        .all(db)
        .await
        .context("loading scope members")?;
    users.push(owner);
    Ok(users)
}

/// Publishes `event` to the scope's owner and members, plus `extra`.
async fn to_scope(state: &AppState, scope: Uuid, extra: Option<Uuid>, event: ServerEvent) {
    match scope_recipients(&state.db, scope).await {
        Ok(mut recipients) => {
            recipients.extend(extra);
            state.events.publish(recipients, event);
        }
        Err(err) => tracing::warn!(
            error = %format!("{err:#}"),
            event = event.name(),
            "event not published"
        ),
    }
}

pub async fn folder_head(state: &AppState, scope: Uuid, folder: Uuid, snapshot: Uuid) {
    let event = ServerEvent::FolderHead(FolderHeadEvent {
        scope_id: scope.to_string(),
        folder_id: folder.to_string(),
        snapshot_id: snapshot.to_string(),
    });
    to_scope(state, scope, None, event).await;
}

pub async fn conflict(state: &AppState, scope: Uuid, folder: Uuid, snapshot: Uuid) {
    let event = ServerEvent::Conflict(ConflictEvent {
        scope_id: scope.to_string(),
        folder_id: folder.to_string(),
        snapshot_id: snapshot.to_string(),
    });
    to_scope(state, scope, None, event).await;
}

pub async fn history_changed(state: &AppState, scope: Uuid, folder: Uuid) {
    let event = ServerEvent::HistoryChanged(HistoryChangedEvent {
        scope_id: scope.to_string(),
        folder_id: folder.to_string(),
    });
    to_scope(state, scope, None, event).await;
}

pub fn folder_changed_event(scope: Uuid, folder: Uuid, change: ChangeKind) -> ServerEvent {
    ServerEvent::FolderChanged(FolderChangedEvent {
        scope_id: scope.to_string(),
        folder_id: folder.to_string(),
        change,
    })
}

pub async fn folder_changed(state: &AppState, scope: Uuid, folder: Uuid, change: ChangeKind) {
    to_scope(
        state,
        scope,
        None,
        folder_changed_event(scope, folder, change),
    )
    .await;
}

pub fn scope_changed_event(scope: Uuid, change: ChangeKind) -> ServerEvent {
    ServerEvent::ScopeChanged(ScopeChangedEvent {
        scope_id: scope.to_string(),
        change,
    })
}

pub async fn scope_changed(state: &AppState, scope: Uuid, change: ChangeKind) {
    to_scope(state, scope, None, scope_changed_event(scope, change)).await;
}

/// A membership of `scope` changed for `affected` (added, role changed, removed or
/// left): every member and `affected` hear it.
pub async fn membership_changed(state: &AppState, scope: Uuid, affected: Uuid) {
    let event = ServerEvent::MembershipChanged(MembershipChangedEvent {
        scope_id: scope.to_string(),
    });
    to_scope(state, scope, Some(affected), event).await;
}

/// The usage of `owner` went down.
pub fn usage_changed(state: &AppState, owner: Uuid) {
    state
        .events
        .publish([owner], ServerEvent::UsageChanged(UsageChangedEvent {}));
}
