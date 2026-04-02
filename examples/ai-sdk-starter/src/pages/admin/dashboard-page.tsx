import { useEffect, useState } from "react";
import { Link } from "react-router";
import { configApi, type Capabilities } from "../../lib/config-api";

export function DashboardPage() {
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    configApi.capabilities().then(setCaps).catch((e) => setError(e.message));
  }, []);

  if (error) return <div className="p-6 text-red-600">Error: {error}</div>;
  if (!caps) return <div className="p-6 text-gray-500">Loading...</div>;

  return (
    <div className="p-6 max-w-4xl">
      <h2 className="text-2xl font-bold mb-6">Dashboard</h2>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-8">
        <StatCard label="Agents" count={caps.agents.length} to="/admin/agents" />
        <StatCard label="Models" count={caps.models.length} to="/admin/models" />
        <StatCard
          label="Providers"
          count={caps.providers.length}
          to="/admin/providers"
        />
        <StatCard label="Tools" count={caps.tools.length} />
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div className="bg-white rounded shadow p-4">
          <h3 className="font-semibold mb-3">Plugins</h3>
          {caps.plugins.length === 0 ? (
            <p className="text-gray-400 text-sm">No plugins registered</p>
          ) : (
            <ul className="space-y-1">
              {caps.plugins.map((p) => (
                <li key={p.id} className="text-sm">
                  <span className="font-mono text-blue-700">{p.id}</span>
                  {p.config_schemas.length > 0 && (
                    <span className="ml-2 text-gray-400">
                      ({p.config_schemas.map((s) => s.key).join(", ")})
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="bg-white rounded shadow p-4">
          <h3 className="font-semibold mb-3">Tools</h3>
          <ul className="space-y-1 max-h-48 overflow-auto">
            {caps.tools.map((t) => (
              <li key={t.id} className="text-sm">
                <span className="font-mono text-green-700">{t.id}</span>
                <span className="ml-2 text-gray-400 truncate">
                  {t.description?.slice(0, 60)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}

function StatCard({
  label,
  count,
  to,
}: {
  label: string;
  count: number;
  to?: string;
}) {
  const inner = (
    <div className="bg-white rounded shadow p-4 text-center hover:shadow-md transition">
      <div className="text-3xl font-bold text-blue-600">{count}</div>
      <div className="text-sm text-gray-500 mt-1">{label}</div>
    </div>
  );
  return to ? <Link to={to}>{inner}</Link> : inner;
}
