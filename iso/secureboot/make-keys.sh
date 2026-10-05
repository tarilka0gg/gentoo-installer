#!/bin/bash
# make-keys.sh <dir>: a Secure Boot key set for Simple Linux (PK, KEK, db), RSA-3072, 10 years.
# The PRIVATE keys stay in <dir> (keep them out of git and off the images); only db.cer (DER) is shipped:
# a firmware that has it enrolled will run the signed Limine, and nothing else signed by nobody.
set -euo pipefail
D=${1:?key dir}; mkdir -p "$D"; chmod 700 "$D"; cd "$D"
for k in PK KEK db; do
    [ -e $k.key ] && continue
    openssl req -new -x509 -newkey rsa:3072 -nodes -sha256 -days 3650 \
        -subj "/CN=Simple Linux Secure Boot $k/" -keyout $k.key -out $k.crt 2>/dev/null
    openssl x509 -in $k.crt -outform DER -out $k.cer
    chmod 600 $k.key
done
echo "keys in $D (db.cer is the one to publish)"
