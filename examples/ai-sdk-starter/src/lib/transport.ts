import { DefaultChatTransport } from "ai";
import { chatApiUrl } from "./api-client";

export function createTransport(
  sessionId: string,
  agentId: string,
): DefaultChatTransport {
  return new DefaultChatTransport({
    api: chatApiUrl(agentId),
    headers: { "x-session-id": sessionId },
    prepareSendMessagesRequest: ({ messages }) => ({
      body: {
        threadId: sessionId,
        messages,
      },
    }),
  });
}
