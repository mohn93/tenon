//! Schema-only types: how fields are described to API clients, not how they are stored.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::document::MAX_COORDINATE;

/// A model point `[x, y, z]` in meters, each coordinate within +/-`MAX_COORDINATE` (the limit
/// commands check). For `#[schemars(with = "ModelPoint")]` on `[f64; 3]` fields that are
/// positions; directions, normals and move vectors stay plain `[f64; 3]`.
pub struct ModelPoint;

impl JsonSchema for ModelPoint {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ModelPoint".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": {
                "type": "number",
                "minimum": -MAX_COORDINATE,
                "maximum": MAX_COORDINATE,
            },
            "minItems": 3,
            "maxItems": 3,
        })
    }
}
