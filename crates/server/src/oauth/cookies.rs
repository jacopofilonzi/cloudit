//! The two short-lived cookies of provider sign-in, both `HttpOnly; SameSite=Lax;
//! Path=/api/v1/auth` (+ `Secure` over https):
//!
//! - [`BINDING`] (`cloudit_oauth`, 10 min): a random value whose SHA-256 is stored with
//!   each started flow; the callback only completes in the browser that started it
//!   (login CSRF: an attacker can't make a victim's browser finish the attacker's
//!   sign-in). An existing well-formed value is reused, so several tabs can sign in at
//!   once.
//! - [`ONBOARDING`] (`cloudit_onboarding`, 15 min): the token of a pending signup.

use std::time::Duration;

use anyhow::Context as _;
use axum::http::{HeaderMap, HeaderValue, header};

use crate::config::Secret;

pub const BINDING: &str = "cloudit_oauth";
pub const ONBOARDING: &str = "cloudit_onboarding";
const PATH: &str = "/api/v1/auth";
/// Length of a `tokens::new_token` value.
const TOKEN_LEN: usize = 43;

/// `Set-Cookie` for `name=value`.
pub fn set(
    name: &str,
    value: &Secret,
    max_age: Duration,
    secure: bool,
) -> anyhow::Result<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}={}; Path={PATH}; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        value.expose(),
        max_age.as_secs()
    ))
    .context("building a cookie")
}

/// `Set-Cookie` deleting `name`.
pub fn clear(name: &str, secure: bool) -> anyhow::Result<HeaderValue> {
    set(name, &Secret::new(""), Duration::ZERO, secure)
}

/// The value of cookie `name`, if present and shaped like a token.
#[must_use]
pub fn read(headers: &HeaderMap, name: &str) -> Option<Secret> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, value)| {
            *n == name
                && value.len() == TOKEN_LEN
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(|(_, value)| Secret::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_and_reading() {
        let token = "a".repeat(43);
        let c = set(
            BINDING,
            &Secret::new(token.clone()),
            Duration::from_secs(600),
            true,
        )
        .unwrap();
        assert_eq!(
            c,
            format!("cloudit_oauth={token}; Path=/api/v1/auth; HttpOnly; SameSite=Lax; Max-Age=600; Secure").as_str()
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "x=1; cloudit_oauth={token}; cloudit_onboarding=short"
            ))
            .unwrap(),
        );
        assert_eq!(read(&headers, BINDING).unwrap().expose(), token);
        assert!(
            read(&headers, ONBOARDING).is_none(),
            "malformed value ignored"
        );
        assert_eq!(
            clear(ONBOARDING, false).unwrap(),
            "cloudit_onboarding=; Path=/api/v1/auth; HttpOnly; SameSite=Lax; Max-Age=0"
        );
    }
}
