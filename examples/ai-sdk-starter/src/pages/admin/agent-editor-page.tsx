import { useEffect, useState, useCallback } from "react";
import { useParams, useNavigate } from "react-router";
import Form from "@rjsf/core";
import validator from "@rjsf/validator-ajv8";
import {
  configApi,
  type AgentSpec,
  type Capabilities,
} from "../../lib/config-api";

export function AgentEditorPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const isNew = id === "new";

  const [spec, setSpec] = useState<AgentSpec>({
    id: "",
    model: "",
    system_prompt: "",
    max_rounds: 16,
    plugin_ids: [],
    sections: {},
    allowed_tools: [],
    excluded_tools: [],
    delegates: [],
  });
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  // Load capabilities and existing spec
  useEffect(() => {
    configApi.capabilities().then(setCaps).catch(console.error);
    if (!isNew && id) {
      configApi
        .get<AgentSpec>("agents", id)
        .then(setSpec)
        .catch(() => setError(`Agent "${id}" not found`));
    }
  }, [id, isNew]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      if (isNew) {
        await configApi.create("agents", spec);
        navigate(`/admin/agents/${spec.id}`, { replace: true });
      } else {
        await configApi.update("agents", spec.id, spec);
      }
      setSuccess("Saved");
      setTimeout(() => setSuccess(null), 2000);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [spec, isNew, navigate]);

  const updateField = <K extends keyof AgentSpec>(
    key: K,
    value: AgentSpec[K]
  ) => {
    setSpec((prev) => ({ ...prev, [key]: value }));
  };

  const togglePlugin = (pluginId: string) => {
    setSpec((prev) => {
      const ids = prev.plugin_ids ?? [];
      const next = ids.includes(pluginId)
        ? ids.filter((p) => p !== pluginId)
        : [...ids, pluginId];
      return { ...prev, plugin_ids: next };
    });
  };

  const updateSection = (key: string, data: unknown) => {
    setSpec((prev) => ({
      ...prev,
      sections: { ...prev.sections, [key]: data },
    }));
  };

  return (
    <div className="p-6 max-w-3xl">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">
          {isNew ? "New Agent" : `Edit: ${id}`}
        </h2>
        <button
          onClick={handleSave}
          disabled={saving}
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-500 disabled:opacity-50 text-sm"
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>

      {error && (
        <div className="mb-4 p-3 bg-red-50 text-red-700 rounded text-sm">
          {error}
        </div>
      )}
      {success && (
        <div className="mb-4 p-3 bg-green-50 text-green-700 rounded text-sm">
          {success}
        </div>
      )}

      {/* Basic fields */}
      <section className="bg-white rounded shadow p-4 mb-4">
        <h3 className="font-semibold mb-3">Basic</h3>
        <div className="space-y-3">
          <Field label="ID" disabled={!isNew}>
            <input
              type="text"
              value={spec.id}
              onChange={(e) => updateField("id", e.target.value)}
              disabled={!isNew}
              className="w-full px-3 py-2 border rounded text-sm disabled:bg-gray-100"
            />
          </Field>
          <Field label="Model">
            <select
              value={spec.model}
              onChange={(e) => updateField("model", e.target.value)}
              className="w-full px-3 py-2 border rounded text-sm"
            >
              <option value="">-- Select --</option>
              {caps?.models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.id} ({m.model})
                </option>
              ))}
            </select>
          </Field>
          <Field label="System Prompt">
            <textarea
              value={spec.system_prompt}
              onChange={(e) => updateField("system_prompt", e.target.value)}
              rows={4}
              className="w-full px-3 py-2 border rounded text-sm font-mono"
            />
          </Field>
          <Field label="Max Rounds">
            <input
              type="number"
              value={spec.max_rounds ?? 16}
              onChange={(e) =>
                updateField("max_rounds", parseInt(e.target.value) || 16)
              }
              className="w-32 px-3 py-2 border rounded text-sm"
            />
          </Field>
        </div>
      </section>

      {/* Tools */}
      {caps && caps.tools.length > 0 && (
        <section className="bg-white rounded shadow p-4 mb-4">
          <h3 className="font-semibold mb-3">Tools</h3>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-2 max-h-48 overflow-auto">
            {caps.tools.map((t) => (
              <label key={t.id} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={
                    !spec.allowed_tools ||
                    spec.allowed_tools.length === 0 ||
                    spec.allowed_tools.includes(t.id)
                  }
                  onChange={(e) => {
                    const current = spec.allowed_tools ?? [];
                    if (e.target.checked) {
                      // If adding back, remove from list (empty = all)
                      const next = current.filter((id) => id !== t.id);
                      updateField(
                        "allowed_tools",
                        next.length === 0 ? undefined : next
                      );
                    } else {
                      // First exclusion: start with all tools minus this one
                      const allIds = caps.tools.map((tt) => tt.id);
                      const base =
                        current.length === 0 ? allIds : current;
                      updateField(
                        "allowed_tools",
                        base.filter((id) => id !== t.id)
                      );
                    }
                  }}
                />
                <span className="font-mono">{t.id}</span>
              </label>
            ))}
          </div>
        </section>
      )}

      {/* Plugins */}
      {caps && caps.plugins.length > 0 && (
        <section className="bg-white rounded shadow p-4 mb-4">
          <h3 className="font-semibold mb-3">Plugins</h3>
          <div className="space-y-2 mb-4">
            {caps.plugins.map((p) => (
              <label key={p.id} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={spec.plugin_ids?.includes(p.id) ?? false}
                  onChange={() => togglePlugin(p.id)}
                />
                <span className="font-mono">{p.id}</span>
              </label>
            ))}
          </div>

          {/* Plugin config forms (dynamic, driven by JSON Schema) */}
          {caps.plugins
            .filter((p) => spec.plugin_ids?.includes(p.id))
            .flatMap((p) =>
              p.config_schemas.map((cs) => (
                <div
                  key={`${p.id}:${cs.key}`}
                  className="border rounded p-3 mt-3"
                >
                  <h4 className="text-sm font-semibold text-gray-600 mb-2">
                    {p.id} &rarr; {cs.key}
                  </h4>
                  <Form
                    schema={cs.schema as Record<string, unknown>}
                    formData={
                      (spec.sections?.[cs.key] as Record<string, unknown>) ??
                      {}
                    }
                    onChange={({ formData }) => updateSection(cs.key, formData)}
                    validator={validator}
                    uiSchema={{ "ui:submitButtonOptions": { norender: true } }}
                    liveValidate
                  >
                    <></>
                  </Form>
                </div>
              ))
            )}
        </section>
      )}

      {/* Delegates */}
      {caps && caps.agents.length > 0 && (
        <section className="bg-white rounded shadow p-4 mb-4">
          <h3 className="font-semibold mb-3">Delegates</h3>
          <div className="space-y-1">
            {caps.agents
              .filter((a) => a !== spec.id)
              .map((a) => (
                <label key={a} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={spec.delegates?.includes(a) ?? false}
                    onChange={(e) => {
                      const current = spec.delegates ?? [];
                      updateField(
                        "delegates",
                        e.target.checked
                          ? [...current, a]
                          : current.filter((d) => d !== a)
                      );
                    }}
                  />
                  <span className="font-mono">{a}</span>
                </label>
              ))}
          </div>
        </section>
      )}

      {/* Raw JSON preview */}
      <section className="bg-white rounded shadow p-4">
        <h3 className="font-semibold mb-3">JSON Preview</h3>
        <pre className="text-xs font-mono bg-gray-100 p-3 rounded overflow-auto max-h-64">
          {JSON.stringify(spec, null, 2)}
        </pre>
      </section>
    </div>
  );
}

function Field({
  label,
  disabled,
  children,
}: {
  label: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className={disabled ? "opacity-60" : ""}>
      <label className="block text-sm font-medium text-gray-600 mb-1">
        {label}
      </label>
      {children}
    </div>
  );
}
