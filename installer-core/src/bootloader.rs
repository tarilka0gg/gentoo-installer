//! Limine install/config — the only bootloader this distro supports.

use crate::command::CommandRunner;
use std::path::Path;

/// Writes a minimal `limine.conf` for the installed system: single entry booting
/// the kernel package selected in `kernel::resolve`, root= pointed at the `@` subvolume.
pub fn generate_config(kernel_path: &str, root_partuuid: &str, root_subvol: &str) -> String {
    format!(
        "timeout: 3\n\n\
         /Gentoo\n\
         \tprotocol: linux\n\
         \tkernel_path: boot():{kernel_path}\n\
         \tcmdline: root=PARTUUID={root_partuuid} rootflags=subvol={root_subvol} rw\n"
    )
}

pub async fn write_config(target_boot: &Path, config: &str) -> crate::Result<()> {
    tokio::fs::write(target_boot.join("limine.conf"), config).await?;
    Ok(())
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
        let target_str = target
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
        runner.run_status("limine", &["bios-install", "--target-root", target_str, disk]).await?;
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
        let cfg = generate_config("vmlinuz-6.1-generic", "ABCD-1234", "@");
        assert!(cfg.contains("boot():vmlinuz-6.1-generic"));
        assert!(cfg.contains("root=PARTUUID=ABCD-1234"));
        assert!(cfg.contains("subvol=@"));
    }
}
