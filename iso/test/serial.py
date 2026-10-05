#!/usr/bin/env python3
"""serial.py <ser.sock> <wait-seconds> <command|__wait_prompt__|__keys__ KEYS...>

Talks to a QEMU `-serial unix:...,server,nowait` console of a `live.debug` image (a plain shell).
 * `__wait_prompt__`  — wait until the shell prompt shows up
 * `__keys__ ...`     — send raw keys (\\r, \\x03 ... as Python escapes), print what comes back
 * anything else     — run it as a command in the (fish) shell and print the output
Only one client may be connected at a time (QEMU accepts one); kill stale ones by PID.
"""
import re, socket, sys, time

sock, wait, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3]
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
if cmd == "__wait_prompt__":
    buf, end = "", time.time() + wait
    s.send(b"\x03")
    while time.time() < end and "root@gentoo-live" not in buf:
        s.send(b"\r"); buf += rd(2)
    print("prompt" if "root@gentoo-live" in buf else "NO PROMPT")
elif cmd == "__keys__":
    keys = "".join(sys.argv[4:]).encode().decode("unicode_escape").encode()
    s.send(keys); print(clean(rd(wait)))
else:
    s.send(b"\x03"); rd(.5); s.send((cmd + "\r").encode()); print(clean(rd(wait)))
