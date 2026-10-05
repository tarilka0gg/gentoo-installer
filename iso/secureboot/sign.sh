#!/bin/bash
# sign.sh <keys-dir> <limine.conf> <BOOTX64.EFI>
# Makes Limine verify everything it loads, then signs Limine itself:
#  1. every kernel_path/module_path in limine.conf gets its BLAKE2b hash appended (`path#hash`),
#  2. the BLAKE2b of that finished config is written into BOOTX64.EFI (`limine enroll-config`), so Limine
#     refuses a config that was edited afterwards,
#  3. BOOTX64.EFI is signed with the db key.
# Chain: firmware (db) -> Limine -> config -> kernel and initramfs. The paths are resolved under <root>.
set -euo pipefail
KEYS=$(readlink -f "${1:?keys dir}"); CONF=$(readlink -f "${2:?limine.conf}"); EFI=$(readlink -f "${3:?BOOTX64.EFI}")
ROOT=${ROOT:?ROOT = the directory that boot() or fslabel() in the config maps to}
tmp=$(mktemp); trap 'rm -f "$tmp"' EXIT
while IFS= read -r line; do
    case $line in
        *kernel_path:*|*module_path:*)
            p=${line#*):}; p=${p%%#*}   # the path after boot(): or fslabel(NAME):
            [ -f "$ROOT$p" ] || { echo "sign.sh: $ROOT$p missing" >&2; exit 1; }
            printf '%s#%s\n' "${line%%#*}" "$(b2sum "$ROOT$p" | cut -d' ' -f1)" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$CONF" > "$tmp"
cat "$tmp" > "$CONF"
limine enroll-config "$EFI" "$(b2sum "$CONF" | cut -d' ' -f1)" >/dev/null
sbsign --key "$KEYS/db.key" --cert "$KEYS/db.crt" --output "$EFI.signed" "$EFI" >/dev/null
mv "$EFI.signed" "$EFI"
sbverify --cert "$KEYS/db.crt" "$EFI" >/dev/null && echo "signed: $EFI"
