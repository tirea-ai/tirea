# 第一个 Tool

## 目标

实现一个在执行时从 `ToolCallContext` 读取类型化状态的 tool。

> **状态不是必需的。** 很多工具（API 调用、搜索、shell 命令）根本不需要状态；只要实现 `execute` 并返回 `ToolResult` 即可。

## 前置条件

- 先完成[第一个 Agent](./first-agent.md)
- 复用那一页里的运行时依赖

```toml
[dependencies]
awaken = "0.1"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## 1. 定义一个 `StateKey`

`StateKey` 描述状态映射中的一个具名槽位，它声明值类型、更新方式和生命周期作用域。

```rust,ignore
use awaken::{StateKey, KeyScope, MergeStrategy};

/// 追踪问候工具被调用的次数。
struct GreetCount;

impl StateKey for GreetCount {
    const KEY: &'static str = "greet_count";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;
    const SCOPE: KeyScope = KeyScope::Run;

    type Value = u32;
    type Update = u32;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        *value += update;
    }
}
```

关键点：

- `KeyScope::Run`：每次 run 开始时重置；如果想跨 run 保留，就改成 `KeyScope::Thread`
- `MergeStrategy::Commutative`：适合并发安全累加
- `apply`：定义 `Update` 如何作用到当前值

## 2. 实现 Tool

tool 通过 `ctx.state::<GreetCount>()` 读取当前计数，然后返回一个个性化问候。

```rust,ignore
use async_trait::async_trait;
use serde_json::{json, Value};
use awaken::contract::tool::{Tool, ToolDescriptor, ToolResult, ToolOutput, ToolError, ToolCallContext};

struct GreetTool;

#[async_trait]
impl Tool for GreetTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("greet", "Greet", "Greet a user by name")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name to greet" }
                },
                "required": ["name"]
            }))
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        args["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("name is required".into()))?;
        Ok(())
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext,
    ) -> Result<ToolOutput, ToolError> {
        let name = args["name"].as_str().unwrap_or("world");

        // 读取状态 -- 若该 key 尚未写入则返回 None。
        let count = ctx.state::<GreetCount>().copied().unwrap_or(0);

        Ok(ToolResult::success("greet", json!({
            "greeting": format!("Hello, {}!", name),
            "times_greeted": count,
        })).into())
    }
}
```

## 3. 注册 Tool

```rust,ignore
use std::sync::Arc;
use awaken::engine::GenaiExecutor;
use awaken::registry_spec::{AgentSpec, ModelSpec};
use awaken::AgentRuntimeBuilder;

let agent_spec = AgentSpec::new("assistant")
    .with_model("gpt-4o-mini")
    .with_system_prompt("You are a helpful assistant. Use the greet tool when asked.")
    .with_max_rounds(5);

let runtime = AgentRuntimeBuilder::new()
    .with_provider("openai", Arc::new(GenaiExecutor::new()))
    .with_model(
        "gpt-4o-mini",
        ModelSpec {
            id: "gpt-4o-mini".into(),
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
        },
    )
    .with_agent_spec(agent_spec)
    .with_tool("greet", Arc::new(GreetTool))
    .build()?;
```

## 4. 运行

```rust,ignore
use awaken::contract::message::Message;
use awaken::contract::event_sink::VecEventSink;
use awaken::RunRequest;

let request = RunRequest::new(
    "thread-1",
    vec![Message::user("Greet Alice")],
)
.with_agent_id("assistant");

let sink = Arc::new(VecEventSink::new());
runtime.run(request, sink.clone()).await?;
```

## 5. 验证

检查事件流里是否出现了 `ToolCallDone`：

```rust,ignore
use awaken::contract::event::AgentEvent;

let events = sink.take();
let tool_done = events.iter().any(|e| matches!(
    e,
    AgentEvent::ToolCallDone { id: _, message_id: _, result: _, outcome: _ }
));
println!("tool_call_done_seen: {}", tool_done);
```

预期：

- `tool_call_done_seen: true`
- `ToolCallDone.result` 中包含 `greeting` 和 `times_greeted`

## 你刚刚创建了什么

你完成了一个 tool，它：

1. 通过 `descriptor()` 声明 JSON Schema
2. 通过 `validate_args()` 做参数校验
3. 通过 `ctx.state::<K>()` 读取类型化状态
4. 通过 `ToolResult::success()` 返回结构化 JSON

## 下一步读什么

- 完整理解 tool 生命周期：[Tool Trait](../reference/tool-trait.md)
- 学习如何在 phase 边界扩展行为：[Add a Plugin](../how-to/add-a-plugin.md)
- 学习状态作用域：[State Keys](../reference/state-keys.md)

## 常见错误

- `ctx.state::<K>()` 返回 `None`：这个 key 在当前 run 里还没写入
- `StateError::KeyEncode` / `KeyDecode`：`Value` 类型不能正确序列化 / 反序列化
- `ToolError::InvalidArguments` 没有出现：通常是你跳过了 `validate_args()` 逻辑
- 作用域不匹配：如果希望跨多次 run 保持状态，需要用 `KeyScope::Thread`

