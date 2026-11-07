//! Blob keys of the sync objects (layout in the [`super`] docs).

use cloudit_core::hash::{ChunkHash, ManifestHash};
use uuid::Uuid;

/// `scopes/{scope_id}/chunks/{hash}`.
#[must_use]
pub fn chunk(scope: Uuid, hash: &ChunkHash) -> String {
    format!("scopes/{scope}/chunks/{hash}")
}

/// `scopes/{scope_id}/manifests/{hash}`.
#[must_use]
pub fn manifest(scope: Uuid, hash: &ManifestHash) -> String {
    format!("scopes/{scope}/manifests/{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let scope = Uuid::nil();
        let hash = ChunkHash::of(b"");
        assert_eq!(
            chunk(scope, &hash),
            format!("scopes/00000000-0000-0000-0000-000000000000/chunks/{hash}")
        );
        let hash = ManifestHash::of(b"");
        assert!(manifest(scope, &hash).ends_with(&format!("/manifests/{hash}")));
    }
}
