# Derive Macro Reference

The `#[derive(State)]` macro generates typed state references with getter/setter methods and automatic patch collection.

## Basic Usage

```rust,ignore
use tirea_state::State;
use tirea_state_derive::State;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, State)]
struct User {
    name: String,
    age: i64,
}
```

This generates:

- `UserRef<'a>` — Typed reference with `name()`, `set_name()`, `age()`, `set_age()`, `increment_age()`
- `impl State for User` — Enables `ctx.state::<User>(path)`

## Field Attributes

### `#[tirea(rename = "json_name")]`

Use a different key name in the JSON document:

```rust,ignore
#[derive(State)]
struct Config {
    #[tirea(rename = "display_name")]
    label: String,
}
// Reads/writes JSON key "display_name", Rust field is "label"
```

### `#[tirea(default = "expr")]`

Provide a default value expression when the field is missing from JSON:

```rust,ignore
#[derive(State)]
struct Settings {
    #[tirea(default = "60")]
    timeout_secs: i64,

    #[tirea(default = "\"en\".to_string()")]
    language: String,
}
```

### `#[tirea(skip)]`

Exclude a field from the state ref. The field must implement `Default`:

```rust,ignore
#[derive(State)]
struct Record {
    data: String,
    #[tirea(skip)]
    _internal: Vec<u8>,  // not accessible via RecordRef
}
```

### `#[tirea(nested)]`

Treat a struct field as nested state with its own typed ref:

```rust,ignore
#[derive(State)]
struct User {
    name: String,
    #[tirea(nested)]
    profile: Profile,
}

#[derive(State)]
struct Profile {
    bio: String,
    avatar_url: String,
}

// Usage: user.profile().bio()?
```

Without `#[tirea(nested)]`, the field is treated as a whole JSON value (serialized/deserialized as one unit).

### `#[tirea(flatten)]`

Flatten nested struct fields into the parent JSON object:

```rust,ignore
#[derive(State)]
struct Event {
    name: String,
    #[tirea(flatten)]
    metadata: Metadata,
}

#[derive(State)]
struct Metadata {
    created_at: String,
    updated_at: String,
}

// JSON: {"name": "...", "created_at": "...", "updated_at": "..."}
// No nesting in the JSON document
```

## Generated Methods

For a field `value: i64`, the generated `Ref` type provides:

| Method | Signature | Description |
|--------|-----------|-------------|
| `value()` | `fn value(&self) -> TireaResult<i64>` | Read current value |
| `set_value()` | `fn set_value(&self, v: i64)` | Set value (collects patch) |
| `increment_value()` | `fn increment_value(&self, amount: impl Into<Number>)` | Increment (numeric fields) |

For `String` and other non-numeric types, only `field()` and `set_field()` are generated.
