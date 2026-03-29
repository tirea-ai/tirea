import {
  Component,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Renderer as JsonRenderRenderer,
  JSONUIProvider,
  type ComponentRegistry,
} from "@json-render/react";
import { createSpecStreamCompiler, type Spec } from "@json-render/core";
import { Renderer as OpenUIRenderer } from "@openuidev/react-lang";
import { openuiLibrary } from "@openuidev/react-ui";
import {
  A2UIProvider,
  A2UIRenderer,
  useA2UI,
  useA2UIActions,
  type AnyComponentNode,
  type A2UIClientEventMessage,
  type ServerToClientMessage,
} from "@a2ui/react";
import { injectStyles } from "@a2ui/react/styles";
import {
  extractSurfaceId,
  formatA2uiActionMessage,
  formatA2uiActionMessageWithContext,
  normalizeA2uiMessages,
} from "./a2ui-protocol";

export interface ActivitySnapshot {
  activityType: string;
  messageId?: string;
  content?: unknown;
}

const fallbackRegistry: ComponentRegistry = new Proxy({} as ComponentRegistry, {
  get() {
    return ({
      element,
      children,
    }: {
      element: { type: string; props: Record<string, unknown> };
      children?: unknown;
    }) => {
      const safeChildren =
        children == null ||
        typeof children === "string" ||
          typeof children === "number" ||
          typeof children === "boolean" ||
          Array.isArray(children) ||
          (typeof children === "object" && "$$typeof" in (children as object))
          ? (children as ReactNode)
          : null;
      return (
        <div
          data-jr-type={element.type}
          className="my-1 rounded-lg border border-blue-200 bg-blue-50/50 p-2"
        >
          <div className="mb-1 text-xs font-semibold text-blue-600">
            {element.type}
          </div>
          {element.props &&
            Object.entries(element.props).map(([key, value]) =>
              typeof value === "string" || typeof value === "number" ? (
                <div key={key} className="text-sm text-slate-700">
                  {String(value)}
                </div>
              ) : null,
            )}
          {safeChildren}
        </div>
      );
    };
  },
});

let a2uiStylesInjected = false;
type A2uiActionHelpers = ReturnType<typeof useA2UIActions>;

function ensureA2uiStyles() {
  if (a2uiStylesInjected || typeof document === "undefined") return;
  injectStyles();
  a2uiStylesInjected = true;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseJsonRenderSpec(data: unknown): Spec | null {
  if (isJsonRenderSpec(data)) {
    return data as Spec;
  }

  if (typeof data !== "string") {
    return null;
  }

  const compiled = compileJsonRenderSpecStream(data);
  if (compiled) {
    return compiled;
  }

  try {
    const parsed = JSON.parse(data);
    return isJsonRenderSpec(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function isJsonRenderSpec(value: unknown): value is Spec {
  return (
    isRecord(value) &&
    typeof value.root === "string" &&
    value.root.trim().length > 0 &&
    isRecord(value.elements) &&
    value.root in value.elements
  );
}

function compileJsonRenderSpecStream(data: string): Spec | null {
  const compiler = createSpecStreamCompiler<Spec>({ root: "", elements: {} });
  compiler.push(data);
  const spec = compiler.getResult();
  return isJsonRenderSpec(spec) ? spec : null;
}

function sanitizeActionContext(
  value: unknown,
): Record<string, unknown> | undefined {
  if (!isRecord(value)) return undefined;

  const entries = Object.entries(value).filter(
    ([key, item]) =>
      typeof key === "string" &&
      key.length > 0 &&
      key !== "[object Object]" &&
      item !== undefined,
  );

  return entries.length > 0 ? Object.fromEntries(entries) : undefined;
}

function findComponentNodeById(
  node: AnyComponentNode | null | undefined,
  targetId: string,
): AnyComponentNode | null {
  if (!node) return null;
  if (node.id === targetId) return node;

  return findComponentNodeInValue(
    isRecord(node.properties) ? Object.values(node.properties) : [],
    targetId,
  );
}

function findComponentNodeInValue(
  value: unknown,
  targetId: string,
): AnyComponentNode | null {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findComponentNodeInValue(item, targetId);
      if (found) return found;
    }
    return null;
  }

  if (isRecord(value)) {
    if (
      typeof value.id === "string" &&
      typeof value.type === "string" &&
      isRecord(value.properties)
    ) {
      return findComponentNodeById(value as AnyComponentNode, targetId);
    }

    for (const nested of Object.values(value)) {
      const found = findComponentNodeInValue(nested, targetId);
      if (found) return found;
    }
  }

  return null;
}

function rebuildA2uiActionContext(
  message: A2UIClientEventMessage,
  actions: A2uiActionHelpers | null,
): Record<string, unknown> | undefined {
  const fallbackContext = sanitizeActionContext(message.userAction?.context);
  const surfaceId = message.userAction?.surfaceId;
  const sourceComponentId = message.userAction?.sourceComponentId;
  if (!actions || !surfaceId || !sourceComponentId) return fallbackContext;

  const surface = actions.getSurface(surfaceId);
  if (!surface) return fallbackContext;

  const sourceNode = findComponentNodeById(surface.componentTree, sourceComponentId);
  const componentInstance = surface.components.get(sourceComponentId);
  if (!componentInstance || !isRecord(componentInstance.component)) {
    return fallbackContext;
  }

  const buttonPayload = componentInstance.component.Button;
  if (!isRecord(buttonPayload) || !isRecord(buttonPayload.action)) {
    return fallbackContext;
  }

  const contextEntries = Array.isArray(buttonPayload.action.context)
    ? buttonPayload.action.context.filter(isRecord)
    : [];
  if (contextEntries.length === 0) return fallbackContext;

  const rebuiltContext: Record<string, unknown> = {
    ...(fallbackContext ?? {}),
  };

  for (const entry of contextEntries) {
    const key = typeof entry.key === "string" ? entry.key : null;
    const value = isRecord(entry.value) ? entry.value : null;
    if (!key || !value) continue;

    if (typeof value.literalString === "string") {
      rebuiltContext[key] = value.literalString;
      continue;
    }
    if (typeof value.literalNumber === "number") {
      rebuiltContext[key] = value.literalNumber;
      continue;
    }
    if (typeof value.literalBoolean === "boolean") {
      rebuiltContext[key] = value.literalBoolean;
      continue;
    }
    if (typeof value.path === "string" && sourceNode) {
      const resolvedPath = actions.resolvePath(
        value.path,
        sourceNode.dataContextPath,
      );
      rebuiltContext[key] = actions.getData(sourceNode, resolvedPath, surfaceId);
    }
  }

  return Object.keys(rebuiltContext).length > 0 ? rebuiltContext : undefined;
}

function A2uiActionBridge({
  onReady,
}: {
  onReady: (actions: A2uiActionHelpers) => void;
}) {
  const actions = useA2UIActions();

  useEffect(() => {
    onReady(actions);
  }, [actions, onReady]);

  return null;
}

function A2uiErrorState({
  message,
  testId = "a2ui-panel-error",
}: {
  message: string;
  testId?: string;
}) {
  return (
    <div
      data-testid={testId}
      className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700"
    >
      Failed to render A2UI surface: {message}
    </div>
  );
}

class A2uiRenderBoundary extends Component<
  { children: ReactNode; resetKey: string },
  { error: string | null }
> {
  state = { error: null as string | null };

  static getDerivedStateFromError(error: unknown) {
    return {
      error:
        error instanceof Error && error.message
          ? error.message
          : "Unknown A2UI render error",
    };
  }

  componentDidUpdate(prevProps: { resetKey: string }) {
    if (prevProps.resetKey !== this.props.resetKey && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (this.state.error) {
      return <A2uiErrorState message={this.state.error} />;
    }
    return this.props.children;
  }
}

function A2UIStreamRenderer({
  surfaceId,
  messages,
}: {
  surfaceId: string;
  messages: ServerToClientMessage[];
}) {
  const { clearSurfaces, processMessages } = useA2UI();
  const [processingError, setProcessingError] = useState<string | null>(null);
  const resetKey = useMemo(
    () => `${surfaceId}:${JSON.stringify(messages)}`,
    [messages, surfaceId],
  );

  useEffect(() => {
    clearSurfaces();
    setProcessingError(null);

    try {
      if (messages.length > 0) {
        processMessages(messages);
      }
    } catch (error) {
      setProcessingError(
        error instanceof Error && error.message
          ? error.message
          : "Unknown A2UI protocol error",
      );
    }

    return () => {
      clearSurfaces();
    };
  }, [clearSurfaces, messages, processMessages]);

  if (processingError) {
    return <A2uiErrorState message={processingError} />;
  }

  return (
    <A2uiRenderBoundary resetKey={resetKey}>
      <A2UIRenderer
        surfaceId={surfaceId}
        fallback={
          <div
            data-testid="a2ui-panel-pending"
            className="text-sm text-slate-500"
          >
            Waiting for surface...
          </div>
        }
      />
    </A2uiRenderBoundary>
  );
}

export function JsonRenderPanel({
  data,
  isStreaming = false,
}: {
  data: unknown;
  isStreaming?: boolean;
}) {
  const spec = useMemo(() => parseJsonRenderSpec(data), [data]);
  if (!spec) {
    if (typeof data === "string" && data.trim().length > 0 && isStreaming) {
      return (
        <div
          data-testid="json-render-panel-pending"
          className="my-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-500"
        >
          Generating interface...
        </div>
      );
    }
    return null;
  }

  return (
    <div data-testid="json-render-panel" className="my-2">
      <JSONUIProvider registry={fallbackRegistry}>
        <JsonRenderRenderer
          spec={spec}
          registry={fallbackRegistry}
          loading={false}
        />
      </JSONUIProvider>
    </div>
  );
}

export function OpenUIPanel({
  response,
  isStreaming = false,
}: {
  response: string;
  isStreaming?: boolean;
}) {
  if (!response) return null;

  return (
    <div data-testid="openui-panel" className="my-2">
      <OpenUIRenderer
        response={response}
        library={openuiLibrary}
        isStreaming={isStreaming}
      />
    </div>
  );
}

export function A2UIPanel({
  messages,
  onAction,
}: {
  messages: unknown[];
  onAction?: (text: string) => void;
}) {
  const actionHelpersRef = useRef<A2uiActionHelpers | null>(null);
  const normalizedMessages = useMemo(
    () => normalizeA2uiMessages(messages),
    [messages],
  );
  const surfaceId = useMemo(
    () => extractSurfaceId(normalizedMessages),
    [normalizedMessages],
  );

  useEffect(() => {
    ensureA2uiStyles();
  }, []);

  if (!surfaceId || normalizedMessages.length === 0) return null;

  return (
    <div data-testid="a2ui-panel" className="my-2 overflow-hidden rounded-xl border border-slate-200 bg-white p-3 shadow-sm">
      <A2UIProvider
        onAction={(message: A2UIClientEventMessage) => {
          const rebuiltContext = rebuildA2uiActionContext(
            message,
            actionHelpersRef.current,
          );
          const actionMessage = rebuiltContext
            ? formatA2uiActionMessageWithContext(message, rebuiltContext)
            : formatA2uiActionMessage(message);
          if (actionMessage) {
            onAction?.(actionMessage);
          }
        }}
      >
        <A2uiActionBridge
          onReady={(actions) => {
            actionHelpersRef.current = actions;
          }}
        />
        <A2UIStreamRenderer
          surfaceId={surfaceId}
          messages={normalizedMessages as ServerToClientMessage[]}
        />
      </A2UIProvider>
    </div>
  );
}
