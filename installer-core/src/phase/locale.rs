//! Locale (spec §3, phase 7): keyboard layout, time zone, hostname and system locale.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::{keyboard, locale, timezone};

pub struct LocalePhase;

fn read(ctx: &Ctx, rel: &str) -> Option<String> {
    std::fs::read_to_string(ctx.target.join(rel)).ok()
}

#[async_trait::async_trait]
impl Phase for LocalePhase {
    fn id(&self) -> PhaseId {
        PhaseId::Locale
    }

    fn label(&self) -> &str {
        "Setting language, keyboard and time zone"
    }

    /// Every file must hold what the settings ask for, not merely exist: the stage3
    /// already ships `/etc/timezone` and `/etc/conf.d/{hostname,keymaps}` with its own
    /// defaults, so "exists" would report a fresh target as done. `02locale` is written
    /// last by `locale::apply`, after `locale-gen` succeeded, so a matching one means the
    /// whole phase finished.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let s = &ctx.settings;
        let Some(first_locale) = s.locales.first() else {
            return Ok(false);
        };
        Ok(
            read(ctx, "etc/timezone").as_deref() == Some(&format!("{}\n", s.timezone))
                && read(ctx, "etc/conf.d/keymaps").as_deref()
                    == Some(&format!("keymap=\"{}\"\n", s.keyboard_layout))
                && read(ctx, "etc/conf.d/hostname").as_deref()
                    == Some(&format!("hostname=\"{}\"\n", s.hostname))
                && read(ctx, "etc/env.d/02locale").as_deref()
                    == Some(&format!("LANG=\"{first_locale}\"\n")),
        )
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let s = ctx.settings.clone();

        // Plain file writes first, the chroot'd `locale-gen` last (see `is_satisfied`).
        keyboard::apply(&ctx.target, &s.keyboard_layout).await?;
        timezone::apply(&ctx.target, &s.timezone).await?;
        locale::apply_hostname(&ctx.target, &s.hostname).await?;
        locale::apply(ctx.runner.as_ref(), &ctx.target, &s.locales).await?;

        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        2
    }

    fn reversible(&self) -> bool {
        false
    }
}
