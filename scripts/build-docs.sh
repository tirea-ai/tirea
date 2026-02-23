#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Building cargo doc..."
cargo doc --workspace --no-deps --manifest-path "$WORKSPACE_ROOT/Cargo.toml"

if ! command -v mdbook-mermaid >/dev/null 2>&1; then
    echo "error: mdbook-mermaid is required. Install with: cargo install mdbook-mermaid"
    exit 1
fi

echo "==> Installing Mermaid support..."
mdbook-mermaid install "$WORKSPACE_ROOT/docs/book"

echo "==> Building mdBook..."
mdbook build "$WORKSPACE_ROOT/docs/book"

# Copy cargo doc output into book output for unified serving
if [ -d "$WORKSPACE_ROOT/target/book" ] && [ -d "$WORKSPACE_ROOT/target/doc" ]; then
    cp -r "$WORKSPACE_ROOT/target/doc" "$WORKSPACE_ROOT/target/book/doc"
    echo "==> Unified docs at: target/book/index.html"
    echo "    API docs at:     target/book/doc/tirea_state/index.html"
fi
