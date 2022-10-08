#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../certs"

mkdir -p "$CERTS"

openssl req -x509 -newkey rsa:4096 -days 3650 -nodes -keyout "$CERTS/ca.key" -out "$CERTS/ca.cert" -subj "/O=foo/OU=bar/CN=Marinade test"

openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/localhost.key" -out "$CERTS/localhost.req" -subj "/CN=localhost" -addext "subjectAltName = DNS:localhost" -addext extendedKeyUsage=serverAuth

openssl x509 -req -in "$CERTS/localhost.req" -days 365 -CA "$CERTS/ca.cert" -CAkey "$CERTS/ca.key" -CAcreateserial -out "$CERTS/localhost.cert" -extfile "$CERTS/openssl.server.conf"
