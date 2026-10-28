#!/usr/bin/env python3
"""firmware-excludes.py <rootfs> [--stats]

Prints, one per line, the files of <rootfs>/usr/lib/firmware that the live image leaves out (appended to squashfs-excludes.txt by
assemble-iso.sh). linux-firmware ships every version of every device's firmware (~530 MB, ~80 MB compressed); a kernel only asks for the
files its modules name. Only the directories in TRIM are looked at (those whose drivers list every file they can load); everything
else stays as it is. In a TRIM directory a file stays if

  * a module of the image's own kernel names it (`modinfo -F firmware`, wildcards expanded against the tree),
  * it is the newest of a versioned family whose named version is missing,
  * it belongs to a family whose names the driver builds at run time, which no module lists (KEEP below), or
  * no module could be read at all: then nothing is dropped.

Not every driver lists its files: this kernel's iwlwifi names 44 files, none for the newer `bz-b0`/`gl-c0` Wi-Fi 7 steppings that the
driver does load, so Intel Wi-Fi (and Bluetooth, sound, ...) is never trimmed here. Check a driver's list against its hardware before
adding its directory to TRIM.

The medium is a live system and an installer: the installed system gets its own linux-firmware from its stage3, so this only decides
what the live session can drive. --stats prints the totals on stderr. FIRMWARE_TRIM=0 in assemble-iso.sh skips the step.
"""
import fnmatch, glob, os, re, subprocess, sys

# Directories whose drivers (amdgpu, i915, radeon, the Realtek, Atheros and MediaTek Wi-Fi drivers) list every firmware file they load.
TRIM = "amdgpu radeon i915 rtl_nic rtw88 rtw89 ath10k ath11k ath12k mediatek".split()
# Inside them, families whose names are built at run time and are not in MODULE_FIRMWARE.
KEEP = "mediatek/BT_* mediatek/WIFI_* mediatek/mt7*_wm* mediatek/mt76*".split()

root = sys.argv[1]
fwdir = os.path.join(root, "usr/lib/firmware")
if not os.path.isdir(fwdir):
    sys.exit(0)

mods = glob.glob(os.path.join(root, "usr/lib/modules/*/**/*.ko*"), recursive=True)
wanted = set()
if mods:
    for i in range(0, len(mods), 200):
        out = subprocess.run(["modinfo", "-F", "firmware"] + mods[i:i + 200], capture_output=True, text=True).stdout
        wanted.update(l.strip() for l in out.splitlines() if l.strip())
if not wanted:
    print("firmware-excludes.py: no module names any firmware; dropping nothing", file=sys.stderr)
    sys.exit(0)

files = []
for dp, dn, fn in os.walk(fwdir):
    for f in fn:
        files.append(os.path.relpath(os.path.join(dp, f), fwdir))
fileset = set(files)
SUF = ("", ".zst", ".xz", ".gz")

keep = set(f for f in files if f.split("/")[0] not in TRIM)
for w in wanted:
    if any(c in w for c in "*?["):
        keep.update(f for f in files if fnmatch.fnmatch(f, w) or fnmatch.fnmatch(f, w + ".*"))
    else:
        keep.update(w + s for s in SUF if w + s in fileset)
        # a named version that is not in the tree: keep the newest present of that family (iwlwifi-cc-a0-77.ucode -> -73, -72, ...)
        m = re.match(r"(.*-)(\d+)(\.ucode|\.pnvm)$", w)
        if m and not any(w + s in fileset for s in SUF):
            fam = [(int(re.match(re.escape(m.group(1)) + r"(\d+)", f).group(1)), f) for f in files
                   if f.startswith(m.group(1)) and f.endswith(m.group(3)) and re.match(re.escape(m.group(1)) + r"\d+" + re.escape(m.group(3)), f)]
            if fam:
                keep.add(max(fam)[1])
for pat in KEEP:
    keep.update(f for f in files if fnmatch.fnmatch(f, pat))
# symlinks (aliases) stay when what they point to stays, and the target of a kept symlink stays
for f in files:
    p = os.path.join(fwdir, f)
    if os.path.islink(p):
        t = os.path.normpath(os.path.join(os.path.dirname(f), os.readlink(p)))
        if t in keep:
            keep.add(f)
        elif f in keep and t in fileset:
            keep.add(t)

drop = sorted(f for f in files if f not in keep)
# a directory whose files are all dropped is excluded as one entry
dirs = {}
for f in files:
    d = os.path.dirname(f)
    while d:
        dirs.setdefault(d, [0, 0])
        dirs[d][0] += 1
        if f not in keep:
            dirs[d][1] += 1
        d = os.path.dirname(d)
gone = {d for d, (n, k) in dirs.items() if n == k}
out = []
for f in drop:
    d = os.path.dirname(f)
    top = None
    while d:
        if d in gone:
            top = d
        d = os.path.dirname(d)
    out.append(("usr/lib/firmware/" + (top or f)))
for line in sorted(set(out)):
    print(line)
if "--stats" in sys.argv:
    sz = lambda fs: sum(os.lstat(os.path.join(fwdir, f)).st_size for f in fs)
    print("firmware-excludes.py: %d of %d files kept (%.0f of %.0f MiB)" % (
        len(files) - len(drop), len(files), sz(keep & fileset) / 1048576, sz(files) / 1048576), file=sys.stderr)
