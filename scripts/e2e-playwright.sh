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

# Configurable ports (use high ports to avoid conflicts with dev services).
BACKEND_PORT="${BACKEND_PORT:-8081}"
AI_SDK_PORT="${AI_SDK_PORT:-3021}"
COPILOTKIT_PORT="${COPILOTKIT_PORT:-3022}"

# Check port availability before starting.
for port in $BACKEND_PORT $AI_SDK_PORT $COPILOTKIT_PORT; do
    if ss -tlnH "sport = :$port" 2>/dev/null | grep -q LISTEN; then
        echo "ERROR: Port $port is already in use"
        exit 1
    fi
done

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
echo "Starting backend on :$BACKEND_PORT..."
cargo run --package carve-agentos-server -- \
    --http-addr "127.0.0.1:$BACKEND_PORT" \
    --config e2e/ai-sdk-frontend/agent-config.json &
PIDS+=($!)

# Wait for backend.
for i in $(seq 1 30); do
    if curl -sf "http://localhost:$BACKEND_PORT/health" > /dev/null 2>&1; then
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
echo "Starting AI SDK frontend on :$AI_SDK_PORT..."
(cd e2e/ai-sdk-frontend && BACKEND_URL="http://localhost:$BACKEND_PORT" npx next dev -p "$AI_SDK_PORT") &
PIDS+=($!)

# Start CopilotKit frontend.
echo "Starting CopilotKit frontend on :$COPILOTKIT_PORT..."
(cd e2e/copilotkit-frontend && BACKEND_URL="http://localhost:$BACKEND_PORT" npx next dev -p "$COPILOTKIT_PORT") &
PIDS+=($!)

# Wait for frontends.
for port in $AI_SDK_PORT $COPILOTKIT_PORT; do
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
(cd e2e/playwright && AI_SDK_PORT="$AI_SDK_PORT" COPILOTKIT_PORT="$COPILOTKIT_PORT" npx playwright test --reporter=list)
