#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../certs"

mkdir -p "$CERTS"

openssl req -x509 -newkey rsa:2048 -days 3650 -nodes -keyout "$CERTS/client-ca-key.pem" -out "$CERTS/client-ca-cert.pem" -subj "/CN=Marinade test"

openssl req -x509 -nodes -days 365 -newkey rsa:4096 -keyout "$CERTS/localhost.key" -out "$CERTS/localhost.cert" -subj "/CN=localhost" -addext "subjectAltName = DNS:localhost"
