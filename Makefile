.PHONY: test fmt clippy install build pi

pi:
	cargo build -p pi-coding-agent

build: pi

test:
	cargo test --workspace

fmt:
	cargo fmt --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

install:
	cargo install --path crates/pi-coding-agent --force
