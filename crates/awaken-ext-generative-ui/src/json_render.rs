//! JSON Render format preset.

use serde::Deserialize;
use serde_json::{Map, Value};

/// Activity type for JSON Render streaming events.
pub const ACTIVITY_TYPE: &str = "generative-ui.json-render";

#[derive(Debug, Deserialize)]
struct SpecStreamPatch {
    op: String,
    path: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    value: Option<Value>,
}

/// Returns a system prompt instructing the sub-agent to output JSON Render SpecStream JSONL.
pub fn system_prompt(catalog_json: &str) -> String {
    format!(
        "You are a UI generation agent. Output ONLY JSON Render SpecStream JSONL.\n\n\
         ## Output Format\n\
         - Emit one JSON object per line.\n\
         - Each line must be a JSON Patch-style operation for the flat JSON Render spec.\n\
         - Start streaming immediately. Do not wait to finish the whole interface before writing the first line.\n\
         - Do not wrap the lines in an array or markdown fences.\n\
         - Do not add commentary or prose.\n\n\
         ## Flat Spec Shape\n\
         {{\n  \"root\": \"elementId\",\n  \"elements\": {{\n    \
         \"elementId\": {{\n      \"type\": \"ComponentName\",\n      \
         \"props\": {{}},\n      \"children\": []\n    }}\n  }}\n}}\n\n\
         ## SpecStream Example\n\
         {{\"op\":\"add\",\"path\":\"/root\",\"value\":\"workspace\"}}\n\
         {{\"op\":\"add\",\"path\":\"/elements/workspace\",\"value\":{{\"type\":\"Card\",\"props\":{{\"title\":\"Procurement request\"}},\"children\":[\"header\",\"actions\"]}}}}\n\
         {{\"op\":\"add\",\"path\":\"/elements/header\",\"value\":{{\"type\":\"Text\",\"props\":{{\"text\":\"Request overview\"}},\"children\":[]}}}}\n\
         {{\"op\":\"add\",\"path\":\"/elements/actions\",\"value\":{{\"type\":\"Stack\",\"props\":{{\"direction\":\"horizontal\"}},\"children\":[\"approve\",\"hold\"]}}}}\n\
         {{\"op\":\"add\",\"path\":\"/elements/approve\",\"value\":{{\"type\":\"Button\",\"props\":{{\"label\":\"Approve\"}},\"children\":[]}}}}\n\
         {{\"op\":\"add\",\"path\":\"/elements/hold\",\"value\":{{\"type\":\"Button\",\"props\":{{\"label\":\"Request changes\"}},\"children\":[]}}}}\n\n\
         ## Available Components\n\
         {catalog_json}\n\n\
         ## Rules\n\
         - Preserve the user's business domain, requested fields, and realistic product copy.\n\
         - Keep the layout practical for SaaS or internal business tools.\n\
         - Every referenced child id must be defined in `elements`.\n\
         - Keep `visible`, `repeat`, `watch`, and `on` as top-level element fields, never inside `props`.\n\
         - Use `add` to create structure, `replace` for late corrections, and `remove` only when necessary.\n\
         - Output ONLY SpecStream JSONL."
    )
}

/// Compile a JSON Render SpecStream JSONL string or a complete spec object string.
pub fn compile_output(content: &str) -> Result<Value, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("json-render output was empty".into());
    }

    let mut spec = Value::Object(Map::new());
    let mut patch_count = 0usize;

    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(patch) = serde_json::from_str::<SpecStreamPatch>(line) else {
            continue;
        };

        if patch.path.is_empty() {
            continue;
        }

        apply_spec_stream_patch(&mut spec, patch)?;
        patch_count += 1;
    }

    if patch_count == 0 {
        let parsed: Value = serde_json::from_str(trimmed)
            .map_err(|error| format!("invalid JSON output: {error}"))?;
        validate_spec(&parsed)?;
        return Ok(parsed);
    }

    validate_spec(&spec)?;
    Ok(spec)
}

fn validate_spec(spec: &Value) -> Result<(), String> {
    let obj = spec
        .as_object()
        .ok_or_else(|| "json-render output must be a JSON object".to_string())?;
    let root = obj
        .get("root")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "json-render spec must contain a non-empty string root".to_string())?;
    let elements = obj
        .get("elements")
        .and_then(Value::as_object)
        .ok_or_else(|| "json-render spec must contain an elements object".to_string())?;
    if !elements.contains_key(root) {
        return Err(format!(
            "json-render spec root \"{root}\" does not exist in elements"
        ));
    }
    Ok(())
}

fn apply_spec_stream_patch(target: &mut Value, patch: SpecStreamPatch) -> Result<(), String> {
    match patch.op.as_str() {
        "add" => add_by_path(target, &patch.path, patch.value.unwrap_or(Value::Null)),
        "replace" => set_by_path(target, &patch.path, patch.value.unwrap_or(Value::Null)),
        "remove" => remove_by_path(target, &patch.path),
        "move" => {
            let Some(from) = patch.from.as_deref() else {
                return Err("move patch is missing `from`".into());
            };
            let Some(value) = get_by_path(target, from).cloned() else {
                return Ok(());
            };
            remove_by_path(target, from);
            add_by_path(target, &patch.path, value);
        }
        "copy" => {
            let Some(from) = patch.from.as_deref() else {
                return Err("copy patch is missing `from`".into());
            };
            let Some(value) = get_by_path(target, from).cloned() else {
                return Ok(());
            };
            add_by_path(target, &patch.path, value);
        }
        "test" => {
            let Some(expected) = patch.value.as_ref() else {
                return Err("test patch is missing `value`".into());
            };
            let Some(actual) = get_by_path(target, &patch.path) else {
                return Err(format!("test patch failed at path {}", patch.path));
            };
            if actual != expected {
                return Err(format!("test patch failed at path {}", patch.path));
            }
        }
        other => {
            return Err(format!("unsupported spec stream operation: {other}"));
        }
    }

    Ok(())
}

fn get_by_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() || path == "/" {
        return Some(value);
    }

    let mut current = value;
    for segment in parse_json_pointer(path) {
        match current {
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                current = items.get(index)?;
            }
            Value::Object(map) => {
                current = map.get(&segment)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn set_by_path(target: &mut Value, path: &str, value: Value) {
    if path.is_empty() || path == "/" {
        *target = value;
        return;
    }

    let segments = parse_json_pointer(path);
    if segments.is_empty() {
        *target = value;
        return;
    }

    let current = ensure_parent_container(target, &segments[..segments.len() - 1]);
    let last = &segments[segments.len() - 1];
    set_child_value(current, last, value);
}

fn add_by_path(target: &mut Value, path: &str, value: Value) {
    if path.is_empty() || path == "/" {
        *target = value;
        return;
    }

    let segments = parse_json_pointer(path);
    if segments.is_empty() {
        *target = value;
        return;
    }

    let current = ensure_parent_container(target, &segments[..segments.len() - 1]);
    let last = &segments[segments.len() - 1];

    match current {
        Value::Array(items) => {
            if last == "-" {
                items.push(value);
            } else {
                let index = last
                    .parse::<usize>()
                    .unwrap_or(items.len())
                    .min(items.len());
                items.insert(index, value);
            }
        }
        Value::Object(map) => {
            map.insert(last.clone(), value);
        }
        _ => {}
    }
}

fn remove_by_path(target: &mut Value, path: &str) {
    if path.is_empty() || path == "/" {
        return;
    }

    let segments = parse_json_pointer(path);
    if segments.is_empty() {
        return;
    }

    let Some(current) = get_parent_container_mut(target, &segments[..segments.len() - 1]) else {
        return;
    };
    let last = &segments[segments.len() - 1];

    match current {
        Value::Array(items) => {
            if let Ok(index) = last.parse::<usize>() {
                if index < items.len() {
                    items.remove(index);
                }
            }
        }
        Value::Object(map) => {
            map.remove(last);
        }
        _ => {}
    }
}

fn set_child_value(current: &mut Value, last: &str, value: Value) {
    match current {
        Value::Array(items) => {
            if last == "-" {
                items.push(value);
                return;
            }

            let index = last.parse::<usize>().unwrap_or(items.len());
            if index >= items.len() {
                items.resize(index + 1, Value::Null);
            }
            items[index] = value;
        }
        Value::Object(map) => {
            map.insert(last.to_string(), value);
        }
        _ => {}
    }
}

fn ensure_parent_container<'a>(target: &'a mut Value, segments: &[String]) -> &'a mut Value {
    let mut current = target;
    for (index, segment) in segments.iter().enumerate() {
        let next_is_numeric = segments
            .get(index + 1)
            .map(|next| is_numeric_index(next) || next == "-")
            .unwrap_or(false);

        match current {
            Value::Array(items) => {
                let array_index = segment.parse::<usize>().unwrap_or(items.len());
                if array_index >= items.len() {
                    items.resize(array_index + 1, Value::Null);
                }
                if !items[array_index].is_array() && !items[array_index].is_object() {
                    items[array_index] = if next_is_numeric {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    };
                }
                current = &mut items[array_index];
            }
            Value::Object(map) => {
                let entry = map.entry(segment.clone()).or_insert_with(|| {
                    if next_is_numeric {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    }
                });
                if !entry.is_array() && !entry.is_object() {
                    *entry = if next_is_numeric {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    };
                }
                current = entry;
            }
            _ => {
                *current = Value::Object(Map::new());
                if let Value::Object(map) = current {
                    let entry = map.entry(segment.clone()).or_insert_with(|| {
                        if next_is_numeric {
                            Value::Array(Vec::new())
                        } else {
                            Value::Object(Map::new())
                        }
                    });
                    current = entry;
                }
            }
        }
    }
    current
}

fn get_parent_container_mut<'a>(
    target: &'a mut Value,
    segments: &[String],
) -> Option<&'a mut Value> {
    let mut current = target;

    for segment in segments {
        match current {
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                current = items.get_mut(index)?;
            }
            Value::Object(map) => {
                current = map.get_mut(segment)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

fn parse_json_pointer(path: &str) -> Vec<String> {
    let raw = if let Some(stripped) = path.strip_prefix('/') {
        stripped.split('/')
    } else {
        path.split('/')
    };

    raw.map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn is_numeric_index(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{compile_output, system_prompt};

    #[test]
    fn prompt_mentions_spec_stream_jsonl() {
        let prompt = system_prompt("{\"Card\":\"Container\"}");
        assert!(prompt.contains("SpecStream JSONL"));
        assert!(prompt.contains("\"op\":\"add\""));
    }

    #[test]
    fn compiles_spec_stream_output() {
        let compiled = compile_output(
            r#"{"op":"add","path":"/root","value":"workspace"}
{"op":"add","path":"/elements/workspace","value":{"type":"Card","props":{"title":"Procurement request"},"children":["summary"]}}
{"op":"add","path":"/elements/summary","value":{"type":"Text","props":{"text":"Awaiting approval"},"children":[]}}"#,
        )
        .expect("spec stream should compile");

        assert_eq!(compiled["root"], "workspace");
        assert_eq!(compiled["elements"]["workspace"]["type"], "Card");
        assert_eq!(
            compiled["elements"]["summary"]["props"]["text"],
            "Awaiting approval"
        );
    }

    #[test]
    fn compiles_spec_stream_with_nested_paths() {
        let compiled = compile_output(
            r#"{"op":"add","path":"/root","value":"workspace"}
{"op":"add","path":"/elements/workspace","value":{"type":"Card","props":{},"children":[]}}
{"op":"add","path":"/state/filters/status","value":"pending"}"#,
        )
        .expect("nested add should create parents");

        assert_eq!(compiled["state"]["filters"]["status"], "pending");
    }

    #[test]
    fn accepts_complete_json_spec() {
        let compiled = compile_output(
            r#"{"root":"workspace","elements":{"workspace":{"type":"Card","props":{"title":"Quarterly review"},"children":[]}}}"#,
        )
        .expect("complete JSON spec should compile");

        assert_eq!(
            compiled["elements"]["workspace"]["props"]["title"],
            "Quarterly review"
        );
    }

    #[test]
    fn rejects_output_without_elements() {
        let error = compile_output(r#"{"op":"add","path":"/root","value":"workspace"}"#)
            .expect_err("missing root element should fail");
        assert!(error.contains("elements object"));
    }

    #[test]
    fn supports_replace_and_remove() {
        let compiled = compile_output(
            r#"{"op":"add","path":"/root","value":"workspace"}
{"op":"add","path":"/elements/workspace","value":{"type":"Card","props":{"title":"Draft"},"children":[]}}
{"op":"replace","path":"/elements/workspace/props/title","value":"Approved"}
{"op":"add","path":"/elements/workspace/children/-","value":"footer"}
{"op":"add","path":"/elements/footer","value":{"type":"Text","props":{"text":"Next review in 30 days"},"children":[]}}
{"op":"remove","path":"/elements/workspace/children/0"}"#,
        )
        .expect("patch operations should compile");

        assert_eq!(
            compiled["elements"]["workspace"]["props"]["title"],
            "Approved"
        );
        assert_eq!(compiled["elements"]["workspace"]["children"], json!([]));
    }
}
