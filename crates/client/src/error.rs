//! The engine's error type. Every error has a stable [`ErrorKind`] (snake_case) the UI
//! maps to its own translated text; `Display` is English, for logs only.

use std::fmt;

use cloudit_core::protocol::{ApiErrorBody, ErrorCode};
use serde::Serialize;

use crate::scan::ScanProblem;

/// Result alias of the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Stable, machine-readable error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ErrorKind {
    /// The server can't be reached (or answered 5xx after the retries): try later.
    Network,
    /// The device token is missing, expired or revoked: sign in again.
    Unauthorized,
    /// The account is disabled, waiting for approval or rejected.
    AccountBlocked,
    /// The account's role on the scope is too low.
    Forbidden,
    /// The folder or scope no longer exists on the server (or isn't visible).
    NotFound,
    /// The scope owner's storage quota is full.
    QuotaExceeded,
    /// The server's head moved during the operation.
    Conflict,
    /// Too many requests, even after waiting.
    RateLimited,
    /// Any other server error (the API code is in the error).
    Server,
    /// The server answered something this client can't use.
    Protocol,
    /// Some local files can't be synced (unreadable / locked, invalid or colliding names).
    ScanProblems,
    /// A local file changed while the engine was working on it: nothing was overwritten,
    /// the operation stopped (sync again).
    LocalChanged,
    /// A local file-system error.
    Io,
    /// The disk is full.
    DiskFull,
    /// Cancelled by the caller.
    Cancelled,
    /// No account is signed in.
    NotSignedIn,
    /// No linked folder with this id.
    NotLinked,
    /// The folder is paused (e.g. a conflict decision was postponed).
    Paused,
    /// The local path can't be linked (see [`LinkRefusal`]).
    LinkRefused,
    /// The browser sign-in failed (see [`LoginFailure`]).
    LoginFailed,
    /// Neither the OS keychain nor the fallback file could store or read the token.
    TokenStore,
    /// The local state database failed.
    State,
    /// An argument is invalid (e.g. a malformed server URL or ignore pattern).
    InvalidInput,
}

impl ErrorKind {
    /// The stable snake_case name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Unauthorized => "unauthorized",
            Self::AccountBlocked => "account_blocked",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::QuotaExceeded => "quota_exceeded",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::Server => "server",
            Self::Protocol => "protocol",
            Self::ScanProblems => "scan_problems",
            Self::LocalChanged => "local_changed",
            Self::Io => "io",
            Self::DiskFull => "disk_full",
            Self::Cancelled => "cancelled",
            Self::NotSignedIn => "not_signed_in",
            Self::NotLinked => "not_linked",
            Self::Paused => "paused",
            Self::LinkRefused => "link_refused",
            Self::LoginFailed => "login_failed",
            Self::TokenStore => "token_store",
            Self::State => "state",
            Self::InvalidInput => "invalid_input",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a local path can't be linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum LinkRefusal {
    /// The path exists but is not a directory.
    NotADirectory,
    /// The path is not valid Unicode (it can't be stored).
    NotUtf8,
    /// The path is already linked.
    AlreadyLinked,
    /// The path is inside, or contains, another linked folder.
    Nested,
    /// The path is inside, or contains, the engine's own data directory.
    DataDir,
    /// The server folder is already linked to another local path.
    FolderAlreadyLinked,
}

/// Why the browser sign-in failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum LoginFailure {
    /// The user pressed "Cancel" in the browser.
    Denied,
    /// No answer from the browser in time.
    TimedOut,
    /// The browser came back with an unusable answer.
    BadCallback,
}

/// An engine error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("server unreachable: {0}")]
    Network(String),
    #[error("the server refused the device token")]
    Unauthorized,
    #[error("the account can't sign in ({0:?})")]
    AccountBlocked(ErrorCode),
    #[error("server error {status} {:?}: {}", body.code, body.message)]
    Api {
        status: u16,
        body: Box<ApiErrorBody>,
    },
    #[error("unexpected server answer: {0}")]
    Protocol(String),
    #[error("{} local item(s) can't be synced", .0.len())]
    ScanProblems(Vec<ScanProblem>),
    #[error("{path:?} changed during the operation; nothing was overwritten")]
    LocalChanged { path: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cancelled")]
    Cancelled,
    #[error("not signed in")]
    NotSignedIn,
    #[error("no linked folder with id {0}")]
    NotLinked(i64),
    #[error("the folder is paused")]
    Paused,
    #[error("can't link {path:?}: {reason:?}")]
    LinkRefused { path: String, reason: LinkRefusal },
    #[error("sign-in failed: {0:?}")]
    Login(LoginFailure),
    #[error("token store: {0}")]
    TokenStore(String),
    #[error("local state: {0}")]
    State(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl Error {
    /// The stable kind of this error.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Network(_) => ErrorKind::Network,
            Self::Unauthorized => ErrorKind::Unauthorized,
            Self::AccountBlocked(_) => ErrorKind::AccountBlocked,
            Self::Api { body, .. } => match body.code {
                ErrorCode::Forbidden => ErrorKind::Forbidden,
                ErrorCode::NotFound => ErrorKind::NotFound,
                ErrorCode::QuotaExceeded => ErrorKind::QuotaExceeded,
                ErrorCode::Conflict => ErrorKind::Conflict,
                ErrorCode::RateLimited => ErrorKind::RateLimited,
                ErrorCode::Unavailable => ErrorKind::Network,
                _ => ErrorKind::Server,
            },
            Self::Protocol(_) => ErrorKind::Protocol,
            Self::ScanProblems(_) => ErrorKind::ScanProblems,
            Self::LocalChanged { .. } => ErrorKind::LocalChanged,
            Self::Io { source, .. } => {
                if source.kind() == std::io::ErrorKind::StorageFull {
                    ErrorKind::DiskFull
                } else {
                    ErrorKind::Io
                }
            }
            Self::Cancelled => ErrorKind::Cancelled,
            Self::NotSignedIn => ErrorKind::NotSignedIn,
            Self::NotLinked(_) => ErrorKind::NotLinked,
            Self::Paused => ErrorKind::Paused,
            Self::LinkRefused { .. } => ErrorKind::LinkRefused,
            Self::Login(_) => ErrorKind::LoginFailed,
            Self::TokenStore(_) => ErrorKind::TokenStore,
            Self::State(_) => ErrorKind::State,
            Self::InvalidInput(_) => ErrorKind::InvalidInput,
        }
    }

    /// The API error code, for server errors.
    #[must_use]
    pub fn api_code(&self) -> Option<ErrorCode> {
        match self {
            Self::Api { body, .. } => Some(body.code),
            Self::Unauthorized => Some(ErrorCode::Unauthorized),
            Self::AccountBlocked(code) => Some(*code),
            _ => None,
        }
    }

    /// The API error body, for server errors.
    #[must_use]
    pub fn api_body(&self) -> Option<&ApiErrorBody> {
        match self {
            Self::Api { body, .. } => Some(body),
            _ => None,
        }
    }

    /// An error from an API error answer.
    pub(crate) fn from_api(status: u16, body: ApiErrorBody) -> Self {
        match body.code {
            ErrorCode::Unauthorized => Self::Unauthorized,
            ErrorCode::AccountDisabled | ErrorCode::AccountPending | ErrorCode::AccountRejected => {
                Self::AccountBlocked(body.code)
            }
            _ => Self::Api {
                status,
                body: Box::new(body),
            },
        }
    }

    /// An I/O error with what was being done.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::State(error.to_string())
    }
}
