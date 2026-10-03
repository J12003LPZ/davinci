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
	bash scripts/install.sh

install-davinci: install
	davinci --version
