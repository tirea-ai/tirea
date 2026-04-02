import { useEffect, useState } from "react";
import { configApi, type ModelSpec } from "../../lib/config-api";

export function ModelsPage() {
  const [models, setModels] = useState<ModelSpec[]>([]);
  const [editing, setEditing] = useState<ModelSpec | null>(null);

  const load = () => {
    configApi
      .list<ModelSpec>("models")
      .then((r) => setModels(r.items))
      .catch(() => setModels([]));
  };

  useEffect(load, []);

  const handleSave = async () => {
    if (!editing) return;
    try {
      if (models.find((m) => m.id === editing.id)) {
        await configApi.update("models", editing.id, editing);
      } else {
        await configApi.create("models", editing);
      }
      setEditing(null);
      load();
    } catch (e) {
      alert(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(`Delete model "${id}"?`)) return;
    await configApi.delete("models", id);
    load();
  };

  return (
    <div className="p-6 max-w-3xl">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">Models</h2>
        <button
          onClick={() =>
            setEditing({ id: "", provider: "", model: "" })
          }
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-500 text-sm"
        >
          + New Model
        </button>
      </div>

      {/* Inline editor */}
      {editing && (
        <div className="bg-white rounded shadow p-4 mb-4 space-y-3">
          <input
            placeholder="Model ID (e.g. claude-opus)"
            value={editing.id}
            onChange={(e) => setEditing({ ...editing, id: e.target.value })}
            className="w-full px-3 py-2 border rounded text-sm"
          />
          <input
            placeholder="Provider ID (e.g. anthropic)"
            value={editing.provider}
            onChange={(e) =>
              setEditing({ ...editing, provider: e.target.value })
            }
            className="w-full px-3 py-2 border rounded text-sm"
          />
          <input
            placeholder="API Model Name (e.g. claude-opus-4-6)"
            value={editing.model}
            onChange={(e) => setEditing({ ...editing, model: e.target.value })}
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

      {/* Table */}
      <table className="w-full bg-white rounded shadow">
        <thead>
          <tr className="border-b text-left text-sm text-gray-500">
            <th className="px-4 py-3">ID</th>
            <th className="px-4 py-3">Provider</th>
            <th className="px-4 py-3">API Model</th>
            <th className="px-4 py-3">Actions</th>
          </tr>
        </thead>
        <tbody>
          {models.map((m) => (
            <tr key={m.id} className="border-b last:border-0 hover:bg-gray-50">
              <td className="px-4 py-3 font-mono text-sm">{m.id}</td>
              <td className="px-4 py-3 text-sm">{m.provider}</td>
              <td className="px-4 py-3 text-sm text-gray-600">{m.model}</td>
              <td className="px-4 py-3 space-x-2">
                <button
                  onClick={() => setEditing({ ...m })}
                  className="text-blue-500 text-sm"
                >
                  Edit
                </button>
                <button
                  onClick={() => handleDelete(m.id)}
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
