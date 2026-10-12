#!/bin/bash
# publish-kernels.sh <store-dir> [--upload]
# <store-dir> is an unpacked kernel store: `Packages` and `kernels/<combo>.vmlinuz`, `<combo>-modules.tar.xz`, `<combo>-devel.tar.xz`
# (the layout of simple-linux-kernel-store.tar). The installer takes its kernels from the flat assets of the GitHub release `kernels` of
# tarilka0gg/simple-linux-binhost (a release has no directories): this lists what would be published, checks that every `Packages` entry has
# its kernel and modules, and with --upload creates or updates that release (as the user who owns the repository; --clobber replaces files).
set -euo pipefail
S=$(readlink -f "${1:?store dir}"); UP=${2:-}
REPO=${REPO:-tarilka0gg/simple-linux-binhost}; TAG=${TAG:-kernels}
[ -f "$S/Packages" ] && [ -d "$S/kernels" ] || { echo "$S is not an unpacked kernel store (Packages, kernels/)" >&2; exit 1; }
n=0; files=("$S/Packages")
while read -r cpv; do
    combo=${cpv#CPV: sys-kernel/gentoo-diy-kernel-bin-}; combo=${combo%-1}
    for f in "$combo.vmlinuz" "$combo-modules.tar.xz"; do
        [ -f "$S/kernels/$f" ] || { echo "Packages lists $combo but kernels/$f is missing" >&2; exit 1; }
    done
    for f in "$combo.vmlinuz" "$combo-modules.tar.xz" "$combo-devel.tar.xz"; do [ -f "$S/kernels/$f" ] && files+=("$S/kernels/$f"); done
    n=$((n+1))
done < <(grep '^CPV: ' "$S/Packages")
echo "$n kernels, ${#files[@]} files, $(du -ch "${files[@]}" | tail -1 | cut -f1) for $REPO release $TAG:"
for f in "${files[@]}"; do printf '  %8.1f MiB  %s\n' "$(echo "$(stat -c %s "$f") / 1048576" | bc -l)" "$(basename "$f")"; done
[ "$UP" = --upload ] || { echo "(dry run; --upload publishes)"; exit 0; }
if [ "$(id -u)" = 0 ]; then GH=(runuser -u "${SIGN_USER:-tarilka0gg}" -- gh); else GH=(gh); fi
"${GH[@]}" release view "$TAG" --repo "$REPO" >/dev/null 2>&1 || "${GH[@]}" release create "$TAG" --repo "$REPO" \
    --title "kernels" --notes "Prebuilt kernels (and their modules) the Simple Linux installer picks from by hardware profile: flat assets, \`Packages\` lists the combos. See the README of this repository."
# gh runs as the repository owner, who may not be able to read the store (a root-only directory): upload from a readable copy.
STAGE=$(mktemp -d /var/tmp/publish-kernels.XXXXXX); chmod 755 "$STAGE"; trap 'rm -rf "${STAGE:?}"' EXIT
cp "${files[@]}" "$STAGE/"; chmod 644 "$STAGE"/*
"${GH[@]}" release upload "$TAG" --repo "$REPO" --clobber "$STAGE"/*
echo "published: https://github.com/$REPO/releases/tag/$TAG"
