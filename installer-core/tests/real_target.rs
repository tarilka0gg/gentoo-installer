//! End-to-end checks of the `Locale` and `Users` steps against a REAL unpacked Gentoo
//! stage3, using the real `RealCommandRunner` — the one thing `FakeCommandRunner` cannot
//! tell you (it found that `locale-gen` aborts in a bare chroot, which no unit test did).
//!
//! `#[ignore]`d: needs root, and a stage3 unpacked somewhere. Run with
//!
//! ```text
//! cargo test -p installer-core --no-run          # as your user
//! sudo GENTOO_INSTALLER_STAGE3=/path/to/unpacked-stage3 \
//!      unshare --mount --propagation private \
//!      <that test binary> --ignored --test-threads=1
//! ```
//!
//! `unshare` keeps the /proc, /sys, /dev bind mounts these steps make out of the host's
//! mount table even if a test dies half-way. Each test works on a reflink copy
//! (`cp -a --reflink=always`, so the stage3 must be on btrfs/xfs) and deletes it after.

use installer_core::account::{self, Account};
use installer_core::command::{CommandRunner, RealCommandRunner};
use installer_core::{keyboard, locale, timezone};
use std::path::PathBuf;

/// A throwaway reflink copy of the stage3, removed on drop — including when the test
/// panics half-way, which is exactly when a leftover would otherwise be left behind.
///
/// **Refuses to delete while anything is still mounted under it.** `remove_dir_all`
/// does not stop at mount boundaries: with a `/dev` or `/proc` bind mount still inside
/// the copy it would recurse into the host's real `/dev` and `/proc`. So it first tries
/// `umount -R`, and if the path still shows up in the mount table it leaves the copy in
/// place and says so, rather than risk that.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let src = PathBuf::from(std::env::var("GENTOO_INSTALLER_STAGE3").expect("set GENTOO_INSTALLER_STAGE3 to an unpacked stage3"));
        let dst = src.with_file_name(format!("work-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dst).ok(); // a stale copy from this very pid: never mounted, we own the name
        let status = std::process::Command::new("cp").args(["-a", "--reflink=always"]).arg(&src).arg(&dst).status().unwrap();
        assert!(status.success(), "reflink copy failed");
        Scratch(dst)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn still_mounted(&self) -> bool {
        std::fs::read_to_string("/proc/self/mountinfo").map(|m| m.contains(self.0.to_str().unwrap())).unwrap_or(true)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.still_mounted() {
            for sub in ["dev", "sys", "proc"] {
                std::process::Command::new("umount").arg("-R").arg(self.0.join(sub)).status().ok();
            }
        }
        if self.still_mounted() {
            eprintln!("NOT deleting {}: something is still mounted under it", self.0.display());
            return;
        }
        std::fs::remove_dir_all(&self.0).ok();
    }
}

async fn in_target(target: &std::path::Path, script: &str) -> String {
    RealCommandRunner
        .run("chroot", &[target.to_str().unwrap(), "/bin/bash", "-c", script])
        .await
        .unwrap_or_else(|e| panic!("in-target `{script}` failed: {e}"))
}

#[tokio::test]
#[ignore = "needs root and GENTOO_INSTALLER_STAGE3"]
async fn locale_really_generates_and_becomes_the_login_language() {
    let scratch = Scratch::new("locale");
    let target = scratch.path().to_path_buf();
    let locales = vec!["uk_UA.UTF-8".to_string(), "en_US.UTF-8".to_string()];

    locale::apply(&RealCommandRunner, &target, &locales).await.unwrap();

    // Asked of the target's own tools, not of the files we wrote.
    let generated = in_target(&target, "locale -a").await;
    assert!(generated.contains("uk_UA.utf8"), "uk_UA missing from `locale -a`: {generated}");
    assert!(generated.contains("en_US.utf8"), "en_US missing from `locale -a`: {generated}");
    assert_eq!(in_target(&target, "source /etc/profile; echo $LANG").await.trim(), "uk_UA.UTF-8");

    // The bind mounts must be gone: nothing of the copy may remain mounted.
    assert!(!scratch.still_mounted(), "target still mounted after apply()");
}

#[tokio::test]
#[ignore = "needs root and GENTOO_INSTALLER_STAGE3"]
async fn hostname_timezone_keyboard_land_where_openrc_reads_them() {
    let scratch = Scratch::new("misc");
    let target = scratch.path().to_path_buf();

    locale::apply_hostname(&target, "solomiya-pc").await.unwrap();
    timezone::apply(&target, "Europe/Kyiv").await.unwrap();
    keyboard::apply(&target, "ua").await.unwrap();

    assert_eq!(in_target(&target, ". /etc/conf.d/hostname; echo $hostname").await.trim(), "solomiya-pc");
    assert_eq!(in_target(&target, ". /etc/conf.d/keymaps; echo $keymap").await.trim(), "ua");
    // The relative symlink must resolve INSIDE the target, not against the host.
    assert_eq!(in_target(&target, "TZ= date -d @0 +%Z; readlink -f /etc/localtime").await.lines().last().unwrap(), "/usr/share/zoneinfo/Europe/Kyiv");
}

#[tokio::test]
#[ignore = "needs root and GENTOO_INSTALLER_STAGE3"]
async fn account_is_really_created_in_wheel_with_a_working_hash_and_root_stays_locked() {
    let scratch = Scratch::new("account");
    let target = scratch.path().to_path_buf();
    // A password that starts with '-' is the case that used to break the account step.
    let acct = Account { username: "solomiya".into(), password: "-hunter2 with spaces".into() };

    account::create(&RealCommandRunner, &target, &acct).await.unwrap();
    account::configure_privilege(&target).await.unwrap();

    assert!(in_target(&target, "id -nG solomiya").await.split_whitespace().any(|g| g == "wheel"));
    assert_eq!(in_target(&target, "getent passwd solomiya | cut -d: -f6,7").await.trim(), "/home/solomiya:/bin/bash");
    assert!(std::path::Path::new(&target).join("home/solomiya").is_dir(), "-m must create the home dir");

    // The stored hash verifies against the password we gave — checked with the target's own openssl.
    let shadow = in_target(&target, "getent shadow solomiya | cut -d: -f2").await;
    let hash = shadow.trim();
    assert!(hash.starts_with("$6$"), "not SHA-512 crypt: {hash}");
    let check = RealCommandRunner.run("openssl", &["passwd", "-6", "-salt", hash.split('$').nth(2).unwrap(), "--", &acct.password]).await.unwrap();
    assert_eq!(check.trim(), hash, "password does not verify against the stored hash");

    // Root is still locked, as designed.
    assert_eq!(in_target(&target, "getent shadow root | cut -d: -f2").await.trim(), "*");
}
