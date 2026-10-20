#!/usr/bin/env python3
"""slim-excludes.py <rootfs> [--check]

Prints, one per line, paths of <rootfs> that the live image leaves out (the list is appended to squashfs-excludes.txt by
assemble-iso.sh). The live system compiles nothing, has no man pages, no mail client and no Perl scripts of its own, so:

  * every file of the packages in DROP_PACKAGES (read from the package database, /var/db/pkg/*/*/CONTENTS),
  * Perl itself and every script in usr/bin, usr/sbin, usr/libexec whose shebang is perl,
  * single files of packages that stay (fish's developer tools, the MAC-address vendor lists of hwdata, Portage's saved
    build environments in /var/db/pkg).

--check lists every program or library that stays and still needs a library that would be dropped, and exits 1 if there is any:
a cut that breaks a dependency must not ship. Run it after changing the lists.
"""
import glob, os, re, subprocess, sys

# Whole packages with no use on the medium: build tools, documentation tools, a mail client, assemblers, media and print leftovers.
DROP_PACKAGES = """
net-mail/mailutils virtual/mailx
dev-build/autoconf dev-build/automake dev-build/libtool dev-build/autoconf-archive dev-util/intltool app-text/po4a sys-apps/help2man
sys-devel/bison sys-devel/flex sys-devel/m4 dev-util/re2c dev-lang/nasm dev-lang/vala dev-lang/sassc dev-libs/libsass
dev-util/mesa_clc dev-util/spirv-llvm-translator
sys-apps/texinfo app-doc/info-manual sys-apps/man-db sys-apps/groff app-text/mandoc dev-libs/opensp
llvm-core/lld llvm-core/lld-toolchain-symlinks
# Nothing in the image links the ffmpeg 7 compatibility libraries (21 MB), the print server (the GTK printing back end is not built) or
# libsmi's MIB files (19 MB; tcpdump is built without libsmi). --check proves that no kept program needs a library they carry.
media-video/ffmpeg-compat net-print/cups net-libs/libsmi
llvm-core/llvm llvm-core/clang llvm-runtimes/compiler-rt llvm-runtimes/compiler-rt-sanitizers
""".split()

SINGLE_FILES = """
usr/bin/fish_indent usr/bin/fish_key_reader
usr/share/hwdata/oui.txt usr/share/hwdata/iab.txt
usr/share/noctalia/assets/noctalia-wallpaper.png
""".split()

PERL_PATHS = ["usr/bin/perl*", "usr/lib64/perl5", "usr/lib64/libperl.so*", "usr/lib/perl5", "usr/share/perl5", "usr/share/perl",
              "usr/lib64/perl*"]


def package_files(root, atom):
    cat, name = atom.split("/")
    for d in glob.glob(os.path.join(root, "var/db/pkg", cat, name + "-[0-9]*")):
        try:
            for line in open(os.path.join(d, "CONTENTS"), errors="replace"):
                kind, _, rest = line.rstrip("\n").partition(" ")
                if kind == "obj":
                    yield " ".join(rest.split(" ")[:-2]).lstrip("/")
                elif kind == "sym":
                    yield rest.split(" -> ")[0].lstrip("/")
        except OSError:
            pass


def perl_scripts(root):
    for top in ("usr/bin", "usr/sbin", "usr/libexec", "usr/local/bin"):
        for d, _, files in os.walk(os.path.join(root, top)):
            for f in files:
                p = os.path.join(d, f)
                if os.path.islink(p) or not os.path.isfile(p) or os.path.getsize(p) > 4 << 20:
                    continue
                try:
                    with open(p, "rb") as fh:
                        head = fh.read(80)
                except OSError:
                    continue
                if head.startswith(b"#!") and b"perl" in head.split(b"\n")[0]:
                    yield os.path.relpath(p, root)


def excludes(root):
    out = set()
    for atom in DROP_PACKAGES:
        out.update(package_files(root, atom))
    out.update(SINGLE_FILES)
    # Portage's saved build environments (12 MiB, already bzip2-compressed so the ISO pays all of it): only `emerge -C` reads them.
    out.update(os.path.relpath(p, root) for p in glob.glob(os.path.join(root, "var/db/pkg/*/*/environment.bz2")))
    for pat in PERL_PATHS:
        out.update(os.path.relpath(p, root) for p in glob.glob(os.path.join(root, pat)))
    out.update(perl_scripts(root))
    # dev-perl/* modules sit in the Perl directories above; their own files in /usr/share and /usr/bin are caught by the shebang scan.
    for cat_pf in glob.glob(os.path.join(root, "var/db/pkg/dev-perl/*")):
        name = os.path.basename(cat_pf)
        out.update(package_files(root, "dev-perl/" + re.sub(r"-[0-9].*$", "", name)))
    return sorted(p for p in out if os.path.lexists(os.path.join(root, p)))


def needed(path):
    r = subprocess.run(["readelf", "-d", path], capture_output=True, text=True).stdout
    return re.findall(r"NEEDED.*\[(.+)\]", r)


def soname(path):
    r = subprocess.run(["readelf", "-d", path], capture_output=True, text=True).stdout
    m = re.search(r"SONAME.*\[(.+)\]", r)
    return m.group(1) if m else None


def check(root, dropped):
    gone = set()
    gone_files = set(dropped)
    for p in dropped:
        full = os.path.join(root, p)
        if os.path.isdir(full) and not os.path.islink(full):
            for d, _, files in os.walk(full):
                for f in files:
                    gone_files.add(os.path.relpath(os.path.join(d, f), root))
    for p in gone_files:
        full = os.path.join(root, p)
        base = os.path.basename(p)
        if ".so" in base:
            gone.add(base)
            if os.path.isfile(full) and not os.path.islink(full):
                s = soname(full)
                if s:
                    gone.add(s)
    # a soname is only really gone when no kept file provides it
    gone_real0 = {os.path.realpath(os.path.join(root, g)) for g in gone_files}
    for d, _, files in os.walk(os.path.join(root, "usr")):
        for f in files:
            if f in gone and os.path.realpath(os.path.join(d, f)) not in gone_real0:
                gone.discard(f)
    bad = {}
    gone_real = {os.path.realpath(os.path.join(root, g)) for g in gone_files}
    seen_tops = set()
    for top in ("usr/bin", "usr/sbin", "usr/lib", "usr/lib64", "usr/libexec", "usr/local", "bin", "sbin"):
        real_top = os.path.realpath(os.path.join(root, top))
        if real_top in seen_tops or not os.path.isdir(real_top):
            continue
        seen_tops.add(real_top)
        for d, _, files in os.walk(real_top):
            for f in files:
                p = os.path.join(d, f)
                rel = os.path.relpath(p, os.path.realpath(root))
                if os.path.realpath(p) in gone_real or os.path.islink(p):
                    continue
                try:
                    with open(p, "rb") as fh:
                        if fh.read(4) != b"\x7fELF":
                            continue
                except OSError:
                    continue
                for n in needed(p):
                    if n in gone:
                        bad.setdefault(n, []).append(rel)
    for n, users in sorted(bad.items()):
        print(f"{n} is dropped but needed by: {', '.join(users[:4])}" + (f" (+{len(users) - 4})" if len(users) > 4 else ""), file=sys.stderr)
    return not bad


if __name__ == "__main__":
    root = os.path.abspath(sys.argv[1])
    paths = excludes(root)
    if "--check" in sys.argv:
        sys.exit(0 if check(root, paths) else 1)
    print("\n".join(paths))
