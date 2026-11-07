//! First-run setup, registration, login and password reset (`/api/v1/setup`,
//! `/api/v1/auth/*`).

use serde::{Deserialize, Serialize};

use super::Sensitive;

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
    /// Nobody can register; existing accounts keep working.
    Closed,
}

/// A sign-in provider shown on the login page (filled from milestone 3c on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AuthProvider {
    /// Durable provider id.
    pub id: String,
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
            serde_json::to_value(RegistrationMode::Closed).unwrap(),
            serde_json::json!("closed")
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
