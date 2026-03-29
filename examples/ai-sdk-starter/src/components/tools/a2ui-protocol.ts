export type A2uiComponent = {
  id: string;
  weight?: number;
  component: Record<string, unknown>;
};

export type A2uiDataModelEntry = {
  key: string;
  valueString?: string;
  valueNumber?: number;
  valueBoolean?: boolean;
  valueMap?: Record<string, unknown>[];
};

export type A2uiMessage = {
  beginRendering?: { surfaceId: string; root: string; styles?: Record<string, string> };
  surfaceUpdate?: { surfaceId: string; components: A2uiComponent[] };
  dataModelUpdate?: { surfaceId: string; path?: string; contents: A2uiDataModelEntry[] };
  deleteSurface?: { surfaceId: string };
};

export type A2uiClientEvent = {
  name: string;
  surfaceId: string;
  sourceComponentId: string;
  timestamp: string;
  context?: Record<string, unknown>;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function normalizeComponent(value: unknown): A2uiComponent | null {
  if (!isRecord(value)) return null;
  const id = asString(value.id);
  if (!id || !isRecord(value.component)) return null;

  return {
    id,
    weight: asNumber(value.weight),
    component: value.component,
  };
}

function normalizeDataModelEntry(value: unknown): A2uiDataModelEntry | null {
  if (!isRecord(value)) return null;
  const key = asString(value.key);
  if (!key) return null;

  return {
    key,
    valueString: typeof value.valueString === "string" ? value.valueString : undefined,
    valueNumber: asNumber(value.valueNumber),
    valueBoolean: typeof value.valueBoolean === "boolean" ? value.valueBoolean : undefined,
    valueMap: Array.isArray(value.valueMap)
      ? value.valueMap.filter(isRecord)
      : undefined,
  };
}

function normalizeMessage(value: unknown): A2uiMessage | null {
  if (!isRecord(value)) return null;

  if (isRecord(value.surfaceUpdate)) {
    const surfaceId = asString(value.surfaceUpdate.surfaceId);
    if (!surfaceId) return null;
    const rawComponents = Array.isArray(value.surfaceUpdate.components)
      ? value.surfaceUpdate.components
      : isRecord(value.surfaceUpdate.components)
        ? [value.surfaceUpdate.components]
        : [];
    const components = rawComponents
      .map(normalizeComponent)
      .filter((component): component is A2uiComponent => component !== null);
    if (components.length === 0) return null;
    return {
      surfaceUpdate: {
        surfaceId,
        components,
      },
    };
  }

  if (isRecord(value.dataModelUpdate)) {
    const surfaceId = asString(value.dataModelUpdate.surfaceId);
    if (!surfaceId) return null;
    const rawContents = Array.isArray(value.dataModelUpdate.contents)
      ? value.dataModelUpdate.contents
      : isRecord(value.dataModelUpdate.contents)
        ? [value.dataModelUpdate.contents]
        : [];
    const contents = rawContents
      .map(normalizeDataModelEntry)
      .filter((entry): entry is A2uiDataModelEntry => entry !== null);
    if (contents.length === 0) return null;
    return {
      dataModelUpdate: {
        surfaceId,
        path:
          typeof value.dataModelUpdate.path === "string"
            ? value.dataModelUpdate.path
            : undefined,
        contents,
      },
    };
  }

  if (isRecord(value.beginRendering)) {
    const surfaceId = asString(value.beginRendering.surfaceId);
    const root = asString(value.beginRendering.root);
    if (!surfaceId || !root) return null;
    const rawStyles = isRecord(value.beginRendering.styles)
      ? Object.fromEntries(
          Object.entries(value.beginRendering.styles).filter(
            (entry): entry is [string, string] => typeof entry[1] === "string",
          ),
        )
      : undefined;
    return {
      beginRendering: {
        surfaceId,
        root,
        styles: rawStyles,
      },
    };
  }

  if (isRecord(value.deleteSurface)) {
    const surfaceId = asString(value.deleteSurface.surfaceId);
    if (!surfaceId) return null;
    return {
      deleteSurface: { surfaceId },
    };
  }

  return null;
}

function joinA2uiPath(basePath: string | undefined, key: string): string {
  const normalizedBase =
    typeof basePath === "string" && basePath.length > 0 ? basePath : "/";
  if (normalizedBase === "/") return `/${key}`;
  return `${normalizedBase.replace(/\/+$/, "")}/${key}`;
}

function normalizeDateTimeValue(
  rawValue: string,
  mode: { enableDate: boolean; enableTime: boolean },
): string {
  const trimmed = rawValue.trim();
  const match = trimmed.match(
    /^(\d{4}-\d{2}-\d{2})(?:T(\d{2}:\d{2})(?::\d{2}(?:\.\d{1,3})?)?)?/,
  );
  if (!match) return rawValue;

  const [, datePart, timePart] = match;
  if (mode.enableDate && mode.enableTime) {
    return datePart && timePart ? `${datePart}T${timePart}` : rawValue;
  }
  if (mode.enableTime && !mode.enableDate) {
    return timePart ?? rawValue;
  }
  return datePart ?? rawValue;
}

function normalizeDateTimeMessages(messages: A2uiMessage[]): A2uiMessage[] {
  const bindingsBySurface = new Map<
    string,
    Map<string, { enableDate: boolean; enableTime: boolean }>
  >();

  return messages.map((message) => {
    if (message.surfaceUpdate) {
      const bindings =
        bindingsBySurface.get(message.surfaceUpdate.surfaceId) ?? new Map();

      for (const component of message.surfaceUpdate.components) {
        const dateTimeInput = isRecord(component.component.DateTimeInput)
          ? component.component.DateTimeInput
          : null;
        const valueBinding = dateTimeInput && isRecord(dateTimeInput.value)
          ? dateTimeInput.value
          : null;
        const path = valueBinding && typeof valueBinding.path === "string"
          ? valueBinding.path
          : null;
        if (!path) continue;

        bindings.set(path, {
          enableDate: dateTimeInput.enableDate !== false,
          enableTime: Boolean(dateTimeInput.enableTime),
        });
      }

      bindingsBySurface.set(message.surfaceUpdate.surfaceId, bindings);
      return message;
    }

    if (!message.dataModelUpdate) return message;

    const bindings = bindingsBySurface.get(message.dataModelUpdate.surfaceId);
    if (!bindings || bindings.size === 0) return message;

    let changed = false;
    const contents = message.dataModelUpdate.contents.map((entry) => {
      if (typeof entry.valueString !== "string") return entry;

      const fullPath = joinA2uiPath(message.dataModelUpdate?.path, entry.key);
      const mode = bindings.get(fullPath);
      if (!mode) return entry;

      const normalizedValue = normalizeDateTimeValue(entry.valueString, mode);
      if (normalizedValue === entry.valueString) return entry;

      changed = true;
      return {
        ...entry,
        valueString: normalizedValue,
      };
    });

    if (!changed) return message;

    return {
      dataModelUpdate: {
        ...message.dataModelUpdate,
        contents,
      },
    };
  });
}

export function normalizeA2uiMessages(value: unknown): A2uiMessage[] {
  const normalized = (() => {
  if (Array.isArray(value)) {
      return value.flatMap((entry) => normalizeA2uiMessages(entry));
  }

  if (isRecord(value) && Array.isArray(value.messages)) {
      return value.messages
      .map(normalizeMessage)
      .filter((message): message is A2uiMessage => message !== null);
  }

  const single = normalizeMessage(value);
    return single ? [single] : [];
  })();

  return normalizeDateTimeMessages(normalized);
}

export function extractSurfaceId(messages: A2uiMessage[]): string | null {
  for (const message of messages) {
    if (message.surfaceUpdate) return message.surfaceUpdate.surfaceId;
    if (message.dataModelUpdate) return message.dataModelUpdate.surfaceId;
    if (message.beginRendering) return message.beginRendering.surfaceId;
    if (message.deleteSurface) return message.deleteSurface.surfaceId;
  }
  return null;
}

export function formatA2uiActionMessage(value: unknown): string | null {
  if (!isRecord(value) || !isRecord(value.userAction)) return null;
  const userAction = value.userAction;
  const name = asString(userAction.name);
  const surfaceId = asString(userAction.surfaceId);
  const sourceComponentId = asString(userAction.sourceComponentId);
  const timestamp = asString(userAction.timestamp);
  if (!name || !surfaceId || !sourceComponentId || !timestamp) return null;

  const payload: A2uiClientEvent = {
    name,
    surfaceId,
    sourceComponentId,
    timestamp,
    context: isRecord(userAction.context) ? userAction.context : undefined,
  };

  return `A2UI action: ${JSON.stringify(payload)}`;
}

export function formatA2uiActionMessageWithContext(
  value: unknown,
  context: Record<string, unknown> | undefined,
): string | null {
  if (!isRecord(value) || !isRecord(value.userAction)) return null;
  const userAction = value.userAction;
  const name = asString(userAction.name);
  const surfaceId = asString(userAction.surfaceId);
  const sourceComponentId = asString(userAction.sourceComponentId);
  const timestamp = asString(userAction.timestamp);
  if (!name || !surfaceId || !sourceComponentId || !timestamp) return null;

  const payload: A2uiClientEvent = {
    name,
    surfaceId,
    sourceComponentId,
    timestamp,
    context,
  };

  return `A2UI action: ${JSON.stringify(payload)}`;
}
