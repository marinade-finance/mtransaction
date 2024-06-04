name = mtransaction

help:
	@echo "Usage:"
	@echo "    build-server    builds the dev version of the mtransaction server"
	@echo "    build           compile the package"
	@echo "    image           builds $(name) docker image"
	@echo "    help            show this help"

.PHONY: help build-all build-client build-server build-server-release run-server clean run-client run-client-local run-client-rpc-devnet run-client-rpc-mainnet run-client-blackhole

.DEFAULT_GOAL := build-all

CERT_DIR = certs
JWT_KEY = $(CERT_DIR)/jwtRS256.key

$(CERT_DIR)/ca.cert: $(CERT_DIR)/openssl.server.conf
	openssl req -x509 -newkey rsa:4096 -days 3650 -nodes -keyout "$(CERT_DIR)/ca.key" -out "$(CERT_DIR)/ca.cert" -subj "/CN=Marinade"

$(JWT_KEY):
	ssh-keygen -t rsa -b 4096 -m PEM -f $@ -N ""
	openssl rsa -in $@ -pubout -outform PEM -out $@.pub

cert-client:
	./scripts/cert-client.bash $(cmd) $(validator)

build-server:
	cargo build --bin mtx-server

build-server-release:
	cargo build --bin mtx-server --release

build-client:
	cargo build --bin mtx-client

build-client-release:
	cargo build --bin mtx-client --release

build-all: build-server build-client

build-all-release: build-server-release build-client-release

clean:
	rm -rf target certs/*.cert certs/*.key certs/*.srl certs/*.req demo/node_modules client/node_modules

run-server: build-server
	cargo run --bin mtx-server -- \
		--stake-override-identity foo     bar \
		--stake-override-sol      1000000 2000000 \
		--tls-grpc-server-cert    ./certs/localhost.cert \
		--tls-grpc-server-key     ./certs/localhost.key \
		--tls-grpc-ca-cert        ./certs/ca.cert \
		--jwt-public-key          ./certs/jwtRS256.key.pub

run-server-local: build-server
	cargo run --bin mtx-server -- \
		--stake-override-identity foo     bar \
		--stake-override-sol      1000000 2000000 \
		--tls-grpc-server-cert    ./certs/localhost.cert \
		--tls-grpc-server-key     ./certs/localhost.key \
		--tls-grpc-ca-cert        ./certs/ca.cert

run-client: build-client
	cargo run --bin mtx-client -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--grpc-urls-file       ./client.yml \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \

run-client-rpc-devnet: build-client
	cargo run --bin mtx-client -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--grpc-urls-file       ./client.yml \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--rpc-url              https://api.devnet.solana.com

run-client-rpc-mainnet: build-client
	cargo run --bin mtx-client -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--grpc-urls-file       ./client.yml \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--rpc-url              `<mainnet-rpc.txt`

run-client-blackhole: build-client
	cargo run --bin mtx-client -- \
		--grpc-urls-file       ./client.yml \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--blackhole
