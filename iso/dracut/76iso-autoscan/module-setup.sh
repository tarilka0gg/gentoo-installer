#!/bin/bash
# Finds the live ISO as a file on a partition when no device carries its label (Ventoy normal mode chain-loads the
# bootloader, so the kernel sees the USB stick, not a CD). dmsquash-live's iso-scan needs the file name in advance;
# this one looks for any *.iso whose volume label is the one we boot from.

check() { return 255; }
depends() { echo dmsquash-live; return 0; }
install() {
    inst_multiple losetup find dd mount umount
    inst_script "$moddir/iso-autoscan.sh" /sbin/iso-autoscan
    inst_hook cmdline 32 "$moddir/parse-iso-autoscan.sh"
}
