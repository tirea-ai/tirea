use super::*;
use crate::contracts::runtime::phase::{Phase, StepContext};
use crate::contracts::state::AgentState;
use crate::contracts::storage::{AgentStateReader, AgentStateWriter};
use crate::contracts::tool::ToolDescriptor;
use crate::contracts::tool::{ToolError, ToolResult};
use crate::contracts::AgentState as ContextAgentState;
use crate::extensions::skills::{
    FsSkill, FsSkillRegistryManager, InMemorySkillRegistry, ScriptResult, Skill, SkillError,
    SkillMeta, SkillRegistry, SkillRegistryError, SkillResource, SkillResourceKind,
};
use crate::orchestrator::agent_tools::SCOPE_CALLER_AGENT_ID_KEY;
use crate::runtime::loop_runner::{
    TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use async_trait::async_trait;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn make_skills_root() -> (TempDir, PathBuf) {
    let td = TempDir::new().unwrap();
    let root = td.path().join("skills");
    fs::create_dir_all(root.join("s1")).unwrap();
    fs::write(
        root.join("s1").join("SKILL.md"),
        "---\nname: s1\ndescription: ok\n---\nDo X\n",
    )
    .unwrap();
    (td, root)
}

#[derive(Clone)]
struct FailOnNthAppendStorage {
    inner: Arc<carve_thread_store_adapters::MemoryStore>,
    fail_on_nth_append: usize,
    append_calls: Arc<AtomicUsize>,
}

impl FailOnNthAppendStorage {
    fn new(fail_on_nth_append: usize) -> Self {
        Self {
            inner: Arc::new(carve_thread_store_adapters::MemoryStore::new()),
            fail_on_nth_append,
            append_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn append_call_count(&self) -> usize {
        self.append_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl crate::contracts::storage::AgentStateReader for FailOnNthAppendStorage {
    async fn load(
        &self,
        thread_id: &str,
    ) -> Result<
        Option<crate::contracts::storage::AgentStateHead>,
        crate::contracts::storage::AgentStateStoreError,
    > {
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateReader>::load(
            self.inner.as_ref(),
            thread_id,
        )
        .await
    }

    async fn list_agent_states(
        &self,
        query: &crate::contracts::storage::AgentStateListQuery,
    ) -> Result<
        crate::contracts::storage::AgentStateListPage,
        crate::contracts::storage::AgentStateStoreError,
    > {
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateReader>::list_agent_states(
            self.inner.as_ref(),
            query,
        )
        .await
    }
}

#[async_trait]
impl crate::contracts::storage::AgentStateWriter for FailOnNthAppendStorage {
    async fn create(
        &self,
        thread: &AgentState,
    ) -> Result<crate::contracts::storage::Committed, crate::contracts::storage::AgentStateStoreError>
    {
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateWriter>::create(
            self.inner.as_ref(),
            thread,
        )
        .await
    }

    async fn append(
        &self,
        thread_id: &str,
        changeset: &crate::contracts::AgentChangeSet,
        precondition: crate::contracts::storage::VersionPrecondition,
    ) -> Result<crate::contracts::storage::Committed, crate::contracts::storage::AgentStateStoreError>
    {
        let append_idx = self.append_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if append_idx == self.fail_on_nth_append {
            return Err(
                crate::contracts::storage::AgentStateStoreError::Serialization(format!(
                    "injected append failure on call {append_idx}"
                )),
            );
        }
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateWriter>::append(
            self.inner.as_ref(),
            thread_id,
            changeset,
            precondition,
        )
        .await
    }

    async fn delete(
        &self,
        thread_id: &str,
    ) -> Result<(), crate::contracts::storage::AgentStateStoreError> {
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateWriter>::delete(
            self.inner.as_ref(),
            thread_id,
        )
        .await
    }

    async fn save(
        &self,
        thread: &AgentState,
    ) -> Result<(), crate::contracts::storage::AgentStateStoreError> {
        <carve_thread_store_adapters::MemoryStore as crate::contracts::storage::AgentStateWriter>::save(
            self.inner.as_ref(),
            thread,
        )
        .await
    }
}

#[derive(Debug)]
struct SkipWithRunEndPatchPlugin;

#[async_trait]
impl AgentPlugin for SkipWithRunEndPatchPlugin {
    fn id(&self) -> &str {
        "skip_with_run_end_patch"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &ContextAgentState) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
        if phase == Phase::RunEnd {
            let patch = carve_state::TrackedPatch::new(carve_state::Patch::new().with_op(
                carve_state::Op::set(carve_state::path!("run_end_marker"), json!(true)),
            ))
            .with_source("test:run_end_marker");
            step.pending_patches.push(patch);
        }
    }
}

#[tokio::test]
async fn wire_skills_inserts_tools_and_plugin() {
    let doc = json!({});
    let ctx = ContextAgentState::new_transient(&doc, "test", "test");
    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .build()
        .unwrap();

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    let cfg = AgentDefinition::new("gpt-4o-mini");
    let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

    assert!(tools.contains_key("skill"));
    assert!(tools.contains_key("load_skill_resource"));
    assert!(tools.contains_key("skill_script"));

    assert_eq!(cfg.plugins.len(), 1);
    assert_eq!(cfg.plugins[0].id(), "skills");

    // Verify injection does not panic and includes catalog.
    let thread = AgentState::with_initial_state(
        "s",
        json!({
            "skills": {
                "active": ["s1"],
                "instructions": {"s1": "Do X"},
                "references": {},
                "scripts": {}
            }
        }),
    );
    let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
    cfg.plugins[0]
        .on_phase(Phase::BeforeInference, &mut step, &ctx)
        .await;
    let merged = step.system_context.join("\n");
    assert!(merged.contains("<available_skills>"));
    assert!(
        !merged.contains("<skill_instructions skill=\"s1\">"),
        "runtime skill instructions are delivered via append_user_messages, not system context"
    );
}

#[tokio::test]
async fn wire_skills_runtime_only_injects_active_skills_without_catalog() {
    let doc = json!({});
    let ctx = ContextAgentState::new_transient(&doc, "test", "test");
    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::RuntimeOnly,
            ..SkillsConfig::default()
        })
        .build()
        .unwrap();

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    let cfg = AgentDefinition::new("gpt-4o-mini");
    let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

    assert_eq!(cfg.plugins.len(), 1);
    assert_eq!(cfg.plugins[0].id(), "skills_runtime");

    let thread = AgentState::with_initial_state(
        "s",
        json!({
            "skills": {
                "active": ["s1"],
                "instructions": {"s1": "Do X"},
                "references": {},
                "scripts": {}
            }
        }),
    );
    let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
    cfg.plugins[0]
        .on_phase(Phase::BeforeInference, &mut step, &ctx)
        .await;
    let merged = step.system_context.join("\n");
    assert!(!merged.contains("<available_skills>"));
    assert!(
        !merged.contains("<skill_instructions skill=\"s1\">"),
        "runtime-only plugin is intentionally no-op for system context"
    );
}

#[test]
fn wire_skills_disabled_is_noop() {
    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::Disabled,
            ..SkillsConfig::default()
        })
        .build()
        .unwrap();

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    let cfg = AgentDefinition::new("gpt-4o-mini");
    let cfg2 = os.wire_skills_into(cfg.clone(), &mut tools).unwrap();

    assert!(tools.is_empty());
    assert_eq!(cfg2.plugins.len(), cfg.plugins.len());
}

#[test]
fn wire_plugins_into_orders_policy_then_plugin_then_explicit() {
    #[derive(Debug)]
    struct LocalPlugin(&'static str);

    #[async_trait]
    impl AgentPlugin for LocalPlugin {
        fn id(&self) -> &str {
            self.0
        }

        async fn on_phase(
            &self,
            _phase: Phase,
            _step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
        }
    }

    let os = AgentOs::builder()
        .with_registered_plugin("policy1", Arc::new(LocalPlugin("policy1")))
        .with_registered_plugin("p1", Arc::new(LocalPlugin("p1")))
        .build()
        .unwrap();

    let cfg = AgentDefinition::new("gpt-4o-mini")
        .with_policy_id("policy1")
        .with_plugin_id("p1")
        .with_plugin(Arc::new(LocalPlugin("explicit")));

    let wired = os.wire_plugins_into(cfg).unwrap();
    let ids: Vec<&str> = wired.plugins.iter().map(|p| p.id()).collect();
    assert_eq!(ids, vec!["policy1", "p1", "explicit"]);
}

#[test]
fn wire_plugins_into_rejects_duplicate_plugin_ids_after_assembly() {
    #[derive(Debug)]
    struct LocalPlugin(&'static str);

    #[async_trait]
    impl AgentPlugin for LocalPlugin {
        fn id(&self) -> &str {
            self.0
        }

        async fn on_phase(
            &self,
            _phase: Phase,
            _step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
        }
    }

    let os = AgentOs::builder()
        .with_registered_plugin("p1", Arc::new(LocalPlugin("p1")))
        .build()
        .unwrap();

    let cfg = AgentDefinition::new("gpt-4o-mini")
        .with_plugin_id("p1")
        .with_plugin(Arc::new(LocalPlugin("p1")));

    let err = os.wire_plugins_into(cfg).unwrap_err();
    assert!(matches!(err, AgentOsWiringError::PluginAlreadyInstalled(id) if id == "p1"));
}

#[derive(Debug)]
struct FakeSkillsPlugin;

#[async_trait::async_trait]
impl AgentPlugin for FakeSkillsPlugin {
    fn id(&self) -> &str {
        "skills"
    }

    async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>, _ctx: &ContextAgentState) {
    }
}

#[test]
fn wire_skills_errors_if_plugin_already_installed() {
    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .build()
        .unwrap();

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    let cfg = AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(FakeSkillsPlugin));

    let err = os.wire_skills_into(cfg, &mut tools).unwrap_err();
    assert!(err.to_string().contains("skills plugin already installed"));
}

#[derive(Debug)]
struct FakeAgentToolsPlugin;

#[async_trait::async_trait]
impl AgentPlugin for FakeAgentToolsPlugin {
    fn id(&self) -> &str {
        "agent_tools"
    }

    async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>, _ctx: &ContextAgentState) {
    }
}

#[test]
fn resolve_errors_if_agent_tools_plugin_already_installed() {
    let os = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(FakeAgentToolsPlugin)),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::AgentToolsPluginAlreadyInstalled(ref id))
        if id == "agent_tools"
    ));
}

#[derive(Debug)]
struct FakeAgentRecoveryPlugin;

#[async_trait::async_trait]
impl AgentPlugin for FakeAgentRecoveryPlugin {
    fn id(&self) -> &str {
        "agent_recovery"
    }

    async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>, _ctx: &ContextAgentState) {
    }
}

#[test]
fn resolve_errors_if_agent_recovery_plugin_already_installed() {
    let os = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(FakeAgentRecoveryPlugin)),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(
            AgentOsWiringError::AgentRecoveryPluginAlreadyInstalled(ref id)
        ) if id == "agent_recovery"
    ));
}

#[test]
fn resolve_errors_if_agent_missing() {
    let os = AgentOs::builder().build().unwrap();
    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("missing", thread).err().unwrap();
    assert!(matches!(err, AgentOsResolveError::AgentNotFound(_)));
}

#[tokio::test]
async fn resolve_wires_skills_and_preserves_base_tools() {
    #[derive(Debug)]
    struct BaseTool;

    #[async_trait::async_trait]
    impl Tool for BaseTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("base_tool", "Base Tool", "Base Tool")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("base_tool", json!({"ok": true})))
        }
    }

    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .with_tools(HashMap::from([(
            "base_tool".to_string(),
            Arc::new(BaseTool) as Arc<dyn Tool>,
        )]))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, tools, _thread) = os.resolve("a1", thread).unwrap();

    assert_eq!(cfg.id, "a1");
    assert!(tools.contains_key("base_tool"));
    assert!(tools.contains_key("skill"));
    assert!(tools.contains_key("load_skill_resource"));
    assert!(tools.contains_key("skill_script"));
    assert!(tools.contains_key("agent_run"));
    assert!(tools.contains_key("agent_stop"));
    assert_eq!(cfg.plugins.len(), 3);
    assert_eq!(cfg.plugins[0].id(), "skills");
    assert_eq!(cfg.plugins[1].id(), "agent_tools");
    assert_eq!(cfg.plugins[2].id(), "agent_recovery");
}

#[test]
fn resolve_freezes_tool_snapshot_per_run_boundary() {
    #[derive(Debug)]
    struct NamedTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for NamedTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(self.0, self.0, "dynamic tool")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(self.0, json!({"ok": true})))
        }
    }

    #[derive(Default)]
    struct MutableRegistry {
        tools: std::sync::RwLock<HashMap<String, Arc<dyn Tool>>>,
    }

    impl MutableRegistry {
        fn replace(&self, ids: &[&'static str]) {
            let mut next = HashMap::new();
            for id in ids {
                next.insert((*id).to_string(), Arc::new(NamedTool(id)) as Arc<dyn Tool>);
            }
            match self.tools.write() {
                Ok(mut guard) => *guard = next,
                Err(poisoned) => *poisoned.into_inner() = next,
            }
        }
    }

    impl ToolRegistry for MutableRegistry {
        fn len(&self) -> usize {
            self.snapshot().len()
        }

        fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
            self.snapshot().get(id).cloned()
        }

        fn ids(&self) -> Vec<String> {
            let mut ids: Vec<String> = self.snapshot().keys().cloned().collect();
            ids.sort();
            ids
        }

        fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>> {
            match self.tools.read() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }
    }

    let dynamic_registry = Arc::new(MutableRegistry::default());
    dynamic_registry.replace(&["mcp__s1__echo"]);

    let os = AgentOs::builder()
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .with_tool_registry(dynamic_registry.clone() as Arc<dyn ToolRegistry>)
        .build()
        .expect("build agent os");

    let thread1 = AgentState::with_initial_state("freeze-1", json!({}));
    let (_cfg1, tools_first_run, _thread1) = os.resolve("a1", thread1).expect("resolve #1");
    assert!(tools_first_run.contains_key("mcp__s1__echo"));
    assert!(!tools_first_run.contains_key("mcp__s1__sum"));

    dynamic_registry.replace(&["mcp__s1__sum"]);

    // The first run snapshot is frozen.
    assert!(tools_first_run.contains_key("mcp__s1__echo"));
    assert!(!tools_first_run.contains_key("mcp__s1__sum"));

    // The next resolve picks up refreshed registry state.
    let thread2 = AgentState::with_initial_state("freeze-2", json!({}));
    let (_cfg2, tools_second_run, _thread2) = os.resolve("a1", thread2).expect("resolve #2");
    assert!(!tools_second_run.contains_key("mcp__s1__echo"));
    assert!(tools_second_run.contains_key("mcp__s1__sum"));
}

#[tokio::test]
async fn resolve_freezes_agent_snapshot_per_run_boundary() {
    #[derive(Default)]
    struct MutableAgentRegistry {
        agents: std::sync::RwLock<HashMap<String, AgentDefinition>>,
    }

    impl MutableAgentRegistry {
        fn replace_ids(&self, ids: &[&str]) {
            let mut map = HashMap::new();
            for id in ids {
                map.insert((*id).to_string(), AgentDefinition::new("gpt-4o-mini"));
            }
            match self.agents.write() {
                Ok(mut guard) => *guard = map,
                Err(poisoned) => *poisoned.into_inner() = map,
            }
        }
    }

    impl AgentRegistry for MutableAgentRegistry {
        fn len(&self) -> usize {
            self.snapshot().len()
        }

        fn get(&self, id: &str) -> Option<AgentDefinition> {
            self.snapshot().get(id).cloned()
        }

        fn ids(&self) -> Vec<String> {
            let mut ids: Vec<String> = self.snapshot().keys().cloned().collect();
            ids.sort();
            ids
        }

        fn snapshot(&self) -> HashMap<String, AgentDefinition> {
            match self.agents.read() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }
    }

    let dynamic_agents = Arc::new(MutableAgentRegistry::default());
    dynamic_agents.replace_ids(&["worker_a"]);

    let os = AgentOs::builder()
        .with_agent("root", AgentDefinition::new("gpt-4o-mini"))
        .with_agent_registry(dynamic_agents.clone() as Arc<dyn AgentRegistry>)
        .build()
        .expect("build agent os");

    let thread1 = AgentState::with_initial_state("freeze-agent-1", json!({}));
    let (_cfg1, tools_first_run, _thread1) = os.resolve("root", thread1).expect("resolve #1");
    let run_tool_first = tools_first_run
        .get("agent_run")
        .cloned()
        .expect("agent_run tool should exist");

    // Update source registry after resolve #1. First run should still see worker_a.
    dynamic_agents.replace_ids(&["worker_b"]);

    let scope = {
        let mut scope = carve_state::ScopeState::new();
        scope
            .set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, "owner-thread")
            .expect("set caller thread id");
        scope
            .set(TOOL_SCOPE_CALLER_AGENT_ID_KEY, "root")
            .expect("set caller agent id");
        scope
    };
    let doc = json!({});
    let ctx_first = ContextAgentState::new_transient(&doc, "call-1", "tool:agent_run")
        .with_transient_scope(Some(&scope));
    let first_result = run_tool_first
        .execute(
            json!({
                "agent_id": "worker_a",
                "prompt": "hi",
                "background": true
            }),
            &ctx_first,
        )
        .await
        .expect("execute first run tool");
    assert!(
        first_result.is_success(),
        "first run should use frozen agents"
    );

    // Next resolve should use refreshed source and reject worker_a.
    let thread2 = AgentState::with_initial_state("freeze-agent-2", json!({}));
    let (_cfg2, tools_second_run, _thread2) = os.resolve("root", thread2).expect("resolve #2");
    let run_tool_second = tools_second_run
        .get("agent_run")
        .cloned()
        .expect("agent_run tool should exist");
    let ctx_second = ContextAgentState::new_transient(&doc, "call-2", "tool:agent_run")
        .with_transient_scope(Some(&scope));
    let second_result = run_tool_second
        .execute(
            json!({
                "agent_id": "worker_a",
                "prompt": "hi",
                "background": false
            }),
            &ctx_second,
        )
        .await
        .expect("execute second run tool");
    assert!(
        second_result.is_error(),
        "second run should observe updated agents snapshot"
    );
    assert!(second_result
        .message
        .unwrap_or_default()
        .contains("Unknown or unavailable agent_id"));
}

#[tokio::test]
async fn resolve_freezes_skill_snapshot_per_run_boundary() {
    #[derive(Debug)]
    struct MockSkill {
        meta: SkillMeta,
        raw: String,
    }

    impl MockSkill {
        fn new(id: &str) -> Self {
            Self {
                meta: SkillMeta {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: format!("{id} skill"),
                    allowed_tools: Vec::new(),
                },
                raw: format!("---\nname: {id}\ndescription: ok\n---\nBody\n"),
            }
        }
    }

    #[async_trait::async_trait]
    impl Skill for MockSkill {
        fn meta(&self) -> &SkillMeta {
            &self.meta
        }

        async fn read_instructions(&self) -> Result<String, SkillError> {
            Ok(self.raw.clone())
        }

        async fn load_resource(
            &self,
            _kind: SkillResourceKind,
            _path: &str,
        ) -> Result<SkillResource, SkillError> {
            Err(SkillError::Unsupported("not used".to_string()))
        }

        async fn run_script(
            &self,
            _script: &str,
            _args: &[String],
        ) -> Result<ScriptResult, SkillError> {
            Err(SkillError::Unsupported("not used".to_string()))
        }
    }

    #[derive(Default)]
    struct MutableSkillRegistry {
        skills: std::sync::RwLock<HashMap<String, Arc<dyn Skill>>>,
    }

    impl MutableSkillRegistry {
        fn replace_ids(&self, ids: &[&str]) {
            let mut map: HashMap<String, Arc<dyn Skill>> = HashMap::new();
            for id in ids {
                map.insert((*id).to_string(), Arc::new(MockSkill::new(id)));
            }
            match self.skills.write() {
                Ok(mut guard) => *guard = map,
                Err(poisoned) => *poisoned.into_inner() = map,
            }
        }
    }

    impl SkillRegistry for MutableSkillRegistry {
        fn len(&self) -> usize {
            self.snapshot().len()
        }

        fn get(&self, id: &str) -> Option<Arc<dyn Skill>> {
            self.snapshot().get(id).cloned()
        }

        fn ids(&self) -> Vec<String> {
            let mut ids: Vec<String> = self.snapshot().keys().cloned().collect();
            ids.sort();
            ids
        }

        fn snapshot(&self) -> HashMap<String, Arc<dyn Skill>> {
            match self.skills.read() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }
    }

    let dynamic_skills = Arc::new(MutableSkillRegistry::default());
    dynamic_skills.replace_ids(&["s1"]);

    let os = AgentOs::builder()
        .with_agent("root", AgentDefinition::new("gpt-4o-mini"))
        .with_skill_registry(dynamic_skills.clone() as Arc<dyn SkillRegistry>)
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryOnly,
            discovery_max_entries: 32,
            discovery_max_chars: 8 * 1024,
        })
        .build()
        .expect("build agent os");

    let doc = json!({});
    let thread1 = AgentState::with_initial_state("freeze-skill-1", json!({}));
    let (_cfg1, tools_first_run, _thread1) = os.resolve("root", thread1).expect("resolve #1");
    let activate_first = tools_first_run
        .get("skill")
        .cloned()
        .expect("skill activate tool should exist");

    dynamic_skills.replace_ids(&["s2"]);

    let ctx_first = ContextAgentState::new_transient(&doc, "call-skill-1", "tool:skill");
    let first_result = activate_first
        .execute(json!({"skill": "s1"}), &ctx_first)
        .await
        .expect("execute first skill tool");
    assert!(
        first_result.is_success(),
        "first run should use frozen skills"
    );

    let thread2 = AgentState::with_initial_state("freeze-skill-2", json!({}));
    let (_cfg2, tools_second_run, _thread2) = os.resolve("root", thread2).expect("resolve #2");
    let activate_second = tools_second_run
        .get("skill")
        .cloned()
        .expect("skill activate tool should exist");
    let ctx_second = ContextAgentState::new_transient(&doc, "call-skill-2", "tool:skill");
    let second_result = activate_second
        .execute(json!({"skill": "s1"}), &ctx_second)
        .await
        .expect("execute second skill tool");
    assert!(
        second_result.is_error(),
        "second run should observe updated skills snapshot"
    );
    assert!(second_result
        .message
        .unwrap_or_default()
        .contains("Unknown skill"));
}

#[test]
fn build_skill_registry_refresh_interval_starts_periodic_refresh() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("skills");
    fs::create_dir_all(root.join("s1")).unwrap();
    fs::write(
        root.join("s1").join("SKILL.md"),
        "---\nname: s1\ndescription: ok\n---\nBody\n",
    )
    .unwrap();

    let manager = FsSkillRegistryManager::discover_roots(vec![root.clone()]).unwrap();
    assert!(!manager.periodic_refresh_running());

    let _os = AgentOs::builder()
        .with_agent("root", AgentDefinition::new("gpt-4o-mini"))
        .with_skill_registry(Arc::new(manager.clone()) as Arc<dyn SkillRegistry>)
        .with_skill_registry_refresh_interval(Duration::from_millis(20))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryOnly,
            discovery_max_entries: 32,
            discovery_max_chars: 8 * 1024,
        })
        .build()
        .expect("build agent os");

    assert!(manager.periodic_refresh_running());

    fs::create_dir_all(root.join("s2")).unwrap();
    fs::write(
        root.join("s2").join("SKILL.md"),
        "---\nname: s2\ndescription: ok\n---\nBody\n",
    )
    .unwrap();

    std::thread::sleep(Duration::from_millis(150));
    assert!(manager.get("s2").is_some());
    assert!(manager.stop_periodic_refresh());
}

#[tokio::test]
async fn run_and_run_stream_work_without_llm_when_skip_inference() {
    #[derive(Debug)]
    struct SkipInferencePlugin;

    #[async_trait::async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let def = AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipInferencePlugin));
    let os = AgentOs::builder()
        .with_agent("a1", def)
        .with_agent_state_store(Arc::new(carve_thread_store_adapters::MemoryStore::new()))
        .build()
        .unwrap();

    let run = os
        .run_stream(RunRequest {
            agent_id: "a1".to_string(),
            thread_id: Some("s2".to_string()),
            run_id: None,
            parent_run_id: None,
            resource_id: None,
            state: Some(json!({})),
            messages: vec![],
        })
        .await
        .unwrap();
    let mut stream = run.events;
    let ev = futures::StreamExt::next(&mut stream).await.unwrap();
    assert!(matches!(ev, AgentEvent::RunStart { .. }));
    let ev = futures::StreamExt::next(&mut stream).await.unwrap();
    assert!(matches!(ev, AgentEvent::RunFinish { .. }));
}

#[test]
fn resolve_sets_runtime_caller_agent_id() {
    let os = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini")
                .with_allowed_skills(vec!["s1".to_string()])
                .with_allowed_agents(vec!["worker".to_string()])
                .with_allowed_tools(vec!["echo".to_string()]),
        )
        .build()
        .unwrap();
    let thread = AgentState::with_initial_state("s", json!({}));
    let (_cfg, _tools, thread) = os.resolve("a1", thread).unwrap();
    assert_eq!(
        thread
            .scope
            .value(SCOPE_CALLER_AGENT_ID_KEY)
            .and_then(|v| v.as_str()),
        Some("a1")
    );
    assert_eq!(
        thread
            .scope
            .value(crate::contracts::runtime::SCOPE_ALLOWED_SKILLS_KEY),
        Some(&json!(["s1"]))
    );
    assert_eq!(
        thread
            .scope
            .value(crate::contracts::runtime::SCOPE_ALLOWED_AGENTS_KEY),
        Some(&json!(["worker"]))
    );
    assert_eq!(
        thread
            .scope
            .value(crate::contracts::runtime::SCOPE_ALLOWED_TOOLS_KEY),
        Some(&json!(["echo"]))
    );
}

#[tokio::test]
async fn resolve_errors_on_skills_tool_id_conflict() {
    #[derive(Debug)]
    struct ConflictingTool;

    #[async_trait::async_trait]
    impl Tool for ConflictingTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("skill", "Conflicting", "Conflicting")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("skill", json!({"ok": true})))
        }
    }

    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .with_tools(HashMap::from([(
            "skill".to_string(),
            Arc::new(ConflictingTool) as Arc<dyn Tool>,
        )]))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::SkillsToolIdConflict(ref id))
        if id == "skill"
    ));
}

#[tokio::test]
async fn resolve_wires_agent_tools_by_default() {
    let os = AgentOs::builder()
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, tools, _thread) = os.resolve("a1", thread).unwrap();
    assert!(tools.contains_key("agent_run"));
    assert!(tools.contains_key("agent_stop"));
    assert_eq!(cfg.plugins[0].id(), "agent_tools");
    assert_eq!(cfg.plugins[1].id(), "agent_recovery");
}

#[tokio::test]
async fn resolve_errors_on_agent_tools_tool_id_conflict() {
    #[derive(Debug)]
    struct ConflictingRunTool;

    #[async_trait::async_trait]
    impl Tool for ConflictingRunTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("agent_run", "Conflicting", "Conflicting")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("agent_run", json!({"ok": true})))
        }
    }

    let os = AgentOs::builder()
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .with_tools(HashMap::from([(
            "agent_run".to_string(),
            Arc::new(ConflictingRunTool) as Arc<dyn Tool>,
        )]))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::AgentToolIdConflict(ref id))
        if id == "agent_run"
    ));
}

#[test]
fn build_errors_if_skills_enabled_without_root() {
    let err = AgentOs::builder()
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .build()
        .unwrap_err();
    assert!(matches!(err, AgentOsBuildError::SkillsNotConfigured));
}

#[test]
fn build_errors_on_duplicate_skill_id_across_skill_registries() {
    let (_td, root) = make_skills_root();
    let skills = FsSkill::into_arc_skills(FsSkill::discover(&root).unwrap().skills);
    let duplicate_registry = InMemorySkillRegistry::from_skills(skills.clone());

    let err = AgentOs::builder()
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .with_skills(skills)
        .with_skill_registry(Arc::new(duplicate_registry) as Arc<dyn SkillRegistry>)
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryOnly,
            discovery_max_entries: 32,
            discovery_max_chars: 8 * 1024,
        })
        .build()
        .unwrap_err();

    assert!(matches!(
        err,
        AgentOsBuildError::SkillRegistry(SkillRegistryError::DuplicateSkillId(ref id))
            if id == "s1"
    ));
}

#[tokio::test]
async fn resolve_errors_if_models_registry_present_but_model_missing() {
    let os = AgentOs::builder()
        .with_provider("p1", Client::default())
        .with_model(
            "m1",
            ModelDefinition::new("p1", "gpt-4o-mini")
                .with_chat_options(genai::chat::ChatOptions::default().with_capture_usage(true)),
        )
        .with_agent("a1", AgentDefinition::new("missing_model_ref"))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(err, AgentOsResolveError::ModelNotFound(ref id) if id == "missing_model_ref"));
}

#[tokio::test]
async fn resolve_rewrites_model_when_registry_present() {
    let os = AgentOs::builder()
        .with_provider("p1", Client::default())
        .with_model("m1", ModelDefinition::new("p1", "gpt-4o-mini"))
        .with_agent("a1", AgentDefinition::new("m1"))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, _tools, _thread) = os.resolve("a1", thread).unwrap();
    assert_eq!(cfg.model, "gpt-4o-mini");
}

#[derive(Debug)]
struct TestPlugin(&'static str);

#[async_trait]
impl AgentPlugin for TestPlugin {
    fn id(&self) -> &str {
        self.0
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &ContextAgentState) {
        if phase == Phase::BeforeInference {
            step.system(format!("<plugin id=\"{}\"/>", self.0));
        }
    }
}

#[tokio::test]
async fn resolve_wires_plugins_from_registry() {
    let doc = json!({});
    let ctx = ContextAgentState::new_transient(&doc, "test", "test");
    let os = AgentOs::builder()
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, _tools, _thread) = os.resolve("a1", thread.clone()).unwrap();
    assert!(cfg.plugins.iter().any(|p| p.id() == "p1"));

    let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
    for p in &cfg.plugins {
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
    }
    assert!(step.system_context.iter().any(|s| s.contains("p1")));
}

#[tokio::test]
async fn resolve_wires_policies_before_plugins() {
    let os = AgentOs::builder()
        .with_registered_plugin("policy1", Arc::new(TestPlugin("policy1")))
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini")
                .with_policy_id("policy1")
                .with_plugin_id("p1"),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, _tools, _thread) = os.resolve("a1", thread).unwrap();
    assert_eq!(cfg.plugins[0].id(), "agent_tools");
    assert_eq!(cfg.plugins[1].id(), "agent_recovery");
    assert_eq!(cfg.plugins[2].id(), "policy1");
    assert_eq!(cfg.plugins[3].id(), "p1");
}

#[tokio::test]
async fn resolve_wires_skills_before_policies_plugins_and_explicit_plugins() {
    let (_td, root) = make_skills_root();
    let os = AgentOs::builder()
        .with_skills(FsSkill::into_arc_skills(
            FsSkill::discover(root).unwrap().skills,
        ))
        .with_skills_config(SkillsConfig {
            mode: SkillsMode::DiscoveryAndRuntime,
            ..SkillsConfig::default()
        })
        .with_registered_plugin("policy1", Arc::new(TestPlugin("policy1")))
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini")
                .with_policy_id("policy1")
                .with_plugin_id("p1")
                .with_plugin(Arc::new(TestPlugin("explicit"))),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let (cfg, tools, _thread) = os.resolve("a1", thread).unwrap();
    assert!(tools.contains_key("skill"));
    assert!(tools.contains_key("load_skill_resource"));
    assert!(tools.contains_key("skill_script"));
    assert!(tools.contains_key("agent_run"));
    assert!(tools.contains_key("agent_stop"));

    assert_eq!(cfg.plugins[0].id(), "skills");
    assert_eq!(cfg.plugins[1].id(), "agent_tools");
    assert_eq!(cfg.plugins[2].id(), "agent_recovery");
    assert_eq!(cfg.plugins[3].id(), "policy1");
    assert_eq!(cfg.plugins[4].id(), "p1");
    assert_eq!(cfg.plugins[5].id(), "explicit");
}

#[test]
fn build_errors_if_builder_agent_references_missing_plugin() {
    let err = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentPluginNotFound { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "p1"
    ));
}

#[test]
fn build_errors_if_builder_agent_references_missing_policy() {
    let err = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_policy_id("policy1"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentPluginNotFound { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "policy1"
    ));
}

#[test]
fn resolve_errors_on_duplicate_plugin_id() {
    let os = AgentOs::builder()
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini")
                .with_plugin_id("p1")
                .with_plugin(Arc::new(TestPlugin("p1"))),
        )
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
    ));
}

#[test]
fn resolve_errors_on_duplicate_plugin_id_between_policy_and_plugin_ref() {
    let os = AgentOs::builder()
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent_registry(Arc::new({
            let mut reg = InMemoryAgentRegistry::new();
            reg.upsert(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_policy_id("p1")
                    .with_plugin_id("p1"),
            );
            reg
        }))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
    ));
}

#[test]
fn build_errors_on_duplicate_plugin_ref_in_builder_agent() {
    let err = AgentOs::builder()
        .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini")
                .with_policy_id("p1")
                .with_plugin_id("p1"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentDuplicatePluginRef { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "p1"
    ));
}

#[test]
fn build_errors_on_reserved_plugin_id_in_builder_agent() {
    let err = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_policy_id("skills"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "skills"
    ));
}

#[test]
fn resolve_errors_on_reserved_plugin_id() {
    let os = AgentOs::builder()
        .with_agent_registry(Arc::new({
            let mut reg = InMemoryAgentRegistry::new();
            reg.upsert(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("skills"),
            );
            reg
        }))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
    ));
}

#[test]
fn resolve_errors_on_reserved_policy_id() {
    let os = AgentOs::builder()
        .with_agent_registry(Arc::new({
            let mut reg = InMemoryAgentRegistry::new();
            reg.upsert(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_policy_id("skills"),
            );
            reg
        }))
        .build()
        .unwrap();

    let thread = AgentState::with_initial_state("s", json!({}));
    let err = os.resolve("a1", thread).err().unwrap();
    assert!(matches!(
        err,
        AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
    ));
}

#[test]
fn build_errors_on_reserved_plugin_id_agent_tools_in_builder_agent() {
    let err = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin_id("agent_tools"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "agent_tools"
    ));
}

#[tokio::test]
async fn run_stream_applies_frontend_state_to_existing_thread() {
    use carve_thread_store_adapters::MemoryStore;
    use futures::StreamExt;

    #[derive(Debug)]
    struct SkipPlugin;

    #[async_trait]
    impl AgentPlugin for SkipPlugin {
        fn id(&self) -> &str {
            "skip"
        }
        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let storage = Arc::new(MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipPlugin)),
        )
        .build()
        .unwrap();

    // Create thread with initial state {"counter": 0}
    let thread = AgentState::with_initial_state("t1", json!({"counter": 0}));
    storage.create(&thread).await.unwrap();

    // Verify initial state
    let head = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(head.agent_state.state, json!({"counter": 0}));

    // Run with frontend state that replaces the thread state
    let request = RunRequest {
        agent_id: "a1".to_string(),
        thread_id: Some("t1".to_string()),
        run_id: Some("run-1".to_string()),
        parent_run_id: None,
        resource_id: None,
        state: Some(json!({"counter": 42, "new_field": true})),
        messages: vec![crate::contracts::state::Message::user("hello")],
    };

    let run_stream = os.run_stream(request).await.unwrap();
    // Drain the stream to completion
    let _events: Vec<_> = run_stream.events.collect().await;

    // Verify state was replaced in storage
    let head = storage.load("t1").await.unwrap().unwrap();
    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(state, json!({"counter": 42, "new_field": true}));
}

#[tokio::test]
async fn run_stream_uses_state_as_initial_for_new_thread() {
    use carve_thread_store_adapters::MemoryStore;
    use futures::StreamExt;

    #[derive(Debug)]
    struct SkipPlugin;

    #[async_trait]
    impl AgentPlugin for SkipPlugin {
        fn id(&self) -> &str {
            "skip"
        }
        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let storage = Arc::new(MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipPlugin)),
        )
        .build()
        .unwrap();

    // Run with state on a new thread
    let request = RunRequest {
        agent_id: "a1".to_string(),
        thread_id: Some("t-new".to_string()),
        run_id: Some("run-1".to_string()),
        parent_run_id: None,
        resource_id: None,
        state: Some(json!({"initial": true})),
        messages: vec![crate::contracts::state::Message::user("hello")],
    };

    let run_stream = os.run_stream(request).await.unwrap();
    let _events: Vec<_> = run_stream.events.collect().await;

    // Verify state was set as initial state
    let head = storage.load("t-new").await.unwrap().unwrap();
    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(state, json!({"initial": true}));
}

#[tokio::test]
async fn run_stream_preserves_state_when_no_frontend_state() {
    use carve_thread_store_adapters::MemoryStore;
    use futures::StreamExt;

    #[derive(Debug)]
    struct SkipPlugin;

    #[async_trait]
    impl AgentPlugin for SkipPlugin {
        fn id(&self) -> &str {
            "skip"
        }
        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let storage = Arc::new(MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipPlugin)),
        )
        .build()
        .unwrap();

    // Create thread with initial state
    let thread = AgentState::with_initial_state("t1", json!({"counter": 5}));
    storage.create(&thread).await.unwrap();

    // Run without frontend state — state should be preserved
    let request = RunRequest {
        agent_id: "a1".to_string(),
        thread_id: Some("t1".to_string()),
        run_id: Some("run-1".to_string()),
        parent_run_id: None,
        resource_id: None,
        state: None,
        messages: vec![crate::contracts::state::Message::user("hello")],
    };

    let run_stream = os.run_stream(request).await.unwrap();
    let _events: Vec<_> = run_stream.events.collect().await;

    // Verify state was not changed
    let head = storage.load("t1").await.unwrap().unwrap();
    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(state, json!({"counter": 5}));
}

#[tokio::test]
async fn prepare_run_sets_identity_and_persists_user_delta_before_execution() {
    use carve_thread_store_adapters::MemoryStore;

    #[derive(Debug)]
    struct SkipPlugin;

    #[async_trait]
    impl AgentPlugin for SkipPlugin {
        fn id(&self) -> &str {
            "skip"
        }
        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let storage = Arc::new(MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipPlugin)),
        )
        .build()
        .unwrap();

    let prepared = os
        .prepare_run(RunRequest {
            agent_id: "a1".to_string(),
            thread_id: Some("t-prepare".to_string()),
            run_id: Some("run-prepare".to_string()),
            parent_run_id: Some("run-parent".to_string()),
            resource_id: None,
            state: Some(json!({"count": 1})),
            messages: vec![crate::contracts::state::Message::user("hello")],
        })
        .await
        .unwrap();

    assert_eq!(prepared.thread_id, "t-prepare");
    assert_eq!(prepared.run_id, "run-prepare");
    assert_eq!(
        prepared.thread.scope.value("run_id"),
        Some(&json!("run-prepare"))
    );
    assert_eq!(
        prepared.thread.scope.value("parent_run_id"),
        Some(&json!("run-parent"))
    );

    let head = storage.load("t-prepare").await.unwrap().unwrap();
    assert_eq!(head.agent_state.messages.len(), 1);
    assert_eq!(
        head.agent_state.messages[0].role,
        crate::contracts::state::Role::User
    );
    assert_eq!(head.agent_state.messages[0].content, "hello");
}

#[tokio::test]
async fn execute_prepared_runs_stream() {
    use carve_thread_store_adapters::MemoryStore;
    use futures::StreamExt;

    #[derive(Debug)]
    struct SkipPlugin;

    #[async_trait]
    impl AgentPlugin for SkipPlugin {
        fn id(&self) -> &str {
            "skip"
        }
        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let storage = Arc::new(MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipPlugin)),
        )
        .build()
        .unwrap();

    let prepared = os
        .prepare_run(RunRequest {
            agent_id: "a1".to_string(),
            thread_id: Some("t-exec-prepared".to_string()),
            run_id: Some("run-exec-prepared".to_string()),
            parent_run_id: None,
            resource_id: None,
            state: None,
            messages: vec![crate::contracts::state::Message::user("hello")],
        })
        .await
        .unwrap();

    let run = AgentOs::execute_prepared(prepared);
    let events: Vec<_> = run.events.collect().await;
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::RunStart { .. })),
        "prepared stream should emit RunStart"
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::RunFinish { .. })),
        "prepared stream should emit RunFinish"
    );
}

#[tokio::test]
async fn run_stream_checkpoint_append_failure_keeps_persisted_prefix_consistent() {
    use futures::StreamExt;

    let storage = Arc::new(FailOnNthAppendStorage::new(2));
    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipWithRunEndPatchPlugin)),
        )
        .build()
        .unwrap();

    let request = RunRequest {
        agent_id: "a1".to_string(),
        thread_id: Some("t-checkpoint-fail".to_string()),
        run_id: Some("run-checkpoint-fail".to_string()),
        parent_run_id: None,
        resource_id: None,
        state: Some(json!({"base": 1})),
        messages: vec![crate::contracts::state::Message::user("hello")],
    };

    let run_stream = os.run_stream(request).await.unwrap();
    let events: Vec<_> = run_stream.events.collect().await;

    assert!(
        matches!(events.first(), Some(AgentEvent::RunStart { .. })),
        "expected RunStart as first event, got: {events:?}"
    );
    let err_msg = events
        .iter()
        .find_map(|ev| match ev {
            AgentEvent::Error { message } => Some(message.clone()),
            _ => None,
        })
        .expect("expected checkpoint append failure to emit AgentEvent::Error");
    assert!(
        err_msg.contains("checkpoint append failed"),
        "unexpected error message: {err_msg}"
    );
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::RunFinish { .. })),
        "RunFinish must not be emitted after checkpoint append failure: {events:?}"
    );

    let head = storage.load("t-checkpoint-fail").await.unwrap().unwrap();
    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(
        state,
        json!({"base": 1}),
        "failed checkpoint must not mutate persisted state"
    );
    assert_eq!(
        head.agent_state.messages.len(),
        1,
        "only user message delta should be persisted before checkpoint failure"
    );
    assert_eq!(
        head.agent_state.messages[0].role,
        crate::contracts::state::Role::User
    );
    assert_eq!(
        head.agent_state.messages[0].content.as_str(),
        "hello",
        "unexpected persisted user message content"
    );
    assert_eq!(head.version, 1, "failed append must not advance version");
    assert_eq!(
        storage.append_call_count(),
        2,
        "expected one successful user append and one failed checkpoint append"
    );
}

#[tokio::test]
async fn run_stream_checkpoint_failure_on_existing_thread_keeps_storage_unchanged() {
    use futures::StreamExt;

    let storage = Arc::new(FailOnNthAppendStorage::new(1));
    let initial = AgentState::with_initial_state("t-existing-fail", json!({"counter": 5}));
    storage.create(&initial).await.unwrap();

    let os = AgentOs::builder()
        .with_agent_state_store(
            storage.clone() as Arc<dyn crate::contracts::storage::AgentStateStore>
        )
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipWithRunEndPatchPlugin)),
        )
        .build()
        .unwrap();

    let request = RunRequest {
        agent_id: "a1".to_string(),
        thread_id: Some("t-existing-fail".to_string()),
        run_id: Some("run-existing-fail".to_string()),
        parent_run_id: None,
        resource_id: None,
        state: None,
        messages: vec![],
    };

    let run_stream = os.run_stream(request).await.unwrap();
    let events: Vec<_> = run_stream.events.collect().await;

    assert!(
        matches!(events.first(), Some(AgentEvent::RunStart { .. })),
        "expected RunStart as first event, got: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::Error { message } if message.contains("checkpoint append failed"))),
        "checkpoint failure must emit AgentEvent::Error: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::RunFinish { .. })),
        "RunFinish must not be emitted after checkpoint append failure: {events:?}"
    );

    let head = storage.load("t-existing-fail").await.unwrap().unwrap();
    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(
        state,
        json!({"counter": 5}),
        "existing state must stay unchanged when first checkpoint append fails"
    );
    assert!(
        head.agent_state.state.get("run_end_marker").is_none(),
        "failed checkpoint must not persist RunEnd patch"
    );
    assert_eq!(head.version, 0, "failed append must not advance version");
    assert_eq!(storage.append_call_count(), 1);
}

#[test]
fn build_errors_on_reserved_plugin_id_agent_recovery_in_builder_agent() {
    let err = AgentOs::builder()
        .with_agent(
            "a1",
            AgentDefinition::new("gpt-4o-mini").with_plugin_id("agent_recovery"),
        )
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
        if agent_id == "a1" && plugin_id == "agent_recovery"
    ));
}

#[test]
fn builder_with_agent_state_store_exposes_accessor() {
    let agent_state_store = Arc::new(carve_thread_store_adapters::MemoryStore::new())
        as Arc<dyn crate::contracts::storage::AgentStateStore>;
    let os = AgentOs::builder()
        .with_agent_state_store(agent_state_store)
        .build()
        .unwrap();
    assert!(os.agent_state_store().is_some());
}

#[tokio::test]
async fn load_agent_state_without_store_returns_not_configured() {
    let os = AgentOs::builder().build().unwrap();
    let err = os.load_agent_state("t1").await.unwrap_err();
    assert!(matches!(err, AgentOsRunError::AgentStateStoreNotConfigured));
}

#[tokio::test]
async fn prepare_run_with_extensions_merges_run_scoped_bundle_tools_and_plugins() {
    struct RuntimeOnlyPlugin;
    #[async_trait::async_trait]
    impl AgentPlugin for RuntimeOnlyPlugin {
        fn id(&self) -> &str {
            "runtime_only"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
            _ctx: &ContextAgentState,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    struct RuntimeOnlyTool;
    #[async_trait::async_trait]
    impl Tool for RuntimeOnlyTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("runtime_tool", "Runtime Tool", "run-scoped tool")
                .with_parameters(json!({"type":"object"}))
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("runtime_tool", json!({"ok": true})))
        }
    }

    let storage = Arc::new(carve_thread_store_adapters::MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(storage)
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .build()
        .unwrap();
    let bundle = ToolPluginBundle::new("runtime_only_bundle")
        .with_tool(Arc::new(RuntimeOnlyTool))
        .with_plugin(Arc::new(RuntimeOnlyPlugin));

    let prepared = os
        .prepare_run_with_extensions(
            RunRequest {
                agent_id: "a1".to_string(),
                thread_id: Some("t-run-ext".to_string()),
                run_id: Some("run-ext".to_string()),
                parent_run_id: None,
                resource_id: None,
                state: None,
                messages: vec![Message::user("hello")],
            },
            RunExtensions::new().with_bundle(Arc::new(bundle)),
        )
        .await
        .unwrap();

    let provider = prepared
        .config
        .step_tool_provider
        .as_ref()
        .expect("runtime extensions should inject step tool provider")
        .clone();
    let snapshot = provider
        .provide(crate::runtime::loop_runner::StepToolInput {
            state: &prepared.thread,
        })
        .await
        .expect("step tool provider should resolve");
    assert!(snapshot.tools.contains_key("runtime_tool"));
    assert!(prepared
        .config
        .plugins
        .iter()
        .any(|plugin| plugin.id() == "runtime_only"));
}

#[tokio::test]
async fn prepare_run_with_extensions_skips_duplicate_tool_from_bundle() {
    struct BaseTool;
    #[async_trait::async_trait]
    impl Tool for BaseTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("dup_tool", "Dup Tool", "base")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("dup_tool", json!({})))
        }
    }

    struct BundleToolConflict;
    #[async_trait::async_trait]
    impl Tool for BundleToolConflict {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("dup_tool", "Dup Tool Runtime Bundle", "runtime")
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("dup_tool", json!({})))
        }
    }

    let storage = Arc::new(carve_thread_store_adapters::MemoryStore::new());
    let os = AgentOs::builder()
        .with_agent_state_store(storage)
        .with_tools(HashMap::from([(
            "dup_tool".to_string(),
            Arc::new(BaseTool) as Arc<dyn Tool>,
        )]))
        .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
        .build()
        .unwrap();
    let bundle =
        ToolPluginBundle::new("runtime_bundle_conflict").with_tool(Arc::new(BundleToolConflict));

    // Run extension tools that duplicate existing tools are silently skipped.
    // The base tool is retained; the extension stub is dropped.
    let _prepared = os
        .prepare_run_with_extensions(
            RunRequest {
                agent_id: "a1".to_string(),
                thread_id: Some("t-run-ext-bundle-conflict".to_string()),
                run_id: Some("run-ext-bundle-conflict".to_string()),
                parent_run_id: None,
                resource_id: None,
                state: None,
                messages: vec![Message::user("hello")],
            },
            RunExtensions::new().with_bundle(Arc::new(bundle)),
        )
        .await
        .expect("run extension with duplicate tool should succeed");
}

#[derive(Debug)]
struct BundleTestTool;

#[async_trait::async_trait]
impl Tool for BundleTestTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("dup_tool", "Duplicate Tool", "bundle test tool")
            .with_parameters(json!({"type":"object"}))
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ContextAgentState,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success("dup_tool", json!({"ok": true})))
    }
}

#[derive(Clone)]
struct ToolConflictBundle;

impl RegistryBundle for ToolConflictBundle {
    fn id(&self) -> &str {
        "tool_conflict_bundle"
    }

    fn tool_definitions(&self) -> HashMap<String, Arc<dyn Tool>> {
        HashMap::from([(
            "dup_tool".to_string(),
            Arc::new(BundleTestTool) as Arc<dyn Tool>,
        )])
    }
}

#[test]
fn builder_fails_fast_on_bundle_registry_conflict() {
    let err = AgentOs::builder()
        .with_tools(HashMap::from([(
            "dup_tool".to_string(),
            Arc::new(BundleTestTool) as Arc<dyn Tool>,
        )]))
        .with_bundle(Arc::new(ToolConflictBundle))
        .build()
        .expect_err("duplicate tool id between base and bundle should fail");

    assert!(matches!(
        err,
        AgentOsBuildError::Bundle(BundleComposeError::DuplicateId {
            bundle_id,
            kind: BundleRegistryKind::Tool,
            id,
        }) if bundle_id == "tool_conflict_bundle" && id == "dup_tool"
    ));
}
