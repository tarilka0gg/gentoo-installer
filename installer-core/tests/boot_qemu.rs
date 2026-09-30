//! Boots a hand-built GPT image (ESP + ext4 root from a real stage3) in QEMU/OVMF using
//! the exact `limine.conf` the installer generates. Needs KVM, qemu, OVMF, mtools, e2fsprogs,
//! sgdisk, a host kernel and Limine; no root or loop devices (images built with `mke2fs -d`).
//!
//! GENTOO_INSTALLER_STAGE3=<unpacked stage3> GENTOO_INSTALLER_KERNEL=/boot/vmlinuz-... \
//!   cargo test -p installer-core --test boot_qemu -- --ignored

use std::process::Command;

fn sh(script: &str) -> String {
    let out = Command::new("bash").arg("-ec").arg(script).output().unwrap();
    assert!(out.status.success(), "{script}\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore]
fn installed_layout_boots_through_limine_to_openrc() {
    let stage3 = std::env::var("GENTOO_INSTALLER_STAGE3").expect("GENTOO_INSTALLER_STAGE3");
    let kernel = std::env::var("GENTOO_INSTALLER_KERNEL").expect("GENTOO_INSTALLER_KERNEL");
    let dir = std::env::temp_dir().join(format!("gi-boot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let uuid = "22222222-2222-2222-2222-222222222222";
    let conf = installer_core::bootloader::generate_config("vmlinuz-test", uuid, None) + " console=ttyS0\n";
    // generate_config ends the cmdline with "rw\n"; splice console= onto that line.
    let conf = conf.replace("rw\n console=ttyS0\n", "rw console=ttyS0\n");
    std::fs::write(dir.join("limine.conf"), conf).unwrap();

    let d = dir.display();
    sh(&format!(
        r#"cd {d}; I=disk.img; truncate -s 2500M $I
sgdisk -n1:2048:+300M -t1:ef00 -n2:0:0 -t2:8300 -u2:{uuid} $I >/dev/null
read ES EE < <(sgdisk -i1 $I | awk '/First sector/{{a=$3}}/Last sector/{{print a,$3}}')
read RS RE < <(sgdisk -i2 $I | awk '/First sector/{{a=$3}}/Last sector/{{print a,$3}}')
mkfs.vfat -F32 --offset $ES $I $(( (EE-ES+1)/2 )) >/dev/null
O=$I@@$((ES*512)); mmd -i $O ::/EFI ::/EFI/BOOT
mcopy -i $O /usr/share/limine/BOOTX64.EFI ::/EFI/BOOT/BOOTX64.EFI
mcopy -i $O {kernel} ::/vmlinuz-test; mcopy -i $O limine.conf ::/limine.conf
mke2fs -q -t ext4 -d {stage3} -E offset=$((RS*512)) $I $(( (RE-RS+1)/2 ))k"#
    ));
    sh(&format!(
        r#"cd {d}; timeout 60 qemu-system-x86_64 -machine q35 -enable-kvm -cpu host -m 1024 -display none \
 -serial file:serial.log -monitor none -no-reboot \
 -drive if=pflash,format=raw,readonly=on,file=/usr/share/qemu/edk2-x86_64-code.fd \
 -drive file=disk.img,format=raw,if=none,id=d0 -device ide-hd,drive=d0,bus=ide.0 || true"#
    ));
    let log = std::fs::read_to_string(dir.join("serial.log")).unwrap_or_default();
    std::fs::remove_dir_all(&dir).ok();
    assert!(log.contains("OpenRC") && log.contains("Entering runlevel"), "did not reach OpenRC:\n{log}");
}
