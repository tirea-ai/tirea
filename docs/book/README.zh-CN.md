# Awaken

[English](../../README.md) | [中文](./README.zh-CN.md)

**高性能 Rust AI Agent 运行时 — 一次构建，多协议服务，插件化扩展。**

定义 Agent 的身份和工具，让 LLM 自主编排一切，通过单一二进制文件同时提供多协议服务。通过可组合的插件体系扩展 Agent 行为，享受编译时安全保障。

> **状态：** Awaken 正在积极开发中。契约类型已稳定；运行时和服务器 API 仍可能调整。详见[项目状态](#项目状态)。

> **注意：** Awaken 是 [tirea](../../tree/tirea-0.5) 的全新重写版本，专为简洁性和生产可靠性而设计。tirea 0.5 的代码已归档在 [`tirea-0.5`](../../tree/tirea-0.5) 分支。Awaken 与 tirea **不兼容** — 请参阅[迁移指南](./docs/book/src/appendix/migration-from-tirea.md)。

## 30 秒速览

1. **工具 (Tools)** — 类型化的函数，编译时自动生成 JSON Schema
2. **Agent** — 每个 Agent 拥有系统提示词、模型和允许的工具集；LLM 通过自然语言驱动编排 — 无需预定义流程图
3. **状态 (State)** — 类型化，并按 `thread` / `run` 作用域管理，支持合并策略和不可变快照
4. **插件 (Plugins)** — 8 阶段生命周期钩子，覆盖权限、可观测性、上下文管理、技能、MCP 等

Agent 选择工具、调用工具、读写状态，如此循环 — 全部由运行时通过 8 个类型化阶段编排。每次状态变更都在 gather 阶段后原子提交。

## 快速开始

```bash
git clone https://github.com/AwakenWorks/awaken.git
cd awaken
cargo install lefthook && lefthook install   # git hooks（格式化、lint、密钥检查）
cargo build --workspace
```

> **前置条件**：Rust 工具链 `1.93.0`（由 `rust-toolchain.toml` 固定；workspace `rust-version` 元数据为 `1.85`），以及一个 LLM 提供商 API Key（OpenAI、Anthropic 或 DeepSeek）。

## 使用方式

### 定义工具

```rust
use awaken::prelude::*;
use awaken::contract::tool::ToolOutput;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    query: String,
    max_results: Option<usize>,
}

struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("search", "search", "Search the web for information")
            .with_parameters(schemars::schema_for!(SearchArgs).into())
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext) -> Result<ToolOutput, ToolError> {
        let args: SearchArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        // ... 调用搜索 API ...
        Ok(ToolResult::success("search", json!({
            "results": [{"title": "Rust", "url": "https://rust-lang.org"}]
        })).into())
    }
}
```

### 组装 Agent 到运行时

```rust
use awaken::prelude::*;
use awaken::engine::GenaiExecutor;
use awaken::stores::InMemoryStore;
use std::sync::Arc;

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
    .with_tool("search", Arc::new(SearchTool))
    .with_agent_spec(
        AgentSpec::new("researcher")
            .with_model("gpt-4o-mini")
            .with_system_prompt("You research destinations and provide summaries.")
            .with_max_rounds(4),
    )
    .with_thread_run_store(Arc::new(InMemoryStore::new()))
    .build()?;
```

> **注意：** `with_provider()` 和 `with_model()` 负责接好 LLM 执行器；`with_plugin()` 注册插件；`AgentSpec` 上的 `.with_hook_filter()` 为该 Agent 激活该插件。详见[插件激活模型](#插件激活模型)。

### 连接任意前端

启动服务器后，无需修改代码即可从 React、Next.js 或其他 Agent 接入：

```rust
use awaken::prelude::*;
use awaken::stores::{InMemoryMailboxStore, InMemoryStore};
use std::sync::Arc;

let store = Arc::new(InMemoryStore::new());
let runtime = Arc::new(runtime);
let mailbox = Arc::new(Mailbox::new(
    runtime.clone(),
    Arc::new(InMemoryMailboxStore::new()),
    "default-consumer".into(),
    MailboxConfig::default(),
));

let state = AppState::new(
    runtime.clone(),
    mailbox,
    store,
    runtime.resolver_arc(),
    ServerConfig::default(),
);
serve(state).await?;
```

#### 前端协议

| 协议 | 端点 | 前端 |
|---|---|---|
| AI SDK v6 | `POST /v1/ai-sdk/chat` | React `useChat()` |
| AG-UI | `POST /v1/ag-ui/run` | CopilotKit `<CopilotKit>` |
| A2A | `POST /v1/a2a/tasks/send` | 其他 Agent |

#### 原生 HTTP API

| 端点 | 说明 |
|---|---|
| `POST /v1/runs` | 启动新的 run |
| `GET /v1/threads` | 列出线程 |
| `GET /v1/runs/:id` | 获取 run 状态 |

#### MCP

| 端点 | 状态 |
|---|---|
| `POST /v1/mcp` | JSON-RPC 2.0 请求/响应 |
| `GET /v1/mcp` | SSE — 尚未实现（返回 405） |

#### 运维

| 端点 | 说明 |
|---|---|
| `GET /health` · `GET /health/live` | Kubernetes 探针 |
| `GET /metrics` | Prometheus 指标 |

**React + AI SDK v6：**

```typescript
import { useChat } from "ai/react";

const { messages, input, handleSubmit } = useChat({
  api: "http://localhost:3000/v1/ai-sdk/chat",
});
```

**Next.js + CopilotKit：**

```typescript
import { CopilotKit } from "@copilotkit/react-core";

<CopilotKit runtimeUrl="http://localhost:3000/v1/ag-ui/run">
  <YourApp />
</CopilotKit>
```

## 内置插件

Awaken 通过基于 8 个类型化生命周期阶段的可组合插件体系扩展 Agent 能力。

| 插件 | 功能说明 | Feature Flag |
|---|---|---|
| **Permission** | 防火墙式工具访问控制：按优先级评估 Deny > Allow > Ask 规则，支持 glob/正则匹配工具名和参数。Ask 策略通过邮箱触发 human-in-the-loop 暂停。 | `permission` |
| **Reminder** | 基于规则的上下文注入：当工具调用匹配指定模式和输出条件时，自动向对话注入 system/session/conversation 级别的上下文消息，支持冷却周期防止重复。 | `reminder` |
| **Observability** | 符合 OpenTelemetry GenAI 语义规范的遥测采集：按推理和工具粒度记录指标，支持内存、文件和 OTLP 导出。 | `observability` |
| **MCP** | Model Context Protocol 客户端：连接外部 MCP 服务器，自动发现并注册其工具为 Awaken 原生工具。 | `mcp` |
| **Skills** | 技能包发现与激活：支持文件系统和编译时嵌入两种技能源，推理前自动注入技能目录供 LLM 按需选择和激活。 | `skills` |
| **Generative UI** | 服务端驱动的 UI 渲染：通过 A2UI 协议向前端流式推送声明式 UI 组件，无需前端改动即可实现富交互体验。 | `generative-ui` |

`awaken-ext-deferred-tools` crate 提供基于概率模型的延迟工具加载，但未包含在 `awaken` 门面 crate 中 — 如需使用请直接添加依赖。

### 插件激活模型

启用一个插件需要三个步骤：

1. **Cargo feature flag** — 控制扩展 crate 是否编译进二进制文件（默认通过 `full` feature 全部启用）
2. **Builder 上的 `.with_plugin(id, plugin)`** — 将插件注册到运行时的插件注册表
3. **AgentSpec 上的 `.with_hook_filter(id)`** — 为特定 Agent 激活该插件的钩子

这意味着不同的 Agent 可以使用不同的已注册插件子集。已注册但未被任何 Agent 的 hook filter 引用的插件不会执行。

### Reminder：配置驱动的上下文注入

Reminder 插件是 Awaken 通过配置而非代码管理 Agent 行为的关键机制。它允许你定义声明式规则，根据工具执行模式自动向对话注入上下文消息。

每条规则由三部分组成：

- **触发模式** — 匹配工具名和参数（如 `"Bash"`、`"Edit(file_path ~ '*.toml')"`）
- **输出条件** — 匹配执行结果（success/error/any，或内容模式匹配）
- **注入消息** — 指定级别（system、suffix_system、session 或 conversation）的上下文提示

例如：当 Agent 执行 `Bash` 命令返回错误时，自动注入"检查命令是否需要 sudo 权限"的 system 提示。这意味着 Agent 的行为策略完全由配置文件管理，而非硬编码在 prompt 中 — 便于迭代、版本控制和跨 Agent 共享。

## 为什么选择 Awaken

| 你获得的 | 实现方式 |
|---|---|
| **一个后端服务所有前端** | 从同一个二进制文件提供 React（AI SDK v6）、Next.js（AG-UI）、其他 Agent（A2A）和工具服务器（MCP）。无需分别部署。 |
| **LLM 编排一切 — 无需 DAG** | 定义每个 Agent 的身份和工具访问权限；LLM 决定何时委托、委托给谁、如何组合结果。无需手写流程图或状态机。 |
| **可组合的插件体系** | 8 个类型化生命周期阶段。权限、上下文注入、可观测性、工具发现 — 全部声明式配置。Phase hook 是类型安全的（`PhaseHook` trait），插件注册 API 在构建时捕获配置错误。 |
| **类型安全的状态与回放** | 状态是带编译时检查的 Rust 结构体。合并策略处理并发写入，无需锁。作用域限定为 thread 或 run。每次变更都是可回放的不可变快照。 |
| **内置生产韧性** | 熔断器、指数退避、推理超时、优雅关闭、Prometheus 指标和健康探针 — 开箱即用。 |
| **零 `unsafe` 代码** | 整个工作空间禁止 `unsafe`。内存安全由 Rust 编译器保证。 |

## 与其他框架的定位差异

与其维护一张很快过时的大而全功能矩阵，不如直接说明各框架的主要偏向：

| 框架 | 更擅长什么 | 与 Awaken 的差异 |
|---|---|---|
| **LangGraph** | Python/TS 图式编排、持久化、memory、handoff | 如果你需要 Python 里的 graph-first 编排，LangGraph 更合适。Awaken 更适合需要 Rust 运行时、类型化状态和内建多协议服务端的场景。 |
| **AG2** | Python 多智能体对话、group chat、协议集成 | 如果团队希望用 Python 构建对话式多智能体系统，AG2 更自然。Awaken 更强调明确的运行时分层、类型化状态和服务端控制面。 |
| **CrewAI** | Python `Crews + Flows` 工作流与运维能力 | 如果你需要 workflow-first 的 Python 编排，CrewAI 更合适。Awaken 更偏向由 LLM 驱动工具编排，并把运行时职责留在 Rust 后端。 |
| **OpenAI Agents SDK** | OpenAI-first 的 hosted tools、sessions、MCP、guardrails | 如果你想要 OpenAI 管理的 batteries-included 工具链，它更合适。Awaken 更适合 provider-neutral、自管存储和自管服务面的场景。 |
| **Mastra** | TypeScript 全栈 Agent 应用、AI SDK、MCP、memory、前端集成 | 如果你的技术栈是端到端 TypeScript，Mastra 更自然。Awaken 更适合后端必须是 Rust 的系统。 |

## 适用场景

- 需要 **Rust 后端**构建 AI Agent，享受编译时安全
- 需要从一个后端同时提供**多种前端或 Agent 协议**
- 工具需要在并发执行中**安全共享状态**
- 需要**可审计的线程历史**、checkpoint 与可恢复控制路径
- 能接受自己注册工具、provider 与 model registry，而不是依赖开箱即用的默认能力

## 不适用场景

- 需要**开箱即用的文件/Shell/Web 工具** — 可优先考虑 OpenAI Agents SDK、Dify、CrewAI
- 需要**可视化工作流编辑器** — 考虑 Dify、LangGraph Studio
- 需要 **Python** 快速原型开发 — 考虑 LangGraph、AG2、PydanticAI
- 需要一个**稳定且变化缓慢**的表面 API，而不是持续演进的运行时平台
- 需要 **LLM 自主管理的记忆**（Agent 自行决定记住什么）— 考虑 Letta

### 跨对话状态管理

状态是类型化的，并限定到其预期的生命周期：

```rust
use awaken::prelude::*;

pub struct UserPreferences;  // 在此 thread 的所有 run 中持久化

impl StateKey for UserPreferences {
    const KEY: &'static str = "user.preferences";
    const MERGE: MergeStrategy = MergeStrategy::Exclusive;
    type Value = PreferencesValue;
    type Update = PreferencesValue;

    fn apply(value: &mut PreferencesValue, update: PreferencesValue) {
        *value = update;
    }
}
```

| 作用域 | 生命周期 | 用途 |
|---|---|---|
| `Thread` | 跨所有 run 持久化 | 用户偏好、对话记忆 |
| `Run`（默认） | 每次 run 开始时重置 | 搜索进度、工具调用追踪 |

### 对话持久化

无需修改 Agent 代码即可切换存储后端：

| 后端 | 用途 |
|---|---|
| `InMemoryStore` | 测试和临时演示 |
| `FileStore` | 本地开发、单服务器部署 |
| `PostgresStore` | 生产环境，支持 SQL 查询和备份 |

## 架构

Awaken 由三层运行时组成。`awaken-contract` 定义共享契约：Agent 规格、model/provider 规格、工具、事件、传输 trait，以及类型化状态模型。`awaken-runtime` 负责把 `AgentSpec` 解析成 `ResolvedAgent`，从插件构建 `ExecutionEnv`，执行 phase loop，并管理运行中 run 及其取消、HITL 决策等控制路径。`awaken-server` 则把同一个 runtime 暴露成 HTTP 路由、SSE 回放、mailbox 后台执行，以及 AI SDK v6、AG-UI、A2A、MCP 协议适配器。

围绕这三层的是存储和扩展。`awaken-stores` 提供线程与 run 的内存、文件、PostgreSQL 后端。`awaken-ext-*` crates 在 phase 和 tool 边界扩展运行时能力，例如权限、可观测性、MCP 工具发现、skills、reminder、generative UI 和 deferred tools。

### 仓库结构图

```text
awaken                   门面 crate，管理 feature flags
├─ awaken-contract       契约：spec、tool、event、transport、state model
├─ awaken-runtime        resolver、phase engine、loop runner、runtime control
├─ awaken-server         route、mailbox、SSE transport、protocol adapter
├─ awaken-stores         内存、文件、PostgreSQL 持久化
├─ awaken-tool-pattern   扩展使用的 glob/regex 匹配
└─ awaken-ext-*          可选运行时扩展
```

## Feature Flags

所有功能默认启用。使用 `default-features = false` 可按需关闭。

| Feature | 默认 | 说明 |
|---|:---:|---|
| `permission` | 是 | Allow / Deny / Ask 工具策略 |
| `observability` | 是 | OpenTelemetry 追踪，符合 GenAI 语义规范 |
| `mcp` | 是 | Model Context Protocol 客户端 |
| `skills` | 是 | YAML 技能包发现与激活 |
| `reminder` | 是 | 基于规则的上下文消息注入 |
| `generative-ui` | 是 | 服务端驱动的 UI 组件 |
| `server` | 是 | 多协议 HTTP 服务器与邮箱 |
| `full` | 是 | 以上全部 |

workspace 里也可能存在未接入 `awaken` 门面 feature flags 的独立扩展 crate，目前包括 `awaken-ext-deferred-tools`。

> `awaken-ext-deferred-tools` 是独立 crate，未包含在 `full` feature 中。如需延迟工具加载，请直接添加依赖。

## 示例

| 示例 | 展示内容 | 适合 |
|---|---|---|
| [`live_test`](./crates/awaken/examples/live_test.rs) | 基础 LLM 集成 | 基础 |
| [`multi_turn`](./crates/awaken/examples/multi_turn.rs) | 多轮对话与持久化线程 | 对话状态 |
| [`tool_call_live`](./crates/awaken/examples/tool_call_live.rs) | 工具调用（计算器） | 工具集成 |
| [`ai-sdk-starter`](./examples/ai-sdk-starter/) | React + AI SDK v6 全栈 | 前端集成 |
| [`copilotkit-starter`](./examples/copilotkit-starter/) | Next.js + CopilotKit 全栈 | CopilotKit / AG-UI |

```bash
# 运行时示例
export OPENAI_API_KEY=<your-key>
cargo run --package awaken --example multi_turn

# 全栈演示
cd examples/ai-sdk-starter && npm install && npm run dev
```

## 学习路径

| 目标 | 从这里开始 | 然后 |
|---|---|---|
| 构建第一个 Agent | [First Agent 教程](./docs/book/src/tutorials/first-agent.md) | [构建 Agent 指南](./docs/book/src/how-to/build-an-agent.md) |
| 查看全栈应用 | [AI SDK starter](./examples/ai-sdk-starter/) | [CopilotKit starter](./examples/copilotkit-starter/) |
| 探索 API | [参考文档](./docs/book/src/reference/overview.md) | `cargo doc --workspace --no-deps --open` |
| 参与贡献 | [贡献指南](./CONTRIBUTING.md) | [开发环境搭建](./DEVELOPMENT.md) |
| 从 tirea 迁移 | [迁移指南](./docs/book/src/appendix/migration-from-tirea.md) | |

## 文档

| 资源 | 说明 |
|---|---|
| [`docs/book/`](./docs/book/) | 用户指南 — 教程、操作指南、参考、解释 |
| [`docs/adr/`](./docs/adr/) | 22 篇架构决策记录 |
| [`DEVELOPMENT.md`](./DEVELOPMENT.md) | 构建、测试和贡献指南 |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) | 贡献者指南 |

> 英文与中文 book 页面现在使用同一套目录结构。后续只要 API、路由或配置面发生变化，两边都需要同步更新。

## 项目状态

Awaken 正在积极开发中，API 尚未稳定。

| 组件 | 状态 |
|---|---|
| 契约类型 | 稳定 — 不太可能有破坏性变更 |
| 运行时引擎 | 趋于成熟 — API 可能调整 |
| 服务器 / 协议 | 趋于成熟 — AI SDK v6 和 AG-UI 已充分测试 |
| 存储后端 | 稳定（内存、文件）/ 测试阶段（PostgreSQL） |
| 扩展 | 稳定（permission、observability）/ 测试阶段（其他） |

## 参与贡献

请参阅 [CONTRIBUTING.md](./CONTRIBUTING.md)。欢迎贡献，特别是：

- 新增存储后端（Redis、SQLite）
- 内置工具实现（文件读写、Web 搜索）
- Token 用量追踪和预算控制
- 模型降级链

## 许可证

双重许可：[MIT](./LICENSE-MIT) 或 [Apache-2.0](./LICENSE-APACHE)。
