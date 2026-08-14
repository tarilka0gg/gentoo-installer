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

/// Creates `account.username` in the target with `account.password`, in `wheel`, with
/// `/bin/bash`. The password is hashed via `openssl passwd -6` (SHA-512 crypt) and handed
/// to `useradd -p` rather than piped to `chpasswd` over stdin — `CommandRunner` has no
/// stdin support yet, and passing a pre-computed hash sidesteps needing it.
pub async fn create(runner: &dyn CommandRunner, target: &Path, account: &Account) -> crate::Result<()> {
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    let hash = runner.run("openssl", &["passwd", "-6", &account.password]).await?;
    let hash = hash.trim();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn issues_useradd_with_hashed_password_and_root_flag() {
        let runner = FakeCommandRunner::new();
        runner.respond("openssl", "$6$rounds=5000$abc$hashedvalue\n");

        let account = Account { username: "solomiya".into(), password: "hunter2".into() };
        create(&runner, Path::new("/mnt/gentoo"), &account).await.unwrap();

        runner.assert_call(0, "openssl", &["passwd", "-6", "hunter2"]);
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
}
