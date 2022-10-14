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
curl localhost:3000 -X POST -H "Content-Type: application/json" -d '{"jsonrpc": "2.0", "method": "sendPriorityTransaction", "id":123, "params":["foo"] }'
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
Install MTX client:
```bash
# as root
curl -LSfs https://public.marinade.finance/mtx-client-quic -o /usr/local/bin/mtx-client-quic
chmod +x /usr/local/bin/mtx-client-quic
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
