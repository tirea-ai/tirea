import { useEffect, useState } from "react";
import { configApi, type ProviderSpec } from "../../lib/config-api";

const ADAPTERS = ["anthropic", "openai", "gemini", "ollama", "cohere"];

export function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderSpec[]>([]);
  const [editing, setEditing] = useState<ProviderSpec | null>(null);

  const load = () => {
    configApi
      .list<ProviderSpec>("providers")
      .then((r) => setProviders(r.items))
      .catch(() => setProviders([]));
  };

  useEffect(load, []);

  const handleSave = async () => {
    if (!editing) return;
    try {
      if (providers.find((p) => p.id === editing.id)) {
        await configApi.update("providers", editing.id, editing);
      } else {
        await configApi.create("providers", editing);
      }
      setEditing(null);
      load();
    } catch (e) {
      alert(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(`Delete provider "${id}"?`)) return;
    await configApi.delete("providers", id);
    load();
  };

  return (
    <div className="p-6 max-w-3xl">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">Providers</h2>
        <button
          onClick={() =>
            setEditing({ id: "", adapter: "anthropic" })
          }
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-500 text-sm"
        >
          + New Provider
        </button>
      </div>

      {editing && (
        <div className="bg-white rounded shadow p-4 mb-4 space-y-3">
          <input
            placeholder="Provider ID (e.g. anthropic)"
            value={editing.id}
            onChange={(e) => setEditing({ ...editing, id: e.target.value })}
            className="w-full px-3 py-2 border rounded text-sm"
          />
          <select
            value={editing.adapter}
            onChange={(e) =>
              setEditing({ ...editing, adapter: e.target.value })
            }
            className="w-full px-3 py-2 border rounded text-sm"
          >
            {ADAPTERS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
          <input
            placeholder="API Key (leave empty for env var)"
            type="password"
            value={editing.api_key ?? ""}
            onChange={(e) =>
              setEditing({
                ...editing,
                api_key: e.target.value || undefined,
              })
            }
            className="w-full px-3 py-2 border rounded text-sm"
          />
          <input
            placeholder="Base URL (optional)"
            value={editing.base_url ?? ""}
            onChange={(e) =>
              setEditing({
                ...editing,
                base_url: e.target.value || undefined,
              })
            }
            className="w-full px-3 py-2 border rounded text-sm"
          />
          <div className="flex gap-2">
            <button
              onClick={handleSave}
              className="px-4 py-2 bg-blue-600 text-white rounded text-sm"
            >
              Save
            </button>
            <button
              onClick={() => setEditing(null)}
              className="px-4 py-2 bg-gray-200 rounded text-sm"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      <table className="w-full bg-white rounded shadow">
        <thead>
          <tr className="border-b text-left text-sm text-gray-500">
            <th className="px-4 py-3">ID</th>
            <th className="px-4 py-3">Adapter</th>
            <th className="px-4 py-3">Base URL</th>
            <th className="px-4 py-3">API Key</th>
            <th className="px-4 py-3">Actions</th>
          </tr>
        </thead>
        <tbody>
          {providers.map((p) => (
            <tr key={p.id} className="border-b last:border-0 hover:bg-gray-50">
              <td className="px-4 py-3 font-mono text-sm">{p.id}</td>
              <td className="px-4 py-3 text-sm">{p.adapter}</td>
              <td className="px-4 py-3 text-sm text-gray-600">
                {p.base_url || "(default)"}
              </td>
              <td className="px-4 py-3 text-sm text-gray-400">
                {p.api_key ? "********" : "(env var)"}
              </td>
              <td className="px-4 py-3 space-x-2">
                <button
                  onClick={() => setEditing({ ...p, api_key: "" })}
                  className="text-blue-500 text-sm"
                >
                  Edit
                </button>
                <button
                  onClick={() => handleDelete(p.id)}
                  className="text-red-500 text-sm"
                >
                  Delete
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
