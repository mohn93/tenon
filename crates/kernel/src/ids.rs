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
id_type!(GuideId);
id_type!(CurveId);
id_type!(DimensionId);
id_type!(NoteId);
id_type!(SectionId);

/// Shown as "c12". A curve is not an entity: it names the chain of edges tagged with it.
impl fmt::Display for CurveId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7", "i4", "g9", "r5",
/// "n6", "s2".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EntityId {
    Vertex(VertexId),
    Edge(EdgeId),
    Face(FaceId),
    /// A placed group or component.
    Instance(InstanceId),
    /// A guide line or guide point (see `Guide`).
    Guide(GuideId),
    /// A linear or radial dimension (see `Dimension`).
    Dimension(DimensionId),
    /// A leader or screen note (see `Note`).
    Note(NoteId),
    /// A section plane (see `SectionPlane`).
    Section(SectionId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Vertex(id) => write!(f, "v{}", id.0),
            EntityId::Edge(id) => write!(f, "e{}", id.0),
            EntityId::Face(id) => write!(f, "f{}", id.0),
            EntityId::Instance(id) => write!(f, "i{}", id.0),
            EntityId::Guide(id) => write!(f, "g{}", id.0),
            EntityId::Dimension(id) => write!(f, "r{}", id.0),
            EntityId::Note(id) => write!(f, "n{}", id.0),
            EntityId::Section(id) => write!(f, "s{}", id.0),
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
            'g' => Ok(EntityId::Guide(GuideId(n))),
            'r' => Ok(EntityId::Dimension(DimensionId(n))),
            'n' => Ok(EntityId::Note(NoteId(n))),
            's' => Ok(EntityId::Section(SectionId(n))),
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

/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face), "i4" (instance),
/// "g9" (guide), "r5" (dimension), "n6" (note), "s2" (section plane).
impl schemars::JsonSchema for EntityId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EntityId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[vefigrns](0|[1-9][0-9]*)$",
            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face, i<n> instance, g<n> guide, r<n> dimension, n<n> note, s<n> section plane"
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

/// A material faces and instances are painted with. Serialized as "m<n>". Material ids come
/// from the same counter as entity ids, so they never repeat one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MaterialId(pub u64);

impl fmt::Display for MaterialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m{}", self.0)
    }
}

impl FromStr for MaterialId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('m', n)) => Ok(MaterialId(n)),
            _ => Err(format!("invalid material id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for MaterialId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MaterialId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^m(0|[1-9][0-9]*)$",
            "description": "Material id: m<n>"
        })
    }
}

impl From<MaterialId> for String {
    fn from(id: MaterialId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for MaterialId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A tag entities are put on to show or hide them together. Serialized as "t<n>". Tag ids
/// come from the same counter as entity ids, so they never repeat one. An entity on no tag is
/// "Untagged".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TagId(pub u64);

impl fmt::Display for TagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

impl FromStr for TagId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('t', n)) => Ok(TagId(n)),
            _ => Err(format!("invalid tag id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for TagId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TagId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^t(0|[1-9][0-9]*)$",
            "description": "Tag id: t<n>"
        })
    }
}

impl From<TagId> for String {
    fn from(id: TagId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for TagId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A saved scene (a SketchUp "page"): a stored view and tag visibility. Serialized as
/// "p<n>". Page ids come from the same counter as entity ids, so they never repeat one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PageId(pub u64);

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

impl FromStr for PageId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('p', n)) => Ok(PageId(n)),
            _ => Err(format!("invalid scene id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for PageId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "PageId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^p(0|[1-9][0-9]*)$",
            "description": "Scene id: p<n>"
        })
    }
}

impl From<PageId> for String {
    fn from(id: PageId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for PageId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A joint between two parts (or a part and the ground). Serialized as "j<n>". Joint ids
/// come from the same counter as entity ids, so they never repeat one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct JointId(pub u64);

impl fmt::Display for JointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "j{}", self.0)
    }
}

impl FromStr for JointId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('j', n)) => Ok(JointId(n)),
            _ => Err(format!("invalid joint id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for JointId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "JointId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^j(0|[1-9][0-9]*)$",
            "description": "Joint id: j<n>"
        })
    }
}

impl From<JointId> for String {
    fn from(id: JointId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for JointId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A coupling between two joints' values (see `Coupling`). Serialized as "k<n>" ("c" is
/// taken by curves). Coupling ids come from the same counter as entity ids, so they never
/// repeat one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CouplingId(pub u64);

impl fmt::Display for CouplingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "k{}", self.0)
    }
}

impl FromStr for CouplingId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('k', n)) => Ok(CouplingId(n)),
            _ => Err(format!("invalid coupling id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for CouplingId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CouplingId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^k(0|[1-9][0-9]*)$",
            "description": "Coupling id: k<n>"
        })
    }
}

impl From<CouplingId> for String {
    fn from(id: CouplingId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for CouplingId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// A style: named edge, background and fog settings a view may choose. Serialized as
/// "y<n>". Style ids come from the same counter as entity ids, so they never repeat one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StyleId(pub u64);

impl fmt::Display for StyleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "y{}", self.0)
    }
}

impl FromStr for StyleId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse_prefixed(s) {
            Some(('y', n)) => Ok(StyleId(n)),
            _ => Err(format!("invalid style id {s:?}")),
        }
    }
}

impl schemars::JsonSchema for StyleId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "StyleId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^y(0|[1-9][0-9]*)$",
            "description": "Style id: y<n>"
        })
    }
}

impl From<StyleId> for String {
    fn from(id: StyleId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for StyleId {
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
            EntityId::Guide(GuideId(9)),
            EntityId::Dimension(DimensionId(5)),
            EntityId::Note(NoteId(6)),
            EntityId::Section(SectionId(2)),
        ] {
            assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
        }
        assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
        assert_eq!(EntityId::Instance(InstanceId(4)).to_string(), "i4");
        assert_eq!(EntityId::Guide(GuideId(9)).to_string(), "g9");
        assert_eq!(EntityId::Dimension(DimensionId(5)).to_string(), "r5");
        assert_eq!(EntityId::Note(NoteId(6)).to_string(), "n6");
        assert_eq!(EntityId::Section(SectionId(2)).to_string(), "s2");
        assert_eq!("d3".parse::<DefinitionId>(), Ok(DefinitionId(3)));
        assert_eq!(DefinitionId(3).to_string(), "d3");
        for s in ["", "d", "f3", "d03", "d-1"] {
            assert!(s.parse::<DefinitionId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("m5".parse::<MaterialId>(), Ok(MaterialId(5)));
        assert_eq!(MaterialId(5).to_string(), "m5");
        for s in ["", "m", "d5", "m05", "m+5"] {
            assert!(s.parse::<MaterialId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("t5".parse::<TagId>(), Ok(TagId(5)));
        assert_eq!(TagId(5).to_string(), "t5");
        for s in ["", "t", "m5", "t05", "t+5"] {
            assert!(s.parse::<TagId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("p5".parse::<PageId>(), Ok(PageId(5)));
        assert_eq!(PageId(5).to_string(), "p5");
        for s in ["", "p", "t5", "p05", "p+5"] {
            assert!(s.parse::<PageId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("j5".parse::<JointId>(), Ok(JointId(5)));
        assert_eq!(JointId(5).to_string(), "j5");
        for s in ["", "j", "p5", "j05", "j+5"] {
            assert!(s.parse::<JointId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("k5".parse::<CouplingId>(), Ok(CouplingId(5)));
        assert_eq!(CouplingId(5).to_string(), "k5");
        for s in ["", "k", "c5", "k05", "k+5"] {
            assert!(s.parse::<CouplingId>().is_err(), "{s:?} parsed");
        }
        assert_eq!("y5".parse::<StyleId>(), Ok(StyleId(5)));
        assert_eq!(StyleId(5).to_string(), "y5");
        for s in ["", "y", "p5", "y05", "y+5"] {
            assert!(s.parse::<StyleId>().is_err(), "{s:?} parsed");
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
