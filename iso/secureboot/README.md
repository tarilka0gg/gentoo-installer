# Secure Boot

`assemble-iso.sh` signs the images when `SECUREBOOT_KEYS=<dir>` is set (keys from `make-keys.sh`).

**What is signed and pinned.** Firmware verifies `BOOTX64.EFI` (Limine) against the `db` key; Limine verifies
its own `limine.conf` (the config's BLAKE2b is embedded in the EFI binary by `limine enroll-config`), and the
config pins the kernel and initramfs by BLAKE2b (`kernel_path: …#<hash>`). Change any of them and the boot stops.

**Why you must enrol a key.** There is no Microsoft-signed shim for this project, so a stock firmware refuses
the image ("Access Denied", verified under OVMF with the Microsoft keys). Either turn Secure Boot off, or put
`secureboot/simple-linux-db.cer` (on the medium, and in the `simple-linux` repo) into the firmware's `db`:
setup menu → Secure Boot → Key Management → Append/Enrol DB from file. Enrolling it trusts whoever holds the
matching private key (the author's), for everything signed with it.

**Tested:** `test-ovmf.sh <keys> <iso>` — OVMF with our PK/KEK/db enrolled boots through Limine to the installer;
the same image under OVMF's stock Microsoft keys is refused.

**The installed system** can sign its own Limine with a key generated on that machine (`installer-core/src/sltools.rs`, `assets/sb-sign.sh`, same chain
as above); the author's key is not involved. The private keys of the images never go into git or onto an image.
