//! First sign-in of an identity no account owns yet (`/api/v1/auth/onboarding*`).
//! Every endpoint needs the `cloudit_onboarding` cookie set by the provider callback.

use serde::{Deserialize, Serialize};

use super::{ProviderKind, RegistrationMode, Sensitive};

/// The provider the pending identity comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OnboardingProvider {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub icon: String,
}

/// `GET /api/v1/auth/onboarding`: what the onboarding page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OnboardingInfo {
    pub provider: OnboardingProvider,
    /// The provider's email, if it sent one (prefill).
    pub email: Option<String>,
    /// `true` when the provider sent an email; otherwise creating an account requires
    /// one.
    pub email_from_provider: bool,
    /// The provider vouches for `email`: keeping it makes the account's email verified.
    pub email_verified: bool,
    /// Suggested display name.
    pub display_name: String,
    pub registration_mode: RegistrationMode,
    /// Creating an account needs an invite (mode `invite` and no usable invite was
    /// carried from the sign-in start).
    pub invite_required: bool,
    /// Creating an account now would leave it pending until an admin approves it: the
    /// provider requires approval, no usable invite is carried from the sign-in start
    /// and the identity is not in the provider's admin groups. (An invite typed on
    /// the page skips approval too.)
    pub requires_approval: bool,
}

/// `POST /api/v1/auth/onboarding/create`: a new account for the pending identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OnboardingCreateRequest {
    pub display_name: String,
    /// Default: the provider's email (then required if it sent none). Keeping the
    /// provider's verified email makes it verified; any other email is unverified.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub email: Option<String>,
    /// Optional: without one the account signs in only through providers (a reset
    /// link can add one later).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub password: Option<Sensitive>,
    /// Overrides the invite carried from the sign-in start, if any.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub invite_token: Option<Sensitive>,
}

/// `POST /api/v1/auth/onboarding/link`: attach the pending identity to an existing
/// account proven with its email and password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct OnboardingLinkRequest {
    pub email: String,
    pub password: Sensitive,
}
