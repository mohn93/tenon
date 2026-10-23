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

/// The grammar of a batch reference, whole string.
const BATCH_REF_PATTERN: &str = r"^\$(0|[1-9][0-9]*):(([vefigrnsmdy])(0|[1-9][0-9]*|\*)|\*)$";

/// The schema of a batch reference: a string naming ids an earlier command of the batch made.
pub fn batch_ref_schema() -> Value {
    json!({
        "type": "string",
        "pattern": BATCH_REF_PATTERN,
        "description": "In a batch, ids an earlier command of it made: \"$N:KI\" the I-th id of kind K (v vertex, e edge, f face, i instance, g guide, r dimension, n note, s section plane, m material, d definition, y style) command N made, \"$N:K*\" all of that kind (in id lists only), \"$N:*\" every edge, face, instance, guide, dimension, note and section plane it created."
    })
}

/// Lets the `EntityId`, `MaterialId`, `DefinitionId` and `StyleId` definitions in `defs` also be
/// batch references (`anyOf` the id or a `BatchRef`) and adds the `BatchRef` definition.
pub fn with_batch_refs(defs: &mut Map<String, Value>) {
    for name in ["EntityId", "MaterialId", "DefinitionId", "StyleId"] {
        if let Some(def) = defs.get_mut(name) {
            let id = def.take();
            *def = json!({"anyOf": [id, {"$ref": "#/$defs/BatchRef"}]});
        }
    }
    defs.insert("BatchRef".into(), batch_ref_schema());
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
    fn with_batch_refs_wraps_the_id_definitions_present() {
        let mut defs = Map::new();
        defs.insert("EntityId".into(), json!({"type": "string"}));
        defs.insert("Other".into(), json!({"type": "number"}));
        with_batch_refs(&mut defs);
        assert_eq!(
            defs["EntityId"],
            json!({"anyOf": [{"type": "string"}, {"$ref": "#/$defs/BatchRef"}]})
        );
        assert_eq!(defs["Other"], json!({"type": "number"}));
        assert!(!defs.contains_key("MaterialId"));
        assert_eq!(defs["BatchRef"], batch_ref_schema());
    }

    #[test]
    #[should_panic(expected = "differs")]
    fn hoist_refuses_two_definitions_of_one_name() {
        let mut defs = Map::new();
        hoist(&mut defs, json!({"$defs": {"P": {"type": "number"}}}));
        hoist(&mut defs, json!({"$defs": {"P": {"type": "string"}}}));
    }
}
