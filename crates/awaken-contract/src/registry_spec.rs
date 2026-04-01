//! Serializable agent definition — pure data, no trait objects.
//!
//! `AgentSpec` is the unified agent configuration: it describes both the
//! declarative registry references (model, plugins, tools) and the runtime
//! behavior (active_hook_filter filtering, typed plugin sections, context policy).
//!
//! Supersedes the former `AgentProfile` — see ADR-0009.

use std::collections::{HashMap, HashSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contract::inference::{ContextWindowPolicy, ReasoningEffort};
use crate::error::StateError;

// ---------------------------------------------------------------------------
// PluginConfigKey — compile-time binding between key string and config type
// ---------------------------------------------------------------------------

/// Typed plugin configuration key.
///
/// Binds a string key to a concrete config type at compile time.
///
/// ```ignore
/// struct PermissionConfigKey;
/// impl PluginConfigKey for PermissionConfigKey {
///     const KEY: &'static str = "permission";
///     type Config = PermissionConfig;
/// }
/// ```
pub trait PluginConfigKey: 'static + Send + Sync {
    /// Section key in the `sections` map.
    const KEY: &'static str;

    /// Typed configuration value.
    type Config: Default
        + Clone
        + Serialize
        + DeserializeOwned
        + schemars::JsonSchema
        + Send
        + Sync
        + 'static;
}

// ---------------------------------------------------------------------------
// AgentSpec
// ---------------------------------------------------------------------------

/// Serializable agent definition referencing registries by ID.
///
/// Can be saved to JSON, loaded from config files, or transmitted over the network.
/// Resolved at runtime via the resolve pipeline into a `ResolvedAgent`.
///
/// Also serves as the runtime behavior configuration passed to hooks via
/// `PhaseContext.agent_spec`. Plugins read their typed config via `spec.config::<K>()`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSpec {
    /// Unique agent identifier.
    pub id: String,
    /// ModelRegistry ID — resolved to a [`ModelSpec`].
    pub model: String,
    /// System prompt sent to the LLM.
    pub system_prompt: String,
    /// Maximum inference rounds before the agent stops.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    /// Maximum continuation retries for truncated LLM responses.
    #[serde(default = "default_max_continuation_retries")]
    pub max_continuation_retries: usize,
    /// Context window management policy. `None` disables compaction and truncation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_policy: Option<ContextWindowPolicy>,
    /// Default reasoning effort for this agent. `None` means no thinking/reasoning.
    /// Can be overridden per-run via `InferenceOverride` or per-step via plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// PluginRegistry IDs — resolved at build time.
    #[serde(default)]
    pub plugin_ids: Vec<String>,
    /// Runtime hook filter: only hooks from plugins in this set will run.
    /// Empty = no filtering (all loaded plugins' hooks run).
    /// Distinct from `plugin_ids` which controls which plugins are loaded.
    #[serde(
        default,
        skip_serializing_if = "HashSet::is_empty",
        alias = "active_plugins"
    )]
    pub active_hook_filter: HashSet<String>,
    /// Allowed tool IDs (whitelist). `None` = all tools.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Excluded tool IDs (blacklist). Applied after `allowed_tools`.
    #[serde(default)]
    pub excluded_tools: Option<Vec<String>>,
    /// Optional remote endpoint. If set, this agent runs on a remote A2A server.
    /// If None, this agent runs locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<RemoteEndpoint>,
    /// IDs of sub-agents this agent can delegate to.
    /// Each ID must be a registered agent in the AgentSpecRegistry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<String>,
    /// Plugin-specific configuration sections (keyed by PluginConfigKey::KEY).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sections: HashMap<String, Value>,
    /// Registry source this agent was loaded from.
    /// `None` for locally defined agents; `Some("cloud")` for agents from the "cloud" registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// Remote endpoint configuration for agents running on external A2A servers.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RemoteEndpoint {
    pub base_url: String,
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Target agent ID on the remote server. If `None`, omits the agentId
    /// field in the A2A request and lets the remote server use its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

impl Default for RemoteEndpoint {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            bearer_token: None,
            agent_id: None,
            poll_interval_ms: default_poll_interval(),
            timeout_ms: default_timeout(),
        }
    }
}

fn default_poll_interval() -> u64 {
    2000
}

fn default_timeout() -> u64 {
    300_000
}

// ---------------------------------------------------------------------------
// ModelSpec
// ---------------------------------------------------------------------------

/// Serializable model definition — maps a model ID to a provider and API model name.
///
/// Used for both persistent configuration (ConfigStore) and runtime registry lookup.
/// Replaces the former `ModelConfig` (config.rs) and `ModelEntry` (traits.rs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelSpec {
    /// Unique identifier (e.g. "claude-opus", "gpt-4o").
    pub id: String,
    /// Provider spec ID (references a [`ProviderSpec`]).
    pub provider: String,
    /// Actual model name sent to the LLM API (e.g. "claude-opus-4-6").
    pub model: String,
}

// ---------------------------------------------------------------------------
// ProviderSpec
// ---------------------------------------------------------------------------

/// Serializable LLM provider configuration.
///
/// Captures everything needed to construct a `GenaiExecutor` via
/// `genai::Client::builder()`: adapter kind, API key, base URL, timeout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderSpec {
    /// Unique identifier (e.g. "anthropic", "openai-proxy").
    pub id: String,
    /// Adapter kind: "anthropic", "openai", "gemini", "ollama", "cohere".
    /// Maps to `genai::adapter::AdapterKind` for model routing.
    pub adapter: String,
    /// API key. Falls back to the adapter's default environment variable
    /// (e.g. `ANTHROPIC_API_KEY`) if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (e.g. "https://my-proxy.example.com/v1").
    /// `None` = use the adapter's default endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Request timeout in seconds.
    #[serde(default = "default_provider_timeout_secs")]
    pub timeout_secs: u64,
    /// Extra headers to include in every request (e.g. org ID, proxy auth).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

fn default_provider_timeout_secs() -> u64 {
    300
}

impl Default for ProviderSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            adapter: String::new(),
            api_key: None,
            base_url: None,
            timeout_secs: default_provider_timeout_secs(),
            headers: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// McpServerSpec
// ---------------------------------------------------------------------------

/// Serializable MCP server connection configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct McpServerSpec {
    /// Unique identifier (e.g. "my-mcp-server").
    pub id: String,
    /// Transport type: "stdio", "sse", "streamable-http".
    pub transport: String,
    /// Command for stdio transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Args for stdio transport.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// URL for HTTP-based transports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Environment variables to set for the process.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

impl Default for McpServerSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            transport: "stdio".into(),
            command: None,
            args: Vec::new(),
            url: None,
            env: HashMap::new(),
        }
    }
}

impl Default for AgentSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            model: String::new(),
            system_prompt: String::new(),
            max_rounds: default_max_rounds(),
            max_continuation_retries: default_max_continuation_retries(),
            context_policy: None,
            reasoning_effort: None,
            plugin_ids: Vec::new(),
            active_hook_filter: HashSet::new(),
            allowed_tools: None,
            excluded_tools: None,
            endpoint: None,
            delegates: Vec::new(),
            sections: HashMap::new(),
            registry: None,
        }
    }
}

fn default_max_rounds() -> usize {
    16
}

fn default_max_continuation_retries() -> usize {
    2
}

impl AgentSpec {
    /// Create a new agent spec with default settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use awaken_contract::registry_spec::AgentSpec;
    ///
    /// let spec = AgentSpec::new("assistant")
    ///     .with_model("gpt-4o-mini")
    ///     .with_system_prompt("You are helpful.")
    ///     .with_max_rounds(10);
    /// assert_eq!(spec.id, "assistant");
    /// assert_eq!(spec.model, "gpt-4o-mini");
    /// assert_eq!(spec.system_prompt, "You are helpful.");
    /// assert_eq!(spec.max_rounds, 10);
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Default::default()
        }
    }

    // -- Typed config access --

    /// Read a typed plugin config section.
    /// Returns `Config::default()` if the section is missing.
    /// Returns error if the section exists but fails to deserialize.
    pub fn config<K: PluginConfigKey>(&self) -> Result<K::Config, StateError> {
        match self.sections.get(K::KEY) {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|e| StateError::KeyDecode {
                    key: K::KEY.into(),
                    message: e.to_string(),
                })
            }
            None => Ok(K::Config::default()),
        }
    }

    /// Set a typed plugin config section.
    pub fn set_config<K: PluginConfigKey>(&mut self, config: K::Config) -> Result<(), StateError> {
        let value = serde_json::to_value(config).map_err(|e| StateError::KeyEncode {
            key: K::KEY.into(),
            message: e.to_string(),
        })?;
        self.sections.insert(K::KEY.to_string(), value);
        Ok(())
    }

    // -- Builder methods --

    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    #[must_use]
    pub fn with_max_rounds(mut self, n: usize) -> Self {
        self.max_rounds = n;
        self
    }

    #[must_use]
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    #[must_use]
    pub fn with_hook_filter(mut self, plugin_id: impl Into<String>) -> Self {
        self.active_hook_filter.insert(plugin_id.into());
        self
    }

    /// Set a typed plugin config section (builder variant).
    pub fn with_config<K: PluginConfigKey>(
        mut self,
        config: K::Config,
    ) -> Result<Self, StateError> {
        self.set_config::<K>(config)?;
        Ok(self)
    }

    #[must_use]
    pub fn with_delegate(mut self, agent_id: impl Into<String>) -> Self {
        self.delegates.push(agent_id.into());
        self
    }

    #[must_use]
    pub fn with_endpoint(mut self, endpoint: RemoteEndpoint) -> Self {
        self.endpoint = Some(endpoint);
        self
    }

    /// Set a raw JSON section (for tests or untyped usage).
    #[must_use]
    pub fn with_section(mut self, key: impl Into<String>, value: Value) -> Self {
        self.sections.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_spec_serde_roundtrip() {
        let spec = AgentSpec {
            id: "coder".into(),
            model: "claude-opus".into(),
            system_prompt: "You are a coding assistant.".into(),
            max_rounds: 8,
            plugin_ids: vec!["permission".into(), "logging".into()],
            allowed_tools: Some(vec!["read_file".into(), "write_file".into()]),
            excluded_tools: Some(vec!["delete_file".into()]),
            sections: {
                let mut m = HashMap::new();
                m.insert("permission".into(), json!({"mode": "strict"}));
                m
            },
            ..Default::default()
        };

        let json_str = serde_json::to_string(&spec).unwrap();
        let parsed: AgentSpec = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, "coder");
        assert_eq!(parsed.model, "claude-opus");
        assert_eq!(parsed.system_prompt, "You are a coding assistant.");
        assert_eq!(parsed.max_rounds, 8);
        assert_eq!(parsed.plugin_ids, vec!["permission", "logging"]);
        assert_eq!(
            parsed.allowed_tools,
            Some(vec!["read_file".into(), "write_file".into()])
        );
        assert_eq!(parsed.excluded_tools, Some(vec!["delete_file".into()]));
        assert_eq!(parsed.sections["permission"]["mode"], "strict");
    }

    #[test]
    fn agent_spec_defaults() {
        let json_str = r#"{"id":"min","model":"m","system_prompt":"sp"}"#;
        let spec: AgentSpec = serde_json::from_str(json_str).unwrap();

        assert_eq!(spec.max_rounds, 16);
        assert_eq!(spec.max_continuation_retries, 2);
        assert!(spec.context_policy.is_none());
        assert!(spec.plugin_ids.is_empty());
        assert!(spec.active_hook_filter.is_empty());
        assert!(spec.allowed_tools.is_none());
        assert!(spec.excluded_tools.is_none());
        assert!(spec.sections.is_empty());
    }

    // -- Typed config tests (merged from AgentProfile) --

    struct ModelNameKey;
    impl PluginConfigKey for ModelNameKey {
        const KEY: &'static str = "model_name";
        type Config = ModelNameConfig;
    }

    #[derive(
        Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
    )]
    struct ModelNameConfig {
        pub name: String,
    }

    struct PermKey;
    impl PluginConfigKey for PermKey {
        const KEY: &'static str = "permission";
        type Config = PermConfig;
    }

    #[derive(
        Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
    )]
    struct PermConfig {
        pub mode: String,
    }

    #[test]
    fn typed_config_roundtrip() {
        let spec = AgentSpec::new("test")
            .with_config::<ModelNameKey>(ModelNameConfig {
                name: "opus".into(),
            })
            .unwrap()
            .with_config::<PermKey>(PermConfig {
                mode: "strict".into(),
            })
            .unwrap();

        let model: ModelNameConfig = spec.config::<ModelNameKey>().unwrap();
        assert_eq!(model.name, "opus");

        let perm: PermConfig = spec.config::<PermKey>().unwrap();
        assert_eq!(perm.mode, "strict");
    }

    #[test]
    fn missing_config_returns_default() {
        let spec = AgentSpec::new("test");
        let model: ModelNameConfig = spec.config::<ModelNameKey>().unwrap();
        assert_eq!(model, ModelNameConfig::default());
    }

    #[test]
    fn config_serializes_to_json() {
        let spec = AgentSpec::new("coder")
            .with_model("sonnet")
            .with_config::<ModelNameKey>(ModelNameConfig {
                name: "custom".into(),
            })
            .unwrap();

        let json = serde_json::to_string(&spec).unwrap();
        let parsed: AgentSpec = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "coder");
        assert_eq!(parsed.model, "sonnet");

        let model: ModelNameConfig = parsed.config::<ModelNameKey>().unwrap();
        assert_eq!(model.name, "custom");
    }

    #[test]
    fn multiple_configs_independent() {
        let mut spec = AgentSpec::new("test");
        spec.set_config::<ModelNameKey>(ModelNameConfig { name: "a".into() })
            .unwrap();
        spec.set_config::<PermKey>(PermConfig { mode: "b".into() })
            .unwrap();

        // Update one doesn't affect the other
        spec.set_config::<ModelNameKey>(ModelNameConfig {
            name: "updated".into(),
        })
        .unwrap();

        let model: ModelNameConfig = spec.config::<ModelNameKey>().unwrap();
        assert_eq!(model.name, "updated");

        let perm: PermConfig = spec.config::<PermKey>().unwrap();
        assert_eq!(perm.mode, "b");
    }

    #[test]
    fn with_section_raw_json_still_works() {
        let spec =
            AgentSpec::new("test").with_section("custom", serde_json::json!({"key": "value"}));
        assert_eq!(spec.sections["custom"]["key"], "value");
    }

    #[test]
    fn builder() {
        let spec = AgentSpec::new("reviewer")
            .with_model("claude-opus")
            .with_hook_filter("permission")
            .with_config::<PermKey>(PermConfig {
                mode: "strict".into(),
            })
            .unwrap();

        assert_eq!(spec.id, "reviewer");
        assert_eq!(spec.model, "claude-opus");
        assert!(spec.active_hook_filter.contains("permission"));
    }

    // -- ModelSpec tests --

    #[test]
    fn model_spec_serde_roundtrip() {
        let spec = ModelSpec {
            id: "claude-opus".into(),
            provider: "anthropic".into(),
            model: "claude-opus-4-6".into(),
        };
        let json = serde_json::to_value(&spec).unwrap();
        let parsed: ModelSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
    }

    // -- ProviderSpec tests --

    #[test]
    fn provider_spec_serde_roundtrip() {
        let spec = ProviderSpec {
            id: "anthropic".into(),
            adapter: "anthropic".into(),
            api_key: Some("sk-test".into()),
            base_url: Some("https://proxy.example.com/v1".into()),
            timeout_secs: 120,
            headers: {
                let mut h = HashMap::new();
                h.insert("X-Org-Id".into(), "org-123".into());
                h
            },
        };
        let json = serde_json::to_value(&spec).unwrap();
        let parsed: ProviderSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn provider_spec_defaults() {
        let json_str = r#"{"id":"p","adapter":"openai"}"#;
        let spec: ProviderSpec = serde_json::from_str(json_str).unwrap();
        assert_eq!(spec.timeout_secs, 300);
        assert!(spec.api_key.is_none());
        assert!(spec.base_url.is_none());
        assert!(spec.headers.is_empty());
    }

    #[test]
    fn provider_spec_omits_empty_optional_fields() {
        let spec = ProviderSpec {
            id: "ollama".into(),
            adapter: "ollama".into(),
            ..Default::default()
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert!(json.get("api_key").is_none());
        assert!(json.get("base_url").is_none());
        assert!(json.get("headers").is_none());
    }

    // -- McpServerSpec tests --

    #[test]
    fn mcp_server_spec_serde_roundtrip() {
        let spec = McpServerSpec {
            id: "my-server".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "@my/mcp-server".into()],
            url: None,
            env: {
                let mut e = HashMap::new();
                e.insert("NODE_ENV".into(), "production".into());
                e
            },
        };
        let json = serde_json::to_value(&spec).unwrap();
        let parsed: McpServerSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn mcp_server_spec_http_transport() {
        let spec = McpServerSpec {
            id: "remote".into(),
            transport: "sse".into(),
            url: Some("https://mcp.example.com/sse".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["transport"], "sse");
        assert_eq!(json["url"], "https://mcp.example.com/sse");
        assert!(json.get("command").is_none());
    }

    #[test]
    fn mcp_server_spec_defaults() {
        let json_str = r#"{"id":"s","transport":"stdio"}"#;
        let spec: McpServerSpec = serde_json::from_str(json_str).unwrap();
        assert!(spec.command.is_none());
        assert!(spec.args.is_empty());
        assert!(spec.url.is_none());
        assert!(spec.env.is_empty());
    }
}
