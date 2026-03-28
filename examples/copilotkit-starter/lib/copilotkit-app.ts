import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";

import { HttpAgent } from "@ag-ui/client";

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
    // Frontend sends only new messages + threadId.
    // Backend loads thread history from store automatically.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    new HttpAgent({
      url: `${BACKEND_URL}/v1/ag-ui/agents/${agentId}/runs`,
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
