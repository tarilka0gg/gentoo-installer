#!/bin/sh
# sb-sign <keys-dir> <plain-limine.conf> <pristine-BOOTX64.EFI> <out-BOOTX64.EFI> <out-limine.conf>
# Secure Boot for an installed Simple Linux, with the machine's own key (made by the installer, never shipped):
#  1. every kernel_path/module_path in the config gets its BLAKE2b hash appended (`path#hash`),
#  2. the hash of that finished config is written into a copy of Limine (`limine enroll-config`), so Limine refuses a
#     config edited afterwards,
#  3. the copy is signed with the db key.
# Chain: firmware (db) -> Limine -> config -> kernel. Paths are looked up under $ROOT (default /boot, the ESP).
# Run again after every kernel change: the hashes are pinned, a new kernel does not boot until this has run.
set -eu
KEYS=${1:?keys dir}; SRC=${2:?plain limine.conf}; PRISTINE=${3:?unsigned BOOTX64.EFI}; OUT_EFI=${4:?output BOOTX64.EFI}; OUT_CONF=${5:?output limine.conf}
ROOT=${ROOT:-/boot}
for t in limine sbsign sbverify b2sum; do
    command -v "$t" >/dev/null 2>&1 || { echo "sb-sign: '$t' is missing (emerge sys-boot/limine app-crypt/sbsigntools)" >&2; exit 1; }
done
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
while IFS= read -r line; do
    case $line in
        *kernel_path:*|*module_path:*|*image_path:*)
            p=${line#*):}; p=${p%%#*}
            [ -f "$ROOT$p" ] || { echo "sb-sign: $ROOT$p is missing" >&2; exit 1; }
            printf '%s#%s\n' "${line%%#*}" "$(b2sum "$ROOT$p" | cut -d' ' -f1)" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$SRC" > "$tmp/limine.conf"
cp "$PRISTINE" "$tmp/BOOTX64.EFI"
limine enroll-config "$tmp/BOOTX64.EFI" "$(b2sum "$tmp/limine.conf" | cut -d' ' -f1)" >/dev/null
sbsign --key "$KEYS/db.key" --cert "$KEYS/db.crt" --output "$tmp/signed.EFI" "$tmp/BOOTX64.EFI" >/dev/null
sbverify --cert "$KEYS/db.crt" "$tmp/signed.EFI" >/dev/null
# Both files are put in place together; the pair is only valid together, so write each next to its target first.
cp "$tmp/signed.EFI" "$OUT_EFI.new"; cp "$tmp/limine.conf" "$OUT_CONF.new"
mv "$OUT_EFI.new" "$OUT_EFI"; mv "$OUT_CONF.new" "$OUT_CONF"
echo "signed: $OUT_EFI"
