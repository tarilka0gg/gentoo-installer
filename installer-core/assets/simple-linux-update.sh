#!/bin/sh
# simple-linux-update [check|kernel|system|all] [--force] [-y]
# Updates what the installer put on this machine and Portage does not know about, then (optionally) Portage itself.
#   check    say whether a newer kernel build is on the store (changes nothing)
#   kernel   fetch the store's kernel for this machine's hardware profile and switch to it; the old one stays as the
#            "previous kernel" entry in the boot menu. Re-signs the bootloader if Secure Boot is set up.
#   system   emerge --sync, then emerge -uDN @world (with binary packages when the binhost has them)
#   all      kernel, then system (default)
# Not updated here: ustan and portage-store (copied from the live image into /usr/local, they have no published releases).
set -eu
CONF=/etc/simple-linux/update.conf
SBDIR=/etc/simple-linux/secureboot
SBSIGN=/usr/local/lib/simple-linux/sb-sign
STATE=/var/lib/simple-linux
BOOT=${SL_BOOT:-/boot}
MODDIR=${SL_MODULES:-/lib/modules}

mode=all; force=0; yes=0
for a in "$@"; do
    case $a in
        check|kernel|system|all) mode=$a ;;
        --force) force=1 ;;
        -y|--yes) yes=1 ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "simple-linux-update: unknown argument '$a'" >&2; exit 2 ;;
    esac
done
[ "$(id -u)" = 0 ] || { echo "simple-linux-update: run as root (doas simple-linux-update)" >&2; exit 1; }
[ -r "$CONF" ] || { echo "simple-linux-update: $CONF is missing, this system was not installed by the Simple Linux installer" >&2; exit 1; }
# shellcheck disable=SC1090
. "$CONF"   # BINHOST, COMBO
: "${BINHOST:?}" "${COMBO:?}"
BINHOST=${BINHOST%/}
URL=$BINHOST/kernels/$COMBO
say() { printf '%s\n' "$*"; }
die() { printf 'simple-linux-update: %s\n' "$*" >&2; exit 1; }

remote_sig() {   # a cheap identity of the published build: ETag, else Last-Modified + length
    curl -fsSIL "$URL.vmlinuz" | tr -d '\r' | awk -F': ' 'tolower($1)=="etag"{e=$2} tolower($1)=="last-modified"{m=$2} tolower($1)=="content-length"{l=$2} END{print (e!=""?e:m" "l)}'
}

kernel_status() {   # sets NEW=1 when the store has a build that differs from the installed one
    NEW=0
    [ -f "$BOOT/vmlinuz-$COMBO" ] || die "$BOOT/vmlinuz-$COMBO is missing"
    sig=$(remote_sig) || die "cannot reach $URL.vmlinuz"
    if [ -f "$STATE/kernel.sig" ] && [ "$(cat "$STATE/kernel.sig")" = "$sig" ]; then
        say "kernel: up to date ($COMBO)"; return
    fi
    mkdir -p "$STATE"; tmpk=$(mktemp "$BOOT/.vmlinuz.XXXXXX")
    trap 'rm -f "$tmpk"' EXIT
    curl -fsSL -o "$tmpk" "$URL.vmlinuz" || die "download of $URL.vmlinuz failed"
    if cmp -s "$tmpk" "$BOOT/vmlinuz-$COMBO"; then
        printf '%s\n' "$sig" > "$STATE/kernel.sig"; say "kernel: up to date ($COMBO)"; rm -f "$tmpk"; trap - EXIT; return
    fi
    NEW=1; SIG=$sig; NEWK=$tmpk
    say "kernel: a newer build of $COMBO is on the store"
}

write_conf() {   # $1 = plain limine.conf text on stdin-file; adds the "previous kernel" entry and signs when needed
    src=$1
    sed -i '/^\/Gentoo (previous kernel)/,$d' "$src"
    if [ -f "$BOOT/vmlinuz.old" ]; then
        { printf '\n'; sed -n '/^\/Gentoo/,$p' "$src" | sed 's|^/Gentoo.*|/Gentoo (previous kernel)|; s|kernel_path: boot():/[^#[:space:]]*|kernel_path: boot():/vmlinuz.old|'; } >> "$src"
    fi
    if [ -d "$SBDIR" ]; then
        ROOT=$BOOT "$SBSIGN" "$SBDIR" "$src" /usr/local/share/simple-linux/BOOTX64.EFI "$BOOT/EFI/BOOT/BOOTX64.EFI" "$BOOT/limine.conf"
    elif [ "$src" != "$BOOT/limine.conf" ]; then
        cp "$src" "$BOOT/limine.conf"
    fi
}

kernel_update() {
    kernel_status
    [ "$NEW" = 1 ] || return 0
    [ "$mode" = check ] && return 0
    if [ "$force" = 0 ] && find "$MODDIR" -name 'nvidia*.ko*' 2>/dev/null | grep -q .; then
        die "an out-of-tree NVIDIA module is built for the current kernel; a new kernel would leave the desktop without it. Rebuild it after updating (or run with --force and do that yourself)"
    fi
    mods=$(mktemp "$BOOT/.modules.XXXXXX"); trap 'rm -f "$NEWK" "$mods"' EXIT
    curl -fsSL -o "$mods" "$URL-modules.tar.xz" || die "download of $URL-modules.tar.xz failed"
    tar -tf "$mods" >/dev/null || die "the modules archive is damaged"
    tar -xpf "$mods" -C "$MODDIR"
    # the kernel the machine runs now stays bootable as "previous kernel"
    cp -p "$BOOT/vmlinuz-$COMBO" "$BOOT/vmlinuz.old"
    mv "$NEWK" "$BOOT/vmlinuz-$COMBO"
    if [ -d "$SBDIR" ]; then src=/etc/simple-linux/limine.conf; else src=$BOOT/limine.conf; fi
    write_conf "$src"
    printf '%s\n' "$SIG" > "$STATE/kernel.sig"
    # keep the newest two module trees
    ls -1dt "$MODDIR"/*/ 2>/dev/null | tail -n +3 | while read -r d; do rm -rf "$d"; done
    say "kernel: updated. Reboot to use it; if it does not start, pick 'previous kernel' in the boot menu."
}

system_update() {
    command -v emerge >/dev/null 2>&1 || die "emerge not found"
    emerge --sync
    if [ "$yes" = 1 ]; then emerge -uDN --getbinpkg @world; else emerge -uDNav --getbinpkg @world; fi
}

case $mode in
    check)  kernel_update ;;
    kernel) kernel_update ;;
    system) system_update ;;
    all)    kernel_update; system_update ;;
esac
