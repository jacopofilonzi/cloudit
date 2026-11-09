//! Running programs (`sysinfo`): is a folder's associated process running, and the list
//! the folder settings offer to pick from. A process is matched by its executable name;
//! what that name is and how names compare differs per OS (`processes_windows.rs`,
//! `processes_unix.rs`). Polling is blocking: the scheduler calls it on the blocking
//! pool.

use std::collections::BTreeMap;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(windows)]
#[path = "processes_windows.rs"]
mod os;

#[cfg(not(windows))]
#[path = "processes_unix.rs"]
mod os;

/// The process table, refreshed on each poll (kept between polls: cheaper).
pub struct ProcessTable {
    system: System,
}

impl ProcessTable {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            system: System::new().map_err(|e| format!("process list: {e}"))?,
        })
    }

    fn refresh(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
    }

    /// For each wanted name, whether a process with it runs (same order).
    pub fn running(&mut self, wanted: &[String]) -> Vec<bool> {
        self.refresh();
        let names: Vec<os::Names> = self.system.processes().values().map(os::names).collect();
        wanted
            .iter()
            .map(|want| names.iter().any(|n| os::matches(want, n)))
            .collect()
    }

    /// The executable names of the running processes, sorted (case-insensitively) and
    /// without duplicates.
    pub fn list(&mut self) -> Vec<String> {
        self.refresh();
        let mut seen = BTreeMap::new();
        for process in self.system.processes().values() {
            if let Some(name) = os::display_name(&os::names(process)) {
                seen.entry(name.to_lowercase()).or_insert(name);
            }
        }
        seen.into_values().collect()
    }
}
