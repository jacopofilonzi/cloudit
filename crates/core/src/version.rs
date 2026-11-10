//! Release versions (`major.minor.patch`), to tell whether a release is newer than
//! the running server or app. Parsing ignores a leading `v` (Git tags: `v1.2.3`) and
//! anything after a `-` or `+` (pre-release and build suffixes, which cloudit doesn't
//! publish): `1.2.3-rc.1` compares like `1.2.3`.

use std::fmt;

/// A parsed `major.minor.patch`; ordered field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parses `1.2.3`, `v1.2.3`, `1.2.3-rc.1`; `None` for anything else.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let raw = raw
            .strip_prefix('v')
            .or_else(|| raw.strip_prefix('V'))
            .unwrap_or(raw);
        let core = raw.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let mut next = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Whether `candidate` is a strictly newer version than `current` (`false` when
/// either doesn't parse).
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_and_plain_versions() {
        let v = |major, minor, patch| Version {
            major,
            minor,
            patch,
        };
        assert_eq!(Version::parse("1.2.3"), Some(v(1, 2, 3)));
        assert_eq!(Version::parse(" v0.10.0 "), Some(v(0, 10, 0)));
        assert_eq!(Version::parse("V2.0.1"), Some(v(2, 0, 1)));
        assert_eq!(Version::parse("1.2.3-rc.1"), Some(v(1, 2, 3)));
        assert_eq!(Version::parse("1.2.3+build.5"), Some(v(1, 2, 3)));
        for bad in [
            "", "v", "1", "1.2", "1.2.3.4", "1..3", "a.b.c", "1.2.x", "+1.2.3", "1.-2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(v(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn compares_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("junk", "0.1.0"));
        assert!(!is_newer("0.2.0", "junk"));
    }
}
