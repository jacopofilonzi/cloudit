//! Device sign-in ([`login`]: loopback + PKCE through the browser) and token storage
//! ([`tokens`]: OS keychain with a private-file fallback).

pub mod login;
pub mod tokens;
