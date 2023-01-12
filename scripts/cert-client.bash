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
  openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/client.$IDENTITY.key" -out "$CERTS/client.req" -subj "/CN=$IDENTITY" -addext extendedKeyUsage=clientAuth

  cat "$CERTS/client.req"
elif [[ $CMD == "req-mac" ]]
then
  IDENTITY="$2"
  if [[ -z "$IDENTITY" ]]
  then
    echo "Usage: $0 req-mac <validator>"
    exit 1
  fi
  openssl req -newkey rsa:4096 -nodes -keyout "$CERTS/client.$IDENTITY.key" -out "$CERTS/client.req" -config <(printf 'extendedKeyUsage=clientAuth\n[dn]\nCN='"$IDENTITY"'\n[req]\nprompt=no\ndistinguished_name = dn\n')

  cat "$CERTS/client.req"
elif [[ $CMD == "sign" ]]
then
  VALIDATOR=$(openssl req -noout -in "./certs/client.req" -subject | awk -F '=' '{print $NF}')
  openssl req -noout -text -in "$CERTS/client.req"

  echo "Certificate is meant for validator: [$VALIDATOR]"
  printf 'Sign the certificate (y/n)? '
  read answer

  if [[ $answer == ${answer#[Yy]} ]]
  then
      exit 1
  fi

  openssl x509 -req -in "$CERTS/client.req" -days 60 -CA "$CERTS/ca.cert" -CAkey "$CERTS/ca.key" -CAcreateserial -out "$CERTS/client.$VALIDATOR.cert" -extfile "$CERTS/openssl.client.conf"
  cat "$CERTS/client.$VALIDATOR.cert"
else
  echo "Usage: $0 <req|sign> ..."
  exit 1
fi
