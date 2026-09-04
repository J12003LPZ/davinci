#!/usr/bin/env bash
# Install the Rust `davinci` binary as the default product CLI.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v cargo >/dev/null 2>&1; then
	echo "davinci: rustc/cargo is required. Install https://rustup.rs and retry." >&2
	exit 1
fi

cargo install --path "$root/crates/davinci-coding-agent" --force
echo "Installed davinci $(davinci --version) to $(command -v davinci)"
echo "TypeScript sources remain in vendor/pi as the behavioral reference (legacy-pi)."
