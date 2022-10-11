#!/bin/bash

set -e

if [[ -z $1 ]]
then
  echo "Usage: $0 <path-to-identity>"
  exit 1
fi

if ! [[ -f $1 ]]
then
  echo "File '$1' does not exist!"
  exit 1
fi

PUBKEY=$(solana-keygen pubkey $1)

TPU_ADDR=$(solana gossip --output json | jq -r '
  .[]
  | select(.identityPubkey=="'"$PUBKEY"'")
  | [.ipAddress, .tpuPort | tostring]
  | join(":")
')

if [[ -z $TPU_ADDR ]]
then
  echo "Failed to get TPU address!"
  exit 1
fi
