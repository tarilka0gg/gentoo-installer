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
mod locale;
mod mount;
mod partition;
mod portage_config;
mod preflight;
mod users;

pub use bootloader::BootloaderPhase;
pub use deploy::DeployPhase;
pub use finalize::FinalizePhase;
pub use fstab::FstabPhase;
pub use locale::LocalePhase;
pub use mount::MountPhase;
pub use partition::{FormatPhase, PartitionPhase};
pub use portage_config::PortageConfigPhase;
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

/// The phase list in order — what `installer-cli`/`installer-gtk` drive.
///
/// 11 of the 13 `PhaseId::ORDER` phases are implemented. `Initramfs` and `PostHooks`
/// (dracut, machine-id/eix seeding, and installing the escalation tool `Users` only
/// configures) are still missing. They are left out entirely rather than stubbed with a
/// fake no-op `run()`, since a phase that silently "succeeds" without doing anything is
/// exactly the kind of invisible debt §0 warns against — `PhaseId` keeps all 13 variants
/// so the journal format doesn't change shape when they're added for real.
pub fn all_phases() -> Vec<Box<dyn Phase>> {
    vec![
        Box::new(PreflightPhase),
        Box::new(PartitionPhase),
        Box::new(FormatPhase),
        Box::new(MountPhase),
        Box::new(DeployPhase),
        Box::new(FstabPhase),
        Box::new(LocalePhase),
        Box::new(UsersPhase),
        Box::new(PortageConfigPhase),
        Box::new(BootloaderPhase),
        Box::new(FinalizePhase),
    ]
}

/// Runs every phase of [`all_phases`] in order on `ctx`. A phase whose [`Phase::is_satisfied`]
/// is true is skipped (that is what makes re-running after a crash safe); the first failure
/// sends [`Event::Failed`] and is returned, nothing after it runs. Sends [`Event::Complete`]
/// when all phases are done.
pub async fn run_all(ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
    use crate::event::{Event, Level};
    for phase in all_phases() {
        let id = phase.id();
        match phase.is_satisfied(ctx).await {
            Ok(true) => {
                let _ = tx.send(Event::Log {
                    line: format!("{}: already done, skipping", phase.label()),
                    level: Level::Info,
                });
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                let _ = tx.send(Event::Failed {
                    id,
                    error: e.to_string(),
                });
                return Err(e);
            }
        }
        if let Err(e) = phase.run(ctx, tx).await {
            let _ = tx.send(Event::Failed {
                id,
                error: e.to_string(),
            });
            return Err(e);
        }
    }
    let _ = tx.send(Event::Complete);
    Ok(())
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
            stage3: None,
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
        assert_eq!(ids.len(), 11);
    }
}
