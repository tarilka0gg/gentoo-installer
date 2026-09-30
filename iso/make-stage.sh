#!/bin/bash
# make-stage.sh <base-rootfs> <binpkgs-dir> <guru-repo-dir> <out.tar.xz>   (run as root)
#
# Builds a custom stage3: Gentoo's stage3 + fish, eza, dust, gping, micro (installed from
# binary packages built earlier with FEATURES=buildpkg, so nothing is compiled here), `nano`
# removed, and the house fish aliases. The installer unpacks it with
# GENTOO_INSTALLER_STAGE3_URL=<file:///…|https://…> [GENTOO_INSTALLER_STAGE3_SHA512=…]; a user
# created on top of it gets fish as login shell (see account::login_shell).
#
# <base-rootfs> must contain a synced Portage tree (emerge needs the ebuild metadata even for
# binary installs); the tree, caches and temporary files are removed again before packing,
# like in Gentoo's own stage3.
set -euo pipefail
BASE=$(readlink -f "${1:?base rootfs}"); PKGS=$(readlink -f "${2:?binpkgs}"); GURU=$(readlink -f "${3:?guru repo}"); OUT=$(readlink -f "${4:?out}")
W=$(dirname "$OUT")/stage-work
[ -e "$W" ] && { echo "$W exists; remove it first" >&2; exit 1; }

cp -a --reflink=always "$BASE" "$W"
mkdir -p "$W/var/cache/binpkgs" "$W/var/db/repos/guru" "$W/etc/portage/repos.conf" "$W/etc/portage/package.accept_keywords"
cp -a --reflink=auto "$PKGS/." "$W/var/cache/binpkgs/"
rsync -a --exclude=.git "$GURU/" "$W/var/db/repos/guru/"
printf '[guru]\nlocation = /var/db/repos/guru\nsync-type = git\nsync-uri = https://github.com/gentoo-mirror/guru.git\nauto-sync = no\n' > "$W/etc/portage/repos.conf/guru.conf"
printf 'app-editors/micro ~amd64\nnet-analyzer/gping ~amd64\n' > "$W/etc/portage/package.accept_keywords/stage-tools"
cp /etc/resolv.conf "$W/etc/resolv.conf"

cat > "$W/../stage-inner.sh" <<INNER
set -e
mount --bind /proc "$W/proc"; mount --rbind /sys "$W/sys"; mount --rbind /dev "$W/dev"
chroot "$W" env LC_ALL=C.UTF-8 emerge --noreplace --usepkgonly --autounmask-write --autounmask-continue \\
    app-shells/fish sys-apps/eza sys-block/dust net-analyzer/gping app-editors/micro
chroot "$W" env LC_ALL=C.UTF-8 emerge -C app-editors/nano
install -d "$W/etc/fish/conf.d"
cat > "$W/etc/fish/conf.d/10-house.fish" <<'HOUSE'
set -gx EDITOR micro
set -gx VISUAL micro
set fish_greeting
if status is-interactive
    alias ls 'eza --icons --group-directories-first'
    alias ll 'eza -la --icons --group-directories-first --git'
    alias lt 'eza --tree --icons --level=2'
    alias nano micro
    alias du dust
    alias ping gping
end
HOUSE
grep -qx /usr/bin/fish "$W/etc/shells" || echo /usr/bin/fish >> "$W/etc/shells"
INNER
unshare --mount --propagation private bash "$W/../stage-inner.sh"
rm -f "$W/../stage-inner.sh"

# Back to a stage3's shape: no Portage tree, caches, build dirs, or this host's resolver.
rm -rf --one-file-system "$W/var/db/repos/gentoo" "$W/var/db/repos/guru" "$W/var/cache/binpkgs" \
    "$W/var/cache/distfiles" "$W/var/tmp/portage" "$W/tmp" "$W/etc/portage/repos.conf/guru.conf"
mkdir -p "$W/tmp" "$W/var/db/repos"; chmod 1777 "$W/tmp"
: > "$W/etc/resolv.conf"
if grep -q "$W" /proc/self/mountinfo; then echo "still mounted under $W; not packing" >&2; exit 1; fi

tar --xattrs --xattrs-include='*.*' --numeric-owner -C "$W" -cf - . | xz -T0 -9 > "$OUT"
( cd "$(dirname "$OUT")" && sha512sum "$(basename "$OUT")" > "$OUT.sha512" )
ls -lh "$OUT"; cut -c1-40 "$OUT.sha512"
