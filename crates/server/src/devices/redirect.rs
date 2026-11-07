//! Loopback redirect URIs for the desktop login (RFC 8252 §7.3).
//!
//! Accepted: `http://127.0.0.1:<port>/<path>` and `http://[::1]:<port>/<path>` with an
//! explicit port, no user info, query or fragment. `localhost` is refused (it can
//! resolve elsewhere, RFC 8252 §8.3), as is every other host or scheme. The redirect
//! gets `code` and `state` as query parameters.

use url::Url;

use crate::errors::ApiError;

const FIELD: &str = "redirect_uri";
/// Longest accepted redirect URI.
const MAX_LEN: usize = 2048;

/// Parses and checks a loopback redirect URI (see the module docs).
pub fn validate(raw: &str) -> Result<Url, ApiError> {
    let invalid = |why: &str| ApiError::validation(FIELD, why);
    if raw.len() > MAX_LEN {
        return Err(invalid("too long"));
    }
    let url = Url::parse(raw).map_err(|_| invalid("not a URL"))?;
    if url.scheme() != "http" {
        return Err(invalid("must use http://"));
    }
    if !matches!(url.host_str(), Some("127.0.0.1" | "[::1]")) {
        return Err(invalid("host must be 127.0.0.1 or [::1]"));
    }
    if url.port().is_none_or(|p| p == 0) {
        return Err(invalid("an explicit port is required"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("no user info allowed"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("no query or fragment allowed"));
    }
    Ok(url)
}

/// `redirect_uri?code=…&state=…`.
#[must_use]
pub fn with_code(mut redirect_uri: Url, code: &str, state: &str) -> String {
    redirect_uri
        .query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", state);
    redirect_uri.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_loopback_ips_with_port() {
        for ok in [
            "http://127.0.0.1:53682/callback",
            "http://127.0.0.1:1/",
            "http://127.0.0.1:8080",
            "http://[::1]:53682/cb",
        ] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "http://localhost:53682/cb",
            "https://127.0.0.1:53682/cb",
            "http://127.0.0.1/cb",
            "http://127.0.0.1:80/cb",
            "http://127.0.0.2:5000/cb",
            "http://[::2]:5000/cb",
            "http://example.com:5000/cb",
            "http://user@127.0.0.1:5000/cb",
            "http://127.0.0.1:5000/cb?x=1",
            "http://127.0.0.1:5000/cb#frag",
            "myapp://callback",
            "127.0.0.1:5000",
            "",
        ] {
            assert!(validate(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn appends_code_and_state() {
        let url = validate("http://127.0.0.1:5000/cb").unwrap();
        assert_eq!(
            with_code(url, "abc", "s t&x"),
            "http://127.0.0.1:5000/cb?code=abc&state=s+t%26x"
        );
    }
}
