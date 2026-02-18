# CopilotKit Frontend E2E Example

Next.js frontend using [CopilotKit](https://copilotkit.ai) to chat with `carve-agentos-server` via the [AG-UI protocol](https://docs.ag-ui.com/).

## Architecture

```
Browser (CopilotChat) → CopilotKit Runtime → carve-agentos-server (AG-UI SSE) → LLM
```

No protocol bridge needed — CopilotKit's `HttpAgent` connects directly to our AG-UI endpoint at `/v1/ag-ui/agents/default/runs`. The `@copilotkit/runtime` handles the AG-UI event stream natively.

## Prerequisites

- Node.js 18+
- A running `carve-agentos-server` instance (default: `http://localhost:8080`)

## Quick Start

```bash
# 1. Start the Rust backend with DeepSeek (in project root)
DEEPSEEK_API_KEY=<key> cargo run --package carve-agentos-server -- \
  --http-addr 127.0.0.1:8080 \
  --config e2e/copilotkit-frontend/agent-config.json

# 2. Install dependencies and start the frontend
cd e2e/copilotkit-frontend
npm install
npm run dev
```

Open http://localhost:3002 and send a message.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `BACKEND_URL` | `http://localhost:8080` | carve-agentos-server address |

Set via `.env.local` or environment:

```bash
BACKEND_URL=http://localhost:9090 npm run dev
```

## Verify

1. Open http://localhost:3002
2. Type a message (e.g. "What is 2+2?")
3. Confirm streaming response appears in the CopilotKit chat panel
4. Check browser DevTools Network tab: the `/api/copilotkit` request should stream AG-UI events
