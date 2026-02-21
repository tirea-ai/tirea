# Ecosystem Integrations

This page summarizes common AG-UI and AI SDK integration patterns and maps them to this project.

## CopilotKit Integration Patterns

### Pattern A: Runtime Proxy + `HttpAgent` (remote backend)

Used when agent runtime is hosted separately and exposes AG-UI over HTTP.

- CopilotKit runtime runs in Next.js route (`/api/copilotkit`).
- `HttpAgent` points to backend AG-UI run endpoint.

Examples:

- [with-langgraph-fastapi](https://github.com/CopilotKit/with-langgraph-fastapi)
- [with-microsoft-agent-framework-python](https://github.com/CopilotKit/with-microsoft-agent-framework-python)
- [with-microsoft-agent-framework-dotnet](https://github.com/CopilotKit/with-microsoft-agent-framework-dotnet)

### Pattern B: Runtime Proxy + framework-specific adapter

Used when framework has a dedicated CopilotKit adapter.

- `LangGraphHttpAgent` / `LangGraphAgent`
- `MastraAgent`

Examples:

- [with-langgraph-js](https://github.com/CopilotKit/with-langgraph-js)
- [with-mastra](https://github.com/CopilotKit/with-mastra)

### Pattern C: Canvas and HITL focused apps

Used for shared state, generative UI, and human-in-the-loop workflows.

Examples:

- [canvas-with-langgraph-python](https://github.com/CopilotKit/canvas-with-langgraph-python)
- [canvas-with-mastra](https://github.com/CopilotKit/canvas-with-mastra)
- [canvas-with-llamaindex](https://github.com/CopilotKit/canvas-with-llamaindex)
- [with-langgraph-fastapi-persisted-threads](https://github.com/CopilotKit/with-langgraph-fastapi-persisted-threads)

## Official Starter Ecosystem Support

The table below combines AG-UI upstream support status with CopilotKit official starter availability.

| Framework/Spec | AG-UI Upstream Status | CopilotKit Starter Availability | Starter Repositories |
|---|---|---|---|
| LangGraph | Supported | `with-*`, `canvas-*`, `coagents-*`, persisted threads | `with-langgraph-fastapi`, `with-langgraph-fastapi-persisted-threads`, `with-langgraph-js`, `with-langgraph-python`, `canvas-with-langgraph-python`, `coagents-starter-langgraph` |
| Mastra | Supported | `with-*`, `canvas-*` | `with-mastra`, `canvas-with-mastra` |
| Pydantic AI | Supported | `with-*` | `with-pydantic-ai` |
| LlamaIndex | Supported | `with-*`, `canvas-*` | `with-llamaindex`, `canvas-with-llamaindex`, `canvas-with-llamaindex-composio` |
| CrewAI Flows | Supported (CrewAI) | `with-*`, `coagents-*` | `with-crewai-flows`, `coagents-starter-crewai-flows` |
| Microsoft Agent Framework | Supported | `with-*` | `with-microsoft-agent-framework-dotnet`, `with-microsoft-agent-framework-python` |
| Google ADK | Supported | `with-*` | `with-adk` |
| AWS Strands | Supported | `with-*` | `with-strands-python` |
| Agno | Supported | `with-*` | `with-agno` |
| AG2 | Supported | demo-level only | `ag2-feature-viewer` |
| A2A Protocol | Supported | protocol starters | `with-a2a-middleware`, `with-a2a-a2ui` |
| Oracle Agent Spec | Supported | protocol/spec starter | `with-agent-spec` |
| MCP Apps | Supported | protocol/spec starter | `with-mcp-apps` |
| AWS Bedrock Agents | In Progress | none yet | (no official starter repo found) |
| OpenAI Agent SDK | In Progress | none yet | (no official starter repo found) |
| Cloudflare Agents | In Progress | none yet | (no official starter repo found) |

## Minimum Feature Comparison (Official Starters)

This comparison is based on actual `page.tsx` and `route.ts` usage in each starter.

| Starter | Runtime Adapter | Shared State | Frontend Actions | Generative UI | HITL |
|---|---|---|---|---|---|
| `with-langgraph-fastapi` | `LangGraphHttpAgent` | Yes | Yes | Yes | Yes |
| `with-langgraph-js` | `LangGraphAgent` | Yes | Yes | Yes | No |
| `with-mastra` | `MastraAgent` | Yes | Yes | Yes | Yes |
| `with-pydantic-ai` | `HttpAgent` | Yes | Yes | Yes | Yes |
| `with-llamaindex` | `LlamaIndexAgent` | Yes | Yes | Yes | No |
| `with-crewai-flows` | `HttpAgent` | Yes | Yes | Yes | No |
| `with-agno` | `HttpAgent` | No | Yes | Yes | No |
| `with-adk` | `HttpAgent` | Yes | Yes | Yes | Yes |
| `with-strands-python` | `HttpAgent` | Yes | Yes | Yes | No |
| `with-microsoft-agent-framework-dotnet` | `HttpAgent` | Yes | Yes | Yes | Yes |
| `with-microsoft-agent-framework-python` | `HttpAgent` | Yes | Yes | Yes | Yes |
| `with-langgraph-fastapi-persisted-threads` | `LangGraphHttpAgent` | Yes | Yes | Yes | Yes |

Notes:

- Most starters use Next.js `copilotRuntimeNextJSAppRouterEndpoint` as the frontend runtime bridge.
- Most non-LangGraph/Mastra starters use generic `HttpAgent` transport.
- `with-langgraph-fastapi-persisted-threads` is the explicit persisted-thread variant.

## AG-UI Supported Frameworks (Upstream)

AG-UI upstream lists first-party/partner integrations including LangGraph, CrewAI, Microsoft Agent Framework, ADK, Strands, Mastra, Pydantic AI, Agno, and LlamaIndex.

- Upstream list: [AG-UI README integrations table](https://github.com/ag-ui-protocol/ag-ui)

## AI SDK Frontend Pattern

For `@ai-sdk/react`, the common pattern is:

1. `useChat` in the browser.
2. Next.js `/api/chat` route adapts UI messages to backend request body.
3. Route passes through SSE stream and AI SDK headers.

Related docs:

- [AI SDK Transport](https://ai-sdk.dev/docs/ai-sdk-ui/transport)
- [AI SDK `useChat`](https://ai-sdk.dev/docs/reference/ai-sdk-ui/use-chat)

## Mapping to Tirea Endpoints

Run streaming:

- AG-UI: `POST /v1/ag-ui/agents/:agent_id/runs`
- AI SDK v6: `POST /v1/ai-sdk/agents/:agent_id/runs`

History:

- Raw messages: `GET /v1/threads/:id/messages`
- AG-UI encoded: `GET /v1/ag-ui/threads/:id/messages`
- AI SDK encoded: `GET /v1/ai-sdk/threads/:id/messages`

## In-Repo Integration Examples

CopilotKit:

- `e2e/copilotkit-frontend/`
- `e2e/examples/travel-ui/`
- `e2e/examples/research-ui/`

AI SDK:

- `e2e/ai-sdk-frontend/`
