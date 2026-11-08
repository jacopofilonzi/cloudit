//! Background maintenance jobs (`/api/v1/admin/jobs*`, admins only).
//!
//! Every job runs on one server instance at a time (a database lease), at its
//! interval or when an admin asks for an immediate run. Each run is recorded.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A maintenance job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum JobName {
    /// Deletes old snapshots beyond each folder's retention.
    Retention,
    /// Deletes chunks and manifests no snapshot references any more.
    Gc,
    /// Deletes stored objects without a database row (crashes, uploads never
    /// committed).
    OrphanSweep,
    /// Deletes trashed scopes and folders for good.
    TrashPurge,
    /// Deletes expired sessions, codes, sign-in flows, resets, invites and old job runs.
    Cleanup,
}

impl JobName {
    pub const ALL: [Self; 5] = [
        Self::Retention,
        Self::Gc,
        Self::OrphanSweep,
        Self::TrashPurge,
        Self::Cleanup,
    ];

    /// The stored and API name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retention => "retention",
            Self::Gc => "gc",
            Self::OrphanSweep => "orphan_sweep",
            Self::TrashPurge => "trash_purge",
            Self::Cleanup => "cleanup",
        }
    }

    /// Parses a stored or API name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|j| j.as_str() == raw)
    }
}

/// How a job run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum JobOutcome {
    Running,
    Succeeded,
    Failed,
    /// The instance stopped or lost its lease before the end; the next run continues.
    Interrupted,
}

impl JobOutcome {
    /// The stored name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    /// Parses a stored name (unknown values read as `failed`).
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "interrupted" => Self::Interrupted,
            _ => Self::Failed,
        }
    }
}

/// One run of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct JobRunInfo {
    pub id: String,
    pub job: JobName,
    /// The server instance that ran it.
    pub instance_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: JobOutcome,
    /// What the run did, e.g. `{"snapshots_deleted": 3}` (keys depend on the job).
    pub counts: BTreeMap<String, u64>,
    /// English summary of the failure, for admins (`failed` / `interrupted`).
    pub error: Option<String>,
}

/// One job in `GET /api/v1/admin/jobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct JobInfo {
    pub name: JobName,
    /// Time between two scheduled runs.
    pub interval_secs: u64,
    /// When an immediate run was requested and not started yet.
    pub requested_at: Option<String>,
    pub last_started_at: Option<String>,
    /// The latest runs, newest first (at most 10).
    pub recent_runs: Vec<JobRunInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for job in JobName::ALL {
            assert_eq!(JobName::parse(job.as_str()), Some(job));
            assert_eq!(serde_json::to_value(job).unwrap(), job.as_str());
        }
        assert_eq!(JobName::parse("nope"), None);
        for outcome in [
            JobOutcome::Running,
            JobOutcome::Succeeded,
            JobOutcome::Failed,
            JobOutcome::Interrupted,
        ] {
            assert_eq!(JobOutcome::parse(outcome.as_str()), outcome);
        }
    }
}
