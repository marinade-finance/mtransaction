#!/usr/bin/env bash
set -ex

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../cert"

mkdir -p "$CERTS"

openssl req -x509 -newkey rsa:4096 -days 3650 -nodes -keyout "$CERTS/ca-key.pem" -out "$CERTS/ca-cert.pem" \
  -subj "/CN=Marinade Test CA/OU=Mariande/emailAddress=test-mtx@marinade.finance"

openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/server-key.pem" -out "$CERTS/server-req.pem" \
  -subj "/CN=Marinade Test Server 1/OU=Mariande/emailAddress=test-mtx@marinade.finance"

openssl x509 -req -in "$CERTS/server-req.pem" -days 3650 -CA "$CERTS/ca-cert.pem" -CAkey "$CERTS/ca-key.pem" -CAcreateserial -out "$CERTS/server-cert.pem"
