#!/bin/sh
# Creates demo identities. Never use this CA for production.
set -eu
umask 077

dir=${1:-"$HOME/.config/fleet/dev-tls"}
server_name=${2:-localhost}
case "$server_name" in
    *[!a-zA-Z0-9.-]*)
        printf '%s\n' 'Use a simple DNS name for the server certificate.' >&2
        exit 1
        ;;
esac
mkdir -p "$(dirname "$dir")"
if ! mkdir "$dir"; then
    printf '%s\n' 'Use a new directory; existing keys will not be overwritten.' >&2
    exit 1
fi

printf '%s\n' '[req]' 'distinguished_name=dn' '[dn]' '[ca]' \
    'basicConstraints=critical,CA:TRUE,pathlen:0' \
    'keyUsage=critical,keyCertSign,cRLSign' 'subjectKeyIdentifier=hash' > "$dir/ca.cnf"
openssl ecparam -name prime256v1 -genkey -noout -out "$dir/ca.key"
openssl req -new -x509 -sha256 -days 30 -key "$dir/ca.key" \
    -subj '/CN=Fleet Demo CA' -config "$dir/ca.cnf" -extensions ca -out "$dir/ca.crt"

for name in server client; do
    openssl ecparam -name prime256v1 -genkey -noout -out "$dir/$name.key"
    openssl req -new -sha256 -key "$dir/$name.key" -subj "/CN=Fleet $name" -out "$dir/$name.csr"
    printf '%s\n' 'basicConstraints=critical,CA:FALSE' \
        'keyUsage=critical,digitalSignature' > "$dir/$name.ext"
    if [ "$name" = server ]; then
        printf '%s\n' 'extendedKeyUsage=serverAuth' \
            "subjectAltName=DNS:$server_name,IP:127.0.0.1,IP:::1" >> "$dir/$name.ext"
    else
        printf '%s\n' 'extendedKeyUsage=clientAuth' >> "$dir/$name.ext"
    fi
    openssl x509 -req -sha256 -days 30 -in "$dir/$name.csr" \
        -CA "$dir/ca.crt" -CAkey "$dir/ca.key" -CAcreateserial \
        -out "$dir/$name.crt" -extfile "$dir/$name.ext"
done

printf 'Demo certificates created in %s\n' "$dir"
