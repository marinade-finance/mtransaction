.PHONY: build-all build-server build-server-release run-server clean run-client run-client-local
.DEFAULT_GOAL := build-all

cert-server:
	./scripts/cert-server.bash $(cmd) $(host)

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

run-client-local: build-client
	cargo run --bin mtx-client -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--rpc-url              http://127.0.0.1:8899

run-client-blackhole: build-client
	cargo run --bin mtx-client -- \
		--tls-grpc-ca-cert     ./certs/ca.cert \
		--tls-grpc-client-key  ./certs/client.$(client).key \
		--tls-grpc-client-cert ./certs/client.$(client).cert \
		--blackhole
