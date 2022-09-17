.PHONY: build-all build-server run-server clean

cert-server:
	./scripts/cert-server.bash

cert-client:
	./scripts/cert-client.bash

build-server:
	cargo build

build-all: build-server

run-server: build-server
	cargo run --bin mtransaction-server

clean:
	rm -rf target cert
