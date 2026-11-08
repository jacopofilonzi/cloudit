//! First-run setup, registration, login and password reset (`/api/v1/setup`,
//! `/api/v1/auth/*`).

use serde::{Deserialize, Serialize};

use super::{ProviderKind, Sensitive};

/// Who may create an account (`registration_mode` setting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum RegistrationMode {
    /// Anyone who can reach the server.
    Open,
    /// Only with a valid invite from an admin (default).
    #[default]
    Invite,
}

/// Whether accounts created through a sign-up path (password registration, or a sign-in
/// provider) are active immediately. Accounts created with a valid invite are always
/// active (the invite is an admin's approval).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum NewAccounts {
    /// Active immediately.
    Auto,
    /// Created pending: the account can't sign in until an admin approves it
    /// (default).
    #[default]
    Approval,
}

impl NewAccounts {
    /// The stored and serialized name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Approval => "approval",
        }
    }

    /// Parses [`NewAccounts::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Auto, Self::Approval]
            .into_iter()
            .find(|v| v.as_str() == name)
    }
}

/// Admin approval state of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ApprovalStatus {
    /// Can sign in (accounts created before approvals existed, the first admin,
    /// invited accounts, and accounts approved automatically or by an admin).
    Approved,
    /// Waiting for an admin: can't sign in yet (`account_pending`).
    Pending,
    /// Refused or banned by an admin: can't sign in by any method
    /// (`account_rejected`); kept, never deleted.
    Rejected,
}

impl ApprovalStatus {
    pub const ALL: [Self; 3] = [Self::Approved, Self::Pending, Self::Rejected];

    /// The stored and serialized name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
        }
    }

    /// Parses [`ApprovalStatus::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == name)
    }
}

/// HTTP 202 answer of `POST /api/v1/auth/register` and
/// `POST /api/v1/auth/onboarding/create` when the new account waits for an admin's
/// approval: no session is created. (An active account gets 201 with its
/// `UserProfile` and a session cookie instead.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SignupPending {
    /// Always `pending`.
    pub status: ApprovalStatus,
}

impl Default for SignupPending {
    fn default() -> Self {
        Self {
            status: ApprovalStatus::Pending,
        }
    }
}

/// A sign-in provider shown on the login page: enabled, not deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AuthProvider {
    /// Durable provider id (used in `/api/v1/auth/oauth/{id}/start`).
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    /// Preset icon name, or `generic`.
    pub icon: String,
}

/// `GET /api/v1/auth/config`: what the login page needs; no authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AuthConfig {
    pub setup_required: bool,
    pub registration_mode: RegistrationMode,
    /// A password registration without an invite creates a pending account (the
    /// `password_new_accounts` setting is `approval`).
    pub password_requires_approval: bool,
    /// In display order.
    pub providers: Vec<AuthProvider>,
}

/// `GET /api/v1/setup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SetupStatus {
    /// `true` until the first admin has been created.
    pub required: bool,
}

/// `POST /api/v1/setup`: creates the first admin with the token printed in the logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SetupRequest {
    pub token: Sensitive,
    pub email: String,
    pub password: Sensitive,
    pub display_name: String,
    /// Default `invite`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub registration_mode: Option<RegistrationMode>,
    /// The server's public URL (`http(s)://host[:port]`), stored as a setting; the UI
    /// prefills it from the browser's origin. Ignored for use while the `PUBLIC_URL`
    /// environment variable is set.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub public_url: Option<String>,
}

/// `POST /api/v1/auth/register`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct RegisterRequest {
    pub email: String,
    pub password: Sensitive,
    pub display_name: String,
    /// Required in `invite` mode; accepted (and consumed) in `open` mode too.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub invite_token: Option<Sensitive>,
}

/// `POST /api/v1/auth/login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct LoginRequest {
    pub email: String,
    pub password: Sensitive,
}

/// `POST /api/v1/auth/password-reset`: sets a new password with a reset token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct PasswordResetRequest {
    pub token: Sensitive,
    pub new_password: Sensitive,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_mode_is_snake_case_and_defaults_to_invite() {
        assert_eq!(RegistrationMode::default(), RegistrationMode::Invite);
        assert_eq!(
            serde_json::to_value(RegistrationMode::Open).unwrap(),
            serde_json::json!("open")
        );
    }

    #[test]
    fn approval_names_match_serde() {
        for status in ApprovalStatus::ALL {
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(status.as_str())
            );
            assert_eq!(ApprovalStatus::parse(status.as_str()), Some(status));
        }
        for policy in [NewAccounts::Auto, NewAccounts::Approval] {
            assert_eq!(
                serde_json::to_value(policy).unwrap(),
                serde_json::json!(policy.as_str())
            );
            assert_eq!(NewAccounts::parse(policy.as_str()), Some(policy));
        }
        assert_eq!(NewAccounts::default(), NewAccounts::Approval);
        assert_eq!(
            serde_json::to_value(SignupPending::default()).unwrap(),
            serde_json::json!({ "status": "pending" })
        );
    }

    #[test]
    fn requests_reject_unknown_fields_and_allow_missing_optionals() {
        let ok: RegisterRequest = serde_json::from_value(serde_json::json!({
            "email": "a@b.c", "password": "p", "display_name": "A"
        }))
        .unwrap();
        assert!(ok.invite_token.is_none());
        assert!(
            serde_json::from_value::<LoginRequest>(serde_json::json!({
                "email": "a@b.c", "password": "p", "admin": true
            }))
            .is_err()
        );
    }
}
