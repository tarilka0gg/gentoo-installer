#!/bin/bash
# make-offline.sh <out-dir>     (no root needed)
# Builds the payload of an offline image (see installer-core/src/offline.rs for the layout): the binary packages an install pulled from the
# two binary hosts, their indexes cut down to those files, the kernel store, the Portage tree snapshot and the desktop configuration.
#
# The packages are the ones a real install downloaded: run installs in a VM (one with every software group, one per GPU driver) and keep
# /var/cache/binhost/{gentoo,simple-linux} of each installed disk under $COLLECT/{gentoo,simple-linux} (cp -rn merges them).
#
# Environment (defaults in brackets):
#   COLLECT [~/Documents/work/gentoo-installer-test/offline-collect]   the packages, as above
#   KERNEL_STORE [newest ~/Documents/work/gentoo-installer-test/release-*/simple-linux-kernel-store.tar]
#   WM_CONFIGS [sibling simple-linux-configs]           cloned bare into configs.git
#   OFFLINE_EXCLUDE [libreoffice podman telegram gimp vlc mpv]   path prefixes (space separated) of packages left out; keep in step with offline.rs
#   GENTOO_INDEX_URL, SL_INDEX_URL, TREE_URL            where the two indexes and the tree snapshot come from
set -euo pipefail
OUT=$(readlink -m "${1:?out dir}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=${T:-$HOME/Documents/work/gentoo-installer-test}
COLLECT=${COLLECT:-$T/offline-collect}
KERNEL_STORE=${KERNEL_STORE:-$(ls -1 "$T"/release-*/simple-linux-kernel-store.tar | sort -V | tail -n1)}
WM_CONFIGS=${WM_CONFIGS:-$(cd "$HERE/../.." && pwd)/simple-linux-configs}
GENTOO_INDEX_URL=${GENTOO_INDEX_URL:-https://distfiles.gentoo.org/releases/amd64/binpackages/23.0/x86-64/Packages}
SL_INDEX_URL=${SL_INDEX_URL:-https://github.com/tarilka0gg/simple-linux-binhost/releases/download/binhost/Packages}
TREE_URL=${TREE_URL:-https://github.com/tarilka0gg/simple-linux-binhost/releases/download/binhost/tree.tar.zst}
[ -d "$COLLECT/gentoo" ] && [ -d "$COLLECT/simple-linux" ] || { echo "make-offline.sh: no packages in $COLLECT" >&2; exit 1; }
[ -f "$KERNEL_STORE" ] || { echo "make-offline.sh: no kernel store ($KERNEL_STORE)" >&2; exit 1; }

rm -rf "$OUT"; mkdir -p "$OUT/binhost/gentoo" "$OUT/binhost/simple-linux"
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

# kernels: Packages + kernels/ at the top, like the store the installer reads
tar -xf "$KERNEL_STORE" -C "$OUT"

# packages and their indexes: the index of each host keeps only the entries whose file is here (the rest is a different set of packages
# nobody asked for), with the host's own header, so Portage reads it as it reads the real one
curl -fsSL -o "$tmp/gentoo-Packages" "$GENTOO_INDEX_URL"
curl -fsSL -o "$tmp/sl-Packages" "$SL_INDEX_URL"
python3 - "$COLLECT" "$OUT/binhost" "$tmp" <<'PY'
import os, sys, shutil
collect, out, tmp = sys.argv[1:4]
# left out to keep the image under GitHub's 2 GiB (see offline.rs NOT_ON_MEDIUM)
EXCLUDE = tuple(os.environ.get('OFFLINE_EXCLUDE', 'app-office/libreoffice-bin/ app-containers/podman/ telegram-desktop-bin- gimp-3 vlc-4 mpv-').split())
def cut(repo, index):
    files = set()
    root = os.path.join(collect, repo)
    for d, _, names in os.walk(root):
        for n in names:
            if n.endswith(".gpkg.tar"):
                rel = os.path.relpath(os.path.join(d, n), root)
                if not rel.startswith(EXCLUDE):
                    files.add(rel)
    blocks = open(index, encoding="utf-8").read().split("\n\n")
    header, entries = blocks[0], [b for b in blocks[1:] if b.strip()]
    kept, used = [], set()
    for b in entries:
        path = next((l[6:] for l in b.splitlines() if l.startswith("PATH: ")), None)
        if path in files:
            kept.append(b); used.add(path)
    missing = sorted(files - used)
    for rel in sorted(used):
        dst = os.path.join(out, repo, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copy2(os.path.join(root, rel), dst)
    header = "\n".join(l if not l.startswith("PACKAGES: ") else f"PACKAGES: {len(kept)}" for l in header.splitlines())
    open(os.path.join(out, repo, "Packages"), "w", encoding="utf-8").write(header + "\n\n" + "\n\n".join(kept) + "\n\n")
    print(f"{repo}: {len(kept)} packages kept, {len(missing)} collected files not in the current index" + (f" (e.g. {missing[0]})" if missing else ""))
cut("gentoo", os.path.join(tmp, "gentoo-Packages"))
cut("simple-linux", os.path.join(tmp, "sl-Packages"))
PY

# source archives of the packages that have no binary: grim (tiny) and linux-firmware, which is cut down to what real hardware needs (GPU,
# Wi-Fi, Bluetooth, audio, common network cards): 609 MB become ~130 MB, and the image fits GitHub's 2 GiB. Portage checks every archive against
# the Manifest of its ebuild, so the tree snapshot below gets a Manifest line for the cut archive.
DIST=$COLLECT/distfiles; FW=$(cd "$DIST" && ls linux-firmware-*.tar.xz | head -n1); FWVER=${FW%.tar.xz}
mkdir -p "$OUT/distfiles"
cp "$DIST"/grim-*.tar.gz* "$OUT/distfiles/"   # the .sig too: Portage fetches it even without verify-sig
mkdir "$tmp/fw"; tar -xf "$DIST/$FW" -C "$tmp/fw"
CUT="qcom nvidia netronome mellanox mrvl qed dpaa2 liquidio cxgb4 bnx2x ti ti-connectivity ueagle-atm intel/qat intel/ice intel/vpu amdnpu ath11k/IPQ8074 ath11k/WCN6750 ath12k/IPQ5332 ath12k/IPQ5424 ath12k/IPQ9574"
( cd "$tmp/fw/$FWVER" && rm -rf $CUT )
# the ebuild installs what WHENCE lists and fails on a listed file that is not there: take the cut files out of the list too
python3 - "$tmp/fw/$FWVER/WHENCE" $CUT <<'PY'
import posixpath, sys
path, cut = sys.argv[1], tuple(c.rstrip("/") + "/" for c in sys.argv[2:])
out, dropped = [], 0
for line in open(path, encoding="utf-8", errors="surrogateescape").read().splitlines(keepends=True):
    kind, _, rest = line.partition(": ")
    if kind in ("File", "RawFile", "Link"):
        name = (rest.split(" -> ")[0] if kind == "Link" else rest).strip().strip('"')
        gone = name.startswith(cut)
        if kind == "Link" and not gone:   # a link kept in one directory may point into a cut one
            target = posixpath.normpath(posixpath.join(posixpath.dirname(name), rest.split(" -> ")[1].strip().strip('"')))
            gone = target.startswith(cut)
        if gone:
            dropped += 1
            continue
    out.append(line)
open(path, "w", encoding="utf-8", errors="surrogateescape").write("".join(out))
print(f"WHENCE: {dropped} entries of the cut directories removed")
PY
tar -I 'xz -9e -T0' -C "$tmp/fw" -cf "$OUT/distfiles/$FW" "$FWVER"
rm -rf "$tmp/fw"

# the Portage tree the packages were built against (the installer unpacks it from <binhost>/tree.tar.zst), with the cut firmware archive's
# size and checksums in the Manifest
curl -fsSL -o "$tmp/tree.tar.zst" "$TREE_URL"
mkdir "$tmp/tree"; tar --zstd -xf "$tmp/tree.tar.zst" -C "$tmp/tree"
python3 - "$tmp/tree" "$OUT/distfiles/$FW" "$FW" <<'PY'
import hashlib, sys, os
tree, archive, name = sys.argv[1:4]
data = open(archive, "rb").read()
line = f"DIST {name} {len(data)} BLAKE2B {hashlib.blake2b(data).hexdigest()} SHA512 {hashlib.sha512(data).hexdigest()}\n"
manifest = os.path.join(tree, "gentoo/sys-kernel/linux-firmware/Manifest")
lines = open(manifest).read().splitlines(keepends=True)
out, hit = [], False
for l in lines:
    if l.startswith(f"DIST {name} "):
        out.append(line); hit = True
    else:
        out.append(l)
assert hit, f"no DIST line for {name} in {manifest}"
open(manifest, "w").write("".join(out))
PY
tar -I 'zstd -19 -T0' -C "$tmp/tree" -cf "$OUT/binhost/simple-linux/tree.tar.zst" .
rm -rf "$tmp/tree" "$tmp/tree.tar.zst"

# the desktop configuration: a bare repository, cloned by path
git -c safe.directory='*' clone -q --bare "$WM_CONFIGS" "$OUT/configs.git"

{
    echo "Simple Linux offline payload"
    echo "built: $(date -u +%FT%TZ)"
    echo "packages: gentoo $(find "$OUT/binhost/gentoo" -name '*.gpkg.tar' | wc -l), simple-linux $(find "$OUT/binhost/simple-linux" -name '*.gpkg.tar' | wc -l)"
    echo "configs: $(git -c safe.directory='*' -C "$WM_CONFIGS" rev-parse HEAD)"
    echo "kernel store: $(basename "$KERNEL_STORE")"
} > "$OUT/manifest"
du -sh "$OUT"; cat "$OUT/manifest"
