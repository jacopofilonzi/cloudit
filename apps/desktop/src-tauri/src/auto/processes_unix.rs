//! Linux (and other Unix): a process is known by the file name of its executable
//! (`/proc/<pid>/exe`, readable for this user's processes), else of its first argument;
//! names compare exactly. The kernel's `comm` name is cut to 15 bytes, so it is only a
//! fallback: equal to the wanted name, or its first 15 bytes when it is that long.

use std::path::Path;

use sysinfo::Process;

/// Length the kernel truncates `comm` to.
const COMM_LEN: usize = 15;

/// The names a process is known by.
pub struct Names {
    exe: Option<String>,
    arg0: Option<String>,
    comm: String,
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
}

pub fn names(process: &Process) -> Names {
    Names {
        exe: process.exe().and_then(file_name),
        arg0: process
            .cmd()
            .first()
            .and_then(|arg| file_name(Path::new(arg))),
        comm: process.name().to_string_lossy().into_owned(),
    }
}

pub fn matches(wanted: &str, names: &Names) -> bool {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return false;
    }
    if names.exe.as_deref() == Some(wanted) || names.arg0.as_deref() == Some(wanted) {
        return true;
    }
    if names.exe.is_some() {
        // The real executable is known and differs (`comm` can be renamed at will).
        return false;
    }
    names.comm == wanted
        || (names.comm.len() == COMM_LEN && wanted.as_bytes().starts_with(names.comm.as_bytes()))
}

pub fn display_name(names: &Names) -> Option<String> {
    names
        .exe
        .clone()
        .or_else(|| names.arg0.clone())
        .or_else(|| (!names.comm.is_empty()).then(|| names.comm.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(exe: Option<&str>, arg0: Option<&str>, comm: &str) -> Names {
        Names {
            exe: exe.map(str::to_owned),
            arg0: arg0.map(str::to_owned),
            comm: comm.to_owned(),
        }
    }

    #[test]
    fn executable_names_win_over_the_truncated_comm() {
        let long = "SuperLongGameName.x86_64";
        let game = names(Some(long), Some(long), "SuperLongGameNa");
        assert!(matches(long, &game));
        assert!(!matches("superlonggamename.x86_64", &game));
        assert!(!matches("SuperLongGameNa", &game));
        // Other users' processes: no exe, the arguments may still say.
        assert!(matches(long, &names(None, Some(long), "SuperLongGameNa")));
        assert!(matches(long, &names(None, None, "SuperLongGameNa")));
        assert!(matches("game", &names(None, None, "game")));
        assert!(!matches("gamer", &names(None, None, "game")));
        assert!(!matches("", &game));
        assert_eq!(display_name(&game).as_deref(), Some(long));
        assert_eq!(display_name(&names(None, None, "x")).as_deref(), Some("x"));
    }
}
