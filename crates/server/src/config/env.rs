//! Typed access to environment values: trimming, empty-as-unset, and parsing with
//! errors that name the variable.

use std::time::Duration;

use super::ConfigError;

/// Builds a [`ConfigError::Invalid`] for `var`.
pub(super) fn invalid(var: &'static str, value: &str, reason: impl ToString) -> ConfigError {
    ConfigError::Invalid {
        var,
        value: value.to_owned(),
        reason: reason.to_string(),
    }
}

/// Wraps a lookup function; every getter treats empty (or whitespace-only) values as unset.
pub(super) struct Env<F>(pub(super) F);

impl<F: Fn(&str) -> Option<String>> Env<F> {
    pub(super) fn get(&self, key: &str) -> Option<String> {
        (self.0)(key)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    /// Parses with `FromStr`, or returns `default` when unset.
    pub(super) fn parse_or<T>(&self, key: &'static str, default: T) -> Result<T, ConfigError>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        match self.get(key) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|e| invalid(key, &v, e)),
        }
    }

    /// `true`/`false` (also `1`/`0`, `yes`/`no`, `on`/`off`, case-insensitive).
    pub(super) fn bool_or(&self, key: &'static str, default: bool) -> Result<bool, ConfigError> {
        match self.get(key) {
            None => Ok(default),
            Some(v) => match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(true),
                "false" | "0" | "no" | "off" => Ok(false),
                _ => Err(invalid(key, &v, "expected true or false")),
            },
        }
    }

    /// A duration such as `900`, `900s`, `15m` or `1h` (see [`parse_duration`]).
    pub(super) fn duration_or(
        &self,
        key: &'static str,
        default: Duration,
    ) -> Result<Duration, ConfigError> {
        match self.get(key) {
            None => Ok(default),
            Some(v) => parse_duration(&v).map_err(|reason| invalid(key, &v, reason)),
        }
    }
}

/// Parses `<integer><unit>` with unit `s`, `m`, `h` or `d`; a bare integer means seconds.
pub(super) fn parse_duration(value: &str) -> Result<Duration, &'static str> {
    let value = value.trim();
    let (digits, multiplier) = match value.char_indices().last() {
        Some((i, 's')) => (&value[..i], 1),
        Some((i, 'm')) => (&value[..i], 60),
        Some((i, 'h')) => (&value[..i], 3600),
        Some((i, 'd')) => (&value[..i], 86_400),
        _ => (value, 1),
    };
    let n: u64 = digits
        .trim()
        .parse()
        .map_err(|_| "expected a duration like 900, 900s, 15m, 1h or 1d")?;
    n.checked_mul(multiplier)
        .map(Duration::from_secs)
        .ok_or("duration too large")
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("900"), Ok(Duration::from_secs(900)));
        assert_eq!(parse_duration("900s"), Ok(Duration::from_secs(900)));
        assert_eq!(parse_duration("15m"), Ok(Duration::from_secs(900)));
        assert_eq!(parse_duration("2h"), Ok(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Ok(Duration::from_secs(86_400)));
        assert!(parse_duration("").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("1.5h").is_err());
        assert!(parse_duration("-1s").is_err());
        assert!(parse_duration("10x").is_err());
    }
}
