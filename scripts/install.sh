#!/usr/bin/env bash
# Install the Rust `davinci` binary as the default product CLI.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v cargo >/dev/null 2>&1; then
	echo "davinci: rustc/cargo is required. Install https://rustup.rs and retry." >&2
	exit 1
fi

cargo build --release -p davinci-coding-agent --locked
cargo build --release -p davinci-voice --features native --bin davinci-voice-worker --locked
# Build both before replacing either executable; both come from this checkout.
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$bin_dir"
notice_dir="${CARGO_HOME:-$HOME/.cargo}/share/davinci-voice"
mkdir -p "$notice_dir"
cp "$root/crates/davinci-voice/THIRD_PARTY_NOTICES.md" "$notice_dir/"
cp -R "$root/crates/davinci-voice/licenses" "$notice_dir/"
install -m 755 "$root/target/release/davinci-voice-worker" "$bin_dir/davinci-voice-worker"
install -m 755 "$root/target/release/davinci" "$bin_dir/davinci"
echo "Installed davinci $(davinci --version) to $(command -v davinci)"
echo "TypeScript sources remain in vendor/davinci as the behavioral reference (legacy-pi)."
