import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";

import { PersistedThreadHttpAgent } from "@/lib/persisted-http-agent";
import { loadThreadSnapshotFromBackend } from "@/lib/tirea-backend";

const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:38080";
const AGENT_ID = process.env.AGENT_ID ?? "default";

const runtime = new CopilotRuntime({
  agents: {
    // Type cast avoids minor transitive @ag-ui type drift inside CopilotKit runtime deps.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    default: new PersistedThreadHttpAgent({
      url: `${BACKEND_URL}/v1/ag-ui/agents/${AGENT_ID}/runs`,
      loadThreadSnapshot: loadThreadSnapshotFromBackend,
    }) as any,
  },
});

const { handleRequest } = copilotRuntimeNextJSAppRouterEndpoint({
  runtime,
  serviceAdapter: new ExperimentalEmptyAdapter(),
  endpoint: "/api/copilotkit",
});

export { handleRequest };
