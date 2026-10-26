#!/bin/bash
# software-archive.sh <stage3.tar.xz> <built-root> <out-dir>
#
# Makes software-desktop-default.tar.zst: the files that a built root (the desktop flavor's defaults, emerged from binary packages
# into a clean stage3 with the installer's own Portage configuration) adds to the stage3 itself, and that do not belong to one
# machine. The installer unpacks it in place of merging the same packages one by one (installer-core/src/software_tree.rs).
#
# Left out, on purpose: the Portage configuration and the profile (the installer writes those for the user's choice), the users,
# hostname, locale, time zone, fstab, the services (enabled by the installer), the kernel and its modules, the Portage tree and the
# binary-package caches, logs and temporary files, SSH host keys (made on first boot), the machine id, and linux-firmware and
# zen-bin, which the installer emerges (software_tree::NOT_IN_ARCHIVE).
set -euo pipefail
STAGE=$(readlink -f "${1:?stage3 tarball}"); ROOT=$(readlink -f "${2:?built root}"); OUT=$(readlink -m "${3:?out dir}")
WORK=$(mktemp -d "${TMPDIR:-/var/tmp}/software-archive.XXXXXX"); trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/stage" "$OUT"
tar -xpf "$STAGE" -C "$WORK/stage" --numeric-owner

# every path that is new or different in the built root compared with the stage3 (a checksum comparison, not times)
rsync -aHAX --checksum --dry-run --out-format='%n' "$ROOT/" "$WORK/stage/" | sed -e 's|/$||' -e '/^$/d' -e '/^\.$/d' > "$WORK/changed"

grep -vE '^(\./)?(etc/(portage|slp|fstab|hostname|conf\.d/(hostname|keymaps|hwclock|consolefont|modules|net|localmount)|localtime|timezone|locale\.gen|env\.d/02locale|passwd-?|shadow-?|group-?|gshadow-?|subuid-?|subgid-?|os-release|machine-id|resolv\.conf|hosts|issue|motd|runlevels|ssh/ssh_host_)|var/(db/repos|cache|tmp|log|run|lock|spool|mail|lib/dbus/machine-id|lib/random-seed)|usr/src|usr/local|boot|home|root|lib/modules|lib/firmware|usr/lib/firmware|tmp|run|proc|sys|dev|mnt|media|srv)(/|$)' "$WORK/changed" > "$WORK/list"

# The accounts the packages add (seat, messagebus, ...): the lines of the built root's account files that the stage3 does not have.
# They cannot come with the files (the account files belong to the machine), so the installer adds these lines to its own.
mkdir -p "$WORK/extra/usr/lib/simple-linux/accounts"
for f in group gshadow passwd shadow; do
    comm -13 <(sort "$WORK/stage/etc/$f") <(sort "$ROOT/etc/$f") > "$WORK/extra/usr/lib/simple-linux/accounts/$f"
done
# the root's files as listed, then the account lines from the extra directory (GNU tar reads "-C" lines in a list file)
{
    cat "$WORK/list"
    printf '%s\n' "-C $WORK/extra" "usr" "usr/lib" "usr/lib/simple-linux" "usr/lib/simple-linux/accounts"
    for f in group gshadow passwd shadow; do printf '%s\n' "usr/lib/simple-linux/accounts/$f"; done
} > "$WORK/list-full"
# directories are listed once each, without their contents (tar takes the list as it is)
tar --no-recursion --numeric-owner -C "$ROOT" -cf - -T "$WORK/list-full" | zstd -q -T0 -15 -o "$OUT/software-desktop-default.tar.zst"
echo "files: $(wc -l < "$WORK/list"), archive: $(stat -c %s "$OUT/software-desktop-default.tar.zst") bytes"
