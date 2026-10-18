//! Limine install/config — the only bootloader this distro supports.

use crate::command::CommandRunner;
use std::path::Path;

/// Writes a minimal `limine.conf` for the installed system: single entry booting
/// `kernel_file` (a file name in the ESP root, which is `/boot` on the target). `root_subvol`
/// is `Some("@")` for btrfs only — `rootflags=subvol=` on an ext4 root fails the mount.
pub fn generate_config(
    kernel_file: &str,
    root_partuuid: &str,
    root_subvol: Option<&str>,
    extra_cmdline: &str,
    others: &[(String, String)],
) -> String {
    let rootflags = root_subvol
        .map(|s| format!(" rootflags=subvol={s}"))
        .unwrap_or_default();
    let extra = if extra_cmdline.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", extra_cmdline.trim())
    };
    // The other systems found on the shared EFI system partition are started through their own loaders.
    let others: String = others
        .iter()
        .map(|(name, path)| {
            format!("\n/{name}\n\tprotocol: efi_chainload\n\timage_path: boot():{path}\n")
        })
        .collect();
    format!(
        "timeout: {}\n\n\
         /Simple Linux\n\
         \tprotocol: linux\n\
         \tkernel_path: boot():/{kernel_file}\n\
         \tcmdline: root=PARTUUID={root_partuuid}{rootflags} rw{extra}\n{others}",
        if others.is_empty() { 3 } else { 5 }
    )
}

/// The other systems whose boot loaders are on the EFI system partition mounted at `esp` (`EFI/<vendor>/<loader>.efi`, not
/// Limine's own `EFI/BOOT`): `(name, path on the ESP)`. Windows' `bootmgfw.efi`, a distro's `shimx64.efi` or `grubx64.efi`.
pub fn other_os_entries(esp: &Path) -> Vec<(String, String)> {
    const LOADERS: [&str; 4] = [
        "shimx64.efi",
        "grubx64.efi",
        "bootmgfw.efi",
        "systemd-bootx64.efi",
    ];
    let Ok(dirs) = std::fs::read_dir(esp.join("EFI")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case("BOOT") {
            continue;
        }
        // `EFI/Microsoft/Boot/bootmgfw.efi` sits one level deeper than the others.
        let mut candidates: Vec<(std::path::PathBuf, String)> = Vec::new();
        for base in [dir.path(), dir.path().join("Boot")] {
            if let Ok(files) = std::fs::read_dir(&base) {
                for f in files.flatten() {
                    let file = f.file_name().to_string_lossy().to_string();
                    if file.to_lowercase().ends_with(".efi") {
                        let rel = base
                            .strip_prefix(esp)
                            .unwrap_or(&base)
                            .display()
                            .to_string();
                        candidates.push((base.clone(), format!("/{rel}/{file}")));
                    }
                }
            }
        }
        let pick = LOADERS.iter().find_map(|l| {
            candidates
                .iter()
                .find(|(_, p)| p.to_lowercase().ends_with(&format!("/{l}")))
        });
        if let Some((_, path)) = pick {
            let pretty = match name.to_lowercase().as_str() {
                "microsoft" => "Windows".to_string(),
                other => {
                    let mut c = other.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                }
            };
            out.push((pretty, path.clone()));
        }
    }
    out.sort();
    out
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
        [] => Err(crate::Error::Other(anyhow::anyhow!(
            "no vmlinuz-* in {}/boot; cannot write limine.conf",
            target.display()
        ))),
        many => Err(crate::Error::Other(anyhow::anyhow!(
            "several kernels in {}/boot ({many:?}); refusing to guess",
            target.display()
        ))),
    }
}

/// Writes `<target>/boot/limine.conf` pointing at the deployed kernel and the root
/// partition by PARTUUID (GPT PARTUUID, so no initramfs is needed to resolve it).
pub async fn configure(
    runner: &dyn CommandRunner,
    target: &Path,
    layout: &crate::partition::Layout,
    parts: &crate::partition::Partitions,
    extra_cmdline: &str,
) -> crate::Result<()> {
    let kernel = find_kernel(target)?;
    let out = runner
        .run("blkid", &["-s", "PARTUUID", "-o", "value", &parts.root])
        .await?;
    let partuuid = out.trim();
    if partuuid.is_empty() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "blkid returned no PARTUUID for {}",
            parts.root
        )));
    }
    let subvol = matches!(layout.root_fs, crate::partition::RootFs::Btrfs).then_some("@");
    let others = if matches!(
        layout.placement,
        crate::partition::Placement::FreeSpace { esp: Some(_), .. }
    ) {
        other_os_entries(&target.join("boot"))
    } else {
        Vec::new()
    };
    write_config(
        &target.join("boot"),
        &generate_config(&kernel, partuuid, subvol, extra_cmdline, &others),
    )
    .await
}

/// Booted through UEFI (the machine this runs on; the installed system boots the same way).
pub fn firmware_is_uefi() -> bool {
    Path::new("/sys/firmware/efi").is_dir()
}

/// Installs Limine to the ESP (mounted at `target}/boot`) and either registers a UEFI
/// boot entry or writes the legacy MBR/BIOS stage via `limine bios-install`.
/// Assumes the Limine binaries shipped in the live environment/store are available
/// under `/usr/share/limine` (the standard Gentoo `sys-boot/limine` install path).
pub async fn install(
    runner: &dyn CommandRunner,
    target: &Path,
    disk: &str,
    esp_device: &str,
) -> crate::Result<()> {
    let boot = target.join("boot");
    let limine_dir = boot.join("limine");
    tokio::fs::create_dir_all(&limine_dir).await?;

    // Boot protocol modules + config live under boot/limine regardless of firmware mode.
    for file in ["limine-bios.sys"] {
        copy_if_present(
            Path::new("/usr/share/limine").join(file),
            limine_dir.join(file),
        )
        .await?;
    }

    if firmware_is_uefi() {
        let efi_boot = boot.join("EFI/BOOT");
        tokio::fs::create_dir_all(&efi_boot).await?;
        // A shared ESP may already have another system's loader at the fallback path: keep it.
        let fallback = efi_boot.join("BOOTX64.EFI");
        let kept = efi_boot.join("BOOTX64.EFI.previous");
        if fallback.is_file() && !kept.exists() {
            tokio::fs::copy(&fallback, &kept).await.ok();
        }
        copy_if_present("/usr/share/limine/BOOTX64.EFI", fallback).await?;
        // A boot entry of its own, first in the order: without it a firmware that already has entries (the other system's)
        // starts those and never reaches the fallback path. Not fatal: some firmware and VMs have no variables to write.
        if let Some((entry_disk, number)) = split_partition(esp_device) {
            runner
                .run_status(
                    "efibootmgr",
                    &[
                        "-c",
                        "-d",
                        &entry_disk,
                        "-p",
                        &number.to_string(),
                        "-L",
                        "Simple Linux",
                        "-l",
                        "\\EFI\\BOOT\\BOOTX64.EFI",
                    ],
                )
                .await
                .ok();
        }
    } else {
        // Legacy BIOS boot: limine bios-install embeds stage2 in the disk's boot sector,
        // reading limine-bios.sys back from the ESP/boot partition at boot time.
        runner.run_status("limine", &["bios-install", disk]).await?;
    }

    Ok(())
}

/// `/dev/nvme0n1p1` -> (`/dev/nvme0n1`, 1); `/dev/sda3` -> (`/dev/sda`, 3).
fn split_partition(device: &str) -> Option<(String, u32)> {
    let digits = device
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 || digits == device.len() {
        return None;
    }
    let (head, number) = device.split_at(device.len() - digits);
    let head = match head.strip_suffix('p') {
        Some(h) if h.ends_with(|c: char| c.is_ascii_digit()) => h,
        _ => head,
    };
    Some((head.to_string(), number.parse().ok()?))
}

async fn copy_if_present(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> crate::Result<()> {
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
        let cfg = generate_config("vmlinuz-6.1-generic", "ABCD-1234", Some("@"), "", &[]);
        assert!(cfg.contains("kernel_path: boot():/vmlinuz-6.1-generic\n"));
        assert!(cfg.contains("root=PARTUUID=ABCD-1234 rootflags=subvol=@ rw"));
    }

    #[test]
    fn ext4_root_gets_no_rootflags() {
        let cfg = generate_config("vmlinuz-x", "ABCD-1234", None, "", &[]);
        assert!(!cfg.contains("rootflags"), "{cfg}");
    }

    #[test]
    fn bios_install_takes_only_the_disk() {
        // `--target-root` is not a limine option: it made every BIOS install fail.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let runner = crate::command::FakeCommandRunner::new();
        let dir = std::env::temp_dir().join(format!("gi-bios-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // firmware_is_uefi() reflects the test host; only assert when it is BIOS.
        if !firmware_is_uefi() {
            let _ = rt.block_on(install(&runner, &dir, "/dev/sda", "/dev/sda1"));
            runner.assert_call(0, "limine", &["bios-install", "/dev/sda"]);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_other_systems_on_a_shared_esp_get_chainload_entries() {
        let dir = std::env::temp_dir().join(format!("gi-others-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        for (d, f) in [
            ("EFI/Microsoft/Boot", "bootmgfw.efi"),
            ("EFI/ubuntu", "shimx64.efi"),
            ("EFI/ubuntu", "grubx64.efi"),
            ("EFI/BOOT", "BOOTX64.EFI"),
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join(f), "").unwrap();
        }
        let found = other_os_entries(&dir);
        assert_eq!(
            found,
            vec![
                ("Ubuntu".to_string(), "/EFI/ubuntu/shimx64.efi".to_string()),
                (
                    "Windows".to_string(),
                    "/EFI/Microsoft/Boot/bootmgfw.efi".to_string()
                ),
            ]
        );
        let cfg = generate_config("vmlinuz-x", "ABCD-1234", None, "", &found);
        assert!(cfg.contains("/Simple Linux\n\tprotocol: linux"), "{cfg}");
        assert!(cfg.contains("/Windows\n\tprotocol: efi_chainload\n\timage_path: boot():/EFI/Microsoft/Boot/bootmgfw.efi\n"), "{cfg}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_partition_device_splits_into_its_disk_and_number() {
        assert_eq!(
            split_partition("/dev/nvme0n1p1"),
            Some(("/dev/nvme0n1".into(), 1))
        );
        assert_eq!(split_partition("/dev/sda12"), Some(("/dev/sda".into(), 12)));
        assert_eq!(split_partition("/dev/sda"), None);
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

    #[test]
    fn extra_kernel_parameters_are_appended_after_rw() {
        let cfg = generate_config(
            "vmlinuz-x",
            "ABCD-1234",
            None,
            "  nvidia-drm.modeset=1 ",
            &[],
        );
        assert!(cfg.contains("rw nvidia-drm.modeset=1\n"), "{cfg}");
    }
}
