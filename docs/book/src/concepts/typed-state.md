# Typed State & Derive Macro

While `JsonWriter` provides dynamic JSON access, the `#[derive(State)]` macro gives you **type-safe** state access with automatic patch collection.

## How It Works

Given a struct:

```rust,ignore
use carve_state::State;
use carve_state_derive::State;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, State)]
struct Counter {
    value: i64,
    label: String,
}
```

The derive macro generates:

1. **`CounterRef<'a>`** — A typed reference type with getter and setter methods
2. **`impl State for Counter`** — Trait implementation for creating refs from a `Context`

## Using Typed State

In a tool implementation, access state through `Context`:

```rust,ignore
async fn execute(&self, ctx: &Context<'_>) -> Result<()> {
    let counter = ctx.state::<Counter>("counters.main");

    // Read current values
    let current = counter.value()?;
    let label = counter.label()?;

    // Write (patches are automatically collected)
    counter.set_value(current + 1);
    counter.set_label("Updated");

    // Numeric convenience
    counter.increment_value(1);

    Ok(())
}
```

After the tool executes, the framework calls `ctx.take_patch()` to collect all operations into a `Patch`.

## Key Concepts

- **PatchSink** — Internal collector that records operations as you call setters. Transparent to developers.
- **Context** — Provides `state::<T>(path)` method. Holds the current JSON state and a PatchSink.
- **StateRef** — The generated `{Name}Ref<'a>` type. Each field gets:
  - `fn field_name(&self) -> CarveResult<T>` — getter
  - `fn set_field_name(&self, value: T)` — setter
  - `fn increment_field_name(&self, amount)` — for numeric fields

## Field Attributes

| Attribute | Description |
|-----------|-------------|
| `#[carve(rename = "json_name")]` | Use a different name in JSON |
| `#[carve(default = "expr")]` | Default value if field is missing |
| `#[carve(skip)]` | Exclude from state ref (field must impl `Default`) |
| `#[carve(nested)]` | Treat as nested State with its own Ref type |
| `#[carve(flatten)]` | Flatten nested struct fields into parent |

## Nested State

Use `#[carve(nested)]` for struct fields that should have their own typed ref:

```rust,ignore
#[derive(State)]
struct User {
    name: String,
    #[carve(nested)]
    profile: Profile,
}

#[derive(State)]
struct Profile {
    bio: String,
    avatar_url: String,
}
```

This allows `user.profile().bio()?` with full type safety.
