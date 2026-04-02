/**
 * Config management API client.
 *
 * Provides typed access to /v1/config/:namespace and /v1/capabilities.
 */

const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL ?? "http://localhost:38080";

// ── Types ──

export interface AgentSpec {
  id: string;
  model: string;
  system_prompt: string;
  max_rounds?: number;
  plugin_ids?: string[];
  sections?: Record<string, unknown>;
  allowed_tools?: string[];
  excluded_tools?: string[];
  delegates?: string[];
  [key: string]: unknown;
}

export interface ModelSpec {
  id: string;
  provider: string;
  model: string;
}

export interface ProviderSpec {
  id: string;
  adapter: string;
  api_key?: string;
  base_url?: string;
  timeout_secs?: number;
  headers?: Record<string, string>;
}

export interface PluginInfo {
  id: string;
  config_schemas: Array<{ key: string; schema: Record<string, unknown> }>;
}

export interface ToolInfo {
  id: string;
  name: string;
  description: string;
}

export interface Capabilities {
  agents: string[];
  tools: ToolInfo[];
  plugins: PluginInfo[];
  models: ModelSpec[];
  providers: Array<{ id: string }>;
  namespaces: Array<{
    namespace: string;
    schema: Record<string, unknown>;
  }>;
}

export interface ListResponse<T> {
  namespace: string;
  items: T[];
  offset: number;
  limit: number;
}

// ── Generic CRUD ──

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`${res.status}: ${body}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

export function configUrl(namespace: string, id?: string): string {
  const base = `${BACKEND_URL}/v1/config/${namespace}`;
  return id ? `${base}/${encodeURIComponent(id)}` : base;
}

export const configApi = {
  // Generic CRUD
  list: <T = unknown>(ns: string, offset = 0, limit = 100) =>
    fetchJson<ListResponse<T>>(
      `${configUrl(ns)}?offset=${offset}&limit=${limit}`
    ),

  get: <T = unknown>(ns: string, id: string) =>
    fetchJson<T>(configUrl(ns, id)),

  create: <T = unknown>(ns: string, body: T) =>
    fetchJson<T>(configUrl(ns), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),

  update: <T = unknown>(ns: string, id: string, body: T) =>
    fetchJson<T>(configUrl(ns, id), {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),

  delete: (ns: string, id: string) =>
    fetchJson<void>(configUrl(ns, id), { method: "DELETE" }),

  schema: (ns: string) =>
    fetchJson<Record<string, unknown>>(
      `${BACKEND_URL}/v1/config/${ns}/$schema`
    ),

  // Capabilities
  capabilities: () =>
    fetchJson<Capabilities>(`${BACKEND_URL}/v1/capabilities`),
};
