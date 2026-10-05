#!/bin/bash
# test-ovmf.sh <keys-dir> <image.iso> [seconds]  - boots an ISO under OVMF with Secure Boot ON, our keys enrolled,
# and writes the final screen to ./secureboot-<pid>.ppm. Needs virt-fw-vars (pip install virt-firmware), qemu, socat.
# Run it with the Microsoft-only vars (OVMF_VARS.secboot.fd instead) to see the refusal: "Access Denied".
set -euo pipefail
K=$(readlink -f "${1:?keys dir}"); ISO=$(readlink -f "${2:?iso}"); T=${3:-40}
V=$(mktemp); M=$(mktemp -u); trap 'rm -f "$V" "$M"' EXIT
G=a2b0c0de-1111-4222-8333-444455556666
virt-fw-vars -i /usr/share/edk2/OvmfX64/OVMF_VARS.fd -o "$V" --set-pk $G "$K/PK.crt" --add-kek $G "$K/KEK.crt" --add-db $G "$K/db.crt" --secure-boot >/dev/null 2>&1
qemu-system-x86_64 -machine q35,smm=on -enable-kvm -cpu host -smp 4 -m 3072 -global driver=cfi.pflash01,property=secure,value=on \
    -drive if=pflash,format=raw,unit=0,file=/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.fd,readonly=on \
    -drive if=pflash,format=raw,unit=1,file="$V" -cdrom "$ISO" -boot d -display none \
    -monitor unix:"$M",server,nowait -serial none -daemonize -pidfile "$M.pid"
sleep "$T"; OUT=secureboot-$(cat "$M.pid").ppm
echo "screendump $PWD/$OUT" | socat - UNIX-CONNECT:"$M" >/dev/null; sleep 1
kill "$(cat "$M.pid")"; rm -f "$M.pid"; echo "screen: $OUT"
