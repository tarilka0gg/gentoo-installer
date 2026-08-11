//! Limine install/config — the only bootloader this distro supports.

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

/// Installs Limine to the ESP and runs `limine bios-install` / registers the UEFI
/// boot entry via efibootmgr, depending on firmware mode.
pub async fn install(esp_mount: &Path, disk: &str) -> crate::Result<()> {
    let _ = (esp_mount, disk);
    todo!("copy limine-bios.sys + BOOTX64.EFI to ESP, limine bios-install {{disk}} for legacy boot")
}
