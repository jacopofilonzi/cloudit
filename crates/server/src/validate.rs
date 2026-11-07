//! Input normalization and validation shared by the routes. Errors are
//! `validation_failed` naming the request field.

use crate::errors::ApiError;

/// Longest accepted email (RFC 5321 path limit).
const MAX_EMAIL_LEN: usize = 254;
/// Longest display name, device name… in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// Trims and lowercases an email and checks its shape: one `@`, a non-empty local part
/// of at most 64 bytes, a dotted domain, no whitespace or control characters. It is
/// not a deliverability check (there is no SMTP).
pub fn email(raw: &str, field: &'static str) -> Result<String, ApiError> {
    let email = raw.trim().to_lowercase();
    let invalid = || ApiError::validation(field, "not a valid email address");
    if email.len() > MAX_EMAIL_LEN || email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    let domain_ok = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..");
    if local.is_empty() || local.len() > 64 || !domain_ok {
        return Err(invalid());
    }
    Ok(email)
}

/// A trimmed, non-empty single-line text of at most `max_chars` characters.
pub fn text(raw: &str, field: &'static str, max_chars: usize) -> Result<String, ApiError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(ApiError::validation(field, "must not be empty"));
    }
    if value.chars().count() > max_chars {
        return Err(ApiError::validation(
            field,
            format!("at most {max_chars} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ApiError::validation(field, "control characters"));
    }
    Ok(value.to_owned())
}

/// A display name (see [`text`]).
pub fn display_name(raw: &str) -> Result<String, ApiError> {
    text(raw, "display_name", MAX_NAME_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_normalized() {
        assert_eq!(
            email("  Alice@Example.COM ", "email").unwrap(),
            "alice@example.com"
        );
        for bad in [
            "",
            "alice",
            "alice@",
            "@example.com",
            "a@b@c.com",
            "alice@localhost",
            "alice@.example.com",
            "alice@example..com",
            "al ice@example.com",
            "alice@example.com.",
        ] {
            assert!(email(bad, "email").is_err(), "{bad:?}");
        }
        let long = format!("{}@example.com", "a".repeat(65));
        assert!(email(&long, "email").is_err());
    }

    #[test]
    fn texts_are_trimmed_and_bounded() {
        assert_eq!(display_name("  Ada  ").unwrap(), "Ada");
        assert!(display_name("   ").is_err());
        assert!(display_name("a\nb").is_err());
        assert!(display_name(&"é".repeat(100)).is_ok());
        assert!(display_name(&"é".repeat(101)).is_err());
    }
}
