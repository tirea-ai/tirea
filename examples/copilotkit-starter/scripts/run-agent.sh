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

BACKEND_ADDR="${BACKEND_ADDR:-127.0.0.1:38080}"
if [[ -n "${BACKEND_URL:-}" ]]; then
  BACKEND_ADDR="${BACKEND_URL#http://}"
  BACKEND_ADDR="${BACKEND_ADDR#https://}"
fi
AGENT_ID="${AGENT_ID:-default}"
AGENT_MODEL="${AGENT_MODEL:-}"
AGENT_MAX_ROUNDS="${AGENT_MAX_ROUNDS:-5}"
AGENT_SYSTEM_PROMPT="${AGENT_SYSTEM_PROMPT:-You are a helpful assistant. Keep answers concise and actionable.}"
AGENT_MANIFEST="${ROOT_DIR}/agent/Cargo.toml"
AWAKEN_STORAGE_DIR="${AWAKEN_STORAGE_DIR:-${ROOT_DIR}/sessions}"

if [[ ! -f "${AGENT_MANIFEST}" ]]; then
  echo "agent manifest not found: ${AGENT_MANIFEST}"
  exit 1
fi

if [[ -z "${AGENT_MODEL}" ]]; then
  echo "AGENT_MODEL is not set. Set it to a model name (e.g. gpt-4o, claude-sonnet-4-20250514, deepseek-chat)."
  exit 1
fi

cleanup() {
  trap - EXIT INT TERM
}
trap cleanup EXIT INT TERM

echo "Starting awaken backend..."
echo "awaken source: crates.io"
echo "backend addr: ${BACKEND_ADDR}"
echo "agent manifest: ${AGENT_MANIFEST}"
echo "agent model: ${AGENT_MODEL}"
echo "agent id: ${AGENT_ID}"
echo "storage dir: ${AWAKEN_STORAGE_DIR}"

cd "${ROOT_DIR}"
AWAKEN_HTTP_ADDR="${BACKEND_ADDR}" \
AWAKEN_STORAGE_DIR="${AWAKEN_STORAGE_DIR}" \
AGENT_ID="${AGENT_ID}" \
AGENT_MODEL="${AGENT_MODEL}" \
AGENT_MAX_ROUNDS="${AGENT_MAX_ROUNDS}" \
AGENT_SYSTEM_PROMPT="${AGENT_SYSTEM_PROMPT}" \
exec cargo run --manifest-path "${AGENT_MANIFEST}" --bin copilotkit-starter-agent
