//! Windows: the taskbar is light when
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme`
//! is 1 (missing or unreadable: dark, the Windows default). The value is read with
//! `reg.exe` (no console window): the app has no registry crate and no unsafe code.

use std::os::windows::process::CommandExt as _;
use std::process::Command;

use super::Taskbar;

const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const VALUE: &str = "SystemUsesLightTheme";
/// `CREATE_NO_WINDOW`: no console flashes up every minute.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn taskbar() -> Taskbar {
    let output = Command::new("reg")
        .args(["query", KEY, "/v", VALUE])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse(&String::from_utf8_lossy(&output.stdout)).unwrap_or_default()
        }
        // Missing value (older Windows) or no `reg.exe`: the default.
        Ok(_) => Taskbar::Dark,
        Err(error) => {
            tracing::debug!(%error, "could not read the taskbar theme");
            Taskbar::Dark
        }
    }
}

/// Reads `reg query` output: the line `SystemUsesLightTheme    REG_DWORD    0x1`.
fn parse(output: &str) -> Option<Taskbar> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with(VALUE))?;
    let value = line.split_whitespace().nth(2)?;
    let number = u32::from_str_radix(value.trim_start_matches("0x"), 16).ok()?;
    Some(if number == 1 {
        Taskbar::Light
    } else {
        Taskbar::Dark
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_registry_value() {
        let light = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    SystemUsesLightTheme    REG_DWORD    0x1\r\n\r\n";
        assert_eq!(parse(light), Some(Taskbar::Light));
        assert_eq!(parse(&light.replace("0x1", "0x0")), Some(Taskbar::Dark));
        assert_eq!(parse("nothing here"), None);
        assert_eq!(parse("    SystemUsesLightTheme    REG_DWORD    junk"), None);
    }
}
