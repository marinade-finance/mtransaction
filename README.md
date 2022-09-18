# mTransaction
TBD

## Architecture
TBD

## Server
Generate certificate:
```bash
make cert-server
```
Run:
```bash
make run-server
```
Send transaction:
```bash
curl localhost:3000 -X POST -H "Content-Type: application/json" -d '{"jsonrpc": "2.0", "method": "send_transaction", "id":123 }'
```

## Client
Generate certificate:
```bash
make cert-client cmd=req validator=Validator1
make cert-client cmd=sign
```

## Tester
TBD

## Development
Before building the rust server, you need:
```bash
# Ubuntu
apt update && apt upgrade -y
apt install -y protobuf-compiler libprotobuf-dev

# Alpine
apk add protoc protobuf-dev
```
