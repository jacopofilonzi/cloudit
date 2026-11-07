//! Sign-in through identity providers: flow state, identities, pending signups and the
//! outcome of a callback. The provider protocol itself lives in `idp`.
//!
//! - [`flows`]: `oauth_flows`, one row per started authorization (state, PKCE
//!   verifier, nonce, purpose, return path), single use, 10 minutes.
//! - [`cookies`]: `cloudit_oauth` (browser binding) and `cloudit_onboarding`.
//! - [`identities`]: `identities`, account ↔ provider subject.
//! - [`pending`]: `pending_signups`, unknown identities waiting for onboarding.
//! - [`complete`]: what a successful callback does (sign in, link, onboarding).
//!
//! Every step stores its state in the database, so start, callback and onboarding may
//! hit different instances; every single-use row is consumed with one conditional
//! UPDATE.

pub mod complete;
pub mod cookies;
pub mod flows;
pub mod identities;
pub mod pending;

use crate::errors::ApiError;

/// Longest accepted `return_to`.
const MAX_RETURN_TO: usize = 1024;

/// Validates `return_to`: a path on this site (starts with `/`, not `//` or `/\`, no
/// backslash or control characters, at most 1024 bytes); default `/`. Anything else
/// could send the browser to another site after sign-in (open redirect).
pub fn return_to(raw: Option<&str>) -> Result<String, ApiError> {
    let Some(path) = raw else {
        return Ok("/".to_owned());
    };
    let ok = path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && path.len() <= MAX_RETURN_TO
        && !path.chars().any(char::is_control);
    if ok {
        Ok(path.to_owned())
    } else {
        Err(ApiError::validation("return_to", "a path starting with /"))
    }
}

#[cfg(test)]
mod tests {
    use super::return_to;

    #[test]
    fn return_paths() {
        assert_eq!(return_to(None).unwrap(), "/");
        assert_eq!(
            return_to(Some("/settings?tab=1#x")).unwrap(),
            "/settings?tab=1#x"
        );
        for bad in [
            "",
            "settings",
            "//evil.example",
            "/\\evil.example",
            "https://evil.example",
            "/a\nb",
            "/a\\b",
        ] {
            assert!(return_to(Some(bad)).is_err(), "{bad:?}");
        }
        assert!(return_to(Some(&format!("/{}", "a".repeat(1024)))).is_err());
    }
}
