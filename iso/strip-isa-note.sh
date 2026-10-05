#!/bin/sh
# strip-isa-note.sh FILE...  - drop the "x86 ISA needed" property note from binaries built on this machine.
#
# crt1.o and libc on a system built with -march=native carry the note "x86-64-v3 needed", and the linker copies it
# into every binary. glibc then refuses to run it on an older CPU with "CPU ISA level is lower than required" (found
# with a Westmere/Sandy Bridge VM: the image booted, the installer would not start). The Rust code itself is
# compiled for the x86-64 baseline, so the note is wrong for our binaries; removing it makes them run everywhere.
# A binary that really needs more than the baseline must not be passed through this.
set -eu
for f in "$@"; do
    objcopy --remove-section=.note.gnu.property "$f"
    if readelf -n "$f" 2>/dev/null | grep -q 'x86 ISA needed'; then
        echo "strip-isa-note: $f still carries an ISA note" >&2; exit 1
    fi
done
