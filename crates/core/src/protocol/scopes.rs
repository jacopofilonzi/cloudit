//! Scopes, sharing and the trash (`/api/v1/scopes*`, `/api/v1/trash`).
//!
//! A scope is a user-named group of folders, owned by one user (role `owner`) and
//! shareable with other users as `viewer`, `editor` or `admin`. Only the owner manages
//! the members; any member can leave. Resources the caller can't see answer
//! `not_found`, never `forbidden`, so their existence isn't revealed.

use serde::{Deserialize, Serialize};

/// The caller's role in a scope, from least to most powerful.
///
/// - `viewer`: sees the scope, its folders, history and members; downloads.
/// - `editor`: viewer + creates and renames folders, uploads and commits versions,
///   restores, pins and labels snapshots, resolves conflicts.
/// - `admin`: editor + moves folders to the trash and restores them, changes a
///   folder's retention.
/// - `owner`: everything, plus renaming the scope, moving it to the trash and
///   restoring it, and managing the members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ScopeRole {
    Viewer,
    Editor,
    Admin,
    Owner,
}

impl ScopeRole {
    /// The stored and API name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }
}

/// A role a member can be given (the owner is implicit and can't be granted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum MemberRole {
    Viewer,
    #[default]
    Editor,
    Admin,
}

impl MemberRole {
    /// The stored and API name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Admin => "admin",
        }
    }

    /// Parses a stored name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

impl From<MemberRole> for ScopeRole {
    fn from(role: MemberRole) -> Self {
        match role {
            MemberRole::Viewer => Self::Viewer,
            MemberRole::Editor => Self::Editor,
            MemberRole::Admin => Self::Admin,
        }
    }
}

/// A user as shown to the other members of a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ScopeUser {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

/// One scope the caller can see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ScopeInfo {
    pub id: String,
    pub name: String,
    /// The caller's role.
    pub role: ScopeRole,
    pub owner: ScopeUser,
    /// Folders not in the trash.
    pub folder_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// `POST /api/v1/scopes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CreateScopeRequest {
    /// Trimmed; 1 to 100 characters, no control characters; unique among the owner's
    /// scopes outside the trash, ignoring case.
    pub name: String,
}

/// `PATCH /api/v1/scopes/{id}` (owner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateScopeRequest {
    pub name: String,
}

/// Optional body of `POST /api/v1/scopes/{id}/restore` and
/// `POST /api/v1/folders/{id}/restore`: restore under another name when the old one
/// was taken meanwhile (`name_taken`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct RestoreRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
}

/// One member of a scope in `GET /api/v1/scopes/{id}/members` (the owner first, with
/// role `owner`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ScopeMember {
    pub user: ScopeUser,
    pub role: ScopeRole,
    /// Who shared the scope with this member (`null` for the owner).
    pub added_by: Option<String>,
    /// When the member was added (the scope's creation for the owner).
    pub created_at: String,
}

/// `POST /api/v1/scopes/{id}/members` (owner): share with the approved, enabled
/// account that has exactly this email. Sharing again with a member changes its role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AddMemberRequest {
    pub email: String,
    /// Default `editor`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub role: Option<MemberRole>,
}

/// `PATCH /api/v1/scopes/{id}/members/{user_id}` (owner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateMemberRequest {
    pub role: MemberRole,
}

/// What a [`TrashItem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum TrashItemKind {
    Scope,
    Folder,
}

/// One restorable item in `GET /api/v1/trash`: scopes the caller owns, and folders of
/// scopes (outside the trash) where the caller is owner or admin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct TrashItem {
    pub kind: TrashItemKind,
    pub id: String,
    pub name: String,
    /// The scope itself for a scope, the folder's scope for a folder.
    pub scope_id: String,
    pub scope_name: String,
    pub deleted_at: String,
    /// Who moved it to the trash.
    pub deleted_by: Option<String>,
    /// When it will be deleted for good (`deleted_at` + the `trash_retention_days`
    /// setting).
    pub purge_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_order_and_names() {
        assert!(ScopeRole::Viewer < ScopeRole::Editor);
        assert!(ScopeRole::Editor < ScopeRole::Admin);
        assert!(ScopeRole::Admin < ScopeRole::Owner);
        for role in [MemberRole::Viewer, MemberRole::Editor, MemberRole::Admin] {
            assert_eq!(MemberRole::parse(role.as_str()), Some(role));
            assert_eq!(ScopeRole::from(role).as_str(), role.as_str());
            assert_eq!(
                serde_json::to_value(role).unwrap(),
                serde_json::json!(role.as_str())
            );
        }
        assert_eq!(MemberRole::parse("owner"), None);
        assert_eq!(MemberRole::default(), MemberRole::Editor);
        let body: AddMemberRequest = serde_json::from_str(r#"{"email":"a@example.com"}"#).unwrap();
        assert_eq!(body.role, None);
        assert!(
            serde_json::from_str::<AddMemberRequest>(r#"{"email":"a","role":"owner"}"#).is_err()
        );
    }
}
