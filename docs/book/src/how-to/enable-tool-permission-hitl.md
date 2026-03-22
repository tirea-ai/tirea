# Enable Tool Permission HITL

Use this when tool calls must be `allow` / `deny` / `ask` with human approval.

## Prerequisites

- `tirea-extension-permission` is enabled.
- Frontend can return approval decisions to run inputs.

## Steps

1. Register permission behaviors.

```rust,ignore
use std::sync::Arc;
use tirea::composition::{AgentDefinition, AgentDefinitionSpec, AgentOsBuilder};
use tirea_extension_permission::{PermissionPlugin, ToolPolicyPlugin};

let os = AgentOsBuilder::new()
    .with_registered_behavior("tool_policy", Arc::new(ToolPolicyPlugin))
    .with_registered_behavior("permission", Arc::new(PermissionPlugin))
    .with_agent_spec(AgentDefinitionSpec::local_with_id(
        "assistant",
        AgentDefinition::new("deepseek-chat").with_behavior_ids(vec![
            "tool_policy".to_string(),
            "permission".to_string(),
        ]),
    ))
    .build()?;
```

2. Configure permission policy state.

```rust,ignore
use tirea_extension_permission::{
    permission_state_action, PermissionAction, ToolPermissionBehavior,
};

let set_default = permission_state_action(PermissionAction::SetDefault {
    behavior: ToolPermissionBehavior::Ask,
});

let allow_server_info = permission_state_action(PermissionAction::SetTool {
    tool_id: "serverInfo".to_string(),
    behavior: ToolPermissionBehavior::Allow,
});
```

3. Optional: constrain tools per agent via `AgentDefinition`.

```rust,ignore
AgentDefinition::new("deepseek-chat")
    .with_allowed_tools(vec!["search".to_string()])
    .with_excluded_tools(vec!["dangerous_tool".to_string()])
```

These populate `RunPolicy.allowed_tools` / `RunPolicy.excluded_tools`, which are enforced by `ToolPolicyPlugin` before tool execution.

4. Forward approval decisions from client to active run.

```bash
curl -X POST \
  -H 'content-type: application/json' \
  -d '{"decisions":[{"target_id":"fc_call_1","decision_id":"d1","action":"resume","result":{"approved":true},"updated_at":1760000000000}]}' \
  http://127.0.0.1:8080/v1/runs/<run_id>/inputs
```

## Verify

- `allow`: tool executes immediately.
- `deny`: tool execution is rejected by policy.
- `ask`: run suspends until decision is forwarded.

## Common Errors

- Registering plugin but forgetting to include behavior ids in agent definition.
- Wrong behavior order (`permission` before `tool_policy`) makes out-of-scope checks less strict.
- Missing `target_id` / malformed decisions prevents resume.

## Related Example

- `examples/copilotkit-starter/README.md` is the most complete approval-focused frontend integration
- `examples/travel-ui/README.md` shows approval-gated trip creation
- `examples/research-ui/README.md` shows approval-gated resource deletion

## Key Files

- `crates/tirea-extension-permission/src/plugin.rs`
- `examples/src/starter_backend/mod.rs`
- `examples/ai-sdk-starter/src/components/tools/permission-dialog.tsx`
- `examples/travel-ui/hooks/useTripApproval.tsx`
- `examples/research-ui/hooks/useDeleteApproval.tsx`

## Related

- [Run API](../reference/run-api.md)
- [Frontend Interaction and Approval Model](../explanation/frontend-interaction-and-approval-model.md)
- [AG-UI Protocol](../reference/protocols/ag-ui.md)
- [AI SDK v6 Protocol](../reference/protocols/ai-sdk-v6.md)
