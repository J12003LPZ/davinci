.PHONY: test fmt clippy install install-davinci build pi

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

# Install the binary under the name `davinci`. On a machine that already has
# the TypeScript `pi` on PATH ahead of ~/.cargo/bin, the npm shim wins the name
# `pi`, so this build needs one of its own to be reachable at all.
install-davinci:
	cargo install --path crates/pi-coding-agent --force
	cp "$(HOME)/.cargo/bin/pi.exe" "$(HOME)/.cargo/bin/davinci.exe" 2>/dev/null || \
		cp "$(HOME)/.cargo/bin/pi" "$(HOME)/.cargo/bin/davinci"
	davinci --version
