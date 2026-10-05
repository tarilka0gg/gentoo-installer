//! The progress/state event stream (spec §7). `installer-core` never prints — every
//! frontend (GTK, CLI, or a future TUI) consumes this on its own event loop instead.

use crate::phase::PhaseId;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub enum Event {
    PhaseStarted {
        id: PhaseId,
        label: String,
    },
    Progress {
        id: PhaseId,
        done: u64,
        total: Option<u64>,
    },
    Log {
        line: String,
        level: Level,
    },
    PhaseFinished {
        id: PhaseId,
        duration: Duration,
    },
    Failed {
        id: PhaseId,
        error: String,
    },
    Complete,
}

pub type EventTx = tokio::sync::mpsc::UnboundedSender<Event>;
