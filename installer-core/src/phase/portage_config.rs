//! PortageConfig (spec §3 phase 9, detailed in §8): make.conf, binrepos.conf (already
//! written by `store::configure` during Deploy — see the note below), and the git
//! history of `/etc/portage` that makes the installed system able to explain itself.
//!
//! Not yet implemented: merging a chosen **preset** (spec §5.7 — USE flags + starter
//! package list, JSON, from `portage_store`) into make.conf, and copying it to a stable
//! path for the app store to diff against later. That depends on integrating the
//! `portage_store` crate's preset format, which this crate doesn't reference yet.
//! What's here — detected values written to make.conf with provenance comments, and the
//! git init + first commit — stands on its own regardless of presets landing later.

use super::{Ctx, Phase, PhaseId};
use crate::command::CommandRunner;
use crate::detect::{self, DetectedSystem};
use crate::event::{Event, EventTx};

pub struct PortageConfigPhase;

#[async_trait::async_trait]
impl Phase for PortageConfigPhase {
    fn id(&self) -> PhaseId {
        PhaseId::PortageConfig
    }

    fn label(&self) -> &str {
        "Writing your Portage configuration"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(ctx.target.join("etc/portage/.git").is_dir())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });

        let detected = detect::gather(ctx.runner.as_ref()).await;
        write_make_conf(&ctx.target, &detected).await?;
        git_init_and_commit(ctx.runner.as_ref(), &ctx.target).await?;

        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        1
    }

    fn reversible(&self) -> bool {
        false
    }
}

async fn write_make_conf(target: &std::path::Path, detected: &DetectedSystem) -> crate::Result<()> {
    let mut conf = String::new();
    conf.push_str("# Written by the installer. See `git log` in this directory for history.\n\n");

    if let Some(flags) = &detected.cpu_flags {
        conf.push_str("# CPU_FLAGS_X86: detected by installer via cpuid2cpuflags\n");
        conf.push_str(&format!("CPU_FLAGS_X86=\"{flags}\"\n\n"));
    }
    if let Some(video_cards) = &detected.video_cards {
        conf.push_str("# VIDEO_CARDS: detected by installer from PCI GPU vendor/device ID\n");
        conf.push_str(&format!("VIDEO_CARDS=\"{video_cards}\"\n\n"));
    }

    conf.push_str("# Prefer binary packages from the store, fall back to source.\n");
    conf.push_str("EMERGE_DEFAULT_OPTS=\"--getbinpkg --usepkg\"\n");

    let portage_dir = target.join("etc/portage");
    tokio::fs::create_dir_all(&portage_dir).await?;
    tokio::fs::write(portage_dir.join("make.conf"), conf).await?;
    Ok(())
}

/// "What did the installer decide?" becomes `git log`; "undo it" becomes `git revert` —
/// only true if this first commit actually exists. Message format per spec §8.4.
async fn git_init_and_commit(
    runner: &dyn CommandRunner,
    target: &std::path::Path,
) -> crate::Result<()> {
    let portage_dir = target.join("etc/portage");
    let dir_str = portage_dir
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 /etc/portage path")))?;

    runner.run_status("git", &["-C", dir_str, "init"]).await?;
    runner
        .run_status("git", &["-C", dir_str, "add", "-A"])
        .await?;
    runner
        .run_status(
            "git",
            &[
                // The live system has no git identity, and `git commit` refuses without one
                // ("Author identity unknown") — found by running the real phases in a VM.
                "-c",
                "user.name=Gentoo installer",
                "-c",
                "user.email=installer@localhost",
                "-C",
                dir_str,
                "commit",
                "-m",
                "Initial configuration written by installer",
            ],
        )
        .await?;
    Ok(())
}
