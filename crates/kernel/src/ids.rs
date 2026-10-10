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

/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EntityId {
    Vertex(VertexId),
    Edge(EdgeId),
    Face(FaceId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Vertex(id) => write!(f, "v{}", id.0),
            EntityId::Edge(id) => write!(f, "e{}", id.0),
            EntityId::Face(id) => write!(f, "f{}", id.0),
        }
    }
}

impl FromStr for EntityId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let kind = chars
            .next()
            .ok_or_else(|| format!("invalid entity id {s:?}"))?;
        let n: u64 = chars
            .as_str()
            .parse()
            .map_err(|_| format!("invalid entity id {s:?}"))?;
        match kind {
            'v' => Ok(EntityId::Vertex(VertexId(n))),
            'e' => Ok(EntityId::Edge(EdgeId(n))),
            'f' => Ok(EntityId::Face(FaceId(n))),
            _ => Err(format!("invalid entity id {s:?}")),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_round_trip_as_strings() {
        for id in [
            EntityId::Vertex(VertexId(12)),
            EntityId::Edge(EdgeId(0)),
            EntityId::Face(FaceId(7)),
        ] {
            assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
        }
        assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
    }

    #[test]
    fn malformed_entity_ids_are_rejected() {
        for s in ["", "x1", "v", "v-1", "e1.5", "é1"] {
            assert!(s.parse::<EntityId>().is_err(), "{s:?} parsed");
        }
    }
}
