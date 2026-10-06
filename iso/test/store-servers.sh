#!/bin/bash
# store-servers.sh <store-dir> <git-base-dir> start|stop   — loopback-only test "portage store" for VM installs.
# HTTP :8123 serves <store-dir> (Packages, kernels/…, installer binaries); `git daemon` :9418 serves every
# repository under <git-base-dir> (the overlay, simple-linux-configs). A QEMU user-net guest sees the host
# as 10.0.2.2. Run as the unprivileged user; stop kills only the PIDs recorded in /tmp.
set -euo pipefail
STORE=$(readlink -f "$1"); GITBASE=$(readlink -f "$2"); PIDS=/tmp/store-servers.pids
case "${3:-}" in
start)
    ( cd "$STORE" && nohup python3 -m http.server 8123 --bind 127.0.0.1 >/tmp/store-http.log 2>&1 & echo $! >> "$PIDS" )
    nohup git daemon --base-path="$GITBASE" --export-all --listen=127.0.0.1 --port=9418 --reuseaddr >/tmp/store-git.log 2>&1 &
    echo $! >> "$PIDS"; sleep 1; ss -ltn | grep -E ':(8123|9418)\b' ;;
stop)
    [ -s "$PIDS" ] && xargs -r kill < "$PIDS"; rm -f "$PIDS" ;;
*) echo "usage: $0 <store> <git-base> start|stop" >&2; exit 2 ;;
esac
