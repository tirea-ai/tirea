import { useState, useMemo, useCallback, useEffect } from "react";

// --- Types ---

type A2uiComponent = {
  id: string;
  component: string;
  child?: string;
  children?: string[];
  text?: string | DataBinding;
  label?: string;
  value?: string | DataBinding;
  variant?: string;
  action?: { event?: { name: string; context?: Record<string, unknown> } };
  [key: string]: unknown;
};

type DataBinding = { path: string };

export type A2uiMessage = {
  version: string;
  createSurface?: { surfaceId: string; catalogId: string };
  updateComponents?: { surfaceId: string; components: A2uiComponent[] };
  updateDataModel?: { surfaceId: string; path: string; value: unknown };
  deleteSurface?: { surfaceId: string };
};

/** Extract the surfaceId from an array of A2UI messages. */
export function extractSurfaceId(messages: A2uiMessage[]): string | null {
  for (const msg of messages) {
    if (msg.createSurface) return msg.createSurface.surfaceId;
    if (msg.updateComponents) return msg.updateComponents.surfaceId;
    if (msg.updateDataModel) return msg.updateDataModel.surfaceId;
    if (msg.deleteSurface) return msg.deleteSurface.surfaceId;
  }
  return null;
}

// --- Props ---

type A2uiSurfaceProps = {
  /** The a2ui array from the render_a2ui tool output */
  messages: A2uiMessage[];
  /** Called when a Button with action.event is clicked */
  onEvent?: (surfaceId: string, eventName: string, context: Record<string, unknown>) => void;
};

// --- Data model helpers ---

function getByPath(obj: unknown, path: string): unknown {
  if (!path || path === "/") return obj;
  const parts = path.replace(/^\//, "").split("/");
  let current: unknown = obj;
  for (const p of parts) {
    if (current == null || typeof current !== "object") return undefined;
    current = (current as Record<string, unknown>)[p];
  }
  return current;
}

function setByPath(obj: Record<string, unknown>, path: string, value: unknown): Record<string, unknown> {
  if (!path || path === "/") return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : obj;
  const parts = path.replace(/^\//, "").split("/");
  const clone = structuredClone(obj);
  let current: Record<string, unknown> = clone;
  for (let i = 0; i < parts.length - 1; i++) {
    if (!(parts[i] in current) || typeof current[parts[i]] !== "object") {
      current[parts[i]] = {};
    }
    current = current[parts[i]] as Record<string, unknown>;
  }
  current[parts[parts.length - 1]] = value;
  return clone;
}

function isDataBinding(v: unknown): v is DataBinding {
  return typeof v === "object" && v !== null && "path" in v && typeof (v as DataBinding).path === "string";
}

function resolveValue(v: unknown, dataModel: Record<string, unknown>): unknown {
  if (isDataBinding(v)) return getByPath(dataModel, v.path) ?? "";
  return v;
}

// --- Surface renderer ---

export function A2uiSurface({ messages, onEvent }: A2uiSurfaceProps) {
  const { surfaceId, components, initialDataModel } = useMemo(() => {
    let sid = "";
    let comps: A2uiComponent[] = [];
    let dm: Record<string, unknown> = {};
    for (const msg of messages) {
      if (msg.createSurface) sid = msg.createSurface.surfaceId;
      if (msg.updateComponents) comps = msg.updateComponents.components;
      if (msg.updateDataModel) {
        const { path, value } = msg.updateDataModel;
        dm = setByPath(dm, path, value);
      }
    }
    return { surfaceId: sid, components: comps, initialDataModel: dm };
  }, [messages]);

  const [dataModel, setDataModel] = useState<Record<string, unknown>>(initialDataModel);

  // Sync local dataModel when LLM sends new data (e.g. after an interaction event)
  useEffect(() => {
    setDataModel(initialDataModel);
  }, [initialDataModel]);

  const handleFieldChange = useCallback((path: string, value: string) => {
    setDataModel((prev) => setByPath(prev, path, value));
  }, []);

  const handleEvent = useCallback(
    (eventName: string, context: Record<string, unknown>) => {
      onEvent?.(surfaceId, eventName, { ...context, dataModel });
    },
    [surfaceId, onEvent, dataModel],
  );

  if (components.length === 0) return null;

  const componentMap = new Map(components.map((c) => [c.id, c]));
  const root = componentMap.get("root");
  if (!root) return null;

  return (
    <div data-testid="a2ui-surface" data-surface-id={surfaceId} className="my-3">
      <RenderNode
        node={root}
        componentMap={componentMap}
        dataModel={dataModel}
        onFieldChange={handleFieldChange}
        onEvent={handleEvent}
      />
    </div>
  );
}

// --- Component renderers ---

function RenderNode({
  node,
  componentMap,
  dataModel,
  onFieldChange,
  onEvent,
}: {
  node: A2uiComponent;
  componentMap: Map<string, A2uiComponent>;
  dataModel: Record<string, unknown>;
  onFieldChange: (path: string, value: string) => void;
  onEvent: (name: string, context: Record<string, unknown>) => void;
}) {
  const childIds = node.children ?? (node.child ? [node.child] : []);
  const children = childIds
    .map((id) => componentMap.get(id))
    .filter((c): c is A2uiComponent => c != null)
    .map((c) => (
      <RenderNode
        key={c.id}
        node={c}
        componentMap={componentMap}
        dataModel={dataModel}
        onFieldChange={onFieldChange}
        onEvent={onEvent}
      />
    ));

  switch (node.component) {
    case "Card":
      return (
        <div data-testid={`a2ui-card-${node.id}`} className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm">
          {children}
        </div>
      );

    case "Column":
      return (
        <div data-testid={`a2ui-column-${node.id}`} className="flex flex-col gap-2">
          {children}
        </div>
      );

    case "Row":
      return (
        <div data-testid={`a2ui-row-${node.id}`} className="flex flex-row gap-2 items-center">
          {children}
        </div>
      );

    case "Text": {
      const text = String(resolveValue(node.text, dataModel) ?? "");
      const isHeading = node.variant === "h1" || node.variant === "h2" || node.variant === "h3";
      const cls = isHeading ? "text-lg font-semibold text-slate-800" : "text-sm text-slate-700";
      return (
        <p data-testid={`a2ui-text-${node.id}`} className={cls}>
          {text}
        </p>
      );
    }

    case "TextField": {
      const binding = isDataBinding(node.value) ? node.value : null;
      const value = binding ? String(getByPath(dataModel, binding.path) ?? "") : String(node.value ?? "");
      return (
        <div data-testid={`a2ui-textfield-${node.id}`}>
          {node.label && (
            <label className="mb-1 block text-xs font-medium text-slate-600">{node.label}</label>
          )}
          <input
            type="text"
            data-testid={`a2ui-input-${node.id}`}
            value={value}
            onChange={(e) => {
              if (binding) onFieldChange(binding.path, e.target.value);
            }}
            className="w-full rounded-lg border border-slate-300 px-3 py-1.5 text-sm text-slate-800 outline-none focus:border-cyan-500 focus:ring-1 focus:ring-cyan-500"
            readOnly={!binding}
          />
        </div>
      );
    }

    case "Button": {
      const text = String(resolveValue(node.text, dataModel) ?? "Click");
      const event = node.action?.event;
      return (
        <button
          type="button"
          data-testid={`a2ui-button-${node.id}`}
          onClick={() => {
            if (event) {
              onEvent(event.name, event.context ?? {});
            }
          }}
          className="rounded-lg bg-cyan-700 px-4 py-1.5 text-sm font-semibold text-white transition hover:bg-cyan-800"
        >
          {text}
        </button>
      );
    }

    default:
      return (
        <div data-testid={`a2ui-unknown-${node.id}`} className="text-xs text-slate-400">
          [{node.component}: {node.id}]
          {children}
        </div>
      );
  }
}
