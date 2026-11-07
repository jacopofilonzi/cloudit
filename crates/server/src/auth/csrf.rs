//! CSRF protection for cookie-based requests.
//!
//! The session cookie is `SameSite=Lax`, which already blocks most cross-site POSTs;
//! this check closes the rest (same-site subdomains, older browsers). Every unsafe
//! request (not GET/HEAD/OPTIONS) that is not bearer-authenticated must prove it comes
//! from the server's own pages:
//!
//! - `Origin` present: it must equal the server's origin
//!   ([`super::client::ClientInfo::origin`], case-insensitive);
//! - `Origin` absent: only `Sec-Fetch-Site: same-origin` is accepted;
//! - otherwise `csrf_failed` (403).
//!
//! This applies to login, register and setup too (login CSRF). Bearer requests are
//! exempt (a browser can't attach the header cross-site without CORS, which is not
//! enabled), and so is `POST /devices/token`, which the desktop client calls without
//! cookies. Non-browser clients must send a matching `Origin` header.

use axum::http::{HeaderMap, Method};

use crate::errors::ApiError;

/// `true` for methods that may change state.
#[must_use]
pub fn is_unsafe(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Checks the request headers against the expected origin (see the module docs).
pub fn check(headers: &HeaderMap, expected_origin: Option<&str>) -> Result<(), ApiError> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    match header("origin") {
        Some(origin) => match expected_origin {
            Some(expected) if origin.eq_ignore_ascii_case(expected) => Ok(()),
            _ => Err(ApiError::CsrfFailed),
        },
        None if header("sec-fetch-site") == Some("same-origin") => Ok(()),
        None => Err(ApiError::CsrfFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn origin_rules() {
        let expected = Some("http://localhost:8080");
        assert!(check(&headers(&[("origin", "http://localhost:8080")]), expected).is_ok());
        assert!(check(&headers(&[("origin", "HTTP://LOCALHOST:8080")]), expected).is_ok());
        assert!(check(&headers(&[("origin", "http://evil.example")]), expected).is_err());
        assert!(check(&headers(&[("origin", "null")]), expected).is_err());
        assert!(check(&headers(&[("origin", "http://localhost:8080")]), None).is_err());
        // No Origin: only same-origin fetch metadata.
        assert!(check(&headers(&[("sec-fetch-site", "same-origin")]), expected).is_ok());
        assert!(check(&headers(&[("sec-fetch-site", "same-site")]), expected).is_err());
        assert!(check(&headers(&[("sec-fetch-site", "cross-site")]), expected).is_err());
        assert!(check(&headers(&[]), expected).is_err());
        // A mismatching Origin is not rescued by Sec-Fetch-Site.
        assert!(
            check(
                &headers(&[
                    ("origin", "http://evil.example"),
                    ("sec-fetch-site", "same-origin")
                ]),
                expected
            )
            .is_err()
        );
    }

    #[test]
    fn safe_methods() {
        assert!(!is_unsafe(&Method::GET));
        assert!(!is_unsafe(&Method::HEAD));
        assert!(is_unsafe(&Method::POST));
        assert!(is_unsafe(&Method::PATCH));
        assert!(is_unsafe(&Method::DELETE));
    }
}
