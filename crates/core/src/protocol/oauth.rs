//! Sign-in through identity providers (`/api/v1/auth/oauth/*`) and the signed-in
//! user's linked identities (`/api/v1/me/identities`).

use serde::{Deserialize, Serialize};

use super::{ProviderKind, Sensitive};

/// Why a provider sign-in is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum OAuthPurpose {
    /// Sign in (or, for an unknown identity, start onboarding).
    Login,
    /// Link the identity to the signed-in account (needs a web session).
    Link,
}

/// `POST /api/v1/auth/oauth/{provider}/start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OAuthStartRequest {
    pub purpose: OAuthPurpose,
    /// Where to go afterwards: a path starting with `/` (not `//`); default `/`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub return_to: Option<String>,
    /// An invite to use if the sign-in ends in creating an account (`login` only).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub invite_token: Option<Sensitive>,
}

/// Answer of the `start` endpoints: navigate the browser to `redirect_url` (the
/// provider's authorization page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OAuthStartResponse {
    pub redirect_url: String,
}

/// The provider of a linked identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct IdentityProviderRef {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub icon: String,
    /// The provider is disabled: this identity can't be used to sign in for now.
    pub enabled: bool,
    /// The provider was deleted by an admin: this identity can't be used any more.
    pub deleted: bool,
}

/// One identity in `GET /api/v1/me/identities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct IdentityInfo {
    pub id: String,
    pub provider: IdentityProviderRef,
    /// The email the provider reported at the last sign-in, if any.
    pub email: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
}
