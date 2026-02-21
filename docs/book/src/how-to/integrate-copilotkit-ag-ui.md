# Integrate CopilotKit (AG-UI)

Use this when your frontend uses CopilotKit and your backend is `tirea-agentos-server` AG-UI SSE.

## Prerequisites

- `tirea-agentos-server` is reachable (default `http://localhost:8080`).
- AG-UI endpoint is enabled: `POST /v1/ag-ui/agents/:agent_id/runs`.
- Next.js frontend.

## Steps

1. Add frontend dependencies:

```bash
npm install @copilotkit/react-core @copilotkit/react-ui @copilotkit/runtime @ag-ui/client
```

2. Create `lib/copilotkit-app.ts` to connect CopilotKit runtime to AG-UI.

```ts,ignore
import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";
import { HttpAgent } from "@ag-ui/client";

const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:8080";

const runtime = new CopilotRuntime({
  agents: {
    default: new HttpAgent({
      url: `${BACKEND_URL}/v1/ag-ui/agents/default/runs`,
    }) as any,
  },
});

const { handleRequest } = copilotRuntimeNextJSAppRouterEndpoint({
  runtime,
  serviceAdapter: new ExperimentalEmptyAdapter(),
  endpoint: "/api/copilotkit",
});

export { handleRequest };
```

3. Create route handler `app/api/copilotkit/route.ts`.

```ts,ignore
import { handleRequest } from "@/lib/copilotkit-app";

export const POST = handleRequest;
```

4. Wrap app with `CopilotKit` provider in `app/layout.tsx`.

```tsx,ignore
"use client";

import { CopilotKit } from "@copilotkit/react-core";
import "@copilotkit/react-ui/styles.css";

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <CopilotKit runtimeUrl="/api/copilotkit" agent="default">
          {children}
        </CopilotKit>
      </body>
    </html>
  );
}
```

5. Add chat UI in `app/page.tsx` using `CopilotChat` or `CopilotSidebar`.

## Verify

- `POST /api/copilotkit` streams AG-UI events.
- Chat UI receives assistant output in real time.
- Tool calls and shared state updates are reflected in the UI.

## Common Errors

- Wrong AG-UI URL (`/v1/ag-ui/agents/:agent_id/runs`) causes empty responses or 404.
- `agent` in `CopilotKit` provider does not match runtime `agents` key.
- Package version mismatch may require temporary `as any` cast for `HttpAgent`.

## Related

- [AG-UI Protocol](../reference/protocols/ag-ui.md)
- [HTTP API](../reference/http-api.md)
- `e2e/copilotkit-frontend/README.md`
- `e2e/copilotkit-frontend/lib/copilotkit-app.ts`
