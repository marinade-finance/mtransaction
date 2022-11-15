# mTransaction
TBD

## Architecture
TBD

## Server
Generate certificates:
```bash
make cert-server cmd=ca
make cert-server cmd=sign host=localhost
make cert-client cmd=req identity=foo
make cert-client cmd=sign
```
Run:
```bash
make run-server
```
Send transaction:
```bash
curl localhost:3000 -X POST -H "Content-Type: application/json" -d '{"jsonrpc": "2.0", "method": "sendPriorityTransaction", "id":123, "params":["ARshF1FLgiVW50Ni22v0MvVwbG+lzVF3Lny0RXdel49BaJ+h7CD3SsAA2611yJgrzywPPoH61NqEVnEamW8d2ggBAAEDB0RIKu9gtXbw72njHgZGjO2GvCj5asjUDjoWRvAjtgVSd+bmVQdeIet9vYadHhwEFFONs9iJcGGojpal6ubMaQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA0zi3p2jurWfGuUPD/3Ny0mYgNYCpRQhozRYVCJdLYzkBAgIAAQwCAAAARQAAAAAAAAA="] }'
```

## Client
Generate certificate:
```bash
make cert-client cmd=req validator=Validator1
make cert-client cmd=sign # performed on Marinade side
```
Install CA certificate:
```bash
# as root
curl -LSfs https://public.marinade.finance/mtx.ca.cert -o /etc/ssl/certs/mtx.ca.cert
```
Install MTX QUIC client:
```bash
# as root
curl -LSfs https://public.marinade.finance/mtx-client-quic -o /usr/local/bin/mtx-client-quic
chmod +x /usr/local/bin/mtx-client-quic
```
Run MTX QUIC client:
```bash
/usr/local/bin/mtx-client-quic \
  --tls-grpc-ca-cert     /etc/ssl/certs/mtx.ca.cert \
  --tls-grpc-client-key  /etc/ssl/certs/mtx.client.key \
  --tls-grpc-client-cert /etc/ssl/certs/mtx.client.cert \
  --grpc-url             https://mtx-perf-eu-central-1.marinade.finance:50051 \
  --tpu-addr             x.x.x.x \
  --identity             ./keys/key-validator-identity.json
```
Run nodeJS client:
```bash
export TLS_GRPC_SERVER_CERT=/etc/ssl/certs/mtx.ca.cert
export TLS_GRPC_CLIENT_KEY=/etc/ssl/certs/mtx.client.key
export TLS_GRPC_CLIENT_CERT=/etc/ssl/certs/mtx.client.cert
export GRPC_SERVER_ADDR=mtx-perf-eu-central-1.marinade.finance:50051
export SOLANA_CLUSTER_URL=http://localhost:8899
export THROTTLE_LIMIT=200

node ./client/mconnector.js
```

## Development
Before building the rust server, you need:
```bash
# Ubuntu
apt update && apt upgrade -y
apt install -y protobuf-compiler libprotobuf-dev

# Alpine
apk add protoc protobuf-dev
```
