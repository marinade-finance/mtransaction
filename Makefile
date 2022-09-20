.PHONY: build-all build-server run-server clean

cert-server:
	./scripts/cert-server.bash

cert-client:
	./scripts/cert-client.bash $(cmd) $(validator)

build-server:
	cargo build

build-all: build-server

run-server: build-server
	cargo run --bin mtx-server

clean:
	rm -rf target cert
