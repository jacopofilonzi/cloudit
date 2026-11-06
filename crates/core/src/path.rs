//! Validation of the relative paths stored in manifests.
//!
//! Security-critical: a manifest received from the server (or uploaded by a client) must never
//! make anyone write outside the synced folder, on any supported OS. A path is accepted only if
//! it is portable to Windows, Linux and macOS as-is, so the same manifest restores everywhere.
//!
//! Rules for a valid path (durable: the server rejects manifests that break them):
//!
//! - non-empty, at most [`MAX_PATH_BYTES`] UTF-8 bytes;
//! - relative: no leading `/` or `\`, no drive prefix (`C:`, `C:\x`, `C:x`), no UNC prefix;
//! - segments separated by `/` only; a backslash anywhere is rejected;
//! - no empty segment (`a//b`, leading or trailing `/`), no `.` or `..` segment;
//! - each segment at most [`MAX_SEGMENT_BYTES`] UTF-8 bytes;
//! - no control characters (Unicode `Cc`: U+0000–U+001F, U+007F–U+009F, NUL included);
//! - none of `<>:"|?*` (invalid in Windows file names);
//! - no segment ending in a space or a dot (Windows silently strips them, so `a.` would
//!   alias `a`);
//! - no Windows reserved device name in any segment, case-insensitively, alone or followed
//!   by an extension or by trailing spaces before the extension (`con`, `CON.txt`,
//!   `Con .tar.gz`): `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`–`COM9`,
//!   `LPT0`–`LPT9` and the superscript variants `COM¹²³`/`LPT¹²³`.
//!
//! Unicode: paths are stored byte-for-byte as the scanner read them; no normalization (NFC/NFD)
//! is applied, because changing the bytes would make a restored name differ from the
//! original. Names that differ only by letter case are rejected at manifest level (see
//! [`crate::manifest`]) since they collide on case-insensitive file systems.

use crate::error::PathError;

/// Maximum length of a whole path, in UTF-8 bytes.
pub const MAX_PATH_BYTES: usize = 4096;
/// Maximum length of one path segment, in UTF-8 bytes (common file system name limit).
pub const MAX_SEGMENT_BYTES: usize = 255;

const WINDOWS_INVALID: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Checks that `path` is a valid manifest path (see the module docs for the rules).
///
/// # Errors
///
/// The first rule the path breaks, as a [`PathError`].
pub fn validate_path(path: &str) -> Result<(), PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(PathError::TooLong(path.len()));
    }
    let bytes = path.as_bytes();
    let drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.starts_with('/') || path.starts_with('\\') || drive_prefix {
        return Err(PathError::Absolute);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    path.split('/').try_for_each(validate_segment)
}

fn validate_segment(segment: &str) -> Result<(), PathError> {
    if segment.is_empty() {
        return Err(PathError::EmptySegment);
    }
    if segment == "." || segment == ".." {
        return Err(PathError::DotSegment);
    }
    if segment.len() > MAX_SEGMENT_BYTES {
        return Err(PathError::SegmentTooLong(segment.len()));
    }
    for c in segment.chars() {
        if c.is_control() {
            return Err(PathError::ControlChar);
        }
        if WINDOWS_INVALID.contains(&c) {
            return Err(PathError::InvalidChar(c));
        }
    }
    if segment.ends_with(' ') || segment.ends_with('.') {
        return Err(PathError::TrailingSpaceOrDot);
    }
    if is_reserved_name(segment) {
        return Err(PathError::ReservedName(segment.to_owned()));
    }
    Ok(())
}

/// Whether Windows treats `segment` as a device name: the part before the first dot, with
/// trailing spaces removed, matches a reserved name case-insensitively.
fn is_reserved_name(segment: &str) -> bool {
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let Some(rest) = upper
                .strip_prefix("COM")
                .or_else(|| upper.strip_prefix("LPT"))
            else {
                return false;
            };
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_paths() {
        let long_segment = "a".repeat(MAX_SEGMENT_BYTES);
        let max_path = ["b"; MAX_PATH_BYTES / 2].join("/") + "x";
        assert_eq!(max_path.len(), MAX_PATH_BYTES);
        let cases = [
            "a",
            "a/b/c.txt",
            ".hidden",
            "..a",
            "a..b",
            "dir/.config/file",
            " leading space",
            "Spaced Name/file name.txt",
            "unicode/àèìòù/日本語.sav",
            "emoji/🎮.dat",
            "console",
            "CONX",
            "COM10",
            "LPT",
            "nul_file",
            "auxiliary.txt",
            "COM",
            "1/2/3",
            long_segment.as_str(),
            max_path.as_str(),
        ];
        for path in cases {
            assert_eq!(validate_path(path), Ok(()), "{path:?} should be valid");
        }
    }

    #[test]
    fn invalid_paths() {
        let too_long_segment = "a".repeat(MAX_SEGMENT_BYTES + 1);
        let too_long_path = ["b"; MAX_PATH_BYTES / 2].join("/") + "xy";
        let cases: Vec<(&str, PathError)> = vec![
            ("", PathError::Empty),
            (
                too_long_path.as_str(),
                PathError::TooLong(MAX_PATH_BYTES + 1),
            ),
            (
                too_long_segment.as_str(),
                PathError::SegmentTooLong(MAX_SEGMENT_BYTES + 1),
            ),
            ("/etc/passwd", PathError::Absolute),
            ("/", PathError::Absolute),
            ("\\x", PathError::Absolute),
            ("\\\\server\\share", PathError::Absolute),
            ("C:\\x", PathError::Absolute),
            ("C:x", PathError::Absolute),
            ("c:/x", PathError::Absolute),
            ("C:", PathError::Absolute),
            ("a\\b", PathError::Backslash),
            ("a\\..\\..\\b", PathError::Backslash),
            ("a//b", PathError::EmptySegment),
            ("a/", PathError::EmptySegment),
            (".", PathError::DotSegment),
            ("..", PathError::DotSegment),
            ("../x", PathError::DotSegment),
            ("a/../../x", PathError::DotSegment),
            ("a/./b", PathError::DotSegment),
            ("a/b/..", PathError::DotSegment),
            ("a\0b", PathError::ControlChar),
            ("a\nb", PathError::ControlChar),
            ("a\tb", PathError::ControlChar),
            ("a\u{7f}", PathError::ControlChar),
            ("a\u{85}b", PathError::ControlChar),
            ("a<b", PathError::InvalidChar('<')),
            ("a>b", PathError::InvalidChar('>')),
            ("x/C:", PathError::InvalidChar(':')),
            ("a/b:stream", PathError::InvalidChar(':')),
            ("a\"b", PathError::InvalidChar('"')),
            ("a|b", PathError::InvalidChar('|')),
            ("a?b", PathError::InvalidChar('?')),
            ("a*b", PathError::InvalidChar('*')),
            ("name ", PathError::TrailingSpaceOrDot),
            ("dir./x", PathError::TrailingSpaceOrDot),
            ("a/file.", PathError::TrailingSpaceOrDot),
            ("...", PathError::TrailingSpaceOrDot),
            ("CON", PathError::ReservedName("CON".to_owned())),
            ("con", PathError::ReservedName("con".to_owned())),
            ("dir/Nul.txt", PathError::ReservedName("Nul.txt".to_owned())),
            (
                "aux.tar.gz",
                PathError::ReservedName("aux.tar.gz".to_owned()),
            ),
            ("prn .txt", PathError::ReservedName("prn .txt".to_owned())),
            ("COM1", PathError::ReservedName("COM1".to_owned())),
            ("com0.log", PathError::ReservedName("com0.log".to_owned())),
            ("lpt9", PathError::ReservedName("lpt9".to_owned())),
            ("LPT¹", PathError::ReservedName("LPT¹".to_owned())),
            ("com³.x", PathError::ReservedName("com³.x".to_owned())),
            ("conin$", PathError::ReservedName("conin$".to_owned())),
            (
                "CONOUT$.txt",
                PathError::ReservedName("CONOUT$.txt".to_owned()),
            ),
            ("ok/CON/inner", PathError::ReservedName("CON".to_owned())),
        ];
        for (path, expected) in cases {
            assert_eq!(validate_path(path), Err(expected), "{path:?}");
        }
    }
}
