# AI SDK Frontend E2E Example

Next.js frontend using Vercel AI SDK v6 (`@ai-sdk/react`) to chat with `carve-agentos-server`.

## Architecture

```
Browser (useChat) → Next.js API Route → carve-agentos-server → LLM
                    (SSE passthrough)
```

The server emits [AI SDK v6 UI Message Stream](https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol) events as SSE (`data: {"type":"text-delta","id":"txt_0","delta":"..."}`). The Next.js API route at `app/api/chat/route.ts` passes the SSE stream through directly with the `X-Vercel-AI-UI-Message-Stream: v1` header.

## Prerequisites

- Node.js 18+
- A running `carve-agentos-server` instance (default: `http://localhost:8080`)

## Quick Start

```bash
# 1. Start the Rust backend with DeepSeek (in project root)
DEEPSEEK_API_KEY=<key> cargo run --package carve-agentos-server -- \
  --http-addr 127.0.0.1:8080 \
  --config e2e/ai-sdk-frontend/agent-config.json

# 2. Install dependencies and start the frontend
cd e2e/ai-sdk-frontend
npm install
npm run dev
```

Open http://localhost:3001 and send a message.

## With TensorZero (E2E)

```bash
# 1. Start TensorZero + ClickHouse
DEEPSEEK_API_KEY=<key> docker compose -f e2e/tensorzero/docker-compose.yml up -d --wait

# 2. Start the Rust backend with TensorZero
cargo run --package carve-agentos-server -- --http-addr 127.0.0.1:8080

# 3. Start the frontend
cd e2e/ai-sdk-frontend
npm install
npm run dev
```

## Configuration

| Variable | Default | Description |
|---|---|---|
| `BACKEND_URL` | `http://localhost:8080` | carve-agentos-server address |

Set via `.env.local` or environment:

```bash
BACKEND_URL=http://localhost:9090 npm run dev
```

## Verify

1. Open http://localhost:3001
2. Type a message (e.g. "What is 2+2?")
3. Confirm streaming response appears token-by-token
4. Check browser DevTools Network tab: the `/api/chat` request should return `Content-Type: text/event-stream` with `X-Vercel-AI-UI-Message-Stream: v1` header
