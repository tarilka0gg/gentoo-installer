//! Ordered wizard steps shared by the TUI's state machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Network,
    DiskSelect,
    Confirm,
    Installing,
    Done,
}

impl Step {
    pub fn next(self) -> Self {
        match self {
            Step::Network => Step::DiskSelect,
            Step::DiskSelect => Step::Confirm,
            Step::Confirm => Step::Installing,
            Step::Installing => Step::Done,
            Step::Done => Step::Done,
        }
    }
}
