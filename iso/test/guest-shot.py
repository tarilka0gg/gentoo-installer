#!/usr/bin/env python3
"""guest-shot.py <ser.sock> <out.png> [app ...] — drive a booted `live.debug` GUI image over serial.

Waits for the shell, optionally starts apps in the niri session (`niri msg action spawn`), lists the
windows and pulls a niri screenshot out of the guest as base64 (the QEMU monitor cannot screendump a
GL scanout). The image must have been built with EXTRA_CMDLINE="console=ttyS0,115200 live.debug".
"""
import base64, re, socket, sys, time

sock, out_png, apps = sys.argv[1], sys.argv[2], sys.argv[3:]
s = socket.socket(socket.AF_UNIX); s.settimeout(1)
for _ in range(120):
    try:
        s.connect(sock); break
    except OSError:
        time.sleep(0.5)

def rd(t):
    end, out = time.time() + t, b""
    while time.time() < end:
        try:
            d = s.recv(1 << 20)
            if not d: break
            out += d
        except socket.timeout:
            pass
    return out.decode(errors="replace")

clean = lambda t: re.sub(r"\x1b\][^\x07]*\x07|\x1b\[[0-9;?>=]*[a-zA-Z]|\x1b[>=]", "", t).replace("\r", "")
buf, end = "", time.time() + 240
while time.time() < end and "root@gentoo-live" not in buf:
    s.send(b"\r"); buf += rd(2)
time.sleep(25)  # let the niri session settle
def run(cmd, wait=3): s.send(b"\x03"); rd(.3); s.send((cmd + "\r").encode()); return clean(rd(wait))
run("set -gx XDG_RUNTIME_DIR /run/user/0; set -gx NIRI_SOCKET (ls /run/user/0/niri.*.sock | head -1)")
for a in apps: run(f"niri msg action spawn -- {a}", 2)
if apps: time.sleep(30)
print(run("niri msg windows | grep -E 'Title|App ID'", 5))
run("rm -rf ~/Pictures; niri msg action screenshot-screen --write-to-disk true; sleep 3; cp ~/Pictures/Screenshots/*.png /tmp/g.png", 8)
s.send(b"echo BEGIN64; base64 -w0 /tmp/g.png; echo; echo END64\r")
buf, end = "", time.time() + 120
while time.time() < end and "END64" not in buf.split("BEGIN64", 1)[-1]:
    buf += rd(3)
m = re.search(r"BEGIN64\s+([A-Za-z0-9+/=\s]+?)\s+END64", clean(buf))
if m:
    open(out_png, "wb").write(base64.b64decode(re.sub(r"\s", "", m.group(1)))); print("saved", out_png)
else:
    print("no image")
