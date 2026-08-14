//! The phase state machine (spec §3) — the heart of the installer. Each phase is a
//! small struct implementing `Phase`; `Ctx` carries the state phases hand off to each
//! other (partition layout -> concrete partitions -> mount -> ...).
//!
//! Deviation from the spec's literal trait signature: `Phase::run`/`is_satisfied` are
//! `async` here, not sync. The underlying work (streaming the stage3 download over
//! `reqwest`, iwd over D-Bus) is inherently async in this codebase, built before this
//! spec landed — forcing it back to sync would mean either blocking a thread per
//! network call or losing the streaming download's backpressure. `async-trait` is the
//! one deviation from the letter of §3 in service of not reimplementing working code.

mod bootloader;
mod deploy;
mod finalize;
mod fstab;
mod mount;
mod partition;
mod portage_config;
mod preflight;

pub use bootloader::BootloaderPhase;
pub use deploy::DeployPhase;
pub use finalize::FinalizePhase;
pub use fstab::FstabPhase;
pub use mount::MountPhase;
pub use partition::{FormatPhase, PartitionPhase};
pub use portage_config::PortageConfigPhase;
pub use preflight::PreflightPhase;

use crate::command::CommandRunner;
use crate::event::EventTx;
use crate::{hardware, kernel, partition as partition_mod, store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PhaseId {
    Preflight,
    Partition,
    Format,
    Mount,
    Deploy,
    Fstab,
    Locale,
    Users,
    PortageConfig,
    Initramfs,
    Bootloader,
    PostHooks,
    Finalize,
}

impl PhaseId {
    /// The fixed install order (spec §3's table).
    pub const ORDER: [PhaseId; 13] = [
        PhaseId::Preflight,
        PhaseId::Partition,
        PhaseId::Format,
        PhaseId::Mount,
        PhaseId::Deploy,
        PhaseId::Fstab,
        PhaseId::Locale,
        PhaseId::Users,
        PhaseId::PortageConfig,
        PhaseId::Initramfs,
        PhaseId::Bootloader,
        PhaseId::PostHooks,
        PhaseId::Finalize,
    ];
}

#[async_trait::async_trait]
pub trait Phase: Send + Sync {
    /// Stable identifier, used as the journal key.
    fn id(&self) -> PhaseId;

    /// Human-readable, shown in the UI. Sentence case, active voice.
    fn label(&self) -> &str;

    /// Has this phase already completed on this target? Must be cheap and
    /// side-effect free. Called on resume to decide whether to skip.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool>;

    /// Do the work. Must be idempotent: running it twice on the same target
    /// leaves the same result.
    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()>;

    /// Cost hint for the aggregate progress bar (spec §7) — `Deploy` should be set
    /// from the actual image/download size at preflight, everything else is a rough
    /// constant weight.
    fn weight(&self) -> u32;

    /// Can the user still abort safely before this phase starts?
    fn reversible(&self) -> bool;
}

/// State phases hand off to each other. Not every field is populated at every point in
/// the run — `Partition` sets `parts` once it exists, `Deploy` sets `kernel_pkg` once
/// hardware/kernel matching resolves, etc. Phases that need a value another phase should
/// already have set treat a missing one as a bug (`expect`), not a recoverable error.
pub struct Ctx {
    pub runner: Arc<dyn CommandRunner>,
    pub target: PathBuf,
    pub layout: partition_mod::Layout,
    pub parts: Option<partition_mod::Partitions>,
    pub store: store::StoreConfig,
    pub kernel_base_name: String,
    pub profile: Option<hardware::Profile>,
    pub kernel_pkg: Option<kernel::KernelPackage>,
}

impl Ctx {
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        target: PathBuf,
        layout: partition_mod::Layout,
        store: store::StoreConfig,
        kernel_base_name: String,
    ) -> Self {
        Self {
            runner,
            target,
            layout,
            parts: None,
            store,
            kernel_base_name,
            profile: None,
            kernel_pkg: None,
        }
    }

    pub fn target_str(&self) -> crate::Result<&str> {
        self.target
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))
    }
}

/// The phase list in order — what `installer-cli`/`installer-gtk` drive.
///
/// Only 9 of the 13 `PhaseId::ORDER` phases are implemented here: `Locale`, `Users`,
/// `Initramfs`, and `PostHooks` are genuinely new work (keymap/timezone application,
/// useradd/passwd, dracut invocation, machine-id/eix seeding) that nothing in this
/// codebase does yet, unlike the other 9 which all reuse existing, live-verified logic
/// (hardware/kernel matching, partitioning, stage3, store config, Limine). Left out
/// entirely rather than stubbed with a fake no-op `run()`, since a phase that silently
/// "succeeds" without doing anything is exactly the kind of invisible debt §0 warns
/// against — `PhaseId` still has all 13 variants so the journal format doesn't need to
/// change shape when they're added for real.
pub fn all_phases() -> Vec<Box<dyn Phase>> {
    vec![
        Box::new(PreflightPhase),
        Box::new(PartitionPhase),
        Box::new(FormatPhase),
        Box::new(MountPhase),
        Box::new(DeployPhase),
        Box::new(FstabPhase),
        Box::new(PortageConfigPhase),
        Box::new(BootloaderPhase),
        Box::new(FinalizePhase),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;
    use crate::partition::RootFs;

    /// Runs Partition -> Format -> Mount -> Fstab against a fake runner and a real temp
    /// directory standing in for the target root — exactly the `--dry-run`-style
    /// no-root/no-real-disk setup spec §10 requires. `Deploy`/`PortageConfig`/
    /// `Bootloader` are intentionally excluded here: `Deploy` and `PortageConfig` make
    /// real `reqwest`/`git` network calls not behind `CommandRunner` yet (a real gap —
    /// HTTP isn't abstracted the way shell commands are), and `Bootloader` reads a
    /// hardcoded `/usr/share/limine` path rather than something injectable. This test
    /// covers the phases where `Ctx` handoff (layout -> concrete `Partitions` -> mount ->
    /// fstab) is actually exercised, which is the part most likely to break silently.
    #[tokio::test]
    async fn partition_through_fstab_runs_end_to_end_against_fake_runner() {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-phase-test-{}", std::process::id()));
        // `mkdir`/`mount` inside the phases go through the fake runner and don't actually
        // touch the filesystem; `fstab::generate` writes `etc/fstab` for real, though, so
        // that one directory needs to actually exist for the test to reach it.
        tokio::fs::create_dir_all(dir.join("etc")).await.unwrap();

        let fake = FakeCommandRunner::new();
        fake.respond("blkid", "ABCD-1234\n");
        let runner: Arc<dyn crate::command::CommandRunner> = Arc::new(fake);
        let layout = partition_mod::plan("/dev/sda", RootFs::Btrfs, 16 * 1024 * 1024 * 1024);
        let store = store::StoreConfig {
            binhost_url: "https://example.invalid".into(),
            overlay_git_url: "https://example.invalid/overlay.git".into(),
            overlay_name: "test".into(),
        };
        let mut ctx = Ctx::new(runner, dir.clone(), layout, store, "test-kernel".into());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        for phase in [
            Box::new(PartitionPhase) as Box<dyn Phase>,
            Box::new(FormatPhase),
            Box::new(MountPhase),
            Box::new(FstabPhase),
        ] {
            assert!(!phase.is_satisfied(&ctx).await.unwrap(), "{:?} should not be satisfied yet", phase.id());
            phase.run(&mut ctx, &tx).await.unwrap();
        }

        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(events.iter().any(|e| matches!(e, crate::event::Event::PhaseStarted { id: PhaseId::Partition, .. })));
        assert!(events.iter().any(|e| matches!(e, crate::event::Event::PhaseFinished { id: PhaseId::Fstab, .. })));

        assert!(ctx.parts.is_some());
        // fstab::generate should have written a real file to the temp target.
        assert!(dir.join("etc/fstab").exists());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
