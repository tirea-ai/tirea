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
COPILOTKIT_BACKEND_PORT="${COPILOTKIT_BACKEND_PORT:-8083}"
COPILOTKIT_PORT="${COPILOTKIT_PORT:-3022}"
TRAVEL_BACKEND_PORT="${TRAVEL_BACKEND_PORT:-8082}"
TRAVEL_PORT="${TRAVEL_PORT:-3023}"

# Check port availability before starting.
for port in $BACKEND_PORT $AI_SDK_PORT $COPILOTKIT_BACKEND_PORT $COPILOTKIT_PORT $TRAVEL_BACKEND_PORT $TRAVEL_PORT; do
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

# Build backends.
echo "Building tirea-agentos-server, tirea-examples, and copilotkit-starter-agent..."
cargo build --package tirea-agentos-server --package tirea-examples --package copilotkit-starter-agent

# Start backend for ai-sdk.
echo "Starting backend on :$BACKEND_PORT..."
cargo run --package tirea-agentos-server -- \
    --http-addr "127.0.0.1:$BACKEND_PORT" \
    --config examples/ai-sdk-starter/agent-config.json &
PIDS+=($!)

# Start copilotkit backend (copilotkit-starter has its own agent binary).
echo "Starting copilotkit backend on :$COPILOTKIT_BACKEND_PORT..."
cargo run --package copilotkit-starter-agent -- \
    --http-addr "127.0.0.1:$COPILOTKIT_BACKEND_PORT" &
PIDS+=($!)

# Start travel backend (separate binary with custom tools/state).
echo "Starting travel backend on :$TRAVEL_BACKEND_PORT..."
cargo run --package tirea-examples --bin travel -- \
    --http-addr "127.0.0.1:$TRAVEL_BACKEND_PORT" &
PIDS+=($!)

# Wait for backends.
for port in $BACKEND_PORT $COPILOTKIT_BACKEND_PORT $TRAVEL_BACKEND_PORT; do
    for i in $(seq 1 30); do
        if curl -sf "http://localhost:$port/health" > /dev/null 2>&1; then
            echo "Backend on :$port ready."
            break
        fi
        if [[ $i -eq 30 ]]; then
            echo "ERROR: Backend on :$port failed to start"
            exit 1
        fi
        sleep 1
    done
done

# Install frontend dependencies.
echo "Installing frontend dependencies..."
(cd examples/ai-sdk-starter && npm install --silent)
(cd examples/copilotkit-starter && npm install --silent)
(cd examples/travel-ui && npm install --silent)

# Start AI SDK frontend.
echo "Starting AI SDK frontend on :$AI_SDK_PORT..."
(cd examples/ai-sdk-starter && BACKEND_URL="http://localhost:$BACKEND_PORT" npx next dev -p "$AI_SDK_PORT") &
PIDS+=($!)

# Start CopilotKit frontend (points to its own copilotkit backend).
echo "Starting CopilotKit frontend on :$COPILOTKIT_PORT..."
(cd examples/copilotkit-starter && BACKEND_URL="http://localhost:$COPILOTKIT_BACKEND_PORT" npx next dev -p "$COPILOTKIT_PORT") &
PIDS+=($!)

# Start Travel frontend (points to travel backend).
echo "Starting Travel frontend on :$TRAVEL_PORT..."
(cd examples/travel-ui && BACKEND_URL="http://localhost:$TRAVEL_BACKEND_PORT" npx next dev -p "$TRAVEL_PORT") &
PIDS+=($!)

# Wait for frontends.
for port in $AI_SDK_PORT $COPILOTKIT_PORT $TRAVEL_PORT; do
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
(cd e2e/playwright && \
    AI_SDK_PORT="$AI_SDK_PORT" \
    COPILOTKIT_PORT="$COPILOTKIT_PORT" \
    TRAVEL_PORT="$TRAVEL_PORT" \
    npx playwright test --reporter=list)
