//! First-user account creation. Uses `useradd`/`usermod`'s `-R <target>` (GNU
//! shadow-utils) rather than a chroot — plain root-relative operation, no bind mounts
//! needed. Root itself stays locked; this user gets `wheel`.

use crate::command::CommandRunner;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Account {
    pub username: String,
    pub password: String,
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
                "/bin/bash",
                "-p",
                hash,
                &account.username,
            ],
        )
        .await?;

    Ok(())
}

/// `wheel` members may run anything as root through `doas`, asking for their own
/// password once per session (`persist`).
pub const DOAS_CONF: &str = "permit persist :wheel\n";

/// Writes `/etc/doas.conf` so the `wheel` group [`create`] puts the user in actually
/// means something.
///
/// Root stays locked (no password is ever set for it), so without an escalation tool
/// the installed system has a user that can never become root. **This only writes the
/// config**: `app-admin/doas` still has to be installed in the target, and nothing in
/// the phase pipeline does that yet (see the README's "Not yet done"). `doas` refuses a
/// config that other users can write, hence `0400`.
pub async fn configure_privilege(target: &Path) -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let conf = target.join("etc/doas.conf");
    tokio::fs::create_dir_all(target.join("etc")).await?;
    tokio::fs::write(&conf, DOAS_CONF).await?;
    tokio::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o400)).await?;
    Ok(())
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
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "permit persist :wheel\n");
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
    fn valid_usernames() {
        for ok in ["solomiya", "_svc", "user-1", "a", &"a".repeat(32)] {
            assert!(validate_username(ok).is_ok(), "{ok:?}");
        }
    }
}
