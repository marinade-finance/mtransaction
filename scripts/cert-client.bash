#!/usr/bin/env bash
set -ex

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../cert"

mkdir -p "$CERTS"

openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/client-key.pem" -out "$CERTS/client-req.pem" \
  -subj "/OU=Validator1/CN=Validator1/emailAddress=mx-test-validator-1@marinade.finance"

openssl x509 -req -in "$CERTS/client-req.pem" -days 60 -CA "$CERTS/ca-cert.pem" -CAkey "$CERTS/ca-key.pem" -CAcreateserial -out "$CERTS/client-cert.pem"
