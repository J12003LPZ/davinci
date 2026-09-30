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
cargo build --release -p davinci-voice --features native --bin davinci-voice-worker --locked --target-dir "$root/target"
# Build both before replacing either executable; both come from this checkout.
cp "$root/target/release/davinci" "$stage/davinci"
cp "$root/target/release/davinci-voice-worker" "$stage/davinci-voice-worker"
# Freeze and verify both byte sets before changing the installation.
for binary in davinci davinci-voice-worker; do
	python3 "$root/scripts/release_identity.py" record --proof "$proof" --binary "$stage/$binary" --output "$stage/$binary.identity.json"
done
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$bin_dir"
notice_dir="${CARGO_HOME:-$HOME/.cargo}/share/davinci-voice"
mkdir -p "$notice_dir"
cp "$root/crates/davinci-voice/THIRD_PARTY_NOTICES.md" "$notice_dir/"
cp -R "$root/crates/davinci-voice/licenses" "$notice_dir/"
for binary in davinci-voice-worker davinci; do
	install -m 755 "$stage/$binary" "$bin_dir/$binary"
	install -m 644 "$stage/$binary.identity.json" "$bin_dir/$binary.identity.json"
done
echo "Installed davinci $(davinci --version) to $(command -v davinci)"
echo "TypeScript sources remain in vendor/davinci as the behavioral reference (legacy-pi)."
