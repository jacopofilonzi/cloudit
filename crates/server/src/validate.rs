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

/// A server base URL: `http(s)://host[:port]`, trailing `/` removed; no user info,
/// path, query or fragment. Returns the normalized form (lowercase scheme and host).
pub fn public_url(raw: &str, field: &'static str) -> Result<String, ApiError> {
    let invalid = |why: &str| ApiError::validation(field, why.to_owned());
    let url = url::Url::parse(raw.trim()).map_err(|_| invalid("not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("must start with http:// or https://"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid("missing host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("must not contain credentials"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("must not have a path, query or fragment"));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// Largest retention limit (snapshots kept per folder, or per day).
pub const MAX_RETENTION: u32 = 100_000;

/// A number in `min..=max`.
pub fn range(value: u32, field: &'static str, min: u32, max: u32) -> Result<u32, ApiError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::validation(field, format!("{min} to {max}")))
    }
}

/// A retention limit: 0 (unlimited) to [`MAX_RETENTION`], as stored (`i32`).
pub fn retention(value: u32, field: &'static str) -> Result<i32, ApiError> {
    let value = range(value, field, 0, MAX_RETENTION)?;
    i32::try_from(value).map_err(|_| ApiError::validation(field, "too large"))
}

/// Longest folder emoji in UTF-8 bytes: room for a ZWJ sequence (family, flags with
/// tags) but not for text.
pub const MAX_EMOJI_BYTES: usize = 32;

/// A folder emoji: trimmed, `""` = none. At most [`MAX_EMOJI_BYTES`], no whitespace or
/// control characters, no letters (Unicode `Alphabetic`) and at least one non-ASCII
/// character: this keeps text out without a grapheme database. Digits, `#` and `*`
/// stay allowed for keycap sequences (`1️⃣`), and any short emoji sequence fits.
pub fn emoji(raw: &str, field: &'static str) -> Result<Option<String>, ApiError> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let invalid = || ApiError::validation(field, "not an emoji");
    if value.len() > MAX_EMOJI_BYTES
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c.is_alphabetic())
        || value.is_ascii()
    {
        return Err(invalid());
    }
    Ok(Some(value.to_owned()))
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
    fn public_urls() {
        let ok = |raw: &str| public_url(raw, "public_url").unwrap();
        assert_eq!(ok("https://Sync.Example.com/"), "https://sync.example.com");
        assert_eq!(ok(" http://10.0.0.5:8080 "), "http://10.0.0.5:8080");
        assert_eq!(
            ok("https://sync.example.com:443"),
            "https://sync.example.com"
        );
        for bad in [
            "",
            "sync.example.com",
            "ftp://sync.example.com",
            "https://sync.example.com/cloudit",
            "https://sync.example.com/?a=1",
            "https://sync.example.com/#x",
            "https://user:pw@sync.example.com",
            "https://",
        ] {
            assert!(public_url(bad, "public_url").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn emojis() {
        let ok = |raw: &str| emoji(raw, "emoji").unwrap();
        assert_eq!(ok(" 🌍 "), Some("🌍".to_owned()));
        assert_eq!(ok("🏝️"), Some("🏝️".to_owned()));
        assert_eq!(ok("1️⃣"), Some("1️⃣".to_owned()));
        assert_eq!(ok("👨‍👩‍👧‍👦"), Some("👨‍👩‍👧‍👦".to_owned()));
        assert_eq!(ok("🇮🇹"), Some("🇮🇹".to_owned()));
        assert_eq!(ok(""), None);
        assert_eq!(ok("   "), None);
        for bad in ["a", "ab", "12", "🌍 🌍", "🌍\u{7}🌍", "é", &"🌍".repeat(9)] {
            assert!(emoji(bad, "emoji").is_err(), "{bad:?}");
        }
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
