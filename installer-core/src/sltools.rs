//! What the installer leaves on the target for later: the `simple-linux-update` tool and, when asked for, Secure Boot with a key
//! made on this machine.
//!
//! Secure Boot here is the same chain as on the live images (firmware `db` → Limine → `limine.conf` → kernel, the config and kernel
//! pinned by BLAKE2b), but signed with a key generated for this installation instead of the author's. The private key stays in
//! `/etc/simple-linux/secureboot` (root only, **not** encrypted: the installer has no LUKS), so this stops someone swapping the
//! kernel or the config on the disk, not someone who can read the disk. The user still has to enrol `db.cer` in the firmware.

use crate::command::CommandRunner;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub const UPDATE_SH: &str = include_str!("../assets/simple-linux-update.sh");
pub const SB_SIGN_SH: &str = include_str!("../assets/sb-sign.sh");

/// Where the target keeps the Secure Boot key and the unsigned originals (paths relative to the target root).
pub const KEYS_DIR: &str = "etc/simple-linux/secureboot";
pub const PLAIN_CONF: &str = "etc/simple-linux/limine.conf";
pub const PRISTINE_EFI: &str = "usr/local/share/simple-linux/BOOTX64.EFI";
pub const SB_SIGN_PATH: &str = "usr/local/lib/simple-linux/sb-sign";

/// The tools Secure Boot signing needs on the live system.
pub const SB_TOOLS: &[&str] = &["limine", "sbsign", "sbverify", "b2sum", "openssl"];

/// `/etc/simple-linux/update.conf`: what `simple-linux-update` needs to know about this machine.
pub fn update_conf(binhost_url: &str, combo: &str) -> String {
    format!("BINHOST={binhost_url}\nCOMBO={combo}\n")
}

async fn write_exec(path: &Path, text: &str) -> crate::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(path, text).await?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

/// Installs `simple-linux-update`, the signing helper and `update.conf` into `target`.
pub async fn install_update_tool(
    target: &Path,
    binhost_url: &str,
    combo: &str,
) -> crate::Result<()> {
    write_exec(
        &target.join("usr/local/sbin/simple-linux-update"),
        UPDATE_SH,
    )
    .await?;
    write_exec(&target.join(SB_SIGN_PATH), SB_SIGN_SH).await?;
    let conf = target.join("etc/simple-linux/update.conf");
    tokio::fs::create_dir_all(conf.parent().expect("has a parent")).await?;
    tokio::fs::write(conf, update_conf(binhost_url, combo)).await?;
    Ok(())
}

/// Names from [`SB_TOOLS`] that the live system lacks. Checked at the start, so a missing tool fails the install in Preflight
/// rather than at the bootloader step an hour in.
pub async fn missing_tools(runner: &dyn CommandRunner) -> Vec<String> {
    let mut missing = Vec::new();
    for tool in SB_TOOLS {
        if runner
            .run("sh", &["-c", &format!("command -v {tool}")])
            .await
            .map(|o| o.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push((*tool).to_string());
        }
    }
    missing
}

/// Generates this machine's `db` key (once), keeps the unsigned Limine and config for later re-signing, signs, and puts `db.cer`
/// on the ESP (`/boot/secureboot/`) where the firmware's file picker can find it. Run after `bootloader::install` on UEFI.
pub async fn setup_secure_boot(runner: &dyn CommandRunner, target: &Path) -> crate::Result<()> {
    let keys = target.join(KEYS_DIR);
    tokio::fs::create_dir_all(&keys).await?;
    tokio::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700)).await?;
    let s = |p: &Path| -> crate::Result<String> {
        p.to_str()
            .map(str::to_string)
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 path {}", p.display())))
    };
    if !keys.join("db.key").is_file() {
        let (key, crt, cer) = (
            s(&keys.join("db.key"))?,
            s(&keys.join("db.crt"))?,
            s(&keys.join("db.cer"))?,
        );
        runner
            .run_status(
                "openssl",
                &[
                    "req",
                    "-new",
                    "-x509",
                    "-newkey",
                    "rsa:3072",
                    "-nodes",
                    "-sha256",
                    "-days",
                    "3650",
                    "-subj",
                    "/CN=Simple Linux Secure Boot db (this machine)/",
                    "-keyout",
                    &key,
                    "-out",
                    &crt,
                ],
            )
            .await?;
        runner
            .run_status(
                "openssl",
                &["x509", "-in", &crt, "-outform", "DER", "-out", &cer],
            )
            .await?;
        tokio::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).await?;
    }

    let boot = target.join("boot");
    let efi = boot.join("EFI/BOOT/BOOTX64.EFI");
    let conf = boot.join("limine.conf");
    // Unsigned originals: signing rewrites both, and a re-sign after a kernel update starts from these.
    let pristine = target.join(PRISTINE_EFI);
    let plain = target.join(PLAIN_CONF);
    for (from, to) in [(&efi, &pristine), (&conf, &plain)] {
        if let Some(dir) = to.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::copy(from, to).await?;
    }
    write_exec(&target.join(SB_SIGN_PATH), SB_SIGN_SH).await?;

    runner
        .run_status(
            "env",
            &[
                &format!("ROOT={}", s(&boot)?),
                "sh",
                &s(&target.join(SB_SIGN_PATH))?,
                &s(&keys)?,
                &s(&plain)?,
                &s(&pristine)?,
                &s(&efi)?,
                &s(&conf)?,
            ],
        )
        .await?;

    let sb_dir = boot.join("secureboot");
    tokio::fs::create_dir_all(&sb_dir).await?;
    tokio::fs::copy(keys.join("db.cer"), sb_dir.join("simple-linux-db.cer")).await?;
    tokio::fs::write(
        sb_dir.join("README.txt"),
        "Secure Boot for this machine: enrol simple-linux-db.cer in the firmware's db\n\
         (setup menu -> Secure Boot -> Key Management -> append/enrol DB from file), then turn Secure Boot on.\n\
         The key is yours: it was made during the install and lives in /etc/simple-linux/secureboot.\n\
         After a kernel change run simple-linux-update (it signs again); a kernel that was not signed does not boot.\n",
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gi-sl-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn the_update_tool_and_its_config_land_on_the_target() {
        let t = dir("tool");
        install_update_tool(
            &t,
            "http://store.invalid/b/",
            "generic-x86-64-v3-none-desktop",
        )
        .await
        .unwrap();
        let tool = t.join("usr/local/sbin/simple-linux-update");
        assert!(tool.is_file());
        assert_ne!(
            std::fs::metadata(&tool).unwrap().permissions().mode() & 0o111,
            0
        );
        let conf = std::fs::read_to_string(t.join("etc/simple-linux/update.conf")).unwrap();
        assert!(
            conf.contains("COMBO=generic-x86-64-v3-none-desktop\n"),
            "{conf}"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[tokio::test]
    async fn secure_boot_keeps_the_unsigned_originals_and_signs_through_the_helper() {
        let t = dir("sb");
        std::fs::create_dir_all(t.join("boot/EFI/BOOT")).unwrap();
        std::fs::write(t.join("boot/EFI/BOOT/BOOTX64.EFI"), "efi").unwrap();
        std::fs::write(t.join("boot/limine.conf"), "/Gentoo\n").unwrap();
        // The fake runner creates no key; the helper step is a fake too, so only the shape is checked.
        std::fs::create_dir_all(t.join(KEYS_DIR)).unwrap();
        std::fs::write(t.join(KEYS_DIR).join("db.key"), "k").unwrap();
        std::fs::write(t.join(KEYS_DIR).join("db.cer"), "c").unwrap();
        let runner = FakeCommandRunner::new();
        setup_secure_boot(&runner, &t).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(t.join(PRISTINE_EFI)).unwrap(),
            "efi"
        );
        assert_eq!(
            std::fs::read_to_string(t.join(PLAIN_CONF)).unwrap(),
            "/Gentoo\n"
        );
        assert!(t.join("boot/secureboot/simple-linux-db.cer").is_file());
        let calls = runner.calls();
        let (cmd, args) = calls.last().unwrap();
        assert_eq!(cmd, "env");
        assert!(
            args[0].starts_with("ROOT=") && args.iter().any(|a| a.ends_with("sb-sign")),
            "{args:?}"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn the_bundled_scripts_parse() {
        for (name, text) in [("update", UPDATE_SH), ("sb-sign", SB_SIGN_SH)] {
            let mut child = std::process::Command::new("sh")
                .arg("-n")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
            assert!(child.wait().unwrap().success(), "{name} does not parse");
        }
    }
}
