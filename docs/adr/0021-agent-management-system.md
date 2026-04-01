# ADR-0021: Unified Config Store — Persistent Registry & Management API

- **Status**: Proposed
- **Date**: 2026-04-02
- **Depends on**: ADR-0010 (Registry & Resolve), ADR-0009 (Config & State)

## Context

All five registries (Agent, Model, Provider, Tool, Plugin) are populated
programmatically at startup via in-memory `MapRegistry`. No persistence layer
exists for agent definitions or model mappings — they are lost on restart and
cannot be modified at runtime.

Current state:

| Entity | Storage | Mutability | API |
|--------|---------|------------|-----|
| Agent (AgentSpec) | MapAgentSpecRegistry (memory) | Code-only | None |
| Model mapping | MapModelRegistry (memory) | Code-only | None |
| Provider | MapProviderRegistry (memory) | Code-only (trait object) | None |
| Tool | MapToolRegistry (memory) | Code-only (trait object) | None |
| Plugin | MapPluginSource (memory) | Code-only (trait object) | None |
| Skill | FsSkillRegistryManager (fs) | File edit | None |
| MCP server | McpToolRegistryManager (memory) | Code-only | None |
| Thread/Run | FileStore / PostgresStore | Full CRUD | `/v1/threads`, `/v1/runs` |

Multiple data entities (Agent, Model, Provider config, MCP server) need CRUD
persistence. Defining a separate store trait + file impl + postgres impl + API
routes per entity creates unnecessary duplication — they are all fundamentally
**named JSON documents with a namespace**.

## Decision

### D1: Two-tier entity classification

**Data entities** — pure JSON, persisted via unified ConfigStore:

| Namespace | Value type | JSON Schema |
|-----------|-----------|-------------|
| `agents` | `AgentSpec` | Derived via `schemars` |
| `models` | `ModelConfig` | Derived |
| `providers` | `ProviderConfig` | Derived |
| `mcp-servers` | `McpServerConfig` | Derived |

**Code entities** — registered programmatically, read-only discovery:

| Entity | Discovery |
|--------|-----------|
| Provider (LlmExecutor instance) | Created from ProviderConfig via ProviderFactory |
| Tool (trait object) | Registered by builder + plugins |
| Plugin (trait object) | Registered by builder |

### D2: Unified ConfigStore trait

A single async CRUD trait replaces four separate store traits. All data
entities are `(namespace, id) → JSON value`:

```rust
// awaken-contract/src/contract/config_store.rs

#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Get a single entry.
    async fn get(&self, namespace: &str, id: &str) -> Result<Option<Value>, StorageError>;

    /// List entries with pagination.
    async fn list(
        &self,
        namespace: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StorageError>;

    /// Create or overwrite an entry.
    async fn put(&self, namespace: &str, id: &str, value: &Value) -> Result<(), StorageError>;

    /// Delete an entry.
    async fn delete(&self, namespace: &str, id: &str) -> Result<(), StorageError>;
}
```

### D3: ConfigNamespace — typed access with schema validation

Each data entity registers a `ConfigNamespace` that provides type safety
and JSON Schema for API-level validation:

```rust
pub trait ConfigNamespace: 'static + Send + Sync {
    /// Storage namespace (e.g. "agents", "models").
    const NAMESPACE: &'static str;
    /// Typed value. Must be Serialize + Deserialize + JsonSchema.
    type Value: Serialize + DeserializeOwned + JsonSchema + Send + Sync;
    /// Extract the ID from the value (e.g. AgentSpec.id).
    fn id(value: &Self::Value) -> &str;
}

// Example registrations:
pub struct AgentNamespace;
impl ConfigNamespace for AgentNamespace {
    const NAMESPACE: &'static str = "agents";
    type Value = AgentSpec;
    fn id(value: &AgentSpec) -> &str { &value.id }
}

pub struct ModelNamespace;
impl ConfigNamespace for ModelNamespace {
    const NAMESPACE: &'static str = "models";
    type Value = ModelConfig;
    fn id(value: &ModelConfig) -> &str { &value.id }
}
```

### D4: ConfigRegistry — typed wrapper over ConfigStore

```rust
pub struct ConfigRegistry<N: ConfigNamespace> {
    store: Arc<dyn ConfigStore>,
    schema: Value,  // Cached JSON Schema from schemars::schema_for!()
    _ns: PhantomData<N>,
}

impl<N: ConfigNamespace> ConfigRegistry<N> {
    pub async fn get(&self, id: &str) -> Result<Option<N::Value>, StorageError>;
    pub async fn list(&self, offset: usize, limit: usize) -> Result<Vec<N::Value>, StorageError>;
    pub async fn put(&self, value: &N::Value) -> Result<(), StorageError>;
    pub async fn delete(&self, id: &str) -> Result<(), StorageError>;

    /// Validate a value against the JSON Schema without persisting.
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError>;

    /// Return the JSON Schema for this namespace.
    pub fn schema(&self) -> &Value;
}
```

### D5: File-based ConfigStore implementation

Follows the existing `FileStore` atomic-write pattern:

```
<base_path>/
  config/
    agents/<id>.json
    models/<id>.json
    providers/<id>.json
    mcp-servers/<name>.json
```

- Write to `.{name}.{uuid}.tmp` → `fsync` → atomic `rename`
- Namespace maps to subdirectory
- ID maps to filename (sanitized)

### D6: PostgreSQL ConfigStore implementation

Single table for all namespaces:

```sql
CREATE TABLE awaken_config (
    namespace TEXT NOT NULL,
    id        TEXT NOT NULL,
    value     JSONB NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (namespace, id)
);
```

One table, one implementation, all namespaces.

### D7: Unified management API routes

A single set of routes handles all namespaces:

```
GET    /v1/config/:namespace              — List entries (paginated)
POST   /v1/config/:namespace              — Create entry
GET    /v1/config/:namespace/:id          — Get entry
PUT    /v1/config/:namespace/:id          — Update entry
DELETE /v1/config/:namespace/:id          — Delete entry
POST   /v1/config/:namespace/:id/validate — Dry-run validation
GET    /v1/config/:namespace/$schema      — JSON Schema for namespace
```

Route handler dispatches by namespace to the corresponding `ConfigRegistry<N>`:

```rust
async fn get_config(
    State(state): State<AppState>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let value = state.config_store.get(&namespace, &id).await?;
    // Mask sensitive fields for "providers" namespace
    Ok(Json(mask_if_needed(&namespace, value)))
}
```

**Convenience aliases** for discoverability:

```
GET /v1/agents     → GET /v1/config/agents
GET /v1/agents/:id → GET /v1/config/agents/:id
...
```

### D8: Bridging ConfigStore to runtime registries

Registries are synchronous runtime lookup; ConfigStore is async persistence.
Bridge with a cache that loads on startup and invalidates on writes:

```rust
pub struct StoreBackedAgentRegistry {
    config: ConfigRegistry<AgentNamespace>,
    cache: RwLock<HashMap<String, AgentSpec>>,
}

impl AgentSpecRegistry for StoreBackedAgentRegistry {
    fn get_agent(&self, id: &str) -> Option<AgentSpec> {
        self.cache.read().get(id).cloned()
    }
    fn agent_ids(&self) -> Vec<String> {
        self.cache.read().keys().cloned().collect()
    }
}

impl StoreBackedAgentRegistry {
    /// Reload cache from store. Called on startup + after API mutations.
    pub async fn refresh(&self) -> Result<(), StorageError>;
}
```

Same pattern for `StoreBackedModelRegistry` and `StoreBackedProviderRegistry`.
On mutation via API → update store → call `refresh()` → next resolve cycle
sees fresh data.

### D9: ProviderConfig — serializable provider definition

Currently the only `LlmExecutor` implementation is `GenaiExecutor`, which
wraps the `genai` crate. API keys come from environment variables
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.) or a custom
`ServiceTargetResolver`. None of this is persistable or API-editable.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProviderConfig {
    /// Stable identifier for this provider configuration.
    pub id: String,
    /// Adapter kind: "anthropic", "openai", "gemini", "ollama", "cohere".
    pub adapter: String,
    /// API key. Encrypted at rest; masked in GET responses.
    /// Falls back to environment variable if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (e.g. "https://my-proxy.example.com/v1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Request timeout in seconds. Default: 300.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Default chat options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_options: Option<ChatOptionsConfig>,
    /// Extra headers (org ID, proxy auth, etc.).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}
```

A `ProviderFactory` converts `ProviderConfig` → `Arc<dyn LlmExecutor>`:

```rust
pub trait ProviderFactory: Send + Sync {
    fn create(&self, config: &ProviderConfig) -> Result<Arc<dyn LlmExecutor>, BuildError>;
    fn supported_adapters(&self) -> Vec<String>;
}
```

Default implementation builds `GenaiExecutor` via:

```rust
genai::Client::builder()
    .with_service_target_resolver_fn(move |st| {
        Ok(ServiceTarget {
            endpoint: config.base_url.map(Endpoint::from_owned)
                .unwrap_or(st.endpoint),
            auth: config.api_key.map(AuthData::from_single)
                .unwrap_or(st.auth),
            model: ModelIden::new(adapter_kind, st.model.model_name),
        })
    })
    .build()
```

**Security**: API key handling:
- **Storage**: encrypted at rest (AES-256-GCM, key from machine identity).
- **API responses**: masked to `"sk-...a1b2"`.
- **Fallback**: absent `api_key` → environment variable.

### D10: McpServerConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpServerConfig {
    pub id: String,
    /// Transport: "stdio", "sse", "streamable-http".
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}
```

### D11: ModelConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelConfig {
    /// Model ID (matches the key agents reference in AgentSpec.model).
    pub id: String,
    /// Provider config ID (references a ProviderConfig).
    pub provider: String,
    /// Actual model name sent to the LLM API.
    pub model: String,
}
```

### D12: Capabilities API (read-only)

```
GET /v1/capabilities
```

Returns all available components for agent construction:

```json
{
  "providers": [
    { "id": "anthropic", "adapter": "anthropic", "has_api_key": true }
  ],
  "models": [
    { "id": "claude-opus", "provider": "anthropic", "model": "claude-opus-4-6" }
  ],
  "plugins": [
    { "id": "ext-permission", "config_schema": { ... } }
  ],
  "tools": [
    { "id": "Bash", "description": "...", "parameters": { ... } }
  ],
  "skills": [
    { "id": "commit", "description": "...", "when_to_use": "..." }
  ],
  "namespaces": [
    { "namespace": "agents", "schema": { ... } },
    { "namespace": "models", "schema": { ... } },
    { "namespace": "providers", "schema": { ... } },
    { "namespace": "mcp-servers", "schema": { ... } }
  ]
}
```

### D13: Validate endpoint

```
POST /v1/config/agents/:id/validate
```

Runs the three-stage resolve pipeline as dry-run. Returns resolved tool
list, plugin chain, model name on success; structured errors on failure.

### D14: AppState extension

```rust
pub struct AppState {
    // Existing
    pub runtime: Arc<AgentRuntime>,
    pub mailbox: Arc<Mailbox>,
    pub store: Arc<dyn ThreadRunStore>,
    pub resolver: Arc<dyn AgentResolver>,

    // Unified config
    pub config_store: Arc<dyn ConfigStore>,
    pub provider_factory: Arc<dyn ProviderFactory>,

    // Namespace registries (typed wrappers, share same config_store)
    pub agents: StoreBackedAgentRegistry,
    pub models: StoreBackedModelRegistry,
}
```

## Implementation order

Deliver in four increments, each independently useful:

1. **ConfigStore trait + File implementation + Agent/Model CRUD**
   `awaken-contract`: `ConfigStore` trait, `ConfigNamespace` trait.
   `awaken-stores`: `FileConfigStore`.
   `awaken-server`: `/v1/config/:namespace` routes.
   Bridge: `StoreBackedAgentRegistry`, `StoreBackedModelRegistry`.

2. **Capabilities API + validate**
   `/v1/capabilities` (tools, plugins with config schemas, skills).
   `/v1/config/agents/:id/validate`.
   Namespace schema endpoint.

3. **ProviderConfig + McpServerConfig**
   `ProviderConfig`, `ProviderFactory`, `McpServerConfig`.
   Provider/MCP namespaces registered.
   API key masking and encryption.

4. **PostgreSQL ConfigStore**
   Single `awaken_config` table.
   Migration scripts.

## Consequences

- **Single store, single API**: one trait, one file impl, one postgres impl,
  one set of routes — all namespaces.
- **Zero-cost namespace extension**: adding a new config entity = one
  `ConfigNamespace` impl, no store/route/migration changes.
- **Agents runtime-modifiable**: CRUD via API without restart.
- **Plugin configs preserved**: `AgentSpec.sections` stored as-is in JSON;
  inactive plugin configs survive and validate on re-activation.
- **Capabilities discoverable**: management UI enumerates tools, plugins,
  providers, and JSON Schemas.
- **Relation to AgentSpec.sections**: `sections` is the agent-internal plugin
  config (embedded). `ConfigStore` is the top-level entity store (root).
  They are orthogonal — agent stored in `ConfigStore.agents` contains its
  own `sections` as part of the JSON document.
