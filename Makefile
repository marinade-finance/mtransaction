.PHONY: build-all build-server build-server-release run-server clean run-client run-client-local

cert-server:
	./scripts/cert-server.bash

cert-client:
	./scripts/cert-client.bash $(cmd) $(validator)

build-server:
	cargo build

build-server-release:
	cargo build --release

build-all: build-server

run-server: build-server
	cargo run --bin mtx-server -- \
		--stake-override-identity foo bar \
		--stake-override-sol 1000000 2000000 \
		--tls-grpc-server-cert    ./certs/localhost.cert \
		--tls-grpc-server-key     ./certs/localhost.key \
		--tls-grpc-client-ca-cert ./certs/client-ca.cert

clean:
	rm -rf target cert

run-client-local:
	TLS_GRPC_SERVER_CERT=./certs/localhost.cert \
	TLS_GRPC_CLIENT_KEY=./certs/client.$(client).key \
	TLS_GRPC_CLIENT_CERT=./certs/client.$(client).cert \
	GRPC_SERVER_ADDR=localhost:50051 \
		node ./client/mconnector.js

run-client:
	TLS_GRPC_SERVER_CERT=./certs/mtx-dev-eu-central-1.marinade.finance.cert \
	TLS_GRPC_CLIENT_KEY=./certs/client.$(client).key \
	TLS_GRPC_CLIENT_CERT=./certs/client.$(client).cert \
	GRPC_SERVER_ADDR=mtx-dev-eu-central-1.marinade.finance:50051 \
	SOLANA_CLUSTER_URL=https://api.devnet.solana.com \
		node ./client/mconnector.js
