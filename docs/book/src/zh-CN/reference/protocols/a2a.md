# A2A 协议

A2A 适配层实现了 Google 的 Agent-to-Agent 协议，用于远程 agent 发现、任务委托和 agent 间通信。

**Feature gate**：`server`

## 入口

| 路由 | 方法 | 说明 |
|-------|--------|------|
| `/v1/a2a/tasks/send` | `POST` | 提交任务，返回 task ID 与初始状态 |
| `/v1/a2a/tasks/:task_id` | `GET` | 查询任务状态 |
| `/v1/a2a/tasks/:task_id/cancel` | `POST` | 取消任务 |
| `/v1/a2a/.well-known/agent` | `GET` | 返回默认 agent card |
| `/v1/a2a/agents` | `GET` | 列出所有可发现 agent |
| `/v1/a2a/agents/:agent_id/agent-card` | `GET` | 获取指定 agent 的 card |
| `/v1/a2a/agents/:agent_id/message:send` | `POST` | 向指定 agent 发送消息 |
| `/v1/a2a/agents/:agent_id/tasks/:action` | `GET/POST` | agent 作用域下的任务操作 |

## Agent Card

发现接口会返回 `AgentCard`，描述 agent 的能力、地址和 skills：

```json
{
  "id": "assistant",
  "name": "My Agent",
  "description": "A helpful assistant",
  "url": "https://example.com/v1/a2a",
  "version": "1.0.0",
  "capabilities": {
    "streaming": false,
    "push_notifications": false,
    "state_transition_history": false
  },
  "skills": [
    {
      "id": "general",
      "name": "General Q&A",
      "description": "Answer general questions",
      "tags": ["qa"]
    }
  ]
}
```

Agent card 来自已注册的 `AgentSpec`。

## Task Send

```json
{
  "taskId": "optional-client-provided-id",
  "agentId": "optional-agent-id",
  "message": {
    "role": "user",
    "parts": [{ "type": "text", "text": "Summarize this document" }]
  }
}
```

任务会作为后台 `MailboxJob` 提交，接口立即返回 `taskId` 和 `submitted` 状态。客户端之后轮询 `/v1/a2a/tasks/:task_id`。

## Task Status

```json
{
  "taskId": "abc-123",
  "status": {
    "state": "completed",
    "message": { "role": "agent", "parts": [{ "type": "text", "text": "..." }] }
  }
}
```

状态遵循 A2A 生命周期：`submitted`、`working`、`completed`、`failed`、`cancelled`。

## 远程 agent 委托

Awaken 可以通过 `AgentSpec.endpoint` 把某个 agent 配置为远端 A2A agent。对 LLM 来说，这仍然表现为普通 tool call；A2A 传输层由 `A2aBackend` 隐式处理。

```json
{
  "id": "remote-researcher",
  "endpoint": {
    "base_url": "https://remote-agent.example.com",
    "bearer_token": "...",
    "poll_interval_ms": 1000,
    "timeout_ms": 300000
  }
}
```

带 `endpoint` 的 agent 会被解析为远程 agent；没有 `endpoint` 的 agent 在本地执行。

## 相关

- [多智能体模式](../../explanation/multi-agent-patterns.md)
- [A2A Agent Card 规范](https://google.github.io/A2A/#/documentation?id=agent-card)
