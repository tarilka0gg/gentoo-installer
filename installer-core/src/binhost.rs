//! The project's own binary package host.
//!
//! Gentoo's official binhost has most of a desktop, but not `niri`, `noctalia`, `fish`, `micro` and every package whose USE
//! the installer changes; without a binary Portage compiles it (`mesa` and `llvm` pull in hours). `iso/binhost/build.sh`
//! builds exactly those packages in a clean stage3 with the Portage configuration in `assets/binhost/`, signs them, and they are
//! published as a second binhost. This module points the installed system at it: a `binrepos.conf` entry with a higher
//! priority than Gentoo's, the signing key trusted the way `getuto` trusts Gentoo's, and the USE flags and keywords the packages
//! were built with, so Portage finds them usable (a binary with other flags is ignored and compiled again).
//!
//! Tested on a clean stage3: 139 packages for niri, noctalia and ghostty, all binary, signatures verified.

use crate::chroot_emerge::{bind_mount_chroot_dirs, unmount_chroot_dirs, write_portage_entry};
use crate::command::CommandRunner;
use std::path::Path;

/// Where the packages are published (a release of tarilka0gg/simple-linux-binhost). `GENTOO_BINHOST_URL` overrides it
/// (a local test server, a mirror); set empty or `0` to not use it at all.
pub const DEFAULT_URL: &str =
    "https://github.com/tarilka0gg/simple-linux-binhost/releases/download/binhost";
/// Fingerprint of the key the packages are signed with (`assets/binhost/binhost-signing.asc`).
pub const KEY_FINGERPRINT: &str = "F6307025824C538E8C43844590899877B0925EC1";

const KEY: &str = include_str!("../assets/binhost/binhost-signing.asc");
const PACKAGE_USE: &str = include_str!("../assets/binhost/package.use");
const PACKAGE_ACCEPT_KEYWORDS: &str = include_str!("../assets/binhost/package.accept_keywords");
const PACKAGE_LICENSE: &str = include_str!("../assets/binhost/package.license");

const ENTRY_NAME: &str = "simple-linux-binhost";
const KEY_PATH: &str = "etc/portage/simple-linux-binhost.asc";

/// The binhost to use, from `GENTOO_BINHOST_URL` (default [`DEFAULT_URL`]); `None` when it is switched off.
pub fn url_from_env() -> Option<String> {
    match std::env::var("GENTOO_BINHOST_URL") {
        Ok(v) if v.trim().is_empty() || v.trim() == "0" => None,
        Ok(v) => Some(v.trim().trim_end_matches('/').to_string()),
        Err(_) => Some(DEFAULT_URL.to_string()),
    }
}

/// `binrepos.conf` entry. Priority 10 beats the stage3's `[gentoo]` (1): a package we built is preferred, anything else comes
/// from Gentoo's own binhost. Signatures are always verified.
pub fn binrepos_conf(url: &str) -> String {
    format!("[simple-linux]\npriority = 10\nsync-uri = {url}\nverify-signature = true\n")
}

/// Is the binhost already set up in `target`?
pub fn configured(target: &Path) -> bool {
    target
        .join("etc/portage/binrepos.conf/simple-linux.conf")
        .is_file()
        && target.join(KEY_PATH).is_file()
}

/// Writes the entry, the key, and the build's USE/keywords/licences, then trusts the key in Portage's own keyring.
pub async fn configure(runner: &dyn CommandRunner, target: &Path, url: &str) -> crate::Result<()> {
    let portage = target.join("etc/portage");
    let binrepos = portage.join("binrepos.conf");
    tokio::fs::create_dir_all(&binrepos).await?;
    tokio::fs::write(binrepos.join("simple-linux.conf"), binrepos_conf(url)).await?;
    tokio::fs::write(target.join(KEY_PATH), KEY).await?;
    for (dir, body) in [
        ("package.use", PACKAGE_USE),
        ("package.accept_keywords", PACKAGE_ACCEPT_KEYWORDS),
        ("package.license", PACKAGE_LICENSE),
    ] {
        write_portage_entry(&portage.join(dir), ENTRY_NAME, body).await?;
    }

    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    bind_mount_chroot_dirs(runner, target).await?;
    let result = async {
        // getuto creates Portage's keyring and trusts Gentoo's release key; ours is added the same way. Importing the key is
        // not enough: without the trust database being recalculated gpg reports a good signature from an untrusted key and
        // Portage refuses the package.
        let gpg = "gpg --homedir /etc/portage/gnupg --batch";
        runner
            .run_status("chroot", &[target_str, "getuto"])
            .await?;
        runner
            .run_status(
                "chroot",
                &[
                    target_str,
                    "sh",
                    "-c",
                    &format!(
                        "{gpg} --import /{KEY_PATH} && echo '{KEY_FINGERPRINT}:6:' | {gpg} --import-ownertrust && {gpg} --check-trustdb"
                    ),
                ],
            )
            .await
    }
    .await;
    unmount_chroot_dirs(runner, target).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[test]
    fn the_entry_outranks_gentoos_and_verifies_signatures() {
        let c = binrepos_conf("https://x.invalid/b");
        assert!(c.contains("priority = 10"));
        assert!(c.contains("sync-uri = https://x.invalid/b\n"));
        assert!(c.contains("verify-signature = true"));
    }

    #[test]
    fn the_url_can_be_overridden_or_switched_off() {
        // Not run in parallel with itself: the variable is process-wide, and only this test touches it.
        std::env::remove_var("GENTOO_BINHOST_URL");
        assert_eq!(url_from_env().as_deref(), Some(DEFAULT_URL));
        std::env::set_var("GENTOO_BINHOST_URL", "http://10.0.2.2:8765/");
        assert_eq!(url_from_env().as_deref(), Some("http://10.0.2.2:8765"));
        std::env::set_var("GENTOO_BINHOST_URL", "0");
        assert_eq!(url_from_env(), None);
        std::env::remove_var("GENTOO_BINHOST_URL");
    }

    #[tokio::test]
    async fn configure_writes_the_flags_the_binaries_were_built_with_and_trusts_the_key() {
        let t = std::env::temp_dir().join(format!("gi-binhost-{}", std::process::id()));
        std::fs::remove_dir_all(&t).ok();
        // a stage3 ships these as (empty) directories
        for d in ["package.use", "package.accept_keywords", "package.license"] {
            std::fs::create_dir_all(t.join("etc/portage").join(d)).unwrap();
        }
        let runner = FakeCommandRunner::new();
        configure(&runner, &t, "http://h.invalid").await.unwrap();

        assert!(configured(&t));
        let use_file =
            std::fs::read_to_string(t.join("etc/portage/package.use/simple-linux-binhost"))
                .unwrap();
        assert!(
            use_file.contains(">=media-libs/mesa-26.1.8 wayland"),
            "{use_file}"
        );
        assert!(t
            .join("etc/portage/package.accept_keywords/simple-linux-binhost")
            .is_file());
        let calls = runner.calls();
        let chroot: Vec<_> = calls.iter().filter(|c| c.0 == "chroot").collect();
        assert!(chroot[0].1.contains(&"getuto".to_string()), "{chroot:?}");
        let trust = chroot[1].1.last().unwrap();
        assert!(
            trust.contains("--import-ownertrust") && trust.contains("--check-trustdb"),
            "{trust}"
        );
        assert!(trust.contains(KEY_FINGERPRINT));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn the_bundled_key_is_the_one_the_fingerprint_names() {
        // A public key block is the only thing in the file; the fingerprint is checked against gpg when it is available.
        assert!(KEY.contains("BEGIN PGP PUBLIC KEY BLOCK"));
        if let Ok(out) = std::process::Command::new("gpg")
            .args([
                "--show-keys",
                "--with-colons",
                "--homedir",
                "/nonexistent-gi",
                "--no-default-keyring",
            ])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/binhost/binhost-signing.asc"
            ))
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("fpr") {
                assert!(text.contains(KEY_FINGERPRINT), "{text}");
            }
        }
    }
}
