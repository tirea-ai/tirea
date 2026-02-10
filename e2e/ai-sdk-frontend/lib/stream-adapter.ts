/**
 * Transforms our server's SSE events into Vercel AI SDK Data Stream Protocol.
 *
 * Our server emits:  data: {"type":"text-delta","delta":"Hello"}\n\n
 * AI SDK expects:    0:"Hello"\n
 *
 * Protocol mapping:
 *   text-delta  → 0:"<text>"\n        (text part)
 *   finish      → e:{"finishReason":"stop"}\n + d:{"finishReason":"stop"}\n
 *   error       → 3:"<message>"\n     (error part)
 */
export function createSSEToDataStreamTransform(): TransformStream<
  Uint8Array,
  Uint8Array
> {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  let buffer = "";

  return new TransformStream({
    transform(chunk, controller) {
      buffer += decoder.decode(chunk, { stream: true });

      // SSE events are separated by double newlines
      const parts = buffer.split("\n\n");
      // Keep the last (possibly incomplete) part in the buffer
      buffer = parts.pop() ?? "";

      for (const part of parts) {
        const line = part
          .split("\n")
          .find((l) => l.startsWith("data: "));
        if (!line) continue;

        const json = line.slice(6); // strip "data: "
        if (!json) continue;

        let event: { type: string; delta?: string; errorText?: string };
        try {
          event = JSON.parse(json);
        } catch {
          continue;
        }

        let output = "";

        switch (event.type) {
          case "text-delta":
            if (event.delta) {
              // Data Stream Protocol: 0:"text"\n
              output = `0:${JSON.stringify(event.delta)}\n`;
            }
            break;

          case "finish":
            // Emit finish event + done signal
            output =
              `e:${JSON.stringify({ finishReason: "stop", usage: { promptTokens: 0, completionTokens: 0 } })}\n` +
              `d:${JSON.stringify({ finishReason: "stop", usage: { promptTokens: 0, completionTokens: 0 } })}\n`;
            break;

          case "error":
            output = `3:${JSON.stringify(event.errorText ?? "Unknown error")}\n`;
            break;

          // Ignore other event types (message-start, text-start, text-end, run-info, etc.)
        }

        if (output) {
          controller.enqueue(encoder.encode(output));
        }
      }
    },

    flush(controller) {
      // Process any remaining buffered data
      if (buffer.trim()) {
        const line = buffer
          .split("\n")
          .find((l) => l.startsWith("data: "));
        if (line) {
          const json = line.slice(6);
          try {
            const event = JSON.parse(json);
            if (event.type === "text-delta" && event.delta) {
              controller.enqueue(
                encoder.encode(`0:${JSON.stringify(event.delta)}\n`)
              );
            }
          } catch {
            // ignore
          }
        }
      }
    },
  });
}
