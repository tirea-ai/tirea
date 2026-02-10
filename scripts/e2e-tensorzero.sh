#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
    # Try sourcing from .bashrc
    # shellcheck disable=SC1090
    source ~/.bashrc 2>/dev/null || true
    if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
        echo "ERROR: DEEPSEEK_API_KEY not set"
        exit 1
    fi
fi

export DEEPSEEK_API_KEY

echo "Starting TensorZero + ClickHouse..."
docker compose -f e2e/tensorzero/docker-compose.yml up -d --wait

cleanup() {
    echo "Stopping services..."
    docker compose -f e2e/tensorzero/docker-compose.yml down -v
}
trap cleanup EXIT

echo "Running E2E tests via TensorZero..."
cargo test --package carve-agentos-server --test e2e_tensorzero -- --ignored --nocapture
