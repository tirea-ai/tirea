# Operations

Operations (`Op`) are the atomic units of state change. Each operation targets a specific path in the JSON document.

## Operation Types

### `Set`

Set a value at the given path. Creates intermediate objects if they don't exist.

```rust
use tirea_state::{Op, path};
use serde_json::json;

let op = Op::set(path!("user", "name"), json!("Alice"));
```

**Error**: `IndexOutOfBounds` if an array index in the path is out of range.

### `Delete`

Remove the value at the given path. No-op if the path doesn't exist.

```rust
use tirea_state::{Op, path};

let op = Op::delete(path!("user", "temp_field"));
```

### `Append`

Append a value to an array. Creates the array if the path doesn't exist.

```rust
use tirea_state::{Op, path};
use serde_json::json;

let op = Op::append(path!("user", "roles"), json!("admin"));
```

**Error**: `AppendRequiresArray` if the target exists but is not an array.

### `MergeObject`

Merge key-value pairs into an existing object. Creates the object if it doesn't exist.

```rust
use tirea_state::{Op, path};
use serde_json::json;

let op = Op::merge_object(path!("user", "settings"), json!({"theme": "dark"}));
```

**Error**: `MergeRequiresObject` if the target exists but is not an object.

### `Increment`

Add to a numeric value.

```rust
use tirea_state::{Op, path};

let op = Op::increment(path!("counter"), 1i64);
```

**Error**: `NumericOperationOnNonNumber` if the target is not a number.

### `Decrement`

Subtract from a numeric value.

```rust
use tirea_state::{Op, path};

let op = Op::decrement(path!("counter"), 1i64);
```

**Error**: `NumericOperationOnNonNumber` if the target is not a number.

### `Insert`

Insert a value at a specific index in an array, shifting elements to the right.

```rust
use tirea_state::{Op, path};
use serde_json::json;

let op = Op::insert(path!("items"), 0, json!("first"));
```

**Error**: `IndexOutOfBounds` if the index exceeds the array length.

### `Remove`

Remove the first occurrence of a value from an array. No-op if the value is not found.

```rust
use tirea_state::{Op, path};
use serde_json::json;

let op = Op::remove(path!("tags"), json!("deprecated"));
```

## Number Type

Numeric operations use the `Number` enum:

```rust,ignore
pub enum Number {
    Int(i64),
    Float(f64),
}
```

Conversions from `i32`, `i64`, `u32`, `u64`, `f32`, `f64` are provided via `From` implementations. Use `as_i64()` and `as_f64()` for access.

## Paths

The `path!` macro creates paths from segments:

```rust
use tirea_state::path;

let p = path!("users", 0, "name");    // users[0].name
let p = path!("settings", "theme");    // settings.theme
```

Path segments can be strings (object keys) or integers (array indices).

## Serialization

Operations serialize to JSON with a `"op"` discriminator:

```json
{"op": "set", "path": ["user", "name"], "value": "Alice"}
{"op": "increment", "path": ["counter"], "amount": 1}
{"op": "delete", "path": ["temp"]}
```
