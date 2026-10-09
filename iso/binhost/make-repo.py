#!/usr/bin/env python3
"""make-repo.py <work-dir> <out-dir>

Turns the packages a build left in <work-dir>/root/var/cache/binpkgs into the flat directory that is published as the release
assets of tarilka0gg/simple-linux-binhost:

  * only what the Packages index lists (an instance Portage replaced is not copied),
  * packages whose licence forbids redistributing binaries (RESTRICT bindist) or that are only build tools are left out,
  * file names without '+' (GitHub rewrites some characters in asset names, and PATH in the index must match the real name),
  * PATH in the index becomes the bare file name (a release has no directories), Packages.gz and the public key are added.

Several instances of one package (same version, different USE, own build id) all stay in the index: a client picks the one whose
USE matches its own.
"""
import gzip, os, re, shutil, sys

# Not published. bindist: the licence does not allow redistributing binaries; mirror: zen-bin's distfile is not to be mirrored (the
# installer fetches it from upstream anyway); rust-bin and zig-bin are build-only (hundreds of MB, never needed to install niri
# or ghostty from their binaries).
EXCLUDE_PREFIXES = ("sys-kernel/linux-firmware-", "www-client/zen-bin-", "dev-lang/rust-bin-", "dev-lang/zig-bin-")


def main(work, out):
    src = os.path.join(work, "root/var/cache/binpkgs")
    key = os.path.join(work, "out/binhost-signing.asc")
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    text = open(os.path.join(src, "Packages")).read()
    header, _, rest = text.partition("\n\n")
    entries = [e for e in rest.split("\n\n") if e.strip()]
    kept, dropped = [], []
    for e in entries:
        fields = dict(l.split(": ", 1) for l in e.split("\n") if ": " in l)
        cpv = fields["CPV"]
        if cpv.startswith(EXCLUDE_PREFIXES) or "bindist" in fields.get("RESTRICT", "").split():
            dropped.append(cpv)
            continue
        path = fields["PATH"]
        name = os.path.basename(path).replace("+", "plus")
        shutil.copy2(os.path.join(src, path), os.path.join(out, name))
        kept.append(re.sub(r"^PATH: .*$", "PATH: " + name, e, flags=re.M))
    header = re.sub(r"^PACKAGES: \d+$", "PACKAGES: %d" % len(kept), header, flags=re.M)
    index = header + "\n\n" + "\n\n".join(kept) + "\n\n"
    open(os.path.join(out, "Packages"), "w").write(index)
    with gzip.open(os.path.join(out, "Packages.gz"), "wt") as f:
        f.write(index)
    shutil.copy(key, os.path.join(out, "binhost-signing.asc"))
    print("published set: %d packages, left out: %s" % (len(kept), ", ".join(dropped) or "none"))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(*sys.argv[1:])
