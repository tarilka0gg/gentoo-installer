#!/usr/bin/env python3
"""Reports what Simple Linux pins and whether upstream has moved on. Read-only: nothing is changed or downloaded beyond small API answers.

  iso/check-upstream.py            markdown report on stdout; exit 0
  iso/check-upstream.py --check    same, exit 1 when something has an update (for a workflow)

Checked: the kernel of the latest Simple Linux release against kernel.org (same series, and newer series), the stage3 of that release
against Gentoo's latest, the age of the binary host's tree snapshot, and every ebuild of profiles/overlay (a GitHub release each).
GITHUB_TOKEN (optional) raises the GitHub API rate limit.
"""
import datetime, json, os, re, sys, urllib.request

OWNER = os.environ.get("GITHUB_REPOSITORY_OWNER", "tarilka0gg")
HERE = os.path.dirname(os.path.abspath(__file__))
OVERLAY = os.path.join(HERE, "..", "profiles", "overlay")
TREE_MAX_DAYS = 30
STAGE3_MAX_DAYS = 45


def get(url, accept=None):
    h = {"User-Agent": "simple-linux-upstream-check"}
    if accept:
        h["Accept"] = accept
    if "api.github.com" in url and os.environ.get("GITHUB_TOKEN"):
        h["Authorization"] = "Bearer " + os.environ["GITHUB_TOKEN"]
    with urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=30) as r:
        return r.read().decode()


def gh(path):
    return json.loads(get("https://api.github.com" + path, "application/vnd.github+json"))


def vkey(v):
    return [int(p) if p.isdigit() else 0 for p in re.split(r"[.\-_]", v.lstrip("v"))]


def days_since(iso):
    t = datetime.datetime.fromisoformat(iso.replace("Z", "+00:00"))
    return (datetime.datetime.now(datetime.timezone.utc) - t).days


rows = []  # (component, pinned, latest, status, note); status: ok | update | old | error


def add(component, pinned, latest, status, note=""):
    rows.append((component, pinned, latest, status, note))


def kernel(release):
    try:
        names = [a["name"] for a in release["assets"]]
        m = next(re.match(r"simple-linux-kernel-(\d+\.\d+\.\d+)\.vmlinuz", n) for n in names if re.match(r"simple-linux-kernel-(\d+\.\d+\.\d+)\.vmlinuz", n))
        pinned = m.group(1)
        series = ".".join(pinned.split(".")[:2])
        rel = json.loads(get("https://www.kernel.org/releases.json"))["releases"]
        same = [r["version"] for r in rel if r["version"].startswith(series + ".")]
        newest = max((r["version"] for r in rel if re.fullmatch(r"\d+\.\d+(\.\d+)?", r["version"])), key=vkey)
        if same and vkey(same[0]) > vkey(pinned):
            add("kernel", pinned, same[0], "update", f"newer patch release of the {series} series")
        elif not same:
            add("kernel", pinned, newest, "old", f"the {series} series is no longer maintained on kernel.org")
        else:
            add("kernel", pinned, same[0] if same else pinned, "ok", f"newest series is {newest}" if vkey(newest) > vkey(pinned) else "")
    except Exception as e:  # noqa: BLE001
        add("kernel", "?", "?", "error", str(e)[:80])


def stage3():
    try:
        pin = os.path.join(HERE, "stage3.pin")
        pinned = re.search(r"(\d{8})T\d{6}Z", open(pin).read()).group(1)
        index = get("https://distfiles.gentoo.org/releases/amd64/autobuilds/latest-stage3-amd64-openrc.txt")
        latest = re.search(r"stage3-amd64-openrc-(\d{8})T", index).group(1)
        behind = (datetime.date(int(latest[:4]), int(latest[4:6]), int(latest[6:])) - datetime.date(int(pinned[:4]), int(pinned[4:6]), int(pinned[6:]))).days
        add("stage3 (base of the custom stage)", pinned, latest, "update" if behind > STAGE3_MAX_DAYS else "ok", f"{behind} days behind Gentoo's latest")
    except Exception as e:  # noqa: BLE001
        add("stage3 (base of the custom stage)", "?", "?", "error", str(e)[:80])


def tree():
    try:
        rel = gh(f"/repos/{OWNER}/simple-linux-binhost/releases/tags/binhost")
        age = days_since(rel["published_at"] if "published_at" in rel else rel["created_at"])
        add("binary host snapshot", rel["published_at"][:10], "-", "update" if age > TREE_MAX_DAYS else "ok", f"{age} days old (limit {TREE_MAX_DAYS})")
    except Exception as e:  # noqa: BLE001
        add("binary host snapshot", "?", "-", "error", str(e)[:80])


def overlay():
    if not os.path.isdir(OVERLAY):
        return
    for cat in sorted(os.listdir(OVERLAY)):
        cdir = os.path.join(OVERLAY, cat)
        if not os.path.isdir(cdir) or cat in ("profiles", "metadata"):
            continue
        for pkg in sorted(os.listdir(cdir)):
            files = [f for f in os.listdir(os.path.join(cdir, pkg)) if f.endswith(".ebuild")]
            if not files:
                continue
            ebuild = max(files, key=lambda f: vkey(f[len(pkg) + 1:-7]))
            pinned = ebuild[len(pkg) + 1:-7]
            text = open(os.path.join(cdir, pkg, ebuild)).read()
            m = re.search(r"github\.com/([\w.\-]+/[\w.\-]+)/releases/download", text)
            if not m:
                add(f"{cat}/{pkg}", pinned, "?", "error", "no GitHub release in SRC_URI")
                continue
            try:
                tag = gh(f"/repos/{m.group(1)}/releases/latest")["tag_name"]
                latest = tag.lstrip("v")
                add(f"{cat}/{pkg}", pinned, latest, "update" if vkey(latest) > vkey(pinned) else "ok", m.group(1))
            except Exception as e:  # noqa: BLE001
                add(f"{cat}/{pkg}", pinned, "?", "error", str(e)[:80])


try:
    latest_release = gh(f"/repos/{OWNER}/simple-linux/releases/latest")
    kernel(latest_release)
except Exception as e:  # noqa: BLE001
    add("simple-linux release", "?", "?", "error", str(e)[:80])
stage3()
tree()
overlay()

mark = {"ok": "ok", "update": "**update**", "old": "**old**", "error": "error"}
print("| Component | Pinned | Latest | Status | Note |\n|---|---|---|---|---|")
for c, p, l, s, n in rows:
    print(f"| {c} | {p} | {l} | {mark[s]} | {n} |")
needs = [r for r in rows if r[3] in ("update", "old")]
print(f"\n{len(needs)} of {len(rows)} need a look." if needs else f"\nAll {len(rows)} checks are up to date.")
sys.exit(1 if "--check" in sys.argv and needs else 0)
