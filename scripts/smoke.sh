#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if [[ -f ".env.local" ]]; then
  while IFS='=' read -r raw_key raw_value; do
    key="$(echo "${raw_key}" | xargs)"
    if [[ -z "${key}" ]] || [[ "${key}" == \#* ]]; then
      continue
    fi
    if [[ -n "${!key:-}" ]]; then
      continue
    fi
    value="$(echo "${raw_value:-}" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    export "${key}=${value}"
  done < ".env.local"
fi

FRONTEND_URL="${FRONTEND_URL:-http://localhost:3000}"
BACKEND_URL="${BACKEND_URL:-http://localhost:38080}"
AGENT_ID="${AGENT_ID:-default}"

fetch_code() {
  local method="$1"
  local url="$2"
  local code
  if code="$(curl -sS -X "${method}" -o /dev/null -w "%{http_code}" --connect-timeout 3 --max-time 20 "${url}")"; then
    echo "${code}"
  else
    echo "000"
  fi
}

assert_reachable() {
  local name="$1"
  local code="$2"
  if [[ "${code}" == "000" ]]; then
    echo "✗ ${name} is not reachable"
    exit 1
  fi
}

echo "Running smoke checks..."
echo "FRONTEND_URL=${FRONTEND_URL}"
echo "BACKEND_URL=${BACKEND_URL}"
echo "AGENT_ID=${AGENT_ID}"

frontend_code="$(fetch_code GET "${FRONTEND_URL}")"
assert_reachable "frontend root" "${frontend_code}"
echo "✓ Frontend root reachable (HTTP ${frontend_code})"

persisted_page_code="$(fetch_code GET "${FRONTEND_URL}/persisted-threads")"
assert_reachable "persisted threads page" "${persisted_page_code}"
if [[ "${persisted_page_code}" == "404" ]]; then
  echo "✗ /persisted-threads returned 404 (persisted variant route is missing)"
  exit 1
fi
echo "✓ Persisted threads page reachable (HTTP ${persisted_page_code})"

canvas_page_code="$(fetch_code GET "${FRONTEND_URL}/canvas")"
assert_reachable "canvas page" "${canvas_page_code}"
if [[ "${canvas_page_code}" == "404" ]]; then
  echo "✗ /canvas returned 404 (canvas variant route is missing)"
  exit 1
fi
echo "✓ Canvas page reachable (HTTP ${canvas_page_code})"

runtime_code="$(fetch_code GET "${FRONTEND_URL}/api/copilotkit")"
assert_reachable "copilot runtime route" "${runtime_code}"
if [[ "${runtime_code}" == "404" ]]; then
  echo "✗ /api/copilotkit returned 404 (route wiring is missing)"
  exit 1
fi
echo "✓ Copilot runtime route reachable (HTTP ${runtime_code})"

threads_route_code="$(fetch_code GET "${FRONTEND_URL}/api/threads")"
assert_reachable "threads route" "${threads_route_code}"
if [[ "${threads_route_code}" == "404" ]]; then
  echo "✗ /api/threads returned 404 (thread listing route wiring is missing)"
  exit 1
fi
echo "✓ Threads route reachable (HTTP ${threads_route_code})"

backend_runs_url="${BACKEND_URL%/}/v1/ag-ui/agents/${AGENT_ID}/runs"
backend_code="$(fetch_code POST "${backend_runs_url}")"
assert_reachable "backend AG-UI runs endpoint" "${backend_code}"
if [[ "${backend_code}" == "404" ]]; then
  echo "⚠ Backend returned 404 for agent '${AGENT_ID}' (check AGENT_ID or backend route)"
else
  echo "✓ Backend AG-UI endpoint reachable (HTTP ${backend_code})"
fi

echo "Smoke checks completed."
