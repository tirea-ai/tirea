#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
    # shellcheck disable=SC1090
    source ~/.bashrc 2>/dev/null || true
    if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
        echo "ERROR: DEEPSEEK_API_KEY not set"
        exit 1
    fi
fi

export DEEPSEEK_API_KEY

PIDS=()
cleanup() {
    echo "Stopping services..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT

# Build backend.
echo "Building carve-agentos-server..."
cargo build --package carve-agentos-server

# Start backend.
echo "Starting backend on :8081..."
cargo run --package carve-agentos-server -- \
    --http-addr 127.0.0.1:8081 \
    --config e2e/ai-sdk-frontend/agent-config.json &
PIDS+=($!)

# Wait for backend.
for i in $(seq 1 30); do
    if curl -sf http://localhost:8081/health > /dev/null 2>&1; then
        echo "Backend ready."
        break
    fi
    if [[ $i -eq 30 ]]; then
        echo "ERROR: Backend failed to start"
        exit 1
    fi
    sleep 1
done

# Install frontend dependencies.
echo "Installing frontend dependencies..."
(cd e2e/ai-sdk-frontend && npm install --silent)
(cd e2e/copilotkit-frontend && npm install --silent)

# Start AI SDK frontend.
echo "Starting AI SDK frontend on :3001..."
(cd e2e/ai-sdk-frontend && BACKEND_URL=http://localhost:8081 npx next dev -p 3001) &
PIDS+=($!)

# Start CopilotKit frontend.
echo "Starting CopilotKit frontend on :3002..."
(cd e2e/copilotkit-frontend && BACKEND_URL=http://localhost:8081 npx next dev -p 3002) &
PIDS+=($!)

# Wait for frontends.
for port in 3001 3002; do
    for i in $(seq 1 60); do
        if curl -sf "http://localhost:$port" > /dev/null 2>&1; then
            echo "Frontend on :$port ready."
            break
        fi
        if [[ $i -eq 60 ]]; then
            echo "ERROR: Frontend on :$port failed to start"
            exit 1
        fi
        sleep 1
    done
done

# Install Playwright and run tests.
echo "Installing Playwright..."
(cd e2e/playwright && npm install --silent && npx playwright install chromium)

echo "Running Playwright tests..."
(cd e2e/playwright && npx playwright test --reporter=list)
