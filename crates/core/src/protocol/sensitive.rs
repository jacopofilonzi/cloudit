//! [`Sensitive`]: a secret string carried by the API (password, token).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A password or token in a request or response body. Serialized as a plain JSON
/// string (serde newtype), but `Debug` prints `***`, so logging a request struct never
/// leaks it.
/// Read it only where it is used ([`Sensitive::expose`]).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct Sensitive(String);

impl Sensitive {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value: hash or verify it, never log it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Sensitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::Sensitive;

    #[test]
    fn hides_the_value_but_serializes_it() {
        let s = Sensitive::new("hunter2");
        assert_eq!(format!("{s:?}"), "***");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"hunter2\"");
        let back: Sensitive = serde_json::from_str("\"x\"").unwrap();
        assert_eq!(back.expose(), "x");
    }
}
