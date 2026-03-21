#![allow(missing_docs)]
//! Integration tests validating dynamic configuration:
//! - Handoff via ActiveProfileOverride (state-driven profile switch)
//! - Hook filtering by active_plugins
//! - Config values accessible in hooks via PhaseContext::config()

use async_trait::async_trait;
use awaken::agent::config::AgentConfig;
use awaken::agent::loop_runner::run_agent_loop;
use awaken::agent::state::{RunLifecycleSlot, ToolCallStatesSlot};
use awaken::config::profile::{AgentProfile, OsConfig};
use awaken::config::resolve::ActiveProfileOverride;
use awaken::config::spec::ConfigSlot;
use awaken::contract::executor::{InferenceExecutionError, InferenceRequest, LlmExecutor};
use awaken::contract::identity::{RunIdentity, RunOrigin};
use awaken::contract::inference::{StopReason, StreamResult};
use awaken::contract::message::Message;
use awaken::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Test config types
// ---------------------------------------------------------------------------

struct ModelNameConfig;
impl ConfigSlot for ModelNameConfig {
    const KEY: &'static str = "test.model_name";
    type Value = ModelNameSettings;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ModelNameSettings {
    name: String,
}

struct GreetingConfig;
impl ConfigSlot for GreetingConfig {
    const KEY: &'static str = "test.greeting";
    type Value = GreetingSettings;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct GreetingSettings {
    prefix: String,
}

// ---------------------------------------------------------------------------
// Mock LLM
// ---------------------------------------------------------------------------

struct SimpleLlm;

#[async_trait]
impl LlmExecutor for SimpleLlm {
    async fn execute(
        &self,
        _req: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        Ok(StreamResult {
            text: "done".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        })
    }

    fn name(&self) -> &str {
        "simple"
    }
}

// ---------------------------------------------------------------------------
// Tracking hooks — record what they see
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct HookLog {
    entries: Arc<Mutex<Vec<HookEntry>>>,
}

#[derive(Debug, Clone)]
struct HookEntry {
    plugin_id: String,
    phase: Phase,
    model_name: String,
    greeting: String,
}

struct TrackingHook {
    plugin_id: String,
    log: HookLog,
}

#[async_trait]
impl PhaseHook for TrackingHook {
    async fn run(&self, ctx: &PhaseContext) -> Result<StateCommand, StateError> {
        self.log.entries.lock().unwrap().push(HookEntry {
            plugin_id: self.plugin_id.clone(),
            phase: ctx.phase,
            model_name: ctx.config::<ModelNameConfig>().name,
            greeting: ctx.config::<GreetingConfig>().prefix,
        });
        Ok(StateCommand::new())
    }
}

// ---------------------------------------------------------------------------
// Handoff hook — writes ActiveProfileOverride to state
// ---------------------------------------------------------------------------

struct HandoffHook {
    target_profile: String,
}

#[async_trait]
impl PhaseHook for HandoffHook {
    async fn run(&self, _ctx: &PhaseContext) -> Result<StateCommand, StateError> {
        let mut cmd = StateCommand::new();
        // Deref to MutationBatch for update access
        cmd.update::<ActiveProfileOverride>(Some(self.target_profile.clone()));
        Ok(cmd)
    }
}

// ---------------------------------------------------------------------------
// Plugin wrappers
// ---------------------------------------------------------------------------

struct LoopStatePlugin;
impl Plugin for LoopStatePlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor { name: "loop-state" }
    }
    fn register(&self, r: &mut PluginRegistrar) -> Result<(), StateError> {
        r.register_slot::<RunLifecycleSlot>(SlotOptions::default())?;
        r.register_slot::<ToolCallStatesSlot>(SlotOptions::default())?;
        r.register_slot::<ActiveProfileOverride>(SlotOptions::default())?;
        Ok(())
    }
}

/// A single plugin that registers hooks for multiple plugin_ids.
/// Avoids TypeId conflicts from multiple instances of the same struct.
struct MultiTrackerPlugin {
    trackers: Vec<(&'static str, Vec<Phase>)>,
    log: HookLog,
}

impl Plugin for MultiTrackerPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: "multi-tracker",
        }
    }
    fn register(&self, r: &mut PluginRegistrar) -> Result<(), StateError> {
        for (id, phases) in &self.trackers {
            for &phase in phases {
                r.register_phase_hook(
                    *id,
                    phase,
                    TrackingHook {
                        plugin_id: (*id).into(),
                        log: self.log.clone(),
                    },
                )?;
            }
        }
        Ok(())
    }
}

/// Single-id tracker for simple cases.
struct SingleTrackerPlugin {
    id: &'static str,
    log: HookLog,
    phases: Vec<Phase>,
}

impl Plugin for SingleTrackerPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor { name: self.id }
    }
    fn register(&self, r: &mut PluginRegistrar) -> Result<(), StateError> {
        for &phase in &self.phases {
            r.register_phase_hook(
                self.id,
                phase,
                TrackingHook {
                    plugin_id: self.id.into(),
                    log: self.log.clone(),
                },
            )?;
        }
        Ok(())
    }
}

struct HandoffPlugin {
    target_profile: String,
}

impl Plugin for HandoffPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor { name: "handoff" }
    }
    fn register(&self, r: &mut PluginRegistrar) -> Result<(), StateError> {
        r.register_phase_hook(
            "handoff",
            Phase::RunStart,
            HandoffHook {
                target_profile: self.target_profile.clone(),
            },
        )?;
        Ok(())
    }
}

fn test_identity() -> RunIdentity {
    RunIdentity::new(
        "t1".into(),
        None,
        "r1".into(),
        None,
        "agent".into(),
        RunOrigin::User,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_filtering_only_active_plugins_fire() {
    let log = HookLog::default();

    let runtime = PhaseRuntime::new(StateStore::new()).unwrap();
    runtime.install_plugin(LoopStatePlugin).unwrap();

    // Install tracker with two plugin_ids
    runtime.install_plugin(MultiTrackerPlugin {
        trackers: vec![
            ("alpha", vec![Phase::BeforeInference]),
            ("beta", vec![Phase::BeforeInference]),
        ],
        log: log.clone(),
    });

    // Only activate "alpha"
    runtime.configure(|c| {
        c.activate("alpha");
    });

    let agent = AgentConfig::new("test", "m", "sys", Arc::new(SimpleLlm));
    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    let entries = log.entries.lock().unwrap();
    let before_inf: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::BeforeInference)
        .collect();

    assert_eq!(before_inf.len(), 1);
    assert_eq!(before_inf[0].plugin_id, "alpha");
}

#[tokio::test]
async fn empty_active_plugins_runs_all_hooks() {
    let log = HookLog::default();

    let runtime = PhaseRuntime::new(StateStore::new()).unwrap();
    runtime.install_plugin(LoopStatePlugin).unwrap();

    runtime.install_plugin(MultiTrackerPlugin {
        trackers: vec![
            ("alpha", vec![Phase::BeforeInference]),
            ("beta", vec![Phase::BeforeInference]),
        ],
        log: log.clone(),
    });

    // Don't configure active_plugins — empty means "run all"
    let agent = AgentConfig::new("test", "m", "sys", Arc::new(SimpleLlm));
    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    let entries = log.entries.lock().unwrap();
    let before_inf: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::BeforeInference)
        .collect();

    assert_eq!(before_inf.len(), 2); // Both alpha and beta fire
}

#[tokio::test]
async fn config_values_accessible_in_hooks() {
    let log = HookLog::default();

    let mut os = OsConfig::new();
    os.set_default::<ModelNameConfig>(ModelNameSettings {
        name: "default-model".into(),
    });
    os.set_default::<GreetingConfig>(GreetingSettings {
        prefix: "Hello".into(),
    });

    let runtime = PhaseRuntime::new(StateStore::new()).unwrap();
    runtime.set_os_config(os);
    runtime.install_plugin(LoopStatePlugin).unwrap();

    runtime.install_plugin(SingleTrackerPlugin {
        id: "tracker",
        log: log.clone(),
        phases: vec![Phase::BeforeInference],
    });

    // Override model via ActiveConfig
    runtime.configure(|c| {
        c.set::<ModelNameConfig>(ModelNameSettings {
            name: "custom-model".into(),
        });
    });

    let agent = AgentConfig::new("test", "m", "sys", Arc::new(SimpleLlm));
    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    let entries = log.entries.lock().unwrap();
    let entry = entries
        .iter()
        .find(|e| e.phase == Phase::BeforeInference)
        .unwrap();

    // ModelName overridden by ActiveConfig
    assert_eq!(entry.model_name, "custom-model");
    // Greeting comes from OS default
    assert_eq!(entry.greeting, "Hello");
}

#[tokio::test]
async fn handoff_switches_profile_at_next_boundary() {
    let log = HookLog::default();

    let mut os = OsConfig::new();
    os.set_default::<ModelNameConfig>(ModelNameSettings {
        name: "base-model".into(),
    });
    os.register_profile(
        AgentProfile::new("reviewer")
            .activate("review-tracker")
            .configure::<ModelNameConfig>(ModelNameSettings {
                name: "reviewer-model".into(),
            }),
    );

    let runtime = PhaseRuntime::new(StateStore::new()).unwrap();
    runtime.set_os_config(os);
    runtime.install_plugin(LoopStatePlugin).unwrap();

    // Handoff plugin writes ActiveProfileOverride at RunStart
    runtime.install_plugin(HandoffPlugin {
        target_profile: "reviewer".into(),
    });

    // Tracker registered as "review-tracker" — only active when reviewer profile is active
    runtime.install_plugin(SingleTrackerPlugin {
        id: "review-tracker",
        log: log.clone(),
        phases: vec![Phase::BeforeInference],
    });

    // Activate handoff plugin (so its hook runs at RunStart)
    runtime.configure(|c| {
        c.activate("handoff");
        // Don't activate "review-tracker" — it should be activated by profile switch
    });

    let agent = AgentConfig::new("test", "m", "sys", Arc::new(SimpleLlm));
    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    // After RunStart, handoff wrote ActiveProfileOverride = "reviewer"
    // At BeforeInference boundary, resolve picks up the profile
    // "review-tracker" should now be active and see "reviewer-model"
    let entries = log.entries.lock().unwrap();
    let before_inf: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::BeforeInference)
        .collect();

    assert_eq!(before_inf.len(), 1);
    assert_eq!(before_inf[0].plugin_id, "review-tracker");
    // Config comes from profile (reviewer-model) since ActiveConfig didn't override it
    assert_eq!(before_inf[0].model_name, "reviewer-model");
}

#[tokio::test]
async fn deactivate_plugin_mid_run_via_configure() {
    let log = HookLog::default();

    let runtime = PhaseRuntime::new(StateStore::new()).unwrap();
    runtime.install_plugin(LoopStatePlugin).unwrap();

    runtime.install_plugin(SingleTrackerPlugin {
        id: "tracker",
        log: log.clone(),
        phases: vec![Phase::RunStart, Phase::BeforeInference, Phase::RunEnd],
    });

    // Activate tracker
    runtime.configure(|c| {
        c.activate("tracker");
    });

    // Run phase manually to show tracker fires
    runtime.run_phase(Phase::RunStart).await.unwrap();

    {
        let entries = log.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].phase, Phase::RunStart);
    }

    // Deactivate tracker, keep a dummy active so the set isn't empty
    // (empty active_plugins = no filtering = all hooks run)
    runtime.configure(|c| {
        c.deactivate("tracker");
        c.activate("other-plugin");
    });

    // Now BeforeInference should NOT fire tracker (it's not in active_plugins)
    runtime.run_phase(Phase::BeforeInference).await.unwrap();

    {
        let entries = log.entries.lock().unwrap();
        assert_eq!(entries.len(), 1); // Still just the RunStart entry
    }
}
