//! [`Secret`]: a token held in memory. `Debug` prints `***`; there is no `Display` or
//! `Serialize`, so it can't end up in a log line or a UI payload by accident.

use std::fmt;

/// A secret string (device token, PKCE verifier, authorization code).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that must send it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn debug_is_redacted() {
        let secret = Secret::new("cld_supersecret");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(secret.expose(), "cld_supersecret");
    }
}
