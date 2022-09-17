.PHONY: build-all build-server run-server clean

build-server:
	cargo build

build-all: build-server

run-server: build-server
	cargo run --bin mtransaction-server

clean:
	rm -rf target
