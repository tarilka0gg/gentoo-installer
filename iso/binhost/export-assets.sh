#!/bin/bash
# export-assets.sh: copies what the installer needs from a binhost build into installer-core/assets/binhost/:
#   binhost-signing.asc        the public key the packages are signed with (from <build>/out/)
#   package.use, package.accept_keywords, package.license   the Portage configuration the build converged on (spec/)
# usage: export-assets.sh <work-dir of a build>      (the installer then writes exactly these flags, so the binary packages match)
set -euo pipefail
W=$(readlink -f "${1:?work dir of a build}"); HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
A=$HERE/../../installer-core/assets/binhost; mkdir -p "$A"
cp "$W/out/binhost-signing.asc" "$A/binhost-signing.asc"
for d in package.use package.accept_keywords package.license; do
    # one file per kind; repeated lines dropped, comments (autounmask's "# required by") kept
    if [ -d "$HERE/spec/etc-portage/$d" ]; then cat "$HERE"/spec/etc-portage/"$d"/*; else cat "$HERE/spec/etc-portage/$d"; fi | awk '!seen[$0]++' > "$A/$d"
done
gpg --show-keys --with-colons "$A/binhost-signing.asc" | awk -F: '/^fpr/{print "key fingerprint:", $10; exit}'
