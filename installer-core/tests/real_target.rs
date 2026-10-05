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
        Self::from_env("GENTOO_INSTALLER_STAGE3", tag)
    }

    fn from_env(var: &str, tag: &str) -> Self {
        let src = PathBuf::from(std::env::var(var).unwrap_or_else(|_| panic!("set {var} to an unpacked stage3")));
        let dst = src.with_file_name(format!("work-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dst).ok(); // a stale copy from this very pid: never mounted, we own the name
        let status = std::process::Command::new("cp").args(["-a", "--reflink=always"]).arg(&src).arg(&dst).status().unwrap();
        assert!(status.success(), "reflink copy failed");
        Scratch(dst)
    }

    /// An empty directory next to the stage3 (same filesystem), for tests that unpack into it.
    fn empty(tag: &str) -> Self {
        let src = PathBuf::from(std::env::var("GENTOO_INSTALLER_STAGE3").expect("set GENTOO_INSTALLER_STAGE3"));
        let dst = src.with_file_name(format!("work-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dst).ok();
        std::fs::create_dir_all(&dst).unwrap();
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

const PTY_DOAS: &str = include_str!("pty_doas.py");

/// Bind-mounts /proc, /sys, /dev into the copy so a pty (and python) work inside it.
/// `Scratch`'s drop guard unmounts them again even if the test panics.
fn mount_dev(target: &std::path::Path) {
    for d in ["proc", "sys", "dev"] {
        let dir = target.join(d);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [vec!["--rbind", &format!("/{d}"), dir.to_str().unwrap()], vec!["--make-rslave", dir.to_str().unwrap()]] {
            assert!(std::process::Command::new("mount").args(&args).status().unwrap().success(), "mount {args:?}");
        }
    }
}

fn umount_dev(target: &std::path::Path) {
    for d in ["dev", "sys", "proc"] {
        std::process::Command::new("umount").arg("-R").arg(target.join(d)).status().ok();
    }
}

/// Runs `doas -u root id -un` as `user` on a pty inside the target, returns the script's report.
async fn try_doas(target: &std::path::Path, user: &str, password: &str) -> String {
    std::fs::write(target.join("root/pty_doas.py"), PTY_DOAS).unwrap();
    mount_dev(target);
    let out = RealCommandRunner
        .run("chroot", &[target.to_str().unwrap(), "python3", "/root/pty_doas.py", user, password])
        .await;
    umount_dev(target);
    out.unwrap_or_else(|e| panic!("pty script failed: {e}"))
}

/// The whole point of `Users`: root is locked, so the account is only useful if doas is
/// really installed and the wheel rule really works. Needs network (source tarball) and a
/// stage3 with a SYNCED Portage tree and no doas (GENTOO_INSTALLER_STAGE3_SYNCED).
#[tokio::test]
#[ignore = "needs root, network and GENTOO_INSTALLER_STAGE3_SYNCED"]
async fn wheel_user_really_becomes_root_through_the_installed_doas() {
    let scratch = Scratch::from_env("GENTOO_INSTALLER_STAGE3_SYNCED", "doas");
    let target = scratch.path().to_path_buf();
    assert!(!target.join("usr/bin/doas").exists(), "the starting stage3 must not already have doas");

    let acct = Account { username: "solomiya".into(), password: "correct horse".into() };
    account::create(&RealCommandRunner, &target, &acct).await.unwrap();
    account::configure_privilege(&target).await.unwrap();
    account::install_doas(&RealCommandRunner, &target).await.unwrap();
    assert!(!scratch.still_mounted(), "install_doas must unmount what it mounted");

    // setuid root, with its PAM service file
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(target.join("usr/bin/doas")).unwrap().permissions().mode();
    assert!(mode & 0o4000 != 0, "doas must be setuid, mode {mode:o}");
    assert!(target.join("etc/pam.d/doas").is_file());

    let ok = try_doas(&target, "solomiya", "correct horse").await;
    assert!(ok.contains("RAN_AS_ROOT=yes"), "wheel user could not become root: {ok}");
    assert!(ok.contains("PROMPTS=1"), "expected exactly one password prompt: {ok}");

    // A wrong password must NOT work (guards against a config that lets everyone in).
    let bad = try_doas(&target, "solomiya", "wrong password").await;
    assert!(bad.contains("RAN_AS_ROOT=no"), "wrong password got root: {bad}");

    // Root itself is still locked.
    assert_eq!(in_target(&target, "getent shadow root | cut -d: -f2").await.trim(), "*");
}

/// A user who is NOT in wheel must not get root even with the right password.
#[tokio::test]
#[ignore = "needs root, network and GENTOO_INSTALLER_STAGE3_SYNCED"]
async fn a_user_outside_wheel_does_not_get_root() {
    let scratch = Scratch::from_env("GENTOO_INSTALLER_STAGE3_SYNCED", "nowheel");
    let target = scratch.path().to_path_buf();

    account::create(&RealCommandRunner, &target, &Account { username: "solomiya".into(), password: "pw one".into() }).await.unwrap();
    account::configure_privilege(&target).await.unwrap();
    account::install_doas(&RealCommandRunner, &target).await.unwrap();

    // Second account created by hand, deliberately without -G wheel.
    let hash = RealCommandRunner.run("openssl", &["passwd", "-6", "--", "pw two"]).await.unwrap();
    RealCommandRunner
        .run_status("useradd", &["-R", target.to_str().unwrap(), "-m", "-p", hash.trim(), "guest"])
        .await
        .unwrap();

    let out = try_doas(&target, "guest", "pw two").await;
    assert!(out.contains("RAN_AS_ROOT=no"), "a non-wheel user got root: {out}");
}

/// The custom-stage path, end to end with the real tools: the installer's own `download`
/// (local file + SHA512) and `unpack` (`tar --numeric-owner --xattrs-include`), then
/// `account::create`, which must give the user fish because this stage has it.
/// Needs GENTOO_INSTALLER_CUSTOM_STAGE3=<tarball> (and optionally its _SHA512).
#[tokio::test]
#[ignore]
async fn a_custom_stage_is_fetched_verified_unpacked_and_gives_the_user_fish() {
    use installer_core::stage3::{self, Stage3Source};
    let tarball = std::env::var("GENTOO_INSTALLER_CUSTOM_STAGE3").expect("set GENTOO_INSTALLER_CUSTOM_STAGE3");
    let sha = std::env::var("GENTOO_INSTALLER_CUSTOM_STAGE3_SHA512").ok();
    let scratch = Scratch::empty("custom-stage");
    let runner = RealCommandRunner;

    // A wrong digest is refused before anything is unpacked.
    let copy = std::env::temp_dir().join(format!("gi-custom-{}.tar.xz", std::process::id()));
    let bad = Stage3Source::custom(format!("file://{tarball}"), Some("00".repeat(64)));
    assert!(stage3::download(&bad, &copy).await.unwrap_err().to_string().contains("sha512 mismatch"));

    stage3::download(&Stage3Source::custom(format!("file://{tarball}"), sha), &copy).await.unwrap();
    stage3::unpack(&runner, &copy, scratch.path()).await.unwrap();
    std::fs::remove_file(&copy).ok();

    assert!(scratch.path().join("usr/bin/fish").is_file());
    assert!(!scratch.path().join("usr/bin/nano").exists(), "the custom stage has no nano");

    account::create(&runner, scratch.path(), &Account { username: "solomiya".into(), password: "hunter2".into() })
        .await
        .unwrap();
    let passwd = std::fs::read_to_string(scratch.path().join("etc/passwd")).unwrap();
    let line = passwd.lines().find(|l| l.starts_with("solomiya:")).expect("user created");
    assert!(line.ends_with(":/usr/bin/fish"), "{line}");
}

/// `services::enable` against a real stage3: the init script exists (`sshd` ships with
/// openssh), the runlevel symlink appears, a repeat is harmless, a missing service fails
/// instead of pretending.
#[tokio::test]
#[ignore]
async fn rc_update_really_enables_a_service_in_the_target_and_refuses_a_missing_one() {
    use installer_core::services;
    let scratch = Scratch::new("services");
    let runner = RealCommandRunner;

    services::enable_all(&runner, scratch.path(), &["sshd"]).await.unwrap();
    let link = scratch.path().join("etc/runlevels/default/sshd");
    assert!(link.symlink_metadata().is_ok(), "no runlevel symlink at {}", link.display());

    services::enable_all(&runner, scratch.path(), &["sshd"]).await.expect("a second run must be harmless");

    let err = services::enable_all(&runner, scratch.path(), &["no-such-service"]).await.unwrap_err();
    eprintln!("missing service error: {err}");
}
