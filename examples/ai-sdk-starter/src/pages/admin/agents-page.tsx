import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router";
import { configApi, type AgentSpec } from "../../lib/config-api";

export function AgentsPage() {
  const [agents, setAgents] = useState<AgentSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();

  const load = () => {
    setLoading(true);
    configApi
      .list<AgentSpec>("agents")
      .then((r) => setAgents(r.items))
      .catch(() => setAgents([]))
      .finally(() => setLoading(false));
  };

  useEffect(load, []);

  const handleDelete = async (id: string) => {
    if (!confirm(`Delete agent "${id}"?`)) return;
    await configApi.delete("agents", id);
    load();
  };

  return (
    <div className="p-6 max-w-4xl">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">Agents</h2>
        <Link
          to="/admin/agents/new"
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-500 text-sm"
        >
          + New Agent
        </Link>
      </div>

      {loading ? (
        <p className="text-gray-500">Loading...</p>
      ) : agents.length === 0 ? (
        <p className="text-gray-400">No agents configured yet.</p>
      ) : (
        <table className="w-full bg-white rounded shadow">
          <thead>
            <tr className="border-b text-left text-sm text-gray-500">
              <th className="px-4 py-3">ID</th>
              <th className="px-4 py-3">Model</th>
              <th className="px-4 py-3">Plugins</th>
              <th className="px-4 py-3">Actions</th>
            </tr>
          </thead>
          <tbody>
            {agents.map((a) => (
              <tr
                key={a.id}
                className="border-b last:border-0 hover:bg-gray-50 cursor-pointer"
                onClick={() => navigate(`/admin/agents/${a.id}`)}
              >
                <td className="px-4 py-3 font-mono text-sm">{a.id}</td>
                <td className="px-4 py-3 text-sm text-gray-600">{a.model}</td>
                <td className="px-4 py-3 text-sm text-gray-400">
                  {a.plugin_ids?.join(", ") || "-"}
                </td>
                <td className="px-4 py-3">
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDelete(a.id);
                    }}
                    className="text-red-500 hover:text-red-700 text-sm"
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
