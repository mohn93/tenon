//! Helpers that shape generated JSON Schemas for clients: an object root on tagged enums and
//! self-contained schemas built from several, their definitions gathered at the root.

use serde_json::{Map, Value, json};

/// `schema` as JSON with `"type": "object"` at its root (beside `oneOf`/`anyOf`), unchanged
/// otherwise. Clients that want an object root (tool input schemas) take a tagged enum so.
pub fn object_root(mut schema: Value) -> Value {
    if let Some(root) = schema.as_object_mut() {
        root.entry("type").or_insert_with(|| json!("object"));
    }
    schema
}

/// Moves `schema`'s `$defs` into `root_defs` and returns the rest without `$schema`, `$defs`
/// or `title`. Its `#/$defs/<Name>` refs then resolve against the root that holds
/// `root_defs`.
///
/// # Panics
///
/// When a definition's name is already in `root_defs` with different content.
pub fn hoist(root_defs: &mut Map<String, Value>, mut schema: Value) -> Value {
    if let Some(root) = schema.as_object_mut() {
        root.remove("$schema");
        root.remove("title");
        if let Some(Value::Object(defs)) = root.remove("$defs") {
            for (name, def) in defs {
                match root_defs.get(&name) {
                    Some(existing) => assert!(
                        *existing == def,
                        "schema definition {name} differs between schemas"
                    ),
                    None => {
                        root_defs.insert(name, def);
                    }
                }
            }
        }
    }
    schema
}

/// A schema any of `schemas` meets: `{"type": "object", "anyOf": [...], "$defs": {...}}`,
/// self-contained.
pub fn any_of_root(schemas: Vec<Value>) -> Value {
    let mut defs = Map::new();
    let any_of: Vec<Value> = schemas.into_iter().map(|s| hoist(&mut defs, s)).collect();
    json!({"type": "object", "anyOf": any_of, "$defs": defs})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_root_adds_the_type_only_when_absent() {
        let tagged = object_root(json!({"oneOf": [{"type": "object"}]}));
        assert_eq!(tagged["type"], "object");
        assert_eq!(tagged["oneOf"], json!([{"type": "object"}]));
        assert_eq!(object_root(json!({"type": "array"}))["type"], "array");
    }

    #[test]
    fn hoist_gathers_definitions_at_the_root() {
        let a = json!({"$schema": "s", "title": "A", "$ref": "#/$defs/P", "$defs": {"P": {"type": "number"}}});
        let b = json!({"title": "B", "items": {"$ref": "#/$defs/P"}, "$defs": {"P": {"type": "number"}}});
        let root = any_of_root(vec![a, b]);
        assert_eq!(
            root,
            json!({
                "type": "object",
                "anyOf": [{"$ref": "#/$defs/P"}, {"items": {"$ref": "#/$defs/P"}}],
                "$defs": {"P": {"type": "number"}}
            })
        );
    }

    #[test]
    #[should_panic(expected = "differs")]
    fn hoist_refuses_two_definitions_of_one_name() {
        let mut defs = Map::new();
        hoist(&mut defs, json!({"$defs": {"P": {"type": "number"}}}));
        hoist(&mut defs, json!({"$defs": {"P": {"type": "string"}}}));
    }
}
