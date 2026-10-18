//! Wizard steps. Transitions are driven explicitly in `ui::advance` (each step validates
//! its own precondition — e.g. Confirm only proceeds once `install::run` is spawned) rather
//! than a blind `next()`, since Installing must not advance until the background task
//! actually reports `Progress::Done`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Network,
    /// What kind of system: a flavor of the profiles (skipped when `GENTOO_INSTALLER_FLAVOR` says).
    Flavor,
    DiskSelect,
    Account,
    Confirm,
    Installing,
    Done,
}
