# Error Handling

`carve-state` uses the `thiserror` crate with a unified `CarveError` enum.

## CarveError

```rust,ignore
pub enum CarveError {
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

## CarveResult

Convenience type alias:

```rust,ignore
pub type CarveResult<T> = Result<T, CarveError>;
```

## Error Context with `with_prefix`

When working with nested state, errors include the full path context. The `with_prefix` method prepends a path segment:

```rust,ignore
// If a nested struct at "address" has an error at "city":
let err = CarveError::path_not_found(path!("city"));
let contextualized = err.with_prefix(&path!("address"));
// Error now shows: "path not found: address.city"
```

## Convenience Constructors

`CarveError` provides factory methods for each variant:

- `CarveError::path_not_found(path)`
- `CarveError::index_out_of_bounds(path, index, len)`
- `CarveError::type_mismatch(path, expected, found)`
- `CarveError::numeric_on_non_number(path)`
- `CarveError::merge_requires_object(path)`
- `CarveError::append_requires_array(path)`
- `CarveError::invalid_operation(message)`

## Agent-Level Errors

`carve-agent` defines additional error types:

- **`ToolError`** — Errors from tool execution
- **`AgentLoopError`** — Errors in the agent loop (LLM failures, tool errors)
- **`ThreadStoreError`** — Thread persistence failures
- **`AgentOsBuildError`** / **`AgentOsWiringError`** — Configuration errors
