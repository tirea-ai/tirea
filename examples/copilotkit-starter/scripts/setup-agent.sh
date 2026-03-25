#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found in PATH."
  echo "Install Rust toolchain from https://rustup.rs/"
  exit 1
fi

echo "Rust toolchain detected: $(cargo --version)"
echo "Checking backend manifest and prefetching crates..."
cargo fetch --manifest-path "${ROOT_DIR}/agent/Cargo.toml" >/dev/null
echo "Agent setup check passed."
