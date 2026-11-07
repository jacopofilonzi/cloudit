//! Folders: the synced units inside a scope (`/api/v1/scopes/{id}/folders`,
//! `/api/v1/folders/{id}`).
//!
//! Retention (applied by a background job, milestone 4c) keeps per folder the last
//! `max_versions` snapshots and, of those, at most `max_per_day` per UTC day (the most
//! recent of each day); the head, pinned and unresolved conflict snapshots are never
//! deleted and don't count. A folder's override `null` means the server default
//! (`/api/v1/admin/settings`); `0` means unlimited.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    #[cfg_attr(test, ts(as = "Option<Option<u32>>", optional))]
    pub retention_max_versions: Patch<u32>,
    /// Omitted: unchanged; `null`: server default; a number: the folder's own limit.
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    #[cfg_attr(test, ts(as = "Option<Option<u32>>", optional))]
    pub retention_max_per_day: Patch<u32>,
}

/// A request field that distinguishes "omitted" from `null`: with `#[serde(default)]`
/// an omitted field is [`Patch::Missing`], `null` is [`Patch::Null`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> Patch<T> {
    #[must_use]
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// `None` if omitted, `Some(None)` for `null`, `Some(Some(v))` for a value.
    pub fn into_option(self) -> Option<Option<T>> {
        match self {
            Self::Missing => None,
            Self::Null => Some(None),
            Self::Value(v) => Some(Some(v)),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Null, Self::Value))
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing | Self::Null => serializer.serialize_none(),
            Self::Value(v) => serializer.serialize_some(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_distinguishes_omitted_and_null() {
        let parse = |s: &str| serde_json::from_str::<UpdateFolderRequest>(s).unwrap();
        assert_eq!(parse("{}"), UpdateFolderRequest::default());
        let body = parse(r#"{"retention_max_versions":null,"retention_max_per_day":3}"#);
        assert_eq!(body.retention_max_versions, Patch::Null);
        assert_eq!(body.retention_max_per_day, Patch::Value(3));
        assert_eq!(body.retention_max_per_day.into_option(), Some(Some(3)));
        assert_eq!(parse("{}").retention_max_versions.into_option(), None);
        assert!(serde_json::from_str::<UpdateFolderRequest>(r#"{"other":1}"#).is_err());
        assert!(
            serde_json::from_str::<UpdateFolderRequest>(r#"{"retention_max_versions":-1}"#)
                .is_err()
        );
    }
}
