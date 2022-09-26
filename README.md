# mTransaction
TBD

## Architecture
TBD

## Server
Generate certificates:
```bash
make cert-server
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
make cert-client cmd=sign
```
Get server certificate:
```bash
export HOST=mtx-dev-eu-central-1.marinade.finance
export PORT=50051
echo -n | openssl s_client -connect $HOST:$PORT | openssl x509 > ./certs/$HOST.cert
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
