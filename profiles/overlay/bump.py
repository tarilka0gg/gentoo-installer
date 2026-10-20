#!/usr/bin/env python3
"""Moves a package of the slp-local overlay to a new upstream release: new ebuild, new Manifest line. No Portage needed.

  profiles/overlay/bump.py media-gfx/photocraft-bin            latest GitHub release of the project named in SRC_URI
  profiles/overlay/bump.py media-gfx/photocraft-bin 0.11.0     a given version
  options: --keep   keep the old ebuild (default: remove it)     --force   redo the current version (rebuilds its Manifest line)

The new ebuild is the newest one renamed (the version comes from ${PV}); its archive is downloaded once to hash it
(size, BLAKE2B, SHA512, as Portage's Manifest wants). Look at the result, then try it: `ebuild <file> clean install`
(see profiles/README.md) and run `cargo test -p slp-profiles`.
"""
import hashlib, json, os, re, shutil, sys, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))


def vkey(v):
    return [int(p) if p.isdigit() else 0 for p in re.split(r"[.\-_]", v.lstrip("v"))]


def fetch(url, headers=None):
    req = urllib.request.Request(url, headers={"User-Agent": "slp-overlay-bump", **(headers or {})})
    return urllib.request.urlopen(req, timeout=120)


def manifest_line(name, data_file):
    b2, s512, size = hashlib.blake2b(), hashlib.sha512(), 0
    with open(data_file, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            b2.update(chunk); s512.update(chunk); size += len(chunk)
    return f"DIST {name} {size} BLAKE2B {b2.hexdigest()} SHA512 {s512.hexdigest()}"


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    keep, force = "--keep" in argv, "--force" in argv
    if not args:
        sys.exit(__doc__)
    cat, pkg = args[0].split("/")
    pdir = os.path.join(HERE, cat, pkg)
    ebuilds = sorted((f for f in os.listdir(pdir) if f.endswith(".ebuild")), key=lambda f: vkey(f[len(pkg) + 1:-7]))
    old = ebuilds[-1]
    old_ver = old[len(pkg) + 1:-7]
    text = open(os.path.join(pdir, old)).read()
    m = re.search(r"github\.com/([\w.\-]+/[\w.\-]+)/releases/download", text)
    if not m:
        sys.exit("SRC_URI has no GitHub release; bump it by hand")
    if len(args) > 1:
        new_ver = args[1]
    else:
        new_ver = json.load(fetch(f"https://api.github.com/repos/{m.group(1)}/releases/latest", {"Accept": "application/vnd.github+json"}))["tag_name"].lstrip("v")
    if new_ver == old_ver and not force:
        print(f"{cat}/{pkg} is already at {old_ver}"); return
    src = re.search(r'SRC_URI="([^"\s]+)"', text).group(1)
    url = src.replace("${PV}", new_ver).replace("${PN}", pkg)
    name = url.rsplit("/", 1)[1]
    if "${" in url:
        sys.exit(f"cannot expand SRC_URI: {url}")
    print(f"downloading {url}")
    tmp = os.path.join(pdir, ".bump-download")
    try:
        with fetch(url) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        line = manifest_line(name, tmp)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)
    new = f"{pkg}-{new_ver}.ebuild"
    if new != old:
        shutil.copy(os.path.join(pdir, old), os.path.join(pdir, new))
        if not keep:
            os.remove(os.path.join(pdir, old))
    mf = os.path.join(pdir, "Manifest")
    old_name = re.search(r'SRC_URI="([^"\s]+)"', text).group(1).replace("${PV}", old_ver).rsplit("/", 1)[1]
    lines = [l for l in (open(mf).read().splitlines() if os.path.exists(mf) else []) if l.split()[1:2] != [name] and (keep or l.split()[1:2] != [old_name])]
    open(mf, "w").write("\n".join(lines + [line]) + "\n")
    print(f"{cat}/{pkg}: {old_ver} -> {new_ver}\n{line[:100]}...")


main(sys.argv[1:])
