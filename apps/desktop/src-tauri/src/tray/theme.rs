//! Which tray icon suits the taskbar: the monochrome icon is white on dark taskbars
//! and black on light ones. How the taskbar's brightness is known differs per OS
//! (`theme_windows.rs`: the `SystemUsesLightTheme` setting; `theme_unix.rs`: always
//! dark, as most Linux panels are). It is read at startup and again every
//! [`RECHECK`] (a setting change is picked up within that time).

use std::time::Duration;

#[cfg(windows)]
#[path = "theme_windows.rs"]
mod os;

#[cfg(not(windows))]
#[path = "theme_unix.rs"]
mod os;

/// How often the taskbar brightness is read again.
pub const RECHECK: Duration = Duration::from_secs(60);

/// The taskbar's brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Taskbar {
    /// Dark taskbar: white icon (the default when unknown).
    #[default]
    Dark,
    /// Light taskbar: black icon (only Windows reports it).
    #[cfg_attr(not(windows), allow(dead_code))]
    Light,
}

/// The taskbar's brightness now (blocking: run it on the blocking pool).
#[must_use]
pub fn taskbar() -> Taskbar {
    os::taskbar()
}
