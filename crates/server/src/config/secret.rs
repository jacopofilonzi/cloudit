//! A string that must never end up in logs, API responses or error messages.

use std::fmt;

/// Wraps a secret value (tokens, passwords, client secrets). `Debug` and `Display`
/// print `***`; the only way to read the value is the explicit [`Secret::expose`].
/// It deliberately does not implement `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the raw value. Call it only where the secret is actually used
    /// (hashing, an outgoing request), never to log or return it.
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

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn never_prints_the_value() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(format!("{:?}", Some(&secret)), "Some(***)");
        assert_eq!(secret.expose(), "hunter2");
    }
}
