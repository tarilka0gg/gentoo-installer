#!/usr/bin/env python3
"""Print the CRATES and LICENSE blocks for a versioned ebuild, from Cargo.lock and `cargo metadata`.

    packaging/gentoo/gen-ebuild-data.py            # prints both blocks
    packaging/gentoo/gen-ebuild-data.py --check    # fails if a license has no Gentoo name

Run it from the repository root after `cargo update`/a version bump and paste the output into the ebuild.
"""
import json, os, re, subprocess, sys, tomllib

# SPDX id -> name in ::gentoo/licenses
MAP = {
    "MIT": "MIT", "Apache-2.0": "Apache-2.0", "BSD-3-Clause": "BSD", "BSD-2-Clause": "BSD-2", "ISC": "ISC",
    "MPL-2.0": "MPL-2.0", "Zlib": "ZLIB", "Unicode-3.0": "Unicode-3.0", "Unicode-DFS-2016": "Unicode-DFS-2016",
    "CC0-1.0": "CC0-1.0", "Unlicense": "Unlicense", "0BSD": "0BSD", "BSL-1.0": "Boost-1.0",
    "Apache-2.0 WITH LLVM-exception": "Apache-2.0-with-LLVM-exceptions", "LGPL-2.1-or-later": "LGPL-2.1+",
    "GPL-2.0-or-later": "GPL-2+", "GPL-2.0": "GPL-2", "GPL-2.0-only": "GPL-2",
    "bzip2-1.0.6": "BZIP2", "CDLA-Permissive-2.0": "CDLA-Permissive-2.0", "MIT-0": "MIT-0", "BSD-3-Clause-Clear": "BSD", "BSD-1-Clause": "BSD-1",
}
# not Cargo dependencies, but compiled in: xz sources inside liblzma-sys (forced by backhand)
EXTRA = ["openssl"]  # ring bundles BoringSSL-derived code


def licenses_of(expr):
    # "MIT OR Apache-2.0", "MIT/Apache-2.0", "(MIT OR Apache-2.0) AND Unicode-3.0", "Apache-2.0 WITH LLVM-exception"
    expr = expr.replace("/", " OR ").replace("(", " ").replace(")", " ")
    out, bad = set(), set()
    for part in re.split(r"\s+(?:OR|AND)\s+", expr.strip()):
        part = " ".join(part.split())
        if not part:
            continue
        if part in MAP:
            out.add(MAP[part])
        else:
            bad.add(part)
    return out, bad


def main():
    lock = tomllib.load(open("Cargo.lock", "rb"))
    crates = sorted(f'{p["name"]}@{p["version"]}' for p in lock["package"] if str(p.get("source", "")).startswith("registry+"))
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"], text=True))
    known = set(os.listdir("/var/db/repos/gentoo/licenses")) if os.path.isdir("/var/db/repos/gentoo/licenses") else set()
    workspace = {p["id"] for p in meta["packages"] if p.get("source") is None}
    lic, bad = set(EXTRA), set()
    for p in meta["packages"]:
        if p["id"] in workspace or not p.get("license"):
            continue
        ok, nope = licenses_of(p["license"])
        lic |= ok
        bad |= nope
    missing = sorted(l for l in lic if known and l not in known)
    if bad or missing:
        print("UNMAPPED SPDX:", sorted(bad), "NOT IN ::gentoo/licenses:", missing, file=sys.stderr)
        if "--check" in sys.argv:
            sys.exit(1)
    print("CRATES=\"")
    for c in crates:
        print(f"\t{c}")
    print('"\n')
    print("# Licenses of the crates compiled in (all of them, whichever side of an OR is chosen)")
    print('LICENSE+=" ' + " ".join(sorted(lic)) + '"')


main()
