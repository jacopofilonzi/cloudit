//! What the server knows about the client of a request: its IP (for rate limits),
//! whether it came over https (cookie `Secure`) and the server's own origin as the
//! browser sees it (CSRF check).
//!
//! Proxy headers are trusted only with `TRUST_PROXY_HEADERS=true`:
//!
//! - **Client IP**: from `X-Forwarded-For`, the rightmost entry that is not a
//!   loopback, private (RFC 1918 / unique-local) or link-local address, i.e. the first
//!   hop added by something outside our network; if every entry is internal, the
//!   rightmost one. Entries left of that can be forged by the client and are ignored.
//!   Without the header (or when not trusted) the TCP peer address.
//! - **https**: `PUBLIC_URL` starts with `https://`, or (trusted) the first
//!   `X-Forwarded-Proto` value is `https`.
//! - **Origin**: `PUBLIC_URL`'s origin when set; otherwise `http(s)://` + the first
//!   `X-Forwarded-Host` value (trusted) or the `Host` header.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, header};

use crate::config::Config;

/// Request context computed by the auth middleware, available as an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// Client IP as text (`unknown` when there is no peer address, e.g. in tests).
    pub ip: String,
    /// Reached over https (directly or through a trusted proxy).
    pub https: bool,
    /// The server's origin (`scheme://host[:port]`, lowercase), if known.
    pub origin: Option<String>,
    /// `User-Agent`, if any.
    pub user_agent: Option<String>,
}

fn first_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal(IpAddr::V4(v4));
            }
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
    }
}

/// The client IP from `X-Forwarded-For` (see the module docs).
fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let hops: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|hop| hop.trim().parse().ok())
        .collect();
    hops.iter()
        .rev()
        .find(|ip| !is_internal(**ip))
        .or_else(|| hops.last())
        .copied()
}

impl ClientInfo {
    #[must_use]
    pub fn from_request<B>(config: &Config, request: &Request<B>) -> Self {
        let headers = request.headers();
        let trust = config.trust_proxy_headers;
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        let ip = trust
            .then(|| forwarded_ip(headers))
            .flatten()
            .or(peer)
            .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());

        let public = config
            .public_url
            .as_deref()
            .and_then(|u| url::Url::parse(u).ok());
        let https = public.as_ref().is_some_and(|u| u.scheme() == "https")
            || (trust
                && first_value(headers, "x-forwarded-proto")
                    .is_some_and(|p| p.eq_ignore_ascii_case("https")));

        let origin = match &public {
            Some(url) => Some(url.origin().ascii_serialization()),
            None => {
                let host = trust
                    .then(|| first_value(headers, "x-forwarded-host"))
                    .flatten()
                    .or_else(|| first_value(headers, header::HOST.as_str()))
                    .map(str::to_owned)
                    .or_else(|| request.uri().authority().map(ToString::to_string));
                host.map(|h| format!("{}://{h}", if https { "https" } else { "http" }))
            }
        }
        .map(|o| o.to_ascii_lowercase());

        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Self {
            ip,
            https,
            origin,
            user_agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Config {
        Config::from_lookup(|k| {
            vars.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_owned())
        })
        .unwrap()
    }

    fn request(headers: &[(&str, &str)], peer: Option<&str>) -> Request<()> {
        let mut builder = Request::get("/api/v1/me");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let mut req = builder.body(()).unwrap();
        if let Some(peer) = peer {
            req.extensions_mut()
                .insert(ConnectInfo::<SocketAddr>(peer.parse().unwrap()));
        }
        req
    }

    #[test]
    fn direct_request_uses_peer_and_host() {
        let req = request(
            &[
                ("host", "Sync.Example.com:8080"),
                ("x-forwarded-for", "1.2.3.4"),
            ],
            Some("10.0.0.9:5000"),
        );
        let info = ClientInfo::from_request(&config(&[]), &req);
        assert_eq!(info.ip, "10.0.0.9", "proxy headers not trusted");
        assert!(!info.https);
        assert_eq!(info.origin.as_deref(), Some("http://sync.example.com:8080"));
    }

    #[test]
    fn trusted_proxy_headers() {
        let cfg = config(&[("TRUST_PROXY_HEADERS", "true")]);
        let req = request(
            &[
                ("host", "internal:8080"),
                ("x-forwarded-for", "6.6.6.6, 1.2.3.4, 10.0.0.2"),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-host", "sync.example.com"),
            ],
            Some("10.0.0.3:5000"),
        );
        let info = ClientInfo::from_request(&cfg, &req);
        // Rightmost non-internal hop: 6.6.6.6 may be forged by the client.
        assert_eq!(info.ip, "1.2.3.4");
        assert!(info.https);
        assert_eq!(info.origin.as_deref(), Some("https://sync.example.com"));

        // All hops internal (client on the LAN): the rightmost one.
        let req = request(&[("x-forwarded-for", "192.168.1.5, 10.0.0.2")], None);
        assert_eq!(ClientInfo::from_request(&cfg, &req).ip, "10.0.0.2");
        // No header: the peer.
        let req = request(&[], Some("[::1]:1"));
        assert_eq!(ClientInfo::from_request(&cfg, &req).ip, "::1");
        let req = request(&[], None);
        assert_eq!(ClientInfo::from_request(&cfg, &req).ip, "unknown");
    }

    #[test]
    fn public_url_wins_for_origin_and_https() {
        let cfg = config(&[("PUBLIC_URL", "https://Sync.Example.com/cloudit/")]);
        let req = request(&[("host", "localhost:8080")], None);
        let info = ClientInfo::from_request(&cfg, &req);
        assert!(info.https);
        assert_eq!(info.origin.as_deref(), Some("https://sync.example.com"));
    }
}
