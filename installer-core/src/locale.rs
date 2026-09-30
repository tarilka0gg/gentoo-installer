//! System locale + hostname for the installed target.
//!
//! Gentoo needs three separate things for a working locale: the entries listed in
//! `/etc/locale.gen`, `locale-gen` actually run to compile them into
//! `/usr/lib/locale/locale-archive`, and `LANG` exported through `/etc/env.d/02locale`
//! (+ `env-update` to fold it into `/etc/profile.env`). Missing any one leaves the
//! system on the POSIX locale — UTF-8 text (Ukrainian included) renders and sorts wrong.
//!
//! `locale-gen` and `env-update` run through `chroot` **with `/proc`, `/sys` and `/dev`
//! bind-mounted** (the same helpers `wm` and `gpu_driver` use). A bare chroot is not
//! enough: run against a real stage3, `locale-gen` compiles the locales and then aborts
//! with `findmnt: can't read /proc/mounts` (exit 1), leaving `locale -a` at
//! `C, C.utf8, POSIX`. The mounts are always undone, success or failure.

use crate::chroot_emerge::{bind_mount_chroot_dirs, unmount_chroot_dirs};
use crate::command::CommandRunner;
use std::path::Path;

/// What a fresh install falls back to when the user picked nothing.
pub const DEFAULT_LOCALE: &str = "en_US.UTF-8";

/// A `locale.gen` entry is `<name> <charset>`, e.g. `uk_UA.UTF-8 UTF-8`. The charset is
/// the part of the name after the dot, so a name with no dot cannot be turned into a
/// valid entry and is rejected instead of guessed at.
fn locale_gen_line(locale: &str) -> crate::Result<String> {
    let valid_chars = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '@'));
    let (_, charset) = locale
        .split_once('.')
        .ok_or_else(|| bad_locale(locale, "missing the .charset part, e.g. uk_UA.UTF-8"))?;
    let name_ok = locale.split('.').next().is_some_and(valid_chars);
    if !name_ok || !valid_chars(charset) {
        return Err(bad_locale(locale, "only letters, digits, '_', '-' and '@' are allowed"));
    }
    Ok(format!("{locale} {charset}"))
}

fn bad_locale(locale: &str, why: &str) -> crate::Error {
    crate::Error::Other(anyhow::anyhow!("invalid locale {locale:?}: {why}"))
}

/// Generates `locales` in the target and makes the first one the system `LANG`.
///
/// `02locale` is written **last**, after `locale-gen` succeeded, so its presence doubles
/// as the "this really finished" marker `LocalePhase::is_satisfied` checks on resume.
pub async fn apply(runner: &dyn CommandRunner, target: &Path, locales: &[String]) -> crate::Result<()> {
    let first = locales
        .first()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("no locale selected")))?;

    // Validate everything before touching the target, so a bad entry leaves it untouched.
    let mut lines: Vec<String> = Vec::new();
    for locale in locales {
        let line = locale_gen_line(locale)?;
        if !lines.contains(&line) {
            lines.push(line);
        }
    }

    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    tokio::fs::create_dir_all(target.join("etc/env.d")).await?;
    let mut locale_gen = String::from("# Written by gentoo-installer.\n");
    for line in &lines {
        locale_gen.push_str(line);
        locale_gen.push('\n');
    }
    tokio::fs::write(target.join("etc/locale.gen"), locale_gen).await?;

    let mounted = bind_mount_chroot_dirs(runner, target).await;
    let result = match mounted {
        Ok(()) => generate_in_chroot(runner, target, target_str, first).await,
        Err(e) => Err(e),
    };
    unmount_chroot_dirs(runner, target).await;
    result
}

async fn generate_in_chroot(runner: &dyn CommandRunner, target: &Path, target_str: &str, lang: &str) -> crate::Result<()> {
    runner.run_status("chroot", &[target_str, "locale-gen"]).await?;
    tokio::fs::write(target.join("etc/env.d/02locale"), format!("LANG=\"{lang}\"\n")).await?;
    runner.run_status("chroot", &[target_str, "env-update"]).await?;
    Ok(())
}

/// RFC 1123 host label: 1-63 chars of `a-z`, `0-9`, `-`, not starting or ending with `-`.
/// Upper case is rejected rather than lowered — silently rewriting what the user typed
/// would make the name they see in the UI differ from the one the machine ends up with.
pub fn validate_hostname(name: &str) -> crate::Result<()> {
    let ok = (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(crate::Error::Other(anyhow::anyhow!(
            "invalid hostname {name:?}: use 1-63 lowercase letters, digits or '-', not starting or ending with '-'"
        )))
    }
}

/// Writes OpenRC's `/etc/conf.d/hostname` and puts the name next to `localhost` in
/// `/etc/hosts`, so the machine can resolve its own name without DNS.
pub async fn apply_hostname(target: &Path, name: &str) -> crate::Result<()> {
    validate_hostname(name)?;
    tokio::fs::create_dir_all(target.join("etc/conf.d")).await?;
    tokio::fs::write(target.join("etc/conf.d/hostname"), format!("hostname=\"{name}\"\n")).await?;
    tokio::fs::write(
        target.join("etc/hosts"),
        format!("127.0.0.1\tlocalhost {name}\n::1\t\tlocalhost {name}\n"),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    fn temp_target(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-locale-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn writes_locale_gen_then_generates_then_sets_lang_then_env_update() {
        let dir = temp_target("order");
        let runner = FakeCommandRunner::new();

        apply(&runner, &dir, &strings(&["uk_UA.UTF-8", "en_US.UTF-8"])).await.unwrap();

        let locale_gen = std::fs::read_to_string(dir.join("etc/locale.gen")).unwrap();
        assert!(locale_gen.contains("uk_UA.UTF-8 UTF-8\n"));
        assert!(locale_gen.contains("en_US.UTF-8 UTF-8\n"));

        // The first locale is the system language, not the last or a sorted one.
        assert_eq!(std::fs::read_to_string(dir.join("etc/env.d/02locale")).unwrap(), "LANG=\"uk_UA.UTF-8\"\n");

        let target = dir.to_str().unwrap();
        let cmds: Vec<(String, Vec<String>)> = runner.calls();
        let names: Vec<&str> = cmds.iter().map(|(c, a)| if c == "chroot" { a[1].as_str() } else { c.as_str() }).collect();
        // /proc, /sys, /dev in; the two chroot steps; the same three out, in reverse.
        assert_eq!(names, ["mount", "mount", "mount", "locale-gen", "env-update", "umount", "umount", "umount"]);
        assert!(cmds.iter().filter(|(c, _)| c == "chroot").all(|(_, a)| a[0] == target));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn duplicate_locales_produce_one_line() {
        let dir = temp_target("dedup");
        apply(&FakeCommandRunner::new(), &dir, &strings(&["en_US.UTF-8", "en_US.UTF-8"])).await.unwrap();
        let locale_gen = std::fs::read_to_string(dir.join("etc/locale.gen")).unwrap();
        assert_eq!(locale_gen.matches("en_US.UTF-8 UTF-8").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn non_utf8_charsets_are_kept_as_given() {
        let dir = temp_target("charset");
        apply(&FakeCommandRunner::new(), &dir, &strings(&["uk_UA.KOI8-U"])).await.unwrap();
        assert!(std::fs::read_to_string(dir.join("etc/locale.gen")).unwrap().contains("uk_UA.KOI8-U KOI8-U\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_bad_entry_aborts_before_touching_the_target_or_running_anything() {
        for bad in ["uk_UA", "uk UA.UTF-8", "uk_UA.UTF-8\nrm -rf /", "", ".UTF-8", "uk_UA."] {
            let dir = temp_target("bad");
            let runner = FakeCommandRunner::new();

            let result = apply(&runner, &dir, &strings(&["en_US.UTF-8", bad])).await;

            assert!(result.is_err(), "{bad:?} should be rejected");
            assert!(runner.calls().is_empty(), "{bad:?}: no command may run");
            assert!(!dir.join("etc/locale.gen").exists(), "{bad:?}: target must stay untouched");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[tokio::test]
    async fn an_empty_selection_is_an_error_not_a_silent_no_op() {
        let dir = temp_target("empty");
        assert!(apply(&FakeCommandRunner::new(), &dir, &[]).await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_mounts_are_undone_even_when_locale_gen_fails() {
        let dir = temp_target("unmountfail");
        let runner = FakeCommandRunner::new();
        runner.fail("chroot", "locale-gen: boom");

        assert!(apply(&runner, &dir, &strings(&["en_US.UTF-8"])).await.is_err());

        let cmds: Vec<String> = runner.calls().into_iter().map(|(c, _)| c).collect();
        assert_eq!(cmds.iter().filter(|c| *c == "mount").count(), 3);
        assert_eq!(cmds.iter().filter(|c| *c == "umount").count(), 3, "a failed build must not leave /proc etc. mounted in the target");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn lang_is_only_written_if_locale_gen_succeeded() {
        let dir = temp_target("genfail");
        let runner = FakeCommandRunner::new();
        runner.fail("chroot", "locale-gen: not found");

        assert!(apply(&runner, &dir, &strings(&["en_US.UTF-8"])).await.is_err());

        // 02locale is the "finished" marker LocalePhase::is_satisfied looks for.
        assert!(!dir.join("etc/env.d/02locale").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hostname_validation() {
        for ok in ["gentoo", "my-laptop", "a", "host1", &"a".repeat(63)] {
            assert!(validate_hostname(ok).is_ok(), "{ok:?} should be valid");
        }
        for bad in ["", "-x", "x-", "My-PC", "has space", "under_score", "dot.ted", "ук", &"a".repeat(64)] {
            assert!(validate_hostname(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    #[tokio::test]
    async fn hostname_lands_in_conf_d_and_hosts() {
        let dir = temp_target("host");
        apply_hostname(&dir, "solomiya-pc").await.unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("etc/conf.d/hostname")).unwrap(), "hostname=\"solomiya-pc\"\n");
        let hosts = std::fs::read_to_string(dir.join("etc/hosts")).unwrap();
        assert!(hosts.contains("127.0.0.1\tlocalhost solomiya-pc"));
        assert!(hosts.contains("::1\t\tlocalhost solomiya-pc"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_invalid_hostname_writes_nothing() {
        let dir = temp_target("hostbad");
        assert!(apply_hostname(&dir, "Bad Name").await.is_err());
        assert!(!dir.join("etc/conf.d/hostname").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
