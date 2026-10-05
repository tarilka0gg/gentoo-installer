#!/usr/bin/env python3
"""keys.py <mon.sock> <text-or-@key> ... — type into a QEMU guest through the monitor's `sendkey`.

Each argument is typed followed by Enter; `@name` sends a single key instead (@ret, @q, @down, @ctrl-c).
Works when the guest has no usable serial console (it is the same path a person's keyboard takes).
"""
import socket, sys, time

s = socket.socket(socket.AF_UNIX); s.settimeout(5); s.connect(sys.argv[1]); time.sleep(.5); s.recv(4096)
M = {" ": "spc", "-": "minus", "/": "slash", ".": "dot", "=": "equal", "_": "shift-minus", "|": "shift-backslash",
     "$": "shift-4", ";": "semicolon", ",": "comma", ">": "shift-dot", "<": "shift-comma", "'": "apostrophe",
     '"': "shift-apostrophe", "(": "shift-9", ")": "shift-0", "*": "shift-8", ":": "shift-semicolon", "~": "shift-grave_accent",
     "#": "shift-3", "&": "shift-7", "\\": "backslash", "[": "bracket_left", "]": "bracket_right", "+": "shift-equal",
     "%": "shift-5", "@": "shift-2", "!": "shift-1", "?": "shift-slash", "`": "grave_accent", "{": "shift-bracket_left",
     "}": "shift-bracket_right"}
def key(k): s.send(f"sendkey {k}\n".encode()); time.sleep(.08)
for arg in sys.argv[2:]:
    if arg.startswith("@"):
        key(arg[1:]); continue
    for c in arg:
        key(M.get(c, f"shift-{c.lower()}" if c.isupper() else c))
    key("ret"); time.sleep(.5)
