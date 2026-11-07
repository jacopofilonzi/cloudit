//! Timestamp helpers. Every time comes from the application clock in UTC (see the
//! table conventions in `db`).

use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, SecondsFormat, Utc};

/// API form of a timestamp: RFC 3339, UTC, whole seconds (`2026-01-31T12:00:00Z`).
#[must_use]
pub fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// [`rfc3339`] for an optional timestamp.
#[must_use]
pub fn rfc3339_opt(t: Option<DateTime<Utc>>) -> Option<String> {
    t.map(rfc3339)
}

/// `t + d`, failing instead of overflowing.
pub fn add(t: DateTime<Utc>, d: Duration) -> anyhow::Result<DateTime<Utc>> {
    let delta = chrono::TimeDelta::from_std(d).context("duration too large")?;
    t.checked_add_signed(delta).context("timestamp overflows")
}

/// The current time, truncated to whole microseconds: what both databases store, so a
/// value written and read back compares equal.
#[must_use]
pub fn now() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp_micros(now.timestamp_micros()).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_seconds() {
        let t = DateTime::parse_from_rfc3339("2026-01-31T12:00:00.123+02:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(rfc3339(t), "2026-01-31T10:00:00Z");
        assert_eq!(
            rfc3339(add(t, Duration::from_secs(3600)).unwrap()),
            "2026-01-31T11:00:00Z"
        );
    }
}
