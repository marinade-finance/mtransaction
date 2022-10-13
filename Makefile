.PHONY: build-all build-server build-server-release run-server clean run-client run-client-local
.DEFAULT_GOAL := build-all

cert-server:
	./scripts/cert-server.bash

cert-client:
	./scripts/cert-client.bash $(cmd) $(validator)

build-server:
	cargo build --bin mtx-server

build-server-release:
	cargo build --bin mtx-server --release

build-client-quic:
	cargo build --bin mtx-client-quic

build-client-quic-release:
	cargo build --bin mtx-client-quic --release

build-all: build-server build-client-quic

build-all-release: build-server-release build-client-quic-release

clean:
	rm -rf target certs/*.cert certs/*.key certs/*.srl certs/*.req demo/node_modules client/node_modules

run-server: build-server
	cargo run --bin mtx-server -- \
		--stake-override-identity foo     bar \
		--stake-override-sol      1000000 2000000 \
		--tls-grpc-server-cert    ./certs/localhost.cert \
		--tls-grpc-server-key     ./certs/localhost.key \
		--tls-grpc-ca-cert        ./certs/ca.cert

run-client-quic-local: build-client-quic
	cargo run --bin mtx-client-quic -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--grpc-url             http://localhost:50051 \
		--tpu-addr             "$(tpu)"

run-client-local:
	TLS_GRPC_SERVER_CERT=./certs/ca.cert \
	TLS_GRPC_CLIENT_KEY=./certs/client.$(client).key \
	TLS_GRPC_CLIENT_CERT=./certs/client.$(client).cert \
	GRPC_SERVER_ADDR=localhost:50051 \
		node ./client/mconnector.js

run-client:
	TLS_GRPC_SERVER_CERT=./certs/mtx-dev-eu-central-1.marinade.finance.cert \
	TLS_GRPC_CLIENT_KEY=./certs/client.$(client).key \
	TLS_GRPC_CLIENT_CERT=./certs/client.$(client).cert \
	GRPC_SERVER_ADDR=mtx-dev-eu-central-1.marinade.finance:50051 \
	SOLANA_CLUSTER_URL=http://localhost:8899 \
	THROTTLE_LIMIT=100 \
		node ./client/mconnector.js
