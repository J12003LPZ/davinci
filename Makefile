.PHONY: test fmt clippy install install-davinci build davinci pi

davinci:
	cargo build -p davinci-coding-agent

pi: davinci

build: davinci

test:
	cargo test --workspace

fmt:
	cargo fmt --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

install:
	cargo install --path crates/davinci-coding-agent --force

install-davinci: install
	davinci --version

