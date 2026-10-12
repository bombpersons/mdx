//! `mdx schema` -- describe a command's output shape.
//!
//! Rather than hand-maintaining a schema document that drifts out of date, the
//! shape is derived by running the command and walking the value it actually
//! serialised. That guarantees the description is true for the current build.

use serde_json::Value;

/// Render the shape of a serialised response as an indented tree.
pub fn shape(value: &Value) -> String {
    let mut lines = Vec::new();
    walk(value, "root", 0, &mut lines);
    lines.join("\n")
}

fn walk(value: &Value, label: &str, depth: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth);
    match value {
        Value::Object(map) => {
            out.push(format!("{pad}{label}: object"));
            // Sorted so the description is byte-stable across runs.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                walk(&map[key], key, depth + 1, out);
            }
        }
        Value::Array(items) => {
            out.push(format!(
                "{pad}{label}: array<{}> [{n}]",
                items.first().map(type_name).unwrap_or("empty"),
                n = items.len()
            ));
            if let Some(first) = items.first() {
                if first.is_object() || first.is_array() {
                    walk(first, "items", depth + 1, out);
                }
            }
        }
        other => out.push(format!("{pad}{label}: {}", type_name(other))),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn describes_nested_arrays_and_objects() {
        let v = json!({
            "ok": true,
            "count": 3,
            "vectors": [{"name": "reset", "addr": "0x00"}],
            "nested": {"deep": null}
        });
        let s = shape(&v);
        assert!(s.contains("root: object"));
        assert!(s.contains("  count: integer"));
        assert!(s.contains("  ok: boolean"));
        assert!(s.contains("  vectors: array<object> [1]"));
        assert!(s.contains("      addr: string"));
        assert!(s.contains("    deep: null"));
    }

    #[test]
    fn handles_empty_arrays() {
        let s = shape(&json!({"a": []}));
        assert!(s.contains("a: array<empty> [0]"));
    }
}
