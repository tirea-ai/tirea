# Error Handling

`tirea-state` uses the `thiserror` crate with a unified `TireaError` enum.

## TireaError

```rust,ignore
pub enum TireaError {
    PathNotFound { path: Path },
    IndexOutOfBounds { path: Path, index: usize, len: usize },
    TypeMismatch { path: Path, expected: &'static str, found: &'static str },
    NumericOperationOnNonNumber { path: Path },
    MergeRequiresObject { path: Path },
    AppendRequiresArray { path: Path },
    InvalidOperation { message: String },
    Serialization(serde_json::Error),
}
```

### Variants

| Variant | When It Occurs |
|---------|---------------|
| `PathNotFound` | Accessing a path that doesn't exist in the document |
| `IndexOutOfBounds` | Array index exceeds array length |
| `TypeMismatch` | Expected one type, found another (e.g., expected object, found string) |
| `NumericOperationOnNonNumber` | `Increment`/`Decrement` on a non-numeric value |
| `MergeRequiresObject` | `MergeObject` on a non-object value |
| `AppendRequiresArray` | `Append` on a non-array value |
| `InvalidOperation` | General invalid operation |
| `Serialization` | serde_json serialization/deserialization failure |

## TireaResult

Convenience type alias:

```rust,ignore
pub type TireaResult<T> = Result<T, TireaError>;
```

## Error Context with `with_prefix`

When working with nested state, errors include the full path context. The `with_prefix` method prepends a path segment:

```rust,ignore
// If a nested struct at "address" has an error at "city":
let err = TireaError::path_not_found(path!("city"));
let contextualized = err.with_prefix(&path!("address"));
// Error now shows: "path not found: address.city"
```

## Convenience Constructors

`TireaError` provides factory methods for each variant:

- `TireaError::path_not_found(path)`
- `TireaError::index_out_of_bounds(path, index, len)`
- `TireaError::type_mismatch(path, expected, found)`
- `TireaError::numeric_on_non_number(path)`
- `TireaError::merge_requires_object(path)`
- `TireaError::append_requires_array(path)`
- `TireaError::invalid_operation(message)`

## Agent-Level Errors

`tirea` defines additional error types:

- **`ToolError`** — Errors from tool execution
- **`AgentLoopError`** — Errors in the agent loop (LLM failures, tool errors)
- **`ThreadStoreError`** — Thread persistence failures
- **`AgentOsBuildError`** / **`AgentOsWiringError`** — Configuration errors
