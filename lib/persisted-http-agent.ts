import type { BaseEvent, HttpAgentConfig, Message, RunAgentInput } from "@ag-ui/client";
import { HttpAgent } from "@ag-ui/client";
import { randomUUID } from "crypto";
import { Observable } from "rxjs";

import type { ThreadSnapshot } from "@/lib/tirea-backend";

type LoadThreadSnapshot = (threadId: string) => Promise<ThreadSnapshot | null>;

type PersistedThreadHttpAgentConfig = HttpAgentConfig & {
  loadThreadSnapshot: LoadThreadSnapshot;
};

export class PersistedThreadHttpAgent extends HttpAgent {
  private readonly loadThreadSnapshot: LoadThreadSnapshot;

  constructor(config: PersistedThreadHttpAgentConfig) {
    const { loadThreadSnapshot, ...httpConfig } = config;
    super(httpConfig);
    this.loadThreadSnapshot = loadThreadSnapshot;
  }

  run(input: RunAgentInput): Observable<BaseEvent> {
    return new Observable<BaseEvent>((subscriber) => {
      let cancelled = false;
      let innerUnsubscribe: (() => void) | undefined;

      (async () => {
        try {
          const threadId = input.threadId || this.threadId || randomUUID();
          const hydratedInput = await this.hydrateInput(threadId, input);
          if (cancelled) return;

          const inner = super.run(hydratedInput).subscribe(subscriber);
          innerUnsubscribe = () => inner.unsubscribe();
        } catch (error) {
          subscriber.error(error);
        }
      })();

      return () => {
        cancelled = true;
        innerUnsubscribe?.();
      };
    });
  }

  clone(): PersistedThreadHttpAgent {
    const cloned = super.clone() as PersistedThreadHttpAgent;
    // HttpAgent.clone() does not know custom subclass fields.
    Object.defineProperty(cloned, "loadThreadSnapshot", {
      value: this.loadThreadSnapshot,
      writable: false,
      configurable: true,
      enumerable: false,
    });
    return cloned;
  }

  private async hydrateInput(
    threadId: string,
    input: RunAgentInput,
  ): Promise<RunAgentInput> {
    this.threadId = threadId;

    const incomingMessages = Array.isArray(input.messages) ? input.messages : [];
    const incomingState = asObject(input.state) ?? {};
    const snapshot = await this.loadThreadSnapshot(threadId);

    if (!snapshot) {
      const state = mergeHydratedState({}, incomingState, incomingMessages);
      this.setMessages(incomingMessages);
      this.setState(state);
      return {
        ...input,
        threadId,
        messages: incomingMessages,
        state,
      };
    }

    const pendingMessages = takePendingIncomingMessages(
      snapshot.messages,
      incomingMessages,
    );
    // Use backend history as source of truth, append only non-persisted tail messages.
    const messages = [...snapshot.messages, ...pendingMessages];
    const state = mergeHydratedState(
      asObject(snapshot.state) ?? {},
      incomingState,
      messages,
    );

    this.setMessages(messages);
    this.setState(state);

    return {
      ...input,
      threadId,
      messages,
      state,
    };
  }
}

function takePendingIncomingMessages(
  persisted: Message[],
  incoming: Message[],
): Message[] {
  if (incoming.length === 0) return [];

  const persistedKeys = new Set(
    persisted.map(messageKey).filter((key): key is string => Boolean(key)),
  );

  let startIndex = incoming.length;
  for (let index = incoming.length - 1; index >= 0; index -= 1) {
    const key = messageKey(incoming[index]);
    if (key && persistedKeys.has(key)) break;
    startIndex = index;
  }

  const candidateTail = incoming.slice(startIndex);
  const out: Message[] = [];

  for (const message of candidateTail) {
    const key = messageKey(message);
    if (key && persistedKeys.has(key)) continue;
    if (key) persistedKeys.add(key);
    out.push(message);
  }

  return out;
}

function messageKey(message: Message): string | undefined {
  if (typeof message.id === "string" && message.id.length > 0) {
    return `id:${message.id}`;
  }
  if ("toolCallId" in message && typeof message.toolCallId === "string") {
    return `tool:${message.toolCallId}`;
  }
  return undefined;
}

function mergeHydratedState(
  persistedState: Record<string, unknown>,
  incomingState: Record<string, unknown>,
  messages: Message[],
): Record<string, unknown> {
  const persisted = clearStalePendingControl(persistedState, messages);
  const incoming = stripIncomingPendingControl(incomingState);
  return {
    ...persisted,
    ...incoming,
  };
}

function clearStalePendingControl(
  state: Record<string, unknown>,
  messages: Message[],
): Record<string, unknown> {
  const pendingCallId = extractPendingCallId(state);
  if (!pendingCallId) return state;
  if (isPendingToolCall(messages, pendingCallId)) return state;
  return stripIncomingPendingControl(state);
}

function stripIncomingPendingControl(
  state: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...state };

  const loopControl = asObject(out.loop_control);
  if (loopControl) {
    const nextLoopControl: Record<string, unknown> = { ...loopControl };
    delete nextLoopControl.pending_frontend_invocation;
    delete nextLoopControl.pending_interaction;
    if (Object.keys(nextLoopControl).length === 0) {
      delete out.loop_control;
    } else {
      out.loop_control = nextLoopControl;
    }
  }

  const runtime = asObject(out.runtime);
  if (runtime && "pending_interaction" in runtime) {
    const nextRuntime: Record<string, unknown> = { ...runtime };
    delete nextRuntime.pending_interaction;
    if (Object.keys(nextRuntime).length === 0) {
      delete out.runtime;
    } else {
      out.runtime = nextRuntime;
    }
  }

  return out;
}

function extractPendingCallId(state: Record<string, unknown>): string | undefined {
  const loopControl = asObject(state.loop_control);
  const pendingFrontendInvocation = asObject(loopControl?.pending_frontend_invocation);
  if (typeof pendingFrontendInvocation?.call_id === "string") {
    return pendingFrontendInvocation.call_id;
  }

  const pendingInteraction = asObject(loopControl?.pending_interaction);
  if (typeof pendingInteraction?.id === "string") {
    return pendingInteraction.id;
  }

  const runtime = asObject(state.runtime);
  const runtimePendingInteraction = asObject(runtime?.pending_interaction);
  if (typeof runtimePendingInteraction?.id === "string") {
    return runtimePendingInteraction.id;
  }

  return undefined;
}

function isPendingToolCall(messages: Message[], toolCallId: string): boolean {
  let hasToolCall = false;
  let hasToolResult = false;

  for (const message of messages) {
    if (
      "toolCallId" in message &&
      typeof message.toolCallId === "string" &&
      message.toolCallId === toolCallId
    ) {
      hasToolResult = true;
    }

    if (!hasToolCall && messageContainsToolCallId(message, toolCallId)) {
      hasToolCall = true;
    }
  }

  return hasToolCall && !hasToolResult;
}

function messageContainsToolCallId(message: Message, toolCallId: string): boolean {
  const rawMessage = asObject(message);
  const toolCalls = rawMessage?.toolCalls;
  if (!Array.isArray(toolCalls)) return false;

  for (const toolCall of toolCalls) {
    const rawToolCall = asObject(toolCall);
    if (!rawToolCall) continue;
    if (rawToolCall.id === toolCallId) return true;
    if (rawToolCall.toolCallId === toolCallId) return true;
  }

  return false;
}

function asObject(
  value: unknown,
): Record<string, unknown> | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}
