#!/usr/bin/env bash
# Install the Rust `davinci` binary as the default product CLI.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v cargo >/dev/null 2>&1; then
	echo "davinci: rustc/cargo is required. Install https://rustup.rs and retry." >&2
	exit 1
fi

stage="$(mktemp -d)"
proof="$stage/ci-proof.json"
trap 'rm -rf "$stage"' EXIT
python3 "$root/scripts/release_identity.py" preflight --repo "$root" --require-tag --output "$proof"

cargo build --release -p davinci-coding-agent --locked --target-dir "$root/target"
cp "$root/target/release/davinci" "$stage/davinci"
python3 "$root/scripts/release_identity.py" record --proof "$proof" --binary "$stage/davinci" --output "$stage/davinci.identity.json"
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$bin_dir"
install -m 755 "$stage/davinci" "$bin_dir/davinci"
install -m 644 "$stage/davinci.identity.json" "$bin_dir/davinci.identity.json"
echo "Installed davinci $(davinci --version) to $(command -v davinci)"
echo "TypeScript sources remain in vendor/davinci as the behavioral reference (legacy-pi)."
