.PHONY: all debug release test

all: debug release

debug:
	cargo build

release:
	cargo build --release

test:
	cargo test
