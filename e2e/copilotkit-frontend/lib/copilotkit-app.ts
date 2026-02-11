import {
  CopilotRuntime,
  ExperimentalEmptyAdapter,
  copilotRuntimeNextJSAppRouterEndpoint,
} from "@copilotkit/runtime";
import { HttpAgent } from "@ag-ui/client";

const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:8080";

const runtime = new CopilotRuntime({
  agents: {
    // Type cast needed: top-level @ag-ui/client may differ from CopilotKit's internal version
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    default: new HttpAgent({
      url: `${BACKEND_URL}/v1/agents/default/runs/ag-ui/sse`,
    }) as any,
  },
});

const { handleRequest } = copilotRuntimeNextJSAppRouterEndpoint({
  runtime,
  serviceAdapter: new ExperimentalEmptyAdapter(),
  endpoint: "/api/copilotkit",
});

export { handleRequest };
