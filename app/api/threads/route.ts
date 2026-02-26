import { listThreadIdsFromBackend } from "@/lib/tirea-backend";

export async function GET() {
  try {
    const threadIds = await listThreadIdsFromBackend();
    return Response.json(threadIds);
  } catch {
    return Response.json([]);
  }
}
