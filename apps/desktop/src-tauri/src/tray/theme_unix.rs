//! Linux (and macOS later): panels are usually dark and expose no portable setting for
//! their own brightness, so the icon is always the white one.

use super::Taskbar;

pub fn taskbar() -> Taskbar {
    Taskbar::Dark
}
