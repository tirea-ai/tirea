import {
  CopilotRuntime,
  createCopilotEndpoint,
} from "@copilotkit/runtime/v2";
import { HttpAgent } from "@ag-ui/client";

const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:8080";

const runtime = new CopilotRuntime({
  agents: {
    // Type cast needed: top-level @ag-ui/client may differ from CopilotKit's internal version
    default: new HttpAgent({
      url: `${BACKEND_URL}/v1/agents/default/runs/ag-ui/sse`,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    }) as any,
  },
});

const app = createCopilotEndpoint({
  runtime,
  basePath: "/api/copilotkit",
});

async function handler(req: Request) {
  const res = await app.fetch(req);
  // Re-wrap the response to ensure Next.js handles streaming correctly.
  return new Response(res.body, {
    status: res.status,
    headers: Object.fromEntries(res.headers.entries()),
  });
}

export const POST = handler;
export const GET = handler;
