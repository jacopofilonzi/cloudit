//! Windows: a process is known by its image name (`Game.exe`); names compare without
//! letter case, and `game` matches `Game.exe` (typed without the extension).

use sysinfo::Process;

/// The names a process is known by.
pub struct Names {
    image: String,
}

pub fn names(process: &Process) -> Names {
    Names {
        image: process.name().to_string_lossy().into_owned(),
    }
}

fn key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(stem) => stem.to_owned(),
        None => lower,
    }
}

pub fn matches(wanted: &str, names: &Names) -> bool {
    !names.image.is_empty() && key(wanted) == key(&names.image)
}

pub fn display_name(names: &Names) -> Option<String> {
    (!names.image.is_empty()).then(|| names.image.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_names_match_without_case_or_extension() {
        let game = Names {
            image: "EldenRing.exe".to_owned(),
        };
        assert!(matches("eldenring.exe", &game));
        assert!(matches("ELDENRING", &game));
        assert!(matches(" EldenRing.exe ", &game));
        assert!(!matches("eldenring2.exe", &game));
        assert!(!matches(
            "",
            &Names {
                image: String::new()
            }
        ));
        assert_eq!(display_name(&game).as_deref(), Some("EldenRing.exe"));
    }
}
