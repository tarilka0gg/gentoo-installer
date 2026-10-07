#!/bin/sh
# iso-autoscan <volume label>: loop-attach the first *.iso (<=3 directories deep) on any partition whose ISO 9660 volume label
# matches, so that udev sees /dev/disk/by-label/<label> as for a real CD. The partition stays mounted: the loop device reads from it.
command -v getarg > /dev/null || . /lib/dracut-lib.sh
PATH=/usr/sbin:/usr/bin:/sbin:/bin

label=$1
[ -z "$label" ] && exit 1
[ -e "/dev/disk/by-label/$label" ] && exit 0      # a real CD, or Ventoy's virtual one: nothing to do
ismounted /run/initramfs/isoscan && exit 0

mkdir -p /run/initramfs/isoscan
for dev in /dev/disk/by-uuid/*; do
    [ -e "$dev" ] || continue
    name=$(dev_unit_name "$dev")
    [ -e "/tmp/isoautoscan-$name" ] && continue
    : > "/tmp/isoautoscan-$name"
    mount -t auto -o ro "$dev" /run/initramfs/isoscan 2> /dev/null || continue
    find /run/initramfs/isoscan -maxdepth 3 -iname '*.iso' -size +50M 2> /dev/null | while read -r iso; do
        vol=$(dd if="$iso" bs=1 skip=32808 count=32 2> /dev/null | tr -d '\0' | sed 's/ *$//')
        [ "$vol" = "$label" ] && { echo "$iso"; break; }
    done > /tmp/isoautoscan-found
    iso=$(cat /tmp/isoautoscan-found)
    if [ -n "$iso" ]; then
        info "iso-autoscan: $iso on $dev"
        losetup -f "$iso"
        udevadm trigger --action=add > /dev/null 2>&1
        ln -sf "$dev" /run/initramfs/isoscandev
        exit 0
    fi
    umount /run/initramfs/isoscan
done
rmdir /run/initramfs/isoscan 2> /dev/null
exit 1
