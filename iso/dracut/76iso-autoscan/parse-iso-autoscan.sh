#!/bin/sh
# root=live:CDLABEL=<label> (and no explicit iso-scan/filename): add the scan as a settled job.
command -v getarg > /dev/null || . /lib/dracut-lib.sh
[ -n "$(getarg iso-scan/filename)" ] && return 0
[ "$(getarg rd.iso.autoscan)" = 0 ] && return 0
case "$(getarg root=)" in
    live:CDLABEL=*) _l=$(getarg root=); /sbin/initqueue --settled --unique --name iso-autoscan /sbin/iso-autoscan "${_l#live:CDLABEL=}" ;;
esac
