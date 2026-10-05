//! First-user account creation. Uses `useradd`/`usermod`'s `-R <target>` (GNU
//! shadow-utils) rather than a chroot — plain root-relative operation, no bind mounts
//! needed. Root itself stays locked; this user gets `wheel`.

use crate::chroot_emerge::{bind_mount_chroot_dirs, ensure_network_resolves, ensure_portage_tree, unmount_chroot_dirs};
use crate::command::CommandRunner;
use std::path::Path;

#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub password: String,
}

// Not derived: a derived `Debug` would print the password into any log or panic message.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").field("username", &self.username).field("password", &"<hidden>").finish()
    }
}

/// Login names `useradd` accepts on Gentoo, and nothing looser: a leading letter or `_`,
/// then letters/digits/`_`/`-`, at most 32 chars. The name goes straight into `useradd`'s
/// argv, so this also keeps a name like `-o` from being read as an option.
pub fn validate_username(name: &str) -> crate::Result<()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if first_ok && rest_ok && name.len() <= 32 {
        Ok(())
    } else {
        Err(crate::Error::Other(anyhow::anyhow!(
            "invalid username {name:?}: use up to 32 lowercase letters, digits, '_' or '-', starting with a letter or '_'"
        )))
    }
}

/// `/usr/bin/fish` if the target's stage has it (a custom stage), else `/bin/bash`. Checked
/// against the target rather than assumed: `useradd -s` with a missing shell gives an
/// account that cannot log in.
pub fn login_shell(target: &Path) -> &'static str {
    if target.join("usr/bin/fish").is_file() {
        "/usr/bin/fish"
    } else {
        "/bin/bash"
    }
}

/// Creates `account.username` in the target with `account.password`, in `wheel`, with
/// `/bin/bash`. The password is hashed via `openssl passwd -6` (SHA-512 crypt) and handed
/// to `useradd -p` rather than piped to `chpasswd` over stdin — `CommandRunner` has no
/// stdin support yet, and passing a pre-computed hash sidesteps needing it.
///
/// The `--` before the password matters: without it a password that starts with `-`
/// (say `-x`) is parsed by `openssl` as an option and the account step fails, and one
/// like `-help` makes it print usage text to stdout with a zero exit — which used to be
/// taken for the hash and handed to `useradd -p`. The hash is checked for the `$6$`
/// prefix for the same reason: never pass whatever `openssl` happened to print on.
pub async fn create(runner: &dyn CommandRunner, target: &Path, account: &Account) -> crate::Result<()> {
    validate_username(&account.username)?;
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    let hash = runner.run("openssl", &["passwd", "-6", "--", &account.password]).await?;
    let hash = hash.trim();
    if !hash.starts_with("$6$") || hash.contains(char::is_whitespace) {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "openssl did not produce a SHA-512 crypt hash; refusing to create the account"
        )));
    }

    runner
        .run_status(
            "useradd",
            &[
                "-R",
                target_str,
                "-m",
                "-G",
                "wheel",
                "-s",
                login_shell(target),
                "-p",
                hash,
                &account.username,
            ],
        )
        .await?;

    Ok(())
}

/// `wheel` members may run anything as root through `doas`, asking for their own
/// password each time.
///
/// Deliberately no `persist`: Gentoo builds `app-admin/doas` with `-persist` by default
/// (checked: `USE="pam -persist"`), where the keyword is accepted and silently does
/// nothing, and even with `USE=persist` the password was asked on every call in a
/// chroot test (no `/run/doas` for the timestamps). A rule that promises "once per
/// session" and delivers "every time" is worse than one that says what it does.
pub const DOAS_CONF: &str = "permit :wheel\n";

/// Writes `/etc/doas.conf` so the `wheel` group [`create`] puts the user in actually
/// means something. Pair with [`install_doas`], which puts the binary there.
///
/// Root stays locked (`*` in `/etc/shadow`; a stage3 ships neither `doas` nor `sudo`),
/// so without an escalation tool the installed system has a user that can never become
/// root. `doas` refuses to trust a config other users can write, hence `0400`.
pub async fn configure_privilege(target: &Path) -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let conf = target.join("etc/doas.conf");
    tokio::fs::create_dir_all(target.join("etc")).await?;
    tokio::fs::write(&conf, DOAS_CONF).await?;
    tokio::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o400)).await?;
    Ok(())
}

/// Emerges `app-admin/doas` in the target, using the same chroot bootstrap as `wm` and
/// `gpu_driver` (bind mounts, resolver, Portage tree). `--noreplace` makes a second run
/// a no-op.
///
/// Verified on a real stage3 + synced tree: emerges cleanly with default USE
/// (`pam -persist`), installs a setuid `/usr/bin/doas` and `/etc/pam.d/doas`, and a
/// `wheel` user can then run commands as root after entering their password. Needs
/// network (the tree sync and the source tarball). The mounts are undone on failure too.
pub async fn install_doas(runner: &dyn CommandRunner, target: &Path) -> crate::Result<()> {
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    let mounted = bind_mount_chroot_dirs(runner, target).await;
    let result = match mounted {
        Ok(()) => async {
            ensure_network_resolves(target).await?;
            ensure_portage_tree(runner, target, target_str).await?;
            runner.run_status("chroot", &[target_str, "emerge", "--noreplace", "app-admin/doas"]).await
        }
        .await,
        Err(e) => Err(e),
    };
    unmount_chroot_dirs(runner, target).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn doas_conf_lets_wheel_in_and_is_not_writable_by_anyone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("gentoo-installer-doas-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        configure_privilege(&dir).await.unwrap();

        let conf = dir.join("etc/doas.conf");
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "permit :wheel\n");
        assert_eq!(std::fs::metadata(&conf).unwrap().permissions().mode() & 0o777, 0o400);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn issues_useradd_with_hashed_password_and_root_flag() {
        let runner = FakeCommandRunner::new();
        runner.respond("openssl", "$6$rounds=5000$abc$hashedvalue\n");

        let account = Account { username: "solomiya".into(), password: "hunter2".into() };
        create(&runner, Path::new("/mnt/gentoo"), &account).await.unwrap();

        runner.assert_call(0, "openssl", &["passwd", "-6", "--", "hunter2"]);
        runner.assert_call(
            1,
            "useradd",
            &[
                "-R",
                "/mnt/gentoo",
                "-m",
                "-G",
                "wheel",
                "-s",
                "/bin/bash",
                "-p",
                "$6$rounds=5000$abc$hashedvalue",
                "solomiya",
            ],
        );
    }

    #[tokio::test]
    async fn a_password_starting_with_a_dash_is_not_parsed_as_an_option() {
        let runner = FakeCommandRunner::new();
        runner.respond("openssl", "$6$salt$hash\n");
        let account = Account { username: "solomiya".into(), password: "-help".into() };

        create(&runner, Path::new("/mnt/gentoo"), &account).await.unwrap();

        runner.assert_call(0, "openssl", &["passwd", "-6", "--", "-help"]);
    }

    #[tokio::test]
    async fn output_that_is_not_a_sha512_hash_is_refused() {
        for garbage in ["Usage: passwd [options] [password]\n", "", "$1$md5$hash\n"] {
            let runner = FakeCommandRunner::new();
            runner.respond("openssl", garbage);
            let account = Account { username: "solomiya".into(), password: "x".into() };

            assert!(create(&runner, Path::new("/mnt/gentoo"), &account).await.is_err(), "{garbage:?}");
            assert!(runner.calls().iter().all(|(cmd, _)| cmd != "useradd"), "useradd must not run for {garbage:?}");
        }
    }

    #[tokio::test]
    async fn a_bad_username_runs_nothing() {
        for bad in ["", "-o", "Solomiya", "has space", "x;y", "1abc", &"a".repeat(33)] {
            let runner = FakeCommandRunner::new();
            let account = Account { username: bad.to_string(), password: "pw".into() };

            assert!(create(&runner, Path::new("/mnt/gentoo"), &account).await.is_err(), "{bad:?}");
            assert!(runner.calls().is_empty(), "{bad:?}: no command may run");
        }
    }

    #[test]
    fn login_shell_is_fish_only_when_the_stage_has_it() {
        let dir = std::env::temp_dir().join(format!("gi-shell-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        assert_eq!(login_shell(&dir), "/bin/bash");
        std::fs::write(dir.join("usr/bin/fish"), "").unwrap();
        assert_eq!(login_shell(&dir), "/usr/bin/fish");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn valid_usernames() {
        for ok in ["solomiya", "_svc", "user-1", "a", &"a".repeat(32)] {
            assert!(validate_username(ok).is_ok(), "{ok:?}");
        }
    }

    fn temp_target(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-doas-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/resolv.conf"), "nameserver 127.0.0.1\n").ok();
        dir
    }

    fn names(runner: &FakeCommandRunner) -> Vec<String> {
        runner.calls().iter().map(|(c, a)| if c == "chroot" { format!("chroot:{}", a[1..].join(" ")) } else { c.clone() }).collect()
    }

    #[tokio::test]
    async fn install_doas_syncs_the_tree_if_missing_then_emerges_noreplace_and_unmounts() {
        let dir = temp_target("fresh");
        let runner = FakeCommandRunner::new();

        install_doas(&runner, &dir).await.unwrap();

        assert_eq!(
            names(&runner),
            ["mount", "mount", "mount", "chroot:emerge-webrsync", "chroot:emerge --noreplace app-admin/doas", "umount", "umount", "umount"]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn install_doas_skips_the_tree_sync_when_the_tree_already_exists() {
        let dir = temp_target("synced");
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap();
        let runner = FakeCommandRunner::new();

        install_doas(&runner, &dir).await.unwrap();

        assert!(!names(&runner).iter().any(|n| n == "chroot:emerge-webrsync"), "{:?}", names(&runner));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn install_doas_unmounts_even_when_emerge_fails() {
        let dir = temp_target("fail");
        let runner = FakeCommandRunner::new();
        runner.fail("chroot", "emerge: network is unreachable");

        assert!(install_doas(&runner, &dir).await.is_err());

        let n = names(&runner);
        assert_eq!(n.iter().filter(|c| *c == "mount").count(), 3);
        assert_eq!(n.iter().filter(|c| *c == "umount").count(), 3, "{n:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn debug_output_never_contains_the_password() {
        let a = Account { username: "solomiya".into(), password: "hunter2-secret".into() };
        let shown = format!("{a:?}");
        assert!(shown.contains("solomiya") && !shown.contains("hunter2"), "{shown}");
    }
}
