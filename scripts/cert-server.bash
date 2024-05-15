#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../certs"

mkdir -p "$CERTS"

CMD="$1"

if [[ $CMD == "ca" ]]
then
  openssl req -x509 -newkey rsa:4096 -days 3650 -nodes -keyout "$CERTS/ca.key" -out "$CERTS/ca.cert" -subj "/CN=Marinade"
elif [[ $CMD == "sign" ]]
then
  HOST="$2"

  if [[ -z "$HOST" ]]
  then
    echo "Usage: $0 sign <url>"
    exit 1
  fi

  openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/$HOST.key" -out "$CERTS/$HOST.req" -subj "/CN=$HOST" -addext "subjectAltName = DNS:$HOST" -addext extendedKeyUsage=serverAuth

  openssl x509 -req -in "$CERTS/$HOST.req" -days 365 -CA "$CERTS/ca.cert" -CAkey "$CERTS/ca.key" -CAcreateserial -out "$CERTS/$HOST.cert" -extfile "$CERTS/openssl.server.conf"
else
  echo "Usage: $0 <ca|sign> ..."
  exit 1
fi
