//! SHA-256 identifiers: [`ChunkHash`] (a chunk's content) and [`ManifestHash`] (a manifest's
//! canonical JSON).
//!
//! Durable format: both are the 32-byte SHA-256 digest, written everywhere (JSON, object
//! keys, URLs) as exactly 64 **lowercase** hex characters. Parsing is strict: uppercase,
//! a `0x` prefix, whitespace or any other length are rejected, so a hash has exactly one
//! textual form and string comparison equals digest comparison.
//!
//! The two are distinct types so a chunk id can never be passed where a manifest id is
//! expected (they live in different storage namespaces).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::HashParseError;

/// Length of a hash in its textual (hex) form.
pub const HASH_HEX_LEN: usize = 64;

fn parse_hex(s: &str) -> Result<[u8; 32], HashParseError> {
    if s.len() != HASH_HEX_LEN {
        return Err(HashParseError::Length(s.len()));
    }
    if let Some(bad) = s.chars().find(|c| !matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(HashParseError::InvalidChar(bad));
    }
    let mut out = [0_u8; 32];
    hex::decode_to_slice(s, &mut out).map_err(|_| HashParseError::Length(s.len()))?;
    Ok(out)
}

macro_rules! sha256_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(test, derive(ts_rs::TS), ts(export, type = "string"))]
        pub struct $name([u8; 32]);

        impl $name {
            /// SHA-256 of `data`.
            #[must_use]
            pub fn of(data: &[u8]) -> Self {
                Self(Sha256::digest(data).into())
            }

            /// Wraps an already computed digest.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// The raw 32-byte digest.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex::encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({self})", stringify!($name))
            }
        }

        impl FromStr for $name {
            type Err = HashParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                parse_hex(s).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

sha256_id!(
    /// Identifier of a chunk: SHA-256 of its **uncompressed** bytes.
    ChunkHash
);

sha256_id!(
    /// Identifier of a manifest: SHA-256 of its canonical **uncompressed** JSON bytes.
    ManifestHash
);

#[cfg(test)]
mod tests {
    use super::*;

    // FIPS 180-2 test vector: SHA-256("abc").
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn golden_sha256() {
        assert_eq!(ChunkHash::of(b"abc").to_string(), ABC);
        assert_eq!(ManifestHash::of(b"abc").to_string(), ABC);
        assert_eq!(
            ChunkHash::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn parse_round_trip() {
        let h: ChunkHash = ABC.parse().unwrap();
        assert_eq!(h, ChunkHash::of(b"abc"));
        assert_eq!(h.to_string(), ABC);
        assert_eq!(format!("{h:?}"), format!("ChunkHash({ABC})"));
    }

    #[test]
    fn parse_is_strict() {
        let upper = ABC.to_uppercase();
        let cases: Vec<(&str, HashParseError)> = vec![
            ("", HashParseError::Length(0)),
            (&ABC[..63], HashParseError::Length(63)),
            (&upper, HashParseError::InvalidChar('B')),
        ];
        for (input, expected) in cases {
            assert_eq!(input.parse::<ChunkHash>().unwrap_err(), expected, "{input}");
        }
        let long = format!("{ABC}0");
        assert_eq!(
            long.parse::<ChunkHash>().unwrap_err(),
            HashParseError::Length(65)
        );
        let spaced = format!(" {}", &ABC[1..]);
        assert_eq!(
            spaced.parse::<ChunkHash>().unwrap_err(),
            HashParseError::InvalidChar(' ')
        );
        let prefixed = format!("0x{}", &ABC[2..]);
        assert_eq!(
            prefixed.parse::<ChunkHash>().unwrap_err(),
            HashParseError::InvalidChar('x')
        );
        // Multi-byte characters must not panic on slicing.
        let unicode = format!("é{}", &ABC[2..]);
        assert_eq!(
            unicode.parse::<ChunkHash>().unwrap_err(),
            HashParseError::InvalidChar('é')
        );
    }

    #[test]
    fn serde_as_hex_string() {
        let h = ChunkHash::of(b"abc");
        assert_eq!(serde_json::to_string(&h).unwrap(), format!("\"{ABC}\""));
        let back: ChunkHash = serde_json::from_str(&format!("\"{ABC}\"")).unwrap();
        assert_eq!(back, h);
        assert!(serde_json::from_str::<ChunkHash>(&format!("\"{}\"", ABC.to_uppercase())).is_err());
        assert!(serde_json::from_str::<ChunkHash>("42").is_err());
    }
}
