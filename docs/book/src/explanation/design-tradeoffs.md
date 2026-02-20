# Design Tradeoffs

## Immutable Patch History vs In-Place Mutation

- Chosen: immutable patch history.
- Benefit: replayability, auditability, deterministic reasoning.
- Cost: larger history and snapshot management requirements.

## Unified Event Stream vs Protocol-Specific Runtimes

- Chosen: one internal `AgentEvent` stream plus protocol encoders.
- Benefit: one runtime behavior, many transports.
- Cost: protocol adapters must map event details carefully.

## AgentOs Orchestration Layer

- Chosen: explicit resolve/prepare/execute split.
- Benefit: testable and composable pre-run wiring.
- Cost: more concepts compared with direct loop calls.
