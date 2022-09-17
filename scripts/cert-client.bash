#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../cert"

mkdir -p "$CERTS"

CMD="$1"

if [[ $CMD == "req" ]]
then
  VALIDATOR="$2"
  if [[ -z "$VALIDATOR" ]]
  then
    echo "Usage: $0 req <validator>"
    exit 1
  fi
  openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/client-key.pem" -out "$CERTS/client-req.pem" \
    -subj "/OU=$VALIDATOR/CN=$VALIDATOR"

  cat "$CERTS/client-req.pem"
elif [[ $CMD == "sign" ]]
then
  openssl req -noout -text -in "$CERTS/client-req.pem"
  openssl x509 -req -in "$CERTS/client-req.pem" -days 60 -CA "$CERTS/client-ca-cert.pem" -CAkey "$CERTS/client-ca-key.pem" -CAcreateserial -out "$CERTS/client-cert.pem"

  cat "$CERTS/client-cert.pem"
else
  echo "Usage: $0 <req|sign> ..."
  exit 1
fi
