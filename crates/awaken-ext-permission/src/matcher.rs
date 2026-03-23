//! Pattern matching engine — re-exports from [`awaken_tool_pattern`].
//!
//! The matching logic lives in `awaken-tool-pattern` so it can be
//! reused by other extensions (e.g. reminder, observability).

pub use awaken_tool_pattern::{
    MatchResult, Specificity, evaluate_field_condition, evaluate_op, op_precision, pattern_matches,
    resolve_path, schema_has_path, validate_pattern_fields, wildcard_match,
};
