import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";

import { PersistedThreadHttpAgent } from "@/lib/persisted-http-agent";
import { loadThreadSnapshotFromBackend } from "@/lib/awaken-backend";

const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:38080";
const AGENT_IDS = [
  "default",
  "permission",
  "stopper",
  "travel",
  "research",
] as const;

const agents = Object.fromEntries(
  AGENT_IDS.map((agentId) => [
    agentId,
    // Type cast avoids minor transitive @ag-ui type drift inside CopilotKit runtime deps.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    new PersistedThreadHttpAgent({
      url: `${BACKEND_URL}/v1/ag-ui/agents/${agentId}/runs`,
      loadThreadSnapshot: loadThreadSnapshotFromBackend,
    }) as any,
  ]),
);

const runtime = new CopilotRuntime({
  agents,
});

const { handleRequest } = copilotRuntimeNextJSAppRouterEndpoint({
  runtime,
  serviceAdapter: new ExperimentalEmptyAdapter(),
  endpoint: "/api/copilotkit",
});

export { handleRequest };
