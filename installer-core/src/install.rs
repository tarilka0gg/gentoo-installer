//! One install, as the frontends run it. [`InstallOptions`] is what the wizard collected; [`run`] turns it
//! into a [`Ctx`] and hands it to the phase pipeline ([`phase::run_phases`]), which is the only place the
//! steps of an install live. The frontends consume the [`Event`] stream; nothing here prints.
//!
//! (The older linear `run` that duplicated every step is gone: two orchestrators drifted apart — different
//! `make.conf` generators, different step order, no resume — and each fix had to be made twice.)

use crate::account::Account;
use crate::command::{CommandRunner, RealCommandRunner};
use crate::event::{Event, EventTx, Level};
use crate::hardware::Gpu;
use crate::journal;
use crate::phase::{self, Ctx, RunMode, Settings};
use crate::wm::WmChoice;
use crate::{make_conf, partition, stage3, store};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub layout: partition::Layout,
    /// Where the target system gets mounted during install, e.g. `/mnt/gentoo`.
    pub target: PathBuf,
    pub store: store::StoreConfig,
    /// Base package name for kernel atoms in the store, e.g. "mykernel" for
    /// `sys-kernel/mykernel-bin-<combo>`.
    pub kernel_base_name: String,
    /// XKB layout code (e.g. "us", "ua") — auto-detected default, or an Advanced-setup
    /// manual choice.
    pub keyboard_layout: String,
    /// IANA zone name (e.g. "Europe/Kyiv") — auto-detected default, or an Advanced-setup
    /// manual choice.
    pub timezone: String,
    /// Machine hostname (RFC 1123, lowercase) — `"gentoo"` unless the frontend asks.
    pub hostname: String,
    /// `locale.gen` entries; the first becomes `LANG`. `["en_US.UTF-8"]` by default.
    pub locales: Vec<String>,
    /// Overrides the detected GPU — picks the kernel build and whether the proprietary
    /// NVIDIA driver gets compiled. `None` keeps whatever detection found.
    pub gpu_override: Option<Gpu>,
    /// Ids from `packages::GROUPS` to emerge after the desktop.
    pub packages: Vec<String>,
    /// A custom stage3 tarball (your own, with your shell and tools) instead of Gentoo's
    /// latest: URL, `file://` path or plain path, plus its SHA512 if known.
    pub stage3: Option<stage3::Stage3Source>,
    pub account: Account,
    /// Compositor to install alongside Noctalia — auto-detected-default shape (same as
    /// `keyboard_layout`/`timezone`): `WmChoice::default()` (niri) unless Advanced setup
    /// picked something else.
    pub wm: WmChoice,
    /// `make.conf`'s `-O2`/`-O3` — `OptLevel::default()` (O2) unless Advanced setup
    /// picked O3.
    pub opt_level: make_conf::OptLevel,
    /// Whether large main-tree packages come down as prebuilt binaries or get compiled
    /// from source — `PackageMode::default()` (Binary) unless Advanced setup picked
    /// Source.
    pub package_mode: make_conf::PackageMode,
    /// Git URL of the wm-configs preset repo `wm::install` clones for the chosen
    /// compositor's config + Noctalia's shared config + tty1-autostart template.
    pub wm_configs_git_url: String,
    /// UI dry-run: walks through the same `Progress` sequence with the same timing
    /// shape, but never touches a disk, the network, or a chroot — for clicking through
    /// the wizard while iterating on the frontend. Hardware detection still runs for
    /// real (it's read-only), so the fake kernel atom reported is at least honest about
    /// what this machine would actually resolve to.
    pub simulate: bool,
}

impl InstallOptions {
    /// The pipeline's state for this install: the disk layout, the store, and the user's choices as
    /// [`Settings`].
    pub fn into_ctx(self, runner: Arc<dyn CommandRunner>) -> Ctx {
        let settings = Settings {
            timezone: self.timezone,
            keyboard_layout: self.keyboard_layout,
            locales: self.locales,
            hostname: self.hostname,
            account: Some(self.account),
            stage3: self.stage3,
            wm: self.wm,
            wm_configs_git_url: self.wm_configs_git_url,
            packages: self.packages,
            gpu_override: self.gpu_override,
            opt_level: self.opt_level,
            package_mode: self.package_mode,
        };
        Ctx::new(
            runner,
            self.target,
            self.layout,
            self.store,
            self.kernel_base_name,
        )
        .with_settings(settings)
    }
}

/// Runs the install: [`RunMode::Fresh`] from the beginning, [`RunMode::Resume`] to continue one that
/// stopped (see [`resumable`]). Progress and errors arrive on `tx`; the result is the first error, if any.
pub async fn run(opts: InstallOptions, tx: EventTx, mode: RunMode) -> crate::Result<()> {
    if opts.simulate {
        return simulate(opts, tx).await;
    }
    let journal_path = journal::default_path();
    let mut ctx = opts.into_ctx(Arc::new(RealCommandRunner));
    phase::run_phases(&mut ctx, &tx, mode, Some(&journal_path)).await
}

/// What an earlier, unfinished run on a disk left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInfo {
    pub phases_done: usize,
    pub phases_total: usize,
    /// The error the run stopped on, if it recorded one.
    pub error: Option<String>,
}

/// Whether the journal of this live session describes an unfinished install on `disk` — i.e. whether
/// [`RunMode::Resume`] is on offer. The journal lives in `/run`, so it dies with the live session: after a
/// reboot there is nothing to resume, and a fresh start is the only option.
pub async fn resumable(disk: &str) -> Option<ResumeInfo> {
    resumable_at(&journal::default_path(), disk).await
}

pub async fn resumable_at(path: &std::path::Path, disk: &str) -> Option<ResumeInfo> {
    let j = journal::Journal::load(path).await.ok()??;
    if j.plan.get("disk").and_then(|d| d.as_str()) != Some(disk) || !j.is_resumable() {
        return None;
    }
    Some(ResumeInfo {
        phases_done: j
            .phases
            .values()
            .filter(|r| r.status == journal::PhaseStatus::Done)
            .count(),
        phases_total: j.phases.len(),
        error: j.phases.values().find_map(|r| r.error.clone()),
    })
}

/// Fake run for UI iteration: the same events as the real pipeline (one started/finished pair per phase, a
/// log line for the kernel), with short sleeps and no disk, network or chroot access. Hardware detection
/// is real (read-only), so the combo shown is what this machine would actually resolve to.
async fn simulate(opts: InstallOptions, tx: EventTx) -> crate::Result<()> {
    use tokio::time::{sleep, Duration};

    for phase in phase::all_phases() {
        let id = phase.id();
        let _ = tx.send(Event::PhaseStarted {
            id,
            label: phase.label().to_string(),
        });
        if id == phase::PhaseId::Deploy {
            let combo = crate::hardware::Profile::detect()
                .map(|p| p.combo())
                .unwrap_or_else(|_| "unknown".into());
            let _ = tx.send(Event::Log {
                line: format!(
                    "Kernel: sys-kernel/{}-bin-{combo} (simulated)",
                    opts.kernel_base_name
                ),
                level: Level::Info,
            });
        }
        sleep(Duration::from_millis(
            250 + 6 * u64::from(phase.weight().min(100)),
        ))
        .await;
        let _ = tx.send(Event::PhaseFinished {
            id,
            duration: Duration::default(),
        });
    }
    let _ = tx.send(Event::Complete);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;
    use crate::phase::PhaseId;

    fn options() -> InstallOptions {
        InstallOptions {
            layout: partition::plan("/dev/vda", partition::RootFs::Btrfs, 16 << 30),
            target: "/mnt/gentoo".into(),
            store: store::StoreConfig {
                binhost_url: "http://x".into(),
                overlay_git_url: "git://x".into(),
                overlay_name: "local".into(),
            },
            kernel_base_name: "k".into(),
            keyboard_layout: "ua".into(),
            timezone: "Europe/Kyiv".into(),
            hostname: "h".into(),
            locales: vec!["uk_UA.UTF-8".into()],
            gpu_override: Some(Gpu::Amd),
            packages: vec!["wifi".into()],
            stage3: None,
            account: Account {
                username: "u".into(),
                password: "p".into(),
            },
            wm: WmChoice::Sway,
            opt_level: make_conf::OptLevel::default(),
            package_mode: make_conf::PackageMode::default(),
            wm_configs_git_url: "git://wm".into(),
            simulate: false,
        }
    }

    #[test]
    fn every_choice_of_the_wizard_reaches_the_pipeline() {
        let ctx = options().into_ctx(Arc::new(crate::command::FakeCommandRunner::new()));
        let s = &ctx.settings;
        assert_eq!(
            (
                s.keyboard_layout.as_str(),
                s.timezone.as_str(),
                s.hostname.as_str()
            ),
            ("ua", "Europe/Kyiv", "h")
        );
        assert_eq!(s.locales, ["uk_UA.UTF-8"]);
        assert_eq!((s.wm, s.gpu_override), (WmChoice::Sway, Some(Gpu::Amd)));
        assert_eq!(
            (s.wm_configs_git_url.as_str(), s.packages.as_slice()),
            ("git://wm", ["wifi".to_string()].as_slice())
        );
        assert_eq!(s.account.as_ref().map(|a| a.username.as_str()), Some("u"));
        assert_eq!(ctx.layout.disk, "/dev/vda");
    }

    #[tokio::test]
    async fn a_simulated_run_sends_every_phase_and_then_complete() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut opts = options();
        opts.simulate = true;
        run(opts, tx, RunMode::Fresh).await.unwrap();
        let mut started = Vec::new();
        let mut complete = false;
        while let Ok(e) = rx.try_recv() {
            match e {
                Event::PhaseStarted { id, .. } => started.push(id),
                Event::Complete => complete = true,
                _ => {}
            }
        }
        assert_eq!(
            started,
            phase::all_phases()
                .iter()
                .map(|p| p.id())
                .collect::<Vec<_>>()
        );
        assert!(complete);
    }

    #[tokio::test]
    async fn resume_is_offered_only_for_an_unfinished_run_on_the_same_disk() {
        let dir = std::env::temp_dir().join(format!("gi-resumable-{}", std::process::id()));
        let path = dir.join("journal.json");
        let ids = phase::all_phases()
            .iter()
            .map(|p| p.id())
            .collect::<Vec<_>>();
        assert_eq!(
            resumable_at(&path, "/dev/vda").await,
            None,
            "no journal, nothing to resume"
        );

        let mut j = Journal::new(serde_json::json!({"disk": "/dev/vda"}), &ids);
        j.mark_done(PhaseId::Preflight);
        j.mark_done(PhaseId::Partition);
        j.mark_failed(PhaseId::Format, "boom".into());
        j.save(&path).await.unwrap();

        let info = resumable_at(&path, "/dev/vda").await.unwrap();
        assert_eq!((info.phases_done, info.error.as_deref()), (2, Some("boom")));
        assert_eq!(
            resumable_at(&path, "/dev/vdb").await,
            None,
            "another disk is a different install"
        );

        for id in &ids {
            j.mark_done(*id);
        }
        j.save(&path).await.unwrap();
        assert_eq!(
            resumable_at(&path, "/dev/vda").await,
            None,
            "a finished run has nothing left to resume"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
