#!/usr/bin/env bash
set -e

SCRIPT_DIR=$(dirname "$0")
CERTS="$SCRIPT_DIR/../certs"

mkdir -p "$CERTS"

CMD="$1"

if [[ $CMD == "req" ]]
then
  IDENTITY="$2"
  if [[ -z "$IDENTITY" ]]
  then
    echo "Usage: $0 req <validator>"
    exit 1
  fi
  openssl req -newkey rsa:2048 -nodes -keyout "$CERTS/client.$IDENTITY.key" -out "$CERTS/client-req.pem" -subj "/CN=$IDENTITY" -addext extendedKeyUsage=clientAuth

  cat "$CERTS/client-req.pem"
elif [[ $CMD == "sign" ]]
then
  VALIDATOR=$(openssl req -noout -in "./certs/client-req.pem" --subject | awk '{print $NF}')
  openssl req -noout -text -in "$CERTS/client-req.pem"

  echo "Certificate is meant for validator: [$VALIDATOR]"
  printf 'Sign the certificate (y/n)? '
  read answer

  if [[ $answer == ${answer#[Yy]} ]]
  then
      exit 1
  fi

  openssl x509 -req -in "$CERTS/client-req.pem" -days 60 -CA "$CERTS/client-ca.cert" -CAkey "$CERTS/client-ca.key" -CAcreateserial -out "$CERTS/client.$VALIDATOR.cert" -extfile "$CERTS/openssl.conf"
  cat "$CERTS/client.$VALIDATOR.cert"
else
  echo "Usage: $0 <req|sign> ..."
  exit 1
fi
