//! PostHooks: record `/etc/portage` in git, last. "What did the installer decide?" becomes `git log` and
//! "undo it" becomes `git revert` — and the history has to describe the finished system, so it is written
//! after the emerges of the earlier phases added their `package.use` and `package.accept_keywords`.

use super::{Ctx, Phase, PhaseId};
use crate::command::CommandRunner;
use crate::event::{Event, EventTx};

pub struct PostHooksPhase;

#[async_trait::async_trait]
impl Phase for PostHooksPhase {
    fn id(&self) -> PhaseId {
        PhaseId::PostHooks
    }

    fn label(&self) -> &str {
        "Recording the configuration history"
    }

    /// A commit exists once a branch ref does (`git init` alone leaves `refs/heads` empty).
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(
            std::fs::read_dir(ctx.target.join("etc/portage/.git/refs/heads"))
                .map(|mut d| d.next().is_some())
                .unwrap_or(false),
        )
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
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

pub(crate) async fn git_init_and_commit(
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
                "Configuration written by installer",
            ],
        )
        .await?;
    Ok(())
}
