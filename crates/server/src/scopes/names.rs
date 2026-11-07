//! Scope and folder names: trimmed, 1 to 100 characters, no control characters
//! (`validate::text`). The key used for uniqueness is the Unicode lowercase of the
//! trimmed name (no other normalization: NFC and NFD spellings differ, like paths).

use crate::errors::ApiError;
use crate::validate;

/// Longest scope or folder name, in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// A validated name and its uniqueness key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub name: String,
    pub key: String,
}

/// Validates `raw` as the request field `field`.
pub fn parse(raw: &str, field: &'static str) -> Result<Name, ApiError> {
    let name = validate::text(raw, field, MAX_NAME_CHARS)?;
    let key = name.to_lowercase();
    Ok(Name { name, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_keyed_by_lowercase() {
        let n = parse("  Elden Ring ", "name").unwrap();
        assert_eq!(n.name, "Elden Ring");
        assert_eq!(n.key, "elden ring");
        assert_eq!(parse("ÉCOLE", "name").unwrap().key, "école");
        assert!(parse("", "name").is_err());
        assert!(parse("   ", "name").is_err());
        assert!(parse("a\tb", "name").is_err());
        assert!(parse(&"x".repeat(101), "name").is_err());
        assert!(parse(&"x".repeat(100), "name").is_ok());
    }
}
