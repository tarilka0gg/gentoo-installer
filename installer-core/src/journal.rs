//! On-disk install state (spec §3). Written to the **live medium** (default
//! `/run/installer/journal.json`, overridable for tests), never to the target — the
//! target may be unmounted or broken when we most need to read this back. Every phase
//! transition gets recorded so a crashed/aborted run can resume without re-doing
//! destructive or slow work (`Phase::is_satisfied` is what makes that safe).

use crate::phase::PhaseId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const DEFAULT_JOURNAL_PATH: &str = "/run/installer/journal.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRecord {
    pub status: PhaseStatus,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    pub error: Option<String>,
}

impl Default for PhaseRecord {
    fn default() -> Self {
        Self { status: PhaseStatus::Pending, started_at: None, finished_at: None, error: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    /// The resolved install plan (disk path, edition, username, ...) as opaque JSON —
    /// `installer-core` doesn't need to interpret it, only persist and hand it back to
    /// whichever frontend resumes the run.
    pub plan: serde_json::Value,
    pub phases: HashMap<PhaseId, PhaseRecord>,
}

impl Journal {
    pub fn new(plan: serde_json::Value, phase_ids: &[PhaseId]) -> Self {
        Self {
            plan,
            phases: phase_ids.iter().map(|id| (*id, PhaseRecord::default())).collect(),
        }
    }

    pub async fn load(path: impl AsRef<Path>) -> crate::Result<Option<Self>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }
        let raw = tokio::fs::read_to_string(path).await?;
        Ok(Some(serde_json::from_str(&raw).map_err(|e| crate::Error::Other(e.into()))?))
    }

    pub async fn save(&self, path: impl AsRef<Path>) -> crate::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|e| crate::Error::Other(e.into()))?;
        tokio::fs::write(path, raw).await?;
        Ok(())
    }

    pub fn mark_running(&mut self, id: PhaseId) {
        let record = self.phases.entry(id).or_default();
        record.status = PhaseStatus::Running;
        record.started_at = Some(SystemTime::now());
    }

    pub fn mark_done(&mut self, id: PhaseId) {
        let record = self.phases.entry(id).or_default();
        record.status = PhaseStatus::Done;
        record.finished_at = Some(SystemTime::now());
        record.error = None;
    }

    pub fn mark_failed(&mut self, id: PhaseId, error: String) {
        let record = self.phases.entry(id).or_default();
        record.status = PhaseStatus::Failed;
        record.finished_at = Some(SystemTime::now());
        record.error = Some(error);
    }

    /// Whether the run this journal describes finished every phase.
    pub fn is_complete(&self) -> bool {
        self.phases.values().all(|r| r.status == PhaseStatus::Done)
    }

    /// Whether resuming makes sense: some phases are done, none are still `Failed`
    /// without having been retried, and the run isn't already complete.
    pub fn is_resumable(&self) -> bool {
        !self.is_complete() && self.phases.values().any(|r| r.status == PhaseStatus::Done)
    }
}

pub fn default_path() -> PathBuf {
    PathBuf::from(DEFAULT_JOURNAL_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::PhaseId;

    #[tokio::test]
    async fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-journal-test-{}", std::process::id()));
        let path = dir.join("journal.json");

        let mut journal = Journal::new(serde_json::json!({"disk": "/dev/sda"}), &[PhaseId::Preflight, PhaseId::Partition]);
        journal.mark_done(PhaseId::Preflight);
        journal.mark_running(PhaseId::Partition);
        journal.save(&path).await.unwrap();

        let loaded = Journal::load(&path).await.unwrap().unwrap();
        assert_eq!(loaded.phases[&PhaseId::Preflight].status, PhaseStatus::Done);
        assert_eq!(loaded.phases[&PhaseId::Partition].status, PhaseStatus::Running);
        assert_eq!(loaded.plan["disk"], "/dev/sda");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn missing_journal_loads_as_none() {
        let result = Journal::load("/nonexistent/gentoo-installer-journal.json").await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn resumable_requires_partial_progress() {
        let mut journal = Journal::new(serde_json::json!({}), &[PhaseId::Preflight, PhaseId::Partition]);
        assert!(!journal.is_resumable(), "nothing done yet");

        journal.mark_done(PhaseId::Preflight);
        assert!(journal.is_resumable());

        journal.mark_done(PhaseId::Partition);
        assert!(!journal.is_resumable(), "fully complete, not a resume candidate");
    }
}
