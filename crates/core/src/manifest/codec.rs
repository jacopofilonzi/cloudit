//! Manifest serialization, hashing and compression.
//!
//! Durable format:
//!
//! - **Canonical JSON** = `serde_json::to_vec(&manifest)`: compact (no whitespace), object
//!   keys in the fixed declaration order of the structs in [`super::model`] (`version`,
//!   `entries`; `path`, `kind`, `size`, `mtime`, `executable`, `chunks`; `secs`, `nanos`;
//!   `hash`, `size`), integers in plain decimal, strings as UTF-8 with serde_json's escaping
//!   (only `"`, `\` and control characters are escaped; non-ASCII is written as-is). There
//!   are no maps, so the output is deterministic for a given manifest.
//! - **Manifest hash** ([`ManifestHash`]) = SHA-256 of the canonical JSON bytes.
//! - **Stored form** = the canonical JSON as one zstd frame (level 3). The compressed bytes
//!   are not hashed and need not be reproducible.
//!
//! [`decode_manifest`] bounds the decompressed size, checks the version before the rest of
//! the schema (so a newer format gives [`ManifestError::UnsupportedVersion`], not a JSON
//! error), rejects JSON that is valid but not canonical (whitespace, other key order,
//! escapes), so one manifest has exactly one hash, and runs the full validation.

use std::io::Read;

use serde::Deserialize;

use super::model::{MANIFEST_VERSION, Manifest};
use crate::error::ManifestError;
use crate::hash::ManifestHash;

/// Default bound on the decompressed manifest size (64 MiB, roughly 200k entries).
pub const DEFAULT_MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
/// zstd level used to store manifests.
pub const ZSTD_LEVEL: i32 = 3;

/// A manifest ready to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedManifest {
    /// zstd-compressed canonical JSON.
    pub compressed: Vec<u8>,
    /// SHA-256 of the canonical (uncompressed) JSON.
    pub hash: ManifestHash,
}

impl Manifest {
    /// The canonical JSON bytes (see the module docs). Does not validate.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Json`] if serialization fails (not expected for these types).
    pub fn canonical_json(&self) -> Result<Vec<u8>, ManifestError> {
        serde_json::to_vec(self).map_err(ManifestError::Json)
    }

    /// The manifest hash: SHA-256 of [`Manifest::canonical_json`].
    ///
    /// # Errors
    ///
    /// Same as [`Manifest::canonical_json`].
    pub fn hash(&self) -> Result<ManifestHash, ManifestError> {
        Ok(ManifestHash::of(&self.canonical_json()?))
    }

    /// Validates, then serializes and compresses the manifest for storage.
    ///
    /// # Errors
    ///
    /// Any validation error, or a serialization/compression failure.
    pub fn encode(&self) -> Result<EncodedManifest, ManifestError> {
        self.validate()?;
        let json = self.canonical_json()?;
        let compressed =
            zstd::bulk::compress(&json, ZSTD_LEVEL).map_err(ManifestError::Compress)?;
        Ok(EncodedManifest {
            compressed,
            hash: ManifestHash::of(&json),
        })
    }
}

/// Only the version, read before the full schema.
#[derive(Deserialize)]
struct VersionProbe {
    version: u64,
}

/// Decompresses, parses and validates a stored manifest; returns it with its hash.
///
/// `max_bytes` bounds the decompressed JSON size (use [`DEFAULT_MAX_MANIFEST_BYTES`] unless
/// the caller has a reason to differ); nothing larger is ever allocated.
///
/// # Errors
///
/// [`ManifestError::Decompress`], [`ManifestError::TooLarge`], [`ManifestError::Json`],
/// [`ManifestError::UnsupportedVersion`], [`ManifestError::NonCanonical`] or any validation
/// error.
pub fn decode_manifest(
    compressed: &[u8],
    max_bytes: usize,
) -> Result<(Manifest, ManifestHash), ManifestError> {
    let json = decompress_bounded(compressed, max_bytes)?;
    let manifest = from_canonical_json(&json)?;
    Ok((manifest, ManifestHash::of(&json)))
}

/// Parses and validates canonical manifest JSON (already decompressed).
///
/// # Errors
///
/// As [`decode_manifest`], minus the decompression errors.
pub fn from_canonical_json(json: &[u8]) -> Result<Manifest, ManifestError> {
    let probe: VersionProbe = serde_json::from_slice(json).map_err(ManifestError::Json)?;
    if probe.version != u64::from(MANIFEST_VERSION) {
        return Err(ManifestError::UnsupportedVersion(probe.version));
    }
    let manifest: Manifest = serde_json::from_slice(json).map_err(ManifestError::Json)?;
    if manifest.canonical_json()? != json {
        return Err(ManifestError::NonCanonical);
    }
    manifest.validate()?;
    Ok(manifest)
}

fn decompress_bounded(compressed: &[u8], max_bytes: usize) -> Result<Vec<u8>, ManifestError> {
    let decoder =
        zstd::stream::read::Decoder::with_buffer(compressed).map_err(ManifestError::Decompress)?;
    let mut out = Vec::new();
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    decoder
        .take(limit)
        .read_to_end(&mut out)
        .map_err(ManifestError::Decompress)?;
    if out.len() > max_bytes {
        return Err(ManifestError::TooLarge(max_bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::ChunkHash;
    use crate::manifest::model::{ChunkRef, ManifestEntry, Timestamp};

    fn sample() -> Manifest {
        let t = Timestamp {
            secs: 1_700_000_000,
            nanos: 123_456_789,
        };
        Manifest::new(vec![
            ManifestEntry::file(
                "saves/slot1.sav",
                t,
                false,
                vec![
                    ChunkRef {
                        hash: ChunkHash::of(b"chunk-a"),
                        size: 65_536,
                    },
                    ChunkRef {
                        hash: ChunkHash::of(b"chunk-b"),
                        size: 4_464,
                    },
                ],
            ),
            ManifestEntry::dir("screenshots", Timestamp { secs: -5, nanos: 0 }),
            ManifestEntry::file("bin/run.sh", t, true, vec![]),
            ManifestEntry::file(
                "unicode/日本 'q' é.txt",
                Timestamp { secs: 0, nanos: 1 },
                false,
                vec![ChunkRef {
                    hash: ChunkHash::of(b"chunk-c"),
                    size: 10,
                }],
            ),
        ])
        .unwrap()
    }

    /// Golden vector: pins the canonical JSON and the manifest hash. If this fails, the
    /// durable manifest format changed: do not update the expected values, bump the version.
    #[test]
    fn golden_canonical_json_and_hash() {
        let m = sample();
        let json = String::from_utf8(m.canonical_json().unwrap()).unwrap();
        assert_eq!(json, GOLDEN_JSON);
        assert_eq!(m.hash().unwrap().to_string(), GOLDEN_HASH);
    }

    const GOLDEN_JSON: &str = concat!(
        r#"{"version":1,"entries":["#,
        r#"{"path":"bin/run.sh","kind":"file","size":0,"mtime":{"secs":1700000000,"nanos":123456789},"executable":true,"chunks":[]},"#,
        r#"{"path":"saves/slot1.sav","kind":"file","size":70000,"mtime":{"secs":1700000000,"nanos":123456789},"executable":false,"chunks":["#,
        r#"{"hash":"ba6e04391775aa27f39df72198b6394dbf99fc52a39f26acbd5f3d78983b745c","size":65536},"#,
        r#"{"hash":"0f276ace01c571d7587f5417db6f19cb95c1ce4891f3cee9852d278ca385c442","size":4464}]},"#,
        r#"{"path":"screenshots","kind":"dir","size":0,"mtime":{"secs":-5,"nanos":0},"executable":false,"chunks":[]},"#,
        r#"{"path":"unicode/日本 'q' é.txt","kind":"file","size":10,"mtime":{"secs":0,"nanos":1},"executable":false,"chunks":["#,
        r#"{"hash":"36317defccb8df3ec7f8ae18211cd03656e004af2a2a22fc5ee6513f9fdeb7e3","size":10}]}"#,
        r#"]}"#,
    );
    const GOLDEN_HASH: &str = "3f0418cf59d9573b7cc405adbb0d7ec23256f26cc193374ccd603558efbd3036";

    #[test]
    fn round_trip() {
        let m = sample();
        let encoded = m.encode().unwrap();
        assert_eq!(encoded.hash, m.hash().unwrap());
        let (decoded, hash) =
            decode_manifest(&encoded.compressed, DEFAULT_MAX_MANIFEST_BYTES).unwrap();
        assert_eq!(decoded, m);
        assert_eq!(hash, encoded.hash);
    }

    #[test]
    fn empty_manifest_round_trips() {
        let m = Manifest::new(vec![]).unwrap();
        assert_eq!(
            m.canonical_json().unwrap(),
            br#"{"version":1,"entries":[]}"#
        );
        let encoded = m.encode().unwrap();
        let (decoded, _) = decode_manifest(&encoded.compressed, 1024).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn encode_validates() {
        let mut m = sample();
        m.entries.swap(0, 1);
        assert!(matches!(m.encode(), Err(ManifestError::NotSorted { .. })));
    }

    fn compress(json: &str) -> Vec<u8> {
        zstd::bulk::compress(json.as_bytes(), 3).unwrap()
    }

    #[test]
    fn decode_rejects_too_large() {
        let encoded = sample().encode().unwrap();
        let err = decode_manifest(&encoded.compressed, 100).unwrap_err();
        assert!(matches!(err, ManifestError::TooLarge(100)));
    }

    #[test]
    fn decode_rejects_corrupt_zstd() {
        let err = decode_manifest(b"not zstd at all", DEFAULT_MAX_MANIFEST_BYTES).unwrap_err();
        assert!(matches!(err, ManifestError::Decompress(_)));
    }

    #[test]
    fn decode_rejects_other_versions() {
        let future = compress(r#"{"version":2,"entries":[],"new_field":true}"#);
        let err = decode_manifest(&future, DEFAULT_MAX_MANIFEST_BYTES).unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedVersion(2)));
        let err = decode_manifest(&compress(r#"{"entries":[]}"#), 1024).unwrap_err();
        assert!(matches!(err, ManifestError::Json(_)));
    }

    #[test]
    fn decode_rejects_bad_json_and_unknown_fields() {
        for json in [
            "[]",
            "{",
            r#"{"version":1,"entries":[],"extra":1}"#,
            r#"{"version":1,"entries":[{"path":"a","kind":"symlink","size":0,"mtime":{"secs":0,"nanos":0},"executable":false,"chunks":[]}]}"#,
            r#"{"version":1,"entries":[{"path":"a","kind":"file","size":0,"mtime":{"secs":0,"nanos":0},"executable":false}]}"#,
            r#"{"version":1,"entries":[{"path":"a","kind":"file","size":1,"mtime":{"secs":0,"nanos":0},"executable":false,"chunks":[{"hash":"XYZ","size":1}]}]}"#,
        ] {
            let err = decode_manifest(&compress(json), 1024).unwrap_err();
            assert!(matches!(err, ManifestError::Json(_)), "{json}: {err:?}");
        }
    }

    #[test]
    fn decode_rejects_non_canonical_json() {
        // `a` written as a JSON unicode escape (backslash + u0061): same value, other bytes.
        let escaped_a = format!("{}u0061", char::from(92));
        let escaped = r#"{"version":1,"entries":[{"path":"PATH","kind":"dir","size":0,"mtime":{"secs":0,"nanos":0},"executable":false,"chunks":[]}]}"#
            .replace("PATH", &escaped_a);
        for json in [
            r#"{ "version":1,"entries":[]}"#,
            r#"{"entries":[],"version":1}"#,
            "{\"version\":1,\"entries\":[]}\n",
            escaped.as_str(),
        ] {
            let err = decode_manifest(&compress(json), 1024).unwrap_err();
            assert!(
                matches!(err, ManifestError::NonCanonical),
                "{json}: {err:?}"
            );
        }
    }

    #[test]
    fn decode_validates() {
        let json = r#"{"version":1,"entries":[{"path":"../x","kind":"dir","size":0,"mtime":{"secs":0,"nanos":0},"executable":false,"chunks":[]}]}"#;
        let err = decode_manifest(&compress(json), 1024).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidPath { .. }));
    }
}
