# Tool and Plugin Boundary

Tools and plugins solve different problems. Keep the boundary strict.

## Tool Responsibility

Tools implement domain actions:

- Read/write state through `ToolCallContext`
- Call external systems
- Return structured `ToolResult`

Tools should not orchestrate loop policy globally.

## Plugin Responsibility

Plugins implement cross-cutting policy:

- Inject context (`StepStart`, `BeforeInference`)
- Filter/allow/deny tools (`BeforeInference`, `BeforeToolExecute`)
- Add reminders or execution metadata (`AfterToolExecute`)

Plugins should not own domain-side business operations.

## Rule of Thumb

- If it is business capability, build a tool.
- If it is execution policy or guardrail, build a plugin.
