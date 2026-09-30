//! Limine install/config — the only bootloader this distro supports.

use crate::command::CommandRunner;
use std::path::Path;

/// Writes a minimal `limine.conf` for the installed system: single entry booting
/// `kernel_file` (a file name in the ESP root, which is `/boot` on the target). `root_subvol`
/// is `Some("@")` for btrfs only — `rootflags=subvol=` on an ext4 root fails the mount.
pub fn generate_config(kernel_file: &str, root_partuuid: &str, root_subvol: Option<&str>) -> String {
    let rootflags = root_subvol.map(|s| format!(" rootflags=subvol={s}")).unwrap_or_default();
    format!(
        "timeout: 3\n\n\
         /Gentoo\n\
         \tprotocol: linux\n\
         \tkernel_path: boot():/{kernel_file}\n\
         \tcmdline: root=PARTUUID={root_partuuid}{rootflags} rw\n"
    )
}

pub async fn write_config(target_boot: &Path, config: &str) -> crate::Result<()> {
    tokio::fs::write(target_boot.join("limine.conf"), config).await?;
    Ok(())
}

/// The kernel `kernel::deploy` put in `<target>/boot` (`vmlinuz-<combo>`). Exactly one is
/// expected; picking among several by guesswork would boot the wrong one silently.
pub fn find_kernel(target: &Path) -> crate::Result<String> {
    let mut found: Vec<String> = std::fs::read_dir(target.join("boot"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("vmlinuz-"))
        .collect();
    found.sort();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(crate::Error::Other(anyhow::anyhow!("no vmlinuz-* in {}/boot; cannot write limine.conf", target.display()))),
        many => Err(crate::Error::Other(anyhow::anyhow!("several kernels in {}/boot ({many:?}); refusing to guess", target.display()))),
    }
}

/// Writes `<target>/boot/limine.conf` pointing at the deployed kernel and the root
/// partition by PARTUUID (GPT PARTUUID, so no initramfs is needed to resolve it).
pub async fn configure(
    runner: &dyn CommandRunner,
    target: &Path,
    layout: &crate::partition::Layout,
    parts: &crate::partition::Partitions,
) -> crate::Result<()> {
    let kernel = find_kernel(target)?;
    let out = runner.run("blkid", &["-s", "PARTUUID", "-o", "value", &parts.root]).await?;
    let partuuid = out.trim();
    if partuuid.is_empty() {
        return Err(crate::Error::Other(anyhow::anyhow!("blkid returned no PARTUUID for {}", parts.root)));
    }
    let subvol = matches!(layout.root_fs, crate::partition::RootFs::Btrfs).then_some("@");
    write_config(&target.join("boot"), &generate_config(&kernel, partuuid, subvol)).await
}

fn firmware_is_uefi() -> bool {
    Path::new("/sys/firmware/efi").is_dir()
}

/// Installs Limine to the ESP (mounted at `target}/boot`) and either registers a UEFI
/// boot entry or writes the legacy MBR/BIOS stage via `limine bios-install`.
/// Assumes the Limine binaries shipped in the live environment/store are available
/// under `/usr/share/limine` (the standard Gentoo `sys-boot/limine` install path).
pub async fn install(runner: &dyn CommandRunner, target: &Path, disk: &str) -> crate::Result<()> {
    let boot = target.join("boot");
    let limine_dir = boot.join("limine");
    tokio::fs::create_dir_all(&limine_dir).await?;

    // Boot protocol modules + config live under boot/limine regardless of firmware mode.
    for file in ["limine-bios.sys"] {
        copy_if_present(Path::new("/usr/share/limine").join(file), limine_dir.join(file)).await?;
    }

    if firmware_is_uefi() {
        let efi_boot = boot.join("EFI/BOOT");
        tokio::fs::create_dir_all(&efi_boot).await?;
        copy_if_present(
            "/usr/share/limine/BOOTX64.EFI",
            efi_boot.join("BOOTX64.EFI"),
        )
        .await?;
    } else {
        // Legacy BIOS boot: limine bios-install embeds stage2 in the disk's boot sector,
        // reading limine-bios.sys back from the ESP/boot partition at boot time.
        runner.run_status("limine", &["bios-install", disk]).await?;
    }

    Ok(())
}

async fn copy_if_present(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
) -> crate::Result<()> {
    let src = src.as_ref();
    if !src.exists() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "expected limine artifact missing: {}",
            src.display()
        )));
    }
    tokio::fs::copy(src, dst).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_has_expected_shape() {
        let cfg = generate_config("vmlinuz-6.1-generic", "ABCD-1234", Some("@"));
        assert!(cfg.contains("kernel_path: boot():/vmlinuz-6.1-generic\n"));
        assert!(cfg.contains("root=PARTUUID=ABCD-1234 rootflags=subvol=@ rw"));
    }

    #[test]
    fn ext4_root_gets_no_rootflags() {
        let cfg = generate_config("vmlinuz-x", "ABCD-1234", None);
        assert!(!cfg.contains("rootflags"), "{cfg}");
    }

    #[test]
    fn bios_install_takes_only_the_disk() {
        // `--target-root` is not a limine option: it made every BIOS install fail.
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let runner = crate::command::FakeCommandRunner::new();
        let dir = std::env::temp_dir().join(format!("gi-bios-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // firmware_is_uefi() reflects the test host; only assert when it is BIOS.
        if !firmware_is_uefi() {
            let _ = rt.block_on(install(&runner, &dir, "/dev/sda"));
            runner.assert_call(0, "limine", &["bios-install", "/dev/sda"]);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_kernel_wants_exactly_one() {
        let dir = std::env::temp_dir().join(format!("gi-fk-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("boot")).unwrap();
        assert!(find_kernel(&dir).is_err());
        std::fs::write(dir.join("boot/vmlinuz-a"), "").unwrap();
        assert_eq!(find_kernel(&dir).unwrap(), "vmlinuz-a");
        std::fs::write(dir.join("boot/vmlinuz-b"), "").unwrap();
        assert!(find_kernel(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
