//! The one place that decides what a user may do on a scope or folder.
//!
//! Loading answers `not_found` (404) for anything the user can't see: an unknown id,
//! a scope it neither owns nor is a member of, a scope or folder in the trash (only
//! the `*_trashed` loaders reach those, for whoever may restore them), a folder of a
//! trashed scope. `require` answers `forbidden` (403) when the user sees the resource
//! but its role is too low. Server admins have no implicit access. A snapshot is
//! reached through its folder ([`SnapshotAccess`]).

use cloudit_core::protocol::ScopeRole;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use super::{folders, members, store};
use crate::db::entities::{folders as folder_entity, scopes, snapshots};
use crate::errors::ApiError;

/// A scope and the caller's role in it.
#[derive(Debug, Clone)]
pub struct ScopeAccess {
    pub scope: scopes::Model,
    pub role: ScopeRole,
    pub user: Uuid,
}

/// The role of `user` in `scope` (ignoring the trash), if any.
async fn role_in(
    db: &impl ConnectionTrait,
    scope: &scopes::Model,
    user: Uuid,
) -> anyhow::Result<Option<ScopeRole>> {
    if scope.owner_id == user {
        return Ok(Some(ScopeRole::Owner));
    }
    Ok(members::find(db, scope.id, user)
        .await?
        .map(|m| members::role_of(&m)))
}

impl ScopeAccess {
    /// A scope outside the trash that `user` owns or is a member of.
    pub async fn load(
        db: &impl ConnectionTrait,
        user: Uuid,
        scope_id: Uuid,
    ) -> Result<Self, ApiError> {
        let scope = store::find(db, scope_id)
            .await?
            .filter(|s| s.deleted_at.is_none())
            .ok_or(ApiError::NotFound)?;
        let role = role_in(db, &scope, user).await?.ok_or(ApiError::NotFound)?;
        Ok(Self { scope, role, user })
    }

    /// A scope in the trash: visible to its owner only.
    pub async fn load_trashed(
        db: &impl ConnectionTrait,
        user: Uuid,
        scope_id: Uuid,
    ) -> Result<Self, ApiError> {
        let scope = store::find(db, scope_id)
            .await?
            .filter(|s| s.deleted_at.is_some() && s.owner_id == user)
            .ok_or(ApiError::NotFound)?;
        Ok(Self {
            scope,
            role: ScopeRole::Owner,
            user,
        })
    }

    /// `forbidden` unless the caller's role is at least `min`.
    pub fn require(&self, min: ScopeRole) -> Result<(), ApiError> {
        if self.role >= min {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

/// A folder and the caller's access to its scope.
#[derive(Debug, Clone)]
pub struct FolderAccess {
    pub folder: folder_entity::Model,
    pub scope: ScopeAccess,
}

impl FolderAccess {
    /// A folder outside the trash, in a scope outside the trash that `user` can see.
    pub async fn load(
        db: &impl ConnectionTrait,
        user: Uuid,
        folder_id: Uuid,
    ) -> Result<Self, ApiError> {
        let folder = folders::find(db, folder_id)
            .await?
            .filter(|f| f.deleted_at.is_none())
            .ok_or(ApiError::NotFound)?;
        let scope = ScopeAccess::load(db, user, folder.scope_id).await?;
        Ok(Self { folder, scope })
    }

    /// A folder in the trash, in a scope outside the trash: visible to the scope's
    /// owner and admins only (the trash is theirs to manage).
    pub async fn load_trashed(
        db: &impl ConnectionTrait,
        user: Uuid,
        folder_id: Uuid,
    ) -> Result<Self, ApiError> {
        let folder = folders::find(db, folder_id)
            .await?
            .filter(|f| f.deleted_at.is_some())
            .ok_or(ApiError::NotFound)?;
        let scope = ScopeAccess::load(db, user, folder.scope_id).await?;
        if scope.role < ScopeRole::Admin {
            return Err(ApiError::NotFound);
        }
        Ok(Self { folder, scope })
    }

    /// `forbidden` unless the caller's role is at least `min`.
    pub fn require(&self, min: ScopeRole) -> Result<(), ApiError> {
        self.scope.require(min)
    }
}

/// A snapshot and the caller's access to its folder (404 like the folder: unknown id,
/// folder or scope in the trash, no membership).
#[derive(Debug, Clone)]
pub struct SnapshotAccess {
    pub snapshot: snapshots::Model,
    pub folder: FolderAccess,
}

impl SnapshotAccess {
    pub async fn load(
        db: &impl ConnectionTrait,
        user: Uuid,
        snapshot_id: Uuid,
    ) -> Result<Self, ApiError> {
        let snapshot = crate::sync::snapshots::find(db, snapshot_id)
            .await?
            .ok_or(ApiError::NotFound)?;
        let folder = FolderAccess::load(db, user, snapshot.folder_id).await?;
        Ok(Self { snapshot, folder })
    }

    /// `forbidden` unless the caller's role is at least `min`.
    pub fn require(&self, min: ScopeRole) -> Result<(), ApiError> {
        self.folder.require(min)
    }
}
