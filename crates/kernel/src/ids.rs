use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(VertexId);
id_type!(EdgeId);
id_type!(FaceId);
id_type!(InstanceId);

/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7", "i4".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EntityId {
    Vertex(VertexId),
    Edge(EdgeId),
    Face(FaceId),
    /// A placed group or component.
    Instance(InstanceId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Vertex(id) => write!(f, "v{}", id.0),
            EntityId::Edge(id) => write!(f, "e{}", id.0),
            EntityId::Face(id) => write!(f, "f{}", id.0),
            EntityId::Instance(id) => write!(f, "i{}", id.0),
        }
    }
}

impl FromStr for EntityId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, n) = parse_prefixed(s).ok_or_else(|| format!("invalid entity id {s:?}"))?;
        match kind {
            'v' => Ok(EntityId::Vertex(VertexId(n))),
            'e' => Ok(EntityId::Edge(EdgeId(n))),
            'f' => Ok(EntityId::Face(FaceId(n))),
            'i' => Ok(EntityId::Instance(InstanceId(n))),
            _ => Err(format!("invalid entity id {s:?}")),
        }
    }
}

/// A letter followed by a canonical decimal number: digits, no sign, no leading zeros ("v0" is
/// fine, "v007" is not).
fn parse_prefixed(s: &str) -> Option<(char, u64)> {
    let mut chars = s.chars();
    let kind = chars.next()?;
    let digits = chars.as_str();
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    if !canonical {
        return None;
    }
    Some((kind, digits.parse().ok()?))
}

/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face), "i4" (instance).
impl schemars::JsonSchema for EntityId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EntityId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[vefi](0|[1-9][0-9]*)$",
            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face, i<n> instance"
        })
    }
}

impl From<EntityId> for String {
    fn from(id: EntityId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for EntityId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A group or component definition. Serialized as "d<n>".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DefinitionId(pub u64);

impl fmt::Display for DefinitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

impl FromStr for DefinitionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('d', n)) => Ok(DefinitionId(n)),
            _ => Err(format!("invalid definition id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for DefinitionId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DefinitionId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^d(0|[1-9][0-9]*)$",
            "description": "Group or component definition id: d<n>"
        })
    }
}

impl From<DefinitionId> for String {
    fn from(id: DefinitionId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for DefinitionId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_round_trip_as_strings() {
        for id in [
            EntityId::Vertex(VertexId(12)),
            EntityId::Edge(EdgeId(0)),
            EntityId::Face(FaceId(7)),
            EntityId::Instance(InstanceId(4)),
        ] {
            assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
        }
        assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
        assert_eq!(EntityId::Instance(InstanceId(4)).to_string(), "i4");
        assert_eq!("d3".parse::<DefinitionId>(), Ok(DefinitionId(3)));
        assert_eq!(DefinitionId(3).to_string(), "d3");
        for s in ["", "d", "f3", "d03", "d-1"] {
            assert!(s.parse::<DefinitionId>().is_err(), "{s:?} parsed");
        }
    }

    #[test]
    fn malformed_entity_ids_are_rejected() {
        for s in [
            "",
            "x1",
            "v",
            "v-1",
            "e1.5",
            "é1",
            "v+5",
            "v007",
            "f 3",
            "e99999999999999999999",
        ] {
            assert!(s.parse::<EntityId>().is_err(), "{s:?} parsed");
        }
    }
}
