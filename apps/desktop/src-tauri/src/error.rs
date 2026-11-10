//! The error every command returns to the UI: the engine's stable [`ErrorKind`] (the UI
//! translates it), an English message for logs, and the structured details some kinds
//! carry (which path was refused, which files can't be synced…).

use cloudit_client::scan::ScanProblem;
use cloudit_client::{Error, ErrorKind, LinkRefusal, LoginFailure};
use serde::Serialize;

/// Structured details of an error.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ErrorDetails {
    /// `link_refused`.
    LinkRefused { path: String, reason: LinkRefusal },
    /// `login_failed`.
    LoginFailed { reason: LoginFailure },
    /// `scan_problems`: the items that can't be synced.
    ScanProblems { problems: Vec<ScanProblem> },
    /// `local_changed`: the file that changed under the engine's hands.
    LocalChanged { path: String },
    /// A server error: its API code (`name_taken`, …) and, for validation, the field.
    Api { code: String, field: Option<String> },
}

/// A failed command.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CommandError {
    pub kind: ErrorKind,
    /// English, for logs: the UI shows its own text for `kind`.
    pub message: String,
    pub details: Option<ErrorDetails>,
}

impl CommandError {
    /// An error of the app itself (not the engine), e.g. a failed background task.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::State,
            message: message.into(),
            details: None,
        }
    }

    /// A value the UI sent that the app refuses (e.g. a pause too long).
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::InvalidInput,
            message: message.into(),
            details: None,
        }
    }
}

impl From<Error> for CommandError {
    fn from(error: Error) -> Self {
        let details = match &error {
            Error::LinkRefused { path, reason } => Some(ErrorDetails::LinkRefused {
                path: path.clone(),
                reason: *reason,
            }),
            Error::Login(reason) => Some(ErrorDetails::LoginFailed { reason: *reason }),
            Error::ScanProblems(problems) => Some(ErrorDetails::ScanProblems {
                problems: problems.clone(),
            }),
            Error::LocalChanged { path } => Some(ErrorDetails::LocalChanged { path: path.clone() }),
            Error::Api { body, .. } => Some(ErrorDetails::Api {
                code: serde_json::to_value(body.code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                field: body.field.clone(),
            }),
            _ => None,
        };
        Self {
            kind: error.kind(),
            message: error.to_string(),
            details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_errors_keep_their_kind_and_details() {
        let refused = CommandError::from(Error::LinkRefused {
            path: "/x".to_owned(),
            reason: LinkRefusal::Nested,
        });
        assert_eq!(refused.kind, ErrorKind::LinkRefused);
        assert!(matches!(
            refused.details,
            Some(ErrorDetails::LinkRefused {
                reason: LinkRefusal::Nested,
                ..
            })
        ));
        let plain = CommandError::from(Error::Unauthorized);
        assert_eq!(plain.kind, ErrorKind::Unauthorized);
        assert!(plain.details.is_none());
        assert_eq!(CommandError::internal("x").kind, ErrorKind::State);
    }
}
