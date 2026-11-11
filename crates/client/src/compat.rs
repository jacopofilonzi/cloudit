//! Compatibility between this app and a server.
//!
//! The app and the server are released separately, so an installed app may talk to an
//! older or newer server. The server's `/health` announces the API level it implements
//! (`api_level`) and the oldest app version it accepts (`min_client_version`); servers
//! from before those fields count as level 1 with no minimum. [`decide`] compares them
//! with [`APP_VERSION`] and [`REQUIRED_API_LEVEL`]:
//!
//! - this app older than the server's minimum → [`Compat::AppOutdated`] (update the app);
//! - the server's level below what this app needs → [`Compat::ServerOutdated`] (ask the
//!   server's administrator to update it).
//!
//! `/api/v1` changes are additive only, so a newer server with the same or a higher
//! level keeps working with this app. Raise [`REQUIRED_API_LEVEL`] when the app starts
//! relying on something a newer level added.
//!
//! The engine checks at sign-in ([`crate::Engine::begin_login`] refuses an incompatible
//! server), before operations (the answer is cached: [`OK_TTL`] / [`INCOMPATIBLE_TTL`])
//! and on every reconnect of [`crate::events::follow`]. While incompatible, operations
//! fail with [`crate::ErrorKind::AppOutdated`] / [`crate::ErrorKind::ServerOutdated`].

use std::time::Duration;

use cloudit_core::protocol::HealthResponse;
use cloudit_core::version::Version;
use serde::Serialize;

use crate::error::{Error, Result};

/// The API level this app needs from the server.
pub const REQUIRED_API_LEVEL: u32 = 1;
/// This app's version (the engine and the desktop app share it).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a compatible answer is trusted before operations check again.
pub const OK_TTL: Duration = Duration::from_secs(600);
/// How long an incompatible answer is trusted (the server may be updated meanwhile).
pub const INCOMPATIBLE_TTL: Duration = Duration::from_secs(60);

/// Whether this app can work with a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Compat {
    /// They work together.
    Ok,
    /// The server needs a newer app: update it.
    AppOutdated {
        app_version: String,
        min_client_version: String,
    },
    /// The server is too old for this app: its administrator must update it.
    ServerOutdated {
        server_version: String,
        api_level: u32,
        required_api_level: u32,
    },
}

impl Compat {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// `Ok(())`, or the matching error.
    pub fn into_result(self) -> Result<()> {
        match self {
            Self::Ok => Ok(()),
            Self::AppOutdated {
                app_version,
                min_client_version,
            } => Err(Error::AppOutdated {
                app_version,
                min_client_version,
            }),
            Self::ServerOutdated {
                server_version,
                api_level,
                required_api_level,
            } => Err(Error::ServerOutdated {
                server_version,
                api_level,
                required_api_level,
            }),
        }
    }
}

/// Decides from a server's `/health` (pure). An unreadable minimum version counts as
/// none: the server is not refused over a value this app can't parse.
#[must_use]
pub fn decide(health: &HealthResponse, app_version: &str, required_api_level: u32) -> Compat {
    if let (Some(min), Some(app)) = (
        Version::parse(&health.min_client_version),
        Version::parse(app_version),
    ) && app < min
    {
        return Compat::AppOutdated {
            app_version: app_version.to_owned(),
            min_client_version: health.min_client_version.clone(),
        };
    }
    if health.api_level < required_api_level {
        return Compat::ServerOutdated {
            server_version: health.version.clone(),
            api_level: health.api_level,
            required_api_level,
        };
    }
    Compat::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(version: &str, api_level: u32, min: &str) -> HealthResponse {
        HealthResponse {
            status: "ok".to_owned(),
            version: version.to_owned(),
            api_level,
            min_client_version: min.to_owned(),
        }
    }

    #[test]
    fn compatible_when_new_enough_on_both_sides() {
        assert_eq!(decide(&health("0.1.0", 1, "0.1.0"), "0.1.0", 1), Compat::Ok);
        assert_eq!(decide(&health("0.5.0", 3, "0.1.0"), "0.2.0", 2), Compat::Ok);
        // No minimum, or one this app can't read.
        assert_eq!(decide(&health("0.1.0", 1, ""), "0.1.0", 1), Compat::Ok);
        assert_eq!(decide(&health("0.1.0", 1, "junk"), "0.1.0", 1), Compat::Ok);
    }

    #[test]
    fn an_old_app_must_update() {
        assert_eq!(
            decide(&health("1.0.0", 4, "0.10.0"), "0.9.3", 1),
            Compat::AppOutdated {
                app_version: "0.9.3".to_owned(),
                min_client_version: "0.10.0".to_owned(),
            }
        );
        // That comes first: updating the app is what the user can do.
        assert!(matches!(
            decide(&health("1.0.0", 1, "2.0.0"), "1.0.0", 2),
            Compat::AppOutdated { .. }
        ));
    }

    #[test]
    fn an_old_server_must_update() {
        assert_eq!(
            decide(&health("0.1.0", 1, "0.1.0"), "0.4.0", 2),
            Compat::ServerOutdated {
                server_version: "0.1.0".to_owned(),
                api_level: 1,
                required_api_level: 2,
            }
        );
    }

    #[test]
    fn a_server_without_the_fields_is_level_one() {
        let old: HealthResponse =
            serde_json::from_str(r#"{"status":"ok","version":"0.1.0"}"#).unwrap();
        assert_eq!(decide(&old, APP_VERSION, 1), Compat::Ok);
        assert!(matches!(
            decide(&old, APP_VERSION, 2),
            Compat::ServerOutdated { api_level: 1, .. }
        ));
    }

    #[test]
    fn into_result_maps_to_the_error_kinds() {
        assert!(Compat::Ok.into_result().is_ok());
        let err = decide(&health("1.0.0", 1, "9.0.0"), "1.0.0", 1)
            .into_result()
            .err();
        assert_eq!(err.map(|e| e.kind()), Some(crate::ErrorKind::AppOutdated));
        let err = decide(&health("1.0.0", 1, ""), "1.0.0", 2)
            .into_result()
            .err();
        assert_eq!(
            err.map(|e| e.kind()),
            Some(crate::ErrorKind::ServerOutdated)
        );
    }
}
