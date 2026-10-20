#!/bin/bash
# trim-stage.sh <stage-root>   (run as root)
#
# Drops what a stage3 carries but an installed system does not need: DocBook SGML/XML stylesheets
# (only used to build documentation), package docs, info pages, translations other than English and
# Ukrainian (the only languages the installer offers), the Fortran compiler (nothing in an install
# builds Fortran), the LTO dump tool and CMake's test runner, packager and curses front end (nothing
# in an install uses them; `cmake` itself stays), the same translations inside gcc's and binutils' own
# data directories, and compiled Python caches (regenerated on first use). About 85 MiB of the 240 MiB
# archive. The same patterns go into the stage's make.conf as INSTALL_MASK so `emerge -u` does not
# bring them back. Man pages, /usr/share/i18n (locale-gen), 32-bit gcc libraries (Steam) and the rest
# of the toolchain stay.
set -euo pipefail
R=$(readlink -f "${1:?stage root}")
[ -d "$R/usr/share" ] || { echo "$R is not a stage root" >&2; exit 1; }

MASK='/usr/share/sgml/docbook /usr/share/doc /usr/share/info /usr/libexec/gcc/*/*/f951'
MASK+=' /usr/share/locale/[a-df-km-tv-z]* /usr/share/locale/e[a-mo-z]* /usr/share/locale/u[a-jl-z]*'
MASK+=' /usr/bin/ctest /usr/bin/cpack /usr/bin/ccmake /usr/bin/lto-dump /usr/bin/*-lto-dump /usr/*-linux-gnu/gcc-bin/*/lto-dump'
MASK+=' /usr/share/gcc-data/*/*/locale/[a-df-km-tv-z]* /usr/share/gcc-data/*/*/locale/e[a-mo-z]* /usr/share/gcc-data/*/*/locale/u[a-jl-z]*'
MASK+=' /usr/share/binutils-data/*/*/locale/[a-df-km-tv-z]* /usr/share/binutils-data/*/*/locale/e[a-mo-z]* /usr/share/binutils-data/*/*/locale/u[a-jl-z]*'

shopt -s nullglob
rm -rf --one-file-system "$R"/usr/share/sgml/docbook "$R"/usr/share/doc "$R"/usr/share/info
rm -f "$R"/usr/libexec/gcc/*/*/f951
for d in "$R"/usr/share/locale/*/; do
    n=$(basename "$d")
    case $n in en*|uk*|l*) ;; *) rm -rf --one-file-system "$d" ;; esac
done
rm -f "$R"/usr/bin/{ctest,cpack,ccmake,lto-dump} "$R"/usr/bin/*-lto-dump "$R"/usr/*-linux-gnu/gcc-bin/*/lto-dump
for d in "$R"/usr/share/{gcc,binutils}-data/*/*/locale/*; do
    n=$(basename "$d")
    case $n in en*|uk*|l*) ;; *) rm -rf --one-file-system "$d" ;; esac
done
find "$R/usr/lib" -type d -name __pycache__ -prune -exec rm -rf --one-file-system {} +

# make.conf keeps lines it does not own, so the installer's rewrite leaves this one alone.
mkdir -p "$R/etc/portage"
touch "$R/etc/portage/make.conf"
sed -i '/^INSTALL_MASK=/d' "$R/etc/portage/make.conf"
printf '\n# Not installed on this system (see trim-stage.sh); delete the line to get them back.\nINSTALL_MASK="%s"\n' "$MASK" >> "$R/etc/portage/make.conf"
