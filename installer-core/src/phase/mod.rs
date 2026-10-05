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
mod desktop;
mod finalize;
mod fstab;
mod gpu;
mod locale;
mod mount;
mod packages;
mod partition;
mod portage_config;
mod post_hooks;
mod preflight;
mod users;

pub use bootloader::BootloaderPhase;
pub use deploy::DeployPhase;
pub use desktop::DesktopPhase;
pub use finalize::FinalizePhase;
pub use fstab::FstabPhase;
pub use gpu::GpuPhase;
pub use locale::LocalePhase;
pub use mount::MountPhase;
pub use packages::PackagesPhase;
pub use partition::{FormatPhase, PartitionPhase};
pub use portage_config::PortageConfigPhase;
pub use post_hooks::PostHooksPhase;
pub use preflight::PreflightPhase;
pub use users::UsersPhase;

use crate::account::Account;
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
    /// Not implemented: the live kernel boots without one (see `bootloader`). Kept so a journal written by
    /// an older version still deserialises.
    Initramfs,
    Bootloader,
    PostHooks,
    Finalize,
    Gpu,
    Desktop,
    Packages,
}

impl PhaseId {
    /// The fixed install order. `PortageConfig` (make.conf) comes right after `Deploy`: every emerge after
    /// it — `doas`, the driver, the desktop, the packages — must see the tuned `MAKEOPTS`, binhost settings and
    /// `VIDEO_CARDS`. The git history of `/etc/portage` is recorded last (`PostHooks`), after those
    /// emerges wrote their `package.use`/`package.accept_keywords`, so it describes the finished system.
    pub const ORDER: [PhaseId; 15] = [
        PhaseId::Preflight,
        PhaseId::Partition,
        PhaseId::Format,
        PhaseId::Mount,
        PhaseId::Deploy,
        PhaseId::PortageConfig,
        PhaseId::Fstab,
        PhaseId::Locale,
        PhaseId::Users,
        PhaseId::Gpu,
        PhaseId::Desktop,
        PhaseId::Packages,
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

    /// Destructive phases that cannot be probed afterwards (partitioning, formatting) are skipped on a
    /// resumed run on the strength of the journal alone; everything else must also still pass
    /// [`Phase::is_satisfied`]. A guess here would re-format a disk that already holds the install.
    fn trusts_journal(&self) -> bool {
        false
    }
}

/// What the user chose on the wizard's settings pages, as the `Locale` and `Users`
/// phases need it. Everything except `account` has a sensible default, so a Ctx built
/// without calling [`Ctx::with_settings`] still installs a usable system; `account` has
/// none, because inventing a username/password would be worse than failing.
#[derive(Debug, Clone)]
pub struct Settings {
    /// IANA zone, e.g. `Europe/Kyiv`.
    pub timezone: String,
    /// XKB layout code, e.g. `us` or `ua`.
    pub keyboard_layout: String,
    /// `locale-gen` names, e.g. `uk_UA.UTF-8`. The first one becomes the system `LANG`.
    pub locales: Vec<String>,
    pub hostname: String,
    pub account: Option<Account>,
    /// Custom stage3 tarball; `None` means the official mirror's latest.
    pub stage3: Option<crate::stage3::Stage3Source>,
    pub wm: crate::wm::WmChoice,
    /// Git URL of the wm-configs repository the desktop step clones.
    pub wm_configs_git_url: String,
    /// Ids from `packages::GROUPS` to install after the desktop.
    pub packages: Vec<String>,
    /// Replaces the detected GPU (kernel build, driver, `VIDEO_CARDS`); `None` keeps detection.
    pub gpu_override: Option<crate::hardware::Gpu>,
    /// Which GPU the compositor renders on when the machine has several.
    pub render: crate::gpu::RenderPreference,
    pub opt_level: crate::make_conf::OptLevel,
    pub package_mode: crate::make_conf::PackageMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_string(),
            keyboard_layout: "us".to_string(),
            locales: vec![crate::locale::DEFAULT_LOCALE.to_string()],
            hostname: "gentoo".to_string(),
            account: None,
            stage3: None,
            wm: crate::wm::WmChoice::default(),
            wm_configs_git_url: String::new(),
            packages: Vec::new(),
            gpu_override: None,
            render: Default::default(),
            opt_level: crate::make_conf::OptLevel::default(),
            package_mode: crate::make_conf::PackageMode::default(),
        }
    }
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
    pub settings: Settings,
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
            settings: Settings::default(),
        }
    }

    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    pub fn target_str(&self) -> crate::Result<&str> {
        self.target
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))
    }
}

/// The phase list in the order of [`PhaseId::ORDER`] — what the frontends and `--headless` drive.
///
/// `Initramfs` is the one id left out: the live kernel boots without one, and a phase that silently
/// "succeeds" without doing anything is exactly the invisible debt worth refusing.
pub fn all_phases() -> Vec<Box<dyn Phase>> {
    vec![
        Box::new(PreflightPhase),
        Box::new(PartitionPhase),
        Box::new(FormatPhase),
        Box::new(MountPhase),
        Box::new(DeployPhase),
        Box::new(PortageConfigPhase),
        Box::new(FstabPhase),
        Box::new(LocalePhase),
        Box::new(UsersPhase),
        Box::new(GpuPhase),
        Box::new(DesktopPhase),
        Box::new(PackagesPhase),
        Box::new(BootloaderPhase),
        Box::new(PostHooksPhase),
        Box::new(FinalizePhase),
    ]
}

/// Sum of every phase's weight: what 100 % of the progress bar is made of.
pub fn total_weight() -> u32 {
    all_phases().iter().map(|p| p.weight()).sum()
}

/// Progress after the phases in `finished` are done, 0.0..=1.0.
pub fn fraction_done(finished: &[PhaseId]) -> f64 {
    let total = total_weight().max(1) as f64;
    let done: u32 = all_phases()
        .iter()
        .filter(|p| finished.contains(&p.id()))
        .map(|p| p.weight())
        .sum();
    f64::from(done) / total
}

/// Whether a run starts from the beginning or continues one that stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Every phase runs, whatever the disk already holds (a reinstall must not inherit an old system).
    Fresh,
    /// Continue the run the journal describes: phases it records as done are skipped.
    Resume,
}

/// Runs every phase of [`all_phases`] in order on `ctx`, writing `journal_path` (if given) after each step.
///
/// * [`RunMode::Fresh`]: nothing is skipped and a new journal replaces the old one.
/// * [`RunMode::Resume`]: the journal must describe an unfinished run on the same disk. A phase it records
///   as done is skipped (destructive ones on that alone, see [`Phase::trusts_journal`]; the others also
///   have to pass `is_satisfied`); everything else runs again.
///
/// The first failure sends [`Event::Failed`], is recorded in the journal and returned; nothing after it
/// runs. [`Event::Complete`] is sent when all phases are done.
pub async fn run_phases(
    ctx: &mut Ctx,
    tx: &EventTx,
    mode: RunMode,
    journal_path: Option<&std::path::Path>,
) -> crate::Result<()> {
    run_phase_list(ctx, tx, all_phases(), mode, journal_path).await
}

/// [`run_phases`] over an explicit list (the executor itself; split out so its resume rules can be tested
/// with small stand-in phases instead of a disk).
pub async fn run_phase_list(
    ctx: &mut Ctx,
    tx: &EventTx,
    phases: Vec<Box<dyn Phase>>,
    mode: RunMode,
    journal_path: Option<&std::path::Path>,
) -> crate::Result<()> {
    use crate::event::{Event, Level};
    use crate::journal::{Journal, PhaseStatus};

    let ids: Vec<PhaseId> = phases.iter().map(|p| p.id()).collect();
    let plan = serde_json::json!({ "disk": ctx.layout.disk, "hostname": ctx.settings.hostname });

    let mut journal = match (mode, journal_path) {
        (RunMode::Resume, Some(path)) => {
            let loaded = Journal::load(path).await?.ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "nothing to resume: there is no journal at {}",
                    path.display()
                ))
            })?;
            let same_disk =
                loaded.plan.get("disk").and_then(|d| d.as_str()) == Some(ctx.layout.disk.as_str());
            if !same_disk || !loaded.is_resumable() {
                return Err(crate::Error::Other(anyhow::anyhow!(
                    "the journal does not describe an unfinished install on {}",
                    ctx.layout.disk
                )));
            }
            if loaded
                .phases
                .get(&PhaseId::Partition)
                .is_some_and(|r| r.status == PhaseStatus::Done)
            {
                ctx.parts = Some(partition_mod::partitions_for(&ctx.layout));
            }
            Some(loaded)
        }
        (RunMode::Resume, None) => {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "resuming needs a journal path"
            )));
        }
        (RunMode::Fresh, Some(_)) => Some(Journal::new(plan, &ids)),
        (RunMode::Fresh, None) => None,
    };

    macro_rules! record {
        ($j:ident . $m:ident ( $($a:expr),* )) => {
            if let (Some($j), Some(path)) = (journal.as_mut(), journal_path) {
                $j.$m($($a),*);
                let _ = $j.save(path).await;
            }
        };
    }
    if let (Some(j), Some(path)) = (journal.as_ref(), journal_path) {
        let _ = j.save(path).await;
    }

    for phase in phases {
        let id = phase.id();
        if mode == RunMode::Resume {
            let done_before = journal
                .as_ref()
                .and_then(|j| j.phases.get(&id))
                .is_some_and(|r| r.status == PhaseStatus::Done);
            if done_before {
                let skip = if phase.trusts_journal() {
                    true
                } else {
                    match phase.is_satisfied(ctx).await {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = tx.send(Event::Failed {
                                id,
                                error: e.to_string(),
                            });
                            return Err(e);
                        }
                    }
                };
                if skip {
                    let _ = tx.send(Event::Log {
                        line: format!("{}: already done, skipping", phase.label()),
                        level: Level::Info,
                    });
                    let _ = tx.send(Event::PhaseFinished {
                        id,
                        duration: std::time::Duration::default(),
                    });
                    continue;
                }
            }
        }
        record!(journal.mark_running(id));
        if let Err(e) = phase.run(ctx, tx).await {
            record!(journal.mark_failed(id, e.to_string()));
            let _ = tx.send(Event::Failed {
                id,
                error: e.to_string(),
            });
            return Err(e);
        }
        record!(journal.mark_done(id));
    }
    let _ = tx.send(Event::Complete);
    Ok(())
}

/// [`run_phases`] in [`RunMode::Fresh`] without a journal: the whole chain, nothing skipped.
pub async fn run_all(ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
    run_phases(ctx, tx, RunMode::Fresh, None).await
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
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-phase-test-{}",
            std::process::id()
        ));
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
            assert!(
                !phase.is_satisfied(&ctx).await.unwrap(),
                "{:?} should not be satisfied yet",
                phase.id()
            );
            phase.run(&mut ctx, &tx).await.unwrap();
        }

        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(events.iter().any(|e| matches!(
            e,
            crate::event::Event::PhaseStarted {
                id: PhaseId::Partition,
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            crate::event::Event::PhaseFinished {
                id: PhaseId::Fstab,
                ..
            }
        )));

        assert!(ctx.parts.is_some());
        // fstab::generate should have written a real file to the temp target.
        assert!(dir.join("etc/fstab").exists());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    fn settings_for_tests() -> Settings {
        Settings {
            timezone: "Europe/Kyiv".into(),
            keyboard_layout: "ua".into(),
            locales: vec!["uk_UA.UTF-8".into(), "en_US.UTF-8".into()],
            hostname: "solomiya-pc".into(),
            account: Some(Account {
                username: "solomiya".into(),
                password: "hunter2".into(),
            }),
            ..Settings::default()
        }
    }

    #[tokio::test]
    async fn run_all_stops_at_the_first_failure_and_reports_it() {
        use crate::event::Event;
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-run-all-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        let fake = FakeCommandRunner::new();
        fake.fail("parted", "disk vanished");
        fake.fail("sgdisk", "disk vanished");
        fake.fail("mkfs.vfat", "disk vanished");
        let mut ctx = ctx_with(&dir, Arc::new(fake), settings_for_tests());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let result = run_all(&mut ctx, &tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }

        assert!(result.is_err(), "a failing phase must fail the run");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Failed { .. }))
                .count(),
            1
        );
        assert!(
            !events.iter().any(|e| matches!(e, Event::Complete)),
            "no Complete after a failure"
        );
        // Nothing starts after the failure.
        let failed_at = events
            .iter()
            .position(|e| matches!(e, Event::Failed { .. }))
            .unwrap();
        assert!(!events[failed_at..]
            .iter()
            .any(|e| matches!(e, Event::PhaseStarted { .. })));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn deploy_is_only_satisfied_once_stage_overlay_and_kernel_are_all_there() {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-deploy-sat-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc/portage")).unwrap();
        let ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        let phase = DeployPhase;

        assert!(
            !phase.is_satisfied(&ctx).await.unwrap(),
            "a bare stage3 is not a finished deploy"
        );
        std::fs::create_dir_all(dir.join("var/db/repos").join(&ctx.store.overlay_name)).unwrap();
        assert!(!phase.is_satisfied(&ctx).await.unwrap(), "no kernel yet");
        std::fs::create_dir_all(dir.join("boot")).unwrap();
        std::fs::write(dir.join("boot/vmlinuz-generic"), "").unwrap();
        assert!(phase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_stage3s_own_fstab_does_not_count_as_a_written_one() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-fstab-sat-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        // What a real stage3 ships: only comments.
        std::fs::write(
            dir.join("etc/fstab"),
            "# /etc/fstab: static file system information.\n# <fs> <mountpoint> <type>\n",
        )
        .unwrap();
        let ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        assert!(!FstabPhase.is_satisfied(&ctx).await.unwrap());

        std::fs::write(
            dir.join("etc/fstab"),
            format!(
                "{}\n\nUUID=x / ext4 defaults 0 1\n",
                crate::fstab::GENERATED_MARKER
            ),
        )
        .unwrap();
        assert!(FstabPhase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn ctx_with(dir: &std::path::Path, runner: Arc<dyn CommandRunner>, settings: Settings) -> Ctx {
        let layout = partition_mod::plan(
            "/dev/sda",
            crate::partition::RootFs::Btrfs,
            16 * 1024 * 1024 * 1024,
        );
        let store = store::StoreConfig {
            binhost_url: "https://example.invalid".into(),
            overlay_git_url: "https://example.invalid/overlay.git".into(),
            overlay_name: "test".into(),
        };
        Ctx::new(
            runner,
            dir.to_path_buf(),
            layout,
            store,
            "test-kernel".into(),
        )
        .with_settings(settings)
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-phase-{tag}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn locale_phase_applies_every_setting_and_only_then_reports_satisfied() {
        let dir = temp_dir("locale");
        let fake = Arc::new(FakeCommandRunner::new());
        let mut ctx = ctx_with(&dir, fake.clone(), settings_for_tests());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!LocalePhase.is_satisfied(&ctx).await.unwrap());
        LocalePhase.run(&mut ctx, &tx).await.unwrap();
        assert!(LocalePhase.is_satisfied(&ctx).await.unwrap());

        assert_eq!(
            std::fs::read_to_string(dir.join("etc/timezone")).unwrap(),
            "Europe/Kyiv\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/conf.d/keymaps")).unwrap(),
            "keymap=\"ua\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/conf.d/hostname")).unwrap(),
            "hostname=\"solomiya-pc\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/env.d/02locale")).unwrap(),
            "LANG=\"uk_UA.UTF-8\"\n"
        );
        assert!(std::fs::read_to_string(dir.join("etc/locale.gen"))
            .unwrap()
            .contains("uk_UA.UTF-8 UTF-8"));

        let target = dir.to_str().unwrap();
        let chroots: Vec<Vec<String>> = fake
            .calls()
            .into_iter()
            .filter(|(c, _)| c == "chroot")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(chroots, [[target, "locale-gen"], [target, "env-update"]]);

        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(events.iter().any(|e| matches!(
            e,
            crate::event::Event::PhaseStarted {
                id: PhaseId::Locale,
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            crate::event::Event::PhaseFinished {
                id: PhaseId::Locale,
                ..
            }
        )));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stage3_defaults_do_not_count_as_a_finished_locale_phase() {
        // A real stage3 already has these files, with its own values.
        let dir = temp_dir("stage3defaults");
        std::fs::create_dir_all(dir.join("etc/conf.d")).unwrap();
        std::fs::write(dir.join("etc/timezone"), "UTC\n").unwrap();
        std::fs::write(dir.join("etc/conf.d/keymaps"), "keymap=\"us\"\n").unwrap();
        std::fs::write(dir.join("etc/conf.d/hostname"), "hostname=\"localhost\"\n").unwrap();

        let ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        assert!(!LocalePhase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn locale_phase_with_a_failing_locale_gen_stays_unsatisfied_so_resume_retries_it() {
        let dir = temp_dir("localefail");
        let fake = Arc::new(FakeCommandRunner::new());
        fake.fail("chroot", "locale-gen: not found");
        let mut ctx = ctx_with(&dir, fake, settings_for_tests());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(LocalePhase.run(&mut ctx, &tx).await.is_err());
        assert!(!LocalePhase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn users_phase_creates_the_account_writes_the_doas_rule_and_installs_doas() {
        let dir = temp_dir("users");
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap(); // tree "already synced"
        std::fs::write(dir.join("etc/resolv.conf"), "").ok();
        let fake = Arc::new(FakeCommandRunner::new());
        fake.respond("openssl", "$6$salt$hash\n");
        let mut ctx = ctx_with(&dir, fake.clone(), settings_for_tests());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!UsersPhase.is_satisfied(&ctx).await.unwrap());
        UsersPhase.run(&mut ctx, &tx).await.unwrap();

        fake.assert_call(0, "openssl", &["passwd", "-6", "--", "hunter2"]);
        assert_eq!(fake.calls()[1].0, "useradd");
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/doas.conf")).unwrap(),
            "permit :wheel\n"
        );
        assert!(
            fake.calls().iter().any(|(c, a)| c == "chroot"
                && a.ends_with(&[
                    "emerge".into(),
                    "--noreplace".into(),
                    "app-admin/doas".into()
                ])),
            "doas must be emerged in the target: {:?}",
            fake.calls()
        );

        drop(tx);
        while let Some(e) = rx.recv().await {
            // The password must never reach the event stream.
            if let crate::event::Event::Log { line, .. } = &e {
                assert!(!line.contains("hunter2"));
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn users_phase_is_not_satisfied_until_the_doas_binary_is_actually_there() {
        let dir = temp_dir("usersnobin");
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(
            dir.join("etc/passwd"),
            "solomiya:x:1000:1000::/home/solomiya:/bin/bash\n",
        )
        .unwrap();
        std::fs::write(dir.join("etc/doas.conf"), "permit :wheel\n").unwrap();
        let ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );

        // A rule for a program that was never installed is exactly the bug being fixed.
        assert!(!UsersPhase.is_satisfied(&ctx).await.unwrap());

        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        std::fs::write(dir.join("usr/bin/doas"), "").unwrap();
        assert!(UsersPhase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn users_phase_is_idempotent_when_the_user_already_exists() {
        let dir = temp_dir("usersidem");
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap();
        std::fs::write(
            dir.join("etc/passwd"),
            "root:x:0:0::/root:/bin/bash\nsolomiya:x:1000:1000::/home/solomiya:/bin/bash\n",
        )
        .unwrap();
        let fake = Arc::new(FakeCommandRunner::new());
        let mut ctx = ctx_with(&dir, fake.clone(), settings_for_tests());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        UsersPhase.run(&mut ctx, &tx).await.unwrap();

        // `useradd`/`openssl` would fail or waste work on an existing user...
        assert!(
            fake.calls()
                .iter()
                .all(|(c, _)| c != "useradd" && c != "openssl"),
            "{:?}",
            fake.calls()
        );
        // ...but the rule and the package are still (re)applied.
        assert!(dir.join("etc/doas.conf").is_file());
        assert!(fake
            .calls()
            .iter()
            .any(|(c, a)| c == "chroot" && a.contains(&"app-admin/doas".to_string())));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_user_whose_name_merely_starts_like_an_existing_one_is_not_mistaken_for_it() {
        let dir = temp_dir("usersprefix");
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(
            dir.join("etc/passwd"),
            "solomiya2:x:1000:1000::/home/solomiya2:/bin/bash\n",
        )
        .unwrap();
        let ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        std::fs::write(dir.join("etc/doas.conf"), "x").unwrap();

        assert!(!UsersPhase.is_satisfied(&ctx).await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn users_phase_without_an_account_fails_loudly_instead_of_succeeding_empty() {
        let dir = temp_dir("usersnone");
        let mut settings = settings_for_tests();
        settings.account = None;
        let mut ctx = ctx_with(&dir, Arc::new(FakeCommandRunner::new()), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!UsersPhase.is_satisfied(&ctx).await.unwrap());
        let err = UsersPhase.run(&mut ctx, &tx).await.unwrap_err();
        assert!(err.to_string().contains("no user account"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn all_phases_come_in_the_fixed_install_order_and_include_the_new_ones() {
        let ids: Vec<PhaseId> = all_phases().iter().map(|p| p.id()).collect();
        let positions: Vec<usize> = ids
            .iter()
            .map(|id| {
                PhaseId::ORDER
                    .iter()
                    .position(|o| o == id)
                    .expect("id in ORDER")
            })
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "phases out of install order: {ids:?}"
        );
        assert!(ids.contains(&PhaseId::Locale) && ids.contains(&PhaseId::Users));
        assert_eq!(ids.len(), 15);
    }

    /// A stand-in phase that counts its runs and reports a fixed `is_satisfied`.
    struct Counting {
        id: PhaseId,
        satisfied: bool,
        trusts: bool,
        fail: bool,
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Phase for Counting {
        fn id(&self) -> PhaseId {
            self.id
        }
        fn label(&self) -> &str {
            "counting"
        }
        async fn is_satisfied(&self, _ctx: &Ctx) -> crate::Result<bool> {
            Ok(self.satisfied)
        }
        async fn run(&self, _ctx: &mut Ctx, _tx: &EventTx) -> crate::Result<()> {
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                Err(crate::Error::Other(anyhow::anyhow!("{:?} failed", self.id)))
            } else {
                Ok(())
            }
        }
        fn weight(&self) -> u32 {
            1
        }
        fn reversible(&self) -> bool {
            false
        }
        fn trusts_journal(&self) -> bool {
            self.trusts
        }
    }

    fn counting(
        id: PhaseId,
        satisfied: bool,
        trusts: bool,
        fail: bool,
    ) -> (Box<dyn Phase>, Arc<std::sync::atomic::AtomicUsize>) {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Box::new(Counting {
                id,
                satisfied,
                trusts,
                fail,
                runs: runs.clone(),
            }),
            runs,
        )
    }

    fn runs(r: &Arc<std::sync::atomic::AtomicUsize>) -> usize {
        r.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_failed_run_resumes_after_what_is_done_and_never_reformats() {
        let dir = std::env::temp_dir().join(format!("gi-resume-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let journal = dir.join("journal.json");
        let mut ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // First attempt: Partition and Format succeed, Mount fails.
        let (p1, partition_runs) = counting(PhaseId::Partition, false, true, false);
        let (p2, format_runs) = counting(PhaseId::Format, false, true, false);
        let (p3, mount_runs) = counting(PhaseId::Mount, false, false, true);
        let r = run_phase_list(
            &mut ctx,
            &tx,
            vec![p1, p2, p3],
            RunMode::Fresh,
            Some(&journal),
        )
        .await;
        assert!(r.is_err());
        assert_eq!(
            (runs(&partition_runs), runs(&format_runs), runs(&mount_runs)),
            (1, 1, 1)
        );

        // Resume: the destructive phases are skipped on the journal alone (their is_satisfied says false —
        // they cannot be probed), Mount runs again and now succeeds.
        let (p1, partition_runs) = counting(PhaseId::Partition, false, true, false);
        let (p2, format_runs) = counting(PhaseId::Format, false, true, false);
        let (p3, mount_runs) = counting(PhaseId::Mount, false, false, false);
        run_phase_list(
            &mut ctx,
            &tx,
            vec![p1, p2, p3],
            RunMode::Resume,
            Some(&journal),
        )
        .await
        .unwrap();
        assert_eq!(
            (runs(&partition_runs), runs(&format_runs), runs(&mount_runs)),
            (0, 0, 1),
            "nothing already done is redone"
        );
        assert!(
            ctx.parts.is_some(),
            "the device names are rebuilt from the layout"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_done_phase_that_no_longer_checks_out_is_run_again() {
        let dir = std::env::temp_dir().join(format!("gi-resume2-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let journal = dir.join("journal.json");
        let mut ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let (a, _) = counting(PhaseId::Deploy, true, false, false);
        let (b, _) = counting(PhaseId::Fstab, true, false, true);
        assert!(
            run_phase_list(&mut ctx, &tx, vec![a, b], RunMode::Fresh, Some(&journal))
                .await
                .is_err()
        );

        // On resume Deploy is recorded done and still satisfied -> skipped; Fstab failed -> runs.
        let (a, deploy_runs) = counting(PhaseId::Deploy, true, false, false);
        let (b, fstab_runs) = counting(PhaseId::Fstab, true, false, false);
        run_phase_list(&mut ctx, &tx, vec![a, b], RunMode::Resume, Some(&journal))
            .await
            .unwrap();
        assert_eq!((runs(&deploy_runs), runs(&fstab_runs)), (0, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn resuming_on_another_disk_or_without_a_journal_is_refused_and_a_fresh_run_ignores_old_state(
    ) {
        let dir = std::env::temp_dir().join(format!("gi-resume3-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let journal = dir.join("journal.json");
        let mut ctx = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let (p, _) = counting(PhaseId::Mount, true, false, false);
        let err = run_phase_list(&mut ctx, &tx, vec![p], RunMode::Resume, Some(&journal))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no journal"), "{err}");

        // An old unfinished run on /dev/other; this ctx installs to /dev/sda.
        let (a, _) = counting(PhaseId::Deploy, true, false, false);
        let (b, _) = counting(PhaseId::Fstab, true, false, true);
        let mut other = ctx_with(
            &dir,
            Arc::new(FakeCommandRunner::new()),
            settings_for_tests(),
        );
        other.layout.disk = "/dev/other".into();
        let _ = run_phase_list(&mut other, &tx, vec![a, b], RunMode::Fresh, Some(&journal)).await;
        let (p, _) = counting(PhaseId::Deploy, true, false, false);
        assert!(
            run_phase_list(&mut ctx, &tx, vec![p], RunMode::Resume, Some(&journal))
                .await
                .is_err()
        );

        // Fresh ignores whatever is on disk, even phases that claim to be satisfied.
        let (p, p_runs) = counting(PhaseId::Deploy, true, false, false);
        run_phase_list(&mut ctx, &tx, vec![p], RunMode::Fresh, Some(&journal))
            .await
            .unwrap();
        assert_eq!(
            runs(&p_runs),
            1,
            "a reinstall must not inherit an old system"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
