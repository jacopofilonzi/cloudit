//! Folders: the synced units inside a scope (`/api/v1/scopes/{id}/folders`,
//! `/api/v1/folders/{id}`).
//!
//! Retention (applied by a background job, milestone 4c) keeps per folder the last
//! `max_versions` snapshots and, of those, at most `max_per_day` per UTC day (the most
//! recent of each day); the head, pinned and unresolved conflict snapshots are never
//! deleted and don't count. A folder's override `null` means the server default
//! (`/api/v1/admin/settings`); `0` means unlimited.

use serde::{Deserialize, Deserializer, Serialize};

/// One folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct FolderInfo {
    pub id: String,
    pub scope_id: String,
    pub name: String,
    /// The current snapshot (`null` until the first upload, milestone 4b).
    pub head_snapshot_id: Option<String>,
    /// The folder's own limit; `null` = the server default.
    pub retention_max_versions: Option<u32>,
    /// The folder's own limit; `null` = the server default.
    pub retention_max_per_day: Option<u32>,
    /// The limit in force (override, else server default); `0` = unlimited.
    pub effective_retention_max_versions: u32,
    /// The limit in force (override, else server default); `0` = unlimited.
    pub effective_retention_max_per_day: u32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// `POST /api/v1/scopes/{id}/folders` (editor or more; the retention overrides need
/// admin or more).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CreateFolderRequest {
    /// Trimmed; 1 to 100 characters, no control characters; unique in the scope
    /// outside the trash, ignoring case.
    pub name: String,
    /// `null` or omitted = server default; `0` = unlimited.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub retention_max_versions: Option<u32>,
    /// `null` or omitted = server default; `0` = unlimited.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub retention_max_per_day: Option<u32>,
}

/// `PATCH /api/v1/folders/{id}`: omitted fields stay unchanged. Renaming needs editor
/// or more, retention changes admin or more. For the retention fields `null` resets
/// to the server default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateFolderRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    /// Omitted: unchanged; `null`: server default; a number: the folder's own limit.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(test, ts(optional, type = "number | null"))]
    pub retention_max_versions: Option<Option<u32>>,
    /// Omitted: unchanged; `null`: server default; a number: the folder's own limit.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(test, ts(optional, type = "number | null"))]
    pub retention_max_per_day: Option<Option<u32>>,
}

/// A field that is present (possibly `null`) becomes `Some`; with `#[serde(default)]`
/// an omitted one stays `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_distinguishes_omitted_and_null() {
        let parse = |s: &str| serde_json::from_str::<UpdateFolderRequest>(s).unwrap();
        assert_eq!(parse("{}"), UpdateFolderRequest::default());
        let body = parse(r#"{"retention_max_versions":null,"retention_max_per_day":3}"#);
        assert_eq!(body.retention_max_versions, Some(None));
        assert_eq!(body.retention_max_per_day, Some(Some(3)));
        assert!(serde_json::from_str::<UpdateFolderRequest>(r#"{"other":1}"#).is_err());
        assert!(
            serde_json::from_str::<UpdateFolderRequest>(r#"{"retention_max_versions":-1}"#)
                .is_err()
        );
    }
}
