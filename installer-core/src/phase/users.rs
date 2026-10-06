//! Users (spec §3, phase 8): the first account, plus `doas` so that account can become root
//! (root itself stays locked).

use super::{Ctx, Phase, PhaseId};
use crate::account;
use crate::event::{Event, EventTx, Level};

pub struct UsersPhase;

fn user_exists(ctx: &Ctx, username: &str) -> bool {
    std::fs::read_to_string(ctx.target.join("etc/passwd"))
        .map(|passwd| {
            passwd
                .lines()
                .any(|l| l.split(':').next() == Some(username))
        })
        .unwrap_or(false)
}

#[async_trait::async_trait]
impl Phase for UsersPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Users
    }

    fn label(&self) -> &str {
        "Creating your account"
    }

    /// No account configured is "not satisfied", not "nothing to do": an install with no
    /// user is unusable (root stays locked), so `run` fails loudly on it instead.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let Some(acct) = &ctx.settings.account else {
            return Ok(false);
        };
        Ok(user_exists(ctx, &acct.username)
            && ctx.target.join("etc/doas.conf").is_file()
            && ctx.target.join("usr/bin/doas").exists())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let acct = ctx
            .settings
            .account
            .clone()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("no user account configured; root is locked, so the installed system would be unusable")))?;

        // Idempotent: `useradd` fails on an existing user, so a re-run skips creation but
        // still (re)writes the doas rule.
        if !user_exists(ctx, &acct.username) {
            account::create(ctx.runner.as_ref(), &ctx.target, &acct).await?;
        }
        account::configure_privilege(&ctx.target).await?;
        account::install_doas(ctx.runner.as_ref(), &ctx.target).await?;
        if account::set_root_shell(ctx.runner.as_ref(), &ctx.target).await? {
            let _ = tx.send(Event::Log {
                line: "root's login shell is fish".into(),
                level: Level::Info,
            });
        }

        let _ = tx.send(Event::Log {
            line: format!("{} can run commands as root with doas (wheel group); the root account stays locked", acct.username),
            level: Level::Info,
        });
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
