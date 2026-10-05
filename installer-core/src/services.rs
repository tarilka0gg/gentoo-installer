//! Enabling OpenRC services on the installed system. This is an OpenRC-only distribution: no
//! `systemctl`, no unit files — a service is switched on with `rc-update add <name> <runlevel>`,
//! run in the target's chroot (it only makes a symlink under `/etc/runlevels`, so no bind mounts
//! are needed and it is safe to repeat: `rc-update` reports "already installed" and exits 0).

use crate::command::CommandRunner;
use std::path::Path;

pub const DEFAULT_RUNLEVEL: &str = "default";

/// Init-script names are file names under `/etc/init.d`; nothing else may reach the command line.
fn valid_name(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
}

pub async fn enable(runner: &dyn CommandRunner, target: &Path, service: &str, runlevel: &str) -> crate::Result<()> {
    if !valid_name(service) || !valid_name(runlevel) {
        return Err(crate::Error::Other(anyhow::anyhow!("invalid service or runlevel name: {service:?} / {runlevel:?}")));
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    runner.run_status("chroot", &[target_str, "rc-update", "add", service, runlevel]).await
}

/// [`enable`] for several services in the `default` runlevel, in order, stopping at the first
/// failure (order matters: `dbus` before the things that need it).
pub async fn enable_all(runner: &dyn CommandRunner, target: &Path, services: &[&str]) -> crate::Result<()> {
    for s in services {
        enable(runner, target, s, DEFAULT_RUNLEVEL).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn services_are_added_to_the_default_runlevel_in_order() {
        let runner = FakeCommandRunner::new();
        enable_all(&runner, Path::new("/mnt/gentoo"), &["dbus", "seatd"]).await.unwrap();
        runner.assert_call(0, "chroot", &["/mnt/gentoo", "rc-update", "add", "dbus", "default"]);
        runner.assert_call(1, "chroot", &["/mnt/gentoo", "rc-update", "add", "seatd", "default"]);
    }

    #[tokio::test]
    async fn a_name_that_could_be_an_option_or_a_path_runs_nothing() {
        for bad in ["", "-s", "a b", "../x", "x;reboot", "a/b"] {
            let runner = FakeCommandRunner::new();
            assert!(enable(&runner, Path::new("/mnt/gentoo"), bad, "default").await.is_err(), "{bad:?}");
            assert!(enable(&runner, Path::new("/mnt/gentoo"), "dbus", bad).await.is_err(), "{bad:?}");
            assert!(runner.calls().is_empty(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn the_first_failure_stops_the_rest() {
        let runner = FakeCommandRunner::new();
        runner.fail("chroot", "service `nope' does not exist");
        assert!(enable_all(&runner, Path::new("/mnt/gentoo"), &["nope", "dbus"]).await.is_err());
        assert_eq!(runner.calls().len(), 1);
    }
}
