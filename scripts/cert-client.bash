#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../certs"

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
  openssl req -newkey rsa:2048 -nodes -keyout "$CERTS/client.key" -out "$CERTS/client-req.pem" -subj "/CN=$VALIDATOR" -addext extendedKeyUsage=clientAuth

  cat "$CERTS/client-req.pem"
elif [[ $CMD == "sign" ]]
then
  openssl req -noout -text -in "$CERTS/client-req.pem"
  openssl x509 -req -in "$CERTS/client-req.pem" -days 60 -CA "$CERTS/client-ca-cert.pem" -CAkey "$CERTS/client-ca-key.pem" -CAcreateserial -out "$CERTS/client.cert" -extfile "$CERTS/openssl.conf"

  cat "$CERTS/client.cert"
else
  echo "Usage: $0 <req|sign> ..."
  exit 1
fi
