#!/bin/bash
# boot-usb.sh <image.iso> bios|uefi|secureboot [seconds] [keys-dir]
# Boots the image the way a USB stick is seen by a PC (usb-storage, not a CD drive) and leaves the final
# screen in ./usb-<mode>.ppm. "secureboot" needs the keys dir and virt-fw-vars (see ../secureboot/).
set -euo pipefail
ISO=$(readlink -f "${1:?iso}"); MODE=${2:?bios|uefi|secureboot}; T=${3:-60}; KEYS=${4:-}
M=$(mktemp -u); V=$(mktemp); trap 'rm -f "$M" "$M.pid" "$V"' EXIT
FW=(); MACHINE=(-machine q35)
case $MODE in
    bios) ;;
    uefi) cp /usr/share/edk2/OvmfX64/OVMF_VARS.fd "$V"
          FW=(-drive if=pflash,format=raw,unit=0,file=/usr/share/edk2/OvmfX64/OVMF_CODE.fd,readonly=on -drive if=pflash,format=raw,unit=1,file="$V") ;;
    secureboot)
          G=a2b0c0de-1111-4222-8333-444455556666
          virt-fw-vars -i /usr/share/edk2/OvmfX64/OVMF_VARS.fd -o "$V" --set-pk $G "$KEYS/PK.crt" --add-kek $G "$KEYS/KEK.crt" --add-db $G "$KEYS/db.crt" --secure-boot >/dev/null 2>&1
          MACHINE=(-machine q35,smm=on -global driver=cfi.pflash01,property=secure,value=on)
          FW=(-drive if=pflash,format=raw,unit=0,file=/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.fd,readonly=on -drive if=pflash,format=raw,unit=1,file="$V") ;;
esac
qemu-system-x86_64 "${MACHINE[@]}" -enable-kvm -cpu host -smp 4 -m 3072 "${FW[@]}" \
    -drive if=none,id=stick,format=raw,file="$ISO",readonly=on -device qemu-xhci -device usb-storage,drive=stick,bootindex=0 \
    -display none -monitor unix:"$M",server,nowait -serial none -daemonize -pidfile "$M.pid"
sleep "$T"; echo "screendump $PWD/usb-$MODE.ppm" | socat - UNIX-CONNECT:"$M" >/dev/null; sleep 1
kill "$(cat "$M.pid")"; echo "screen: usb-$MODE.ppm"
