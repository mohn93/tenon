//! Batch commands that refer to ids made by earlier commands of the same batch.
//!
//! In a batch, an id field may hold a reference instead of an id: `"$N:KI"` is the I-th id of
//! kind K (`v e f i g r n s` from `changes.created`, `m` from `changes.materials`, `y` from
//! `changes.styles`, `d` from the outcome's `definitions`) in command N's outcome, `"$N:K*"` all of that kind and `"$N:*"` every edge, face, instance, guide,
//! dimension, note and section plane command N created (not vertices: `"$N:v*"` names
//! those). `*` references expand in place in id lists only.
//!
//! A parsed command holds stand-in ids (numbers from `STAND_IN_BASE` up) where its references
//! were; they are replaced by real ids when the command's turn comes.

use std::fmt;

use kernel::{
    DefinitionId, DimensionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, NoteId,
    SectionId, StyleId, VertexId,
};
use serde_json::Value;

use crate::command::{AnchorInput, Command, CommandError, ErrorKind, Outcome};
use crate::library::LibraryRef;

/// What a batch needs to know about the open context to resolve references.
pub trait OpenContext {
    /// Whether the open context holds the entity.
    fn holds(&self, id: EntityId) -> bool;
    /// Why the open context lacks it, naming the path to open when it is elsewhere.
    fn missing_text(&self, id: EntityId) -> String;
}

/// A context that holds every id (for paths, which `open_path` checks).
struct AnyContext;

impl OpenContext for AnyContext {
    fn holds(&self, _: EntityId) -> bool {
        true
    }

    fn missing_text(&self, id: EntityId) -> String {
        format!("{id} is not in the open context")
    }
}

/// The first stand-in id number. The ids a document makes come from one counter that never gets
/// near it, but ids a client names or a document holds (a crafted file) may in principle be that
/// large: `resolve` only replaces the stand-ins `parse` put in, and `parse` refuses literal ids
/// that large in a command that also has references.
pub const STAND_IN_BASE: u64 = 1 << 62;

/// The kind of id a reference picks from an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Vertex,
    Edge,
    Face,
    Instance,
    Guide,
    Dimension,
    Note,
    Section,
    Material,
    Definition,
    Style,
}

impl RefKind {
    fn from_letter(c: char) -> Option<RefKind> {
        Some(match c {
            'v' => RefKind::Vertex,
            'e' => RefKind::Edge,
            'f' => RefKind::Face,
            'i' => RefKind::Instance,
            'g' => RefKind::Guide,
            'r' => RefKind::Dimension,
            'n' => RefKind::Note,
            's' => RefKind::Section,
            'm' => RefKind::Material,
            'd' => RefKind::Definition,
            'y' => RefKind::Style,
            _ => return None,
        })
    }

    fn letter(self) -> char {
        match self {
            RefKind::Vertex => 'v',
            RefKind::Edge => 'e',
            RefKind::Face => 'f',
            RefKind::Instance => 'i',
            RefKind::Guide => 'g',
            RefKind::Dimension => 'r',
            RefKind::Note => 'n',
            RefKind::Section => 's',
            RefKind::Material => 'm',
            RefKind::Definition => 'd',
            RefKind::Style => 'y',
        }
    }

    fn word(self, count: usize) -> &'static str {
        let one = count == 1;
        match self {
            RefKind::Vertex if one => "vertex",
            RefKind::Vertex => "vertices",
            RefKind::Edge if one => "edge",
            RefKind::Edge => "edges",
            RefKind::Face if one => "face",
            RefKind::Face => "faces",
            RefKind::Instance if one => "instance",
            RefKind::Instance => "instances",
            RefKind::Guide if one => "guide",
            RefKind::Guide => "guides",
            RefKind::Dimension if one => "dimension",
            RefKind::Dimension => "dimensions",
            RefKind::Note if one => "note",
            RefKind::Note => "notes",
            RefKind::Section if one => "section plane",
            RefKind::Section => "section planes",
            RefKind::Material if one => "material",
            RefKind::Material => "materials",
            RefKind::Definition if one => "definition",
            RefKind::Definition => "definitions",
            RefKind::Style if one => "style",
            RefKind::Style => "styles",
        }
    }

    fn matches(self, id: &EntityId) -> bool {
        matches!(
            (self, id),
            (RefKind::Vertex, EntityId::Vertex(_))
                | (RefKind::Edge, EntityId::Edge(_))
                | (RefKind::Face, EntityId::Face(_))
                | (RefKind::Instance, EntityId::Instance(_))
                | (RefKind::Guide, EntityId::Guide(_))
                | (RefKind::Dimension, EntityId::Dimension(_))
                | (RefKind::Note, EntityId::Note(_))
                | (RefKind::Section, EntityId::Section(_))
        )
    }
}

/// A reference to ids an earlier command of the batch made: `$<command>:<kind><index>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchRef {
    pub command: usize,
    /// None: `*`, every entity but vertices the command created.
    pub kind: Option<RefKind>,
    /// None: `*`, every id of the kind.
    pub index: Option<usize>,
}

impl fmt::Display for BatchRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}:", self.command)?;
        match self.kind {
            None => write!(f, "*"),
            Some(kind) => match self.index {
                Some(i) => write!(f, "{}{i}", kind.letter()),
                None => write!(f, "{}*", kind.letter()),
            },
        }
    }
}

impl BatchRef {
    /// Reads `^\$(0|[1-9][0-9]*):(([vefigrnsmdy])(0|[1-9][0-9]*|\*)|\*)$`.
    fn parse(text: &str) -> Option<BatchRef> {
        let (command, rest) = text.strip_prefix('$')?.split_once(':')?;
        let command = canonical_number(command)?;
        if rest == "*" {
            return Some(BatchRef {
                command,
                kind: None,
                index: None,
            });
        }
        let mut chars = rest.chars();
        let kind = RefKind::from_letter(chars.next()?)?;
        let index = match chars.as_str() {
            "*" => None,
            digits => Some(canonical_number(digits)?),
        };
        Some(BatchRef {
            command,
            kind: Some(kind),
            index,
        })
    }

    /// The stand-in id that holds this reference's place in a command, `k` the reference's
    /// position among the command's references.
    fn stand_in(&self, k: usize) -> String {
        let letter = self.kind.map_or('v', RefKind::letter);
        format!("{letter}{}", STAND_IN_BASE + k as u64)
    }

    fn outcome<'a>(&self, done: &'a [Outcome]) -> Result<&'a Outcome, CommandError> {
        done.get(self.command).ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                format!("{self}: command {} has not run", self.command),
            )
        })
    }

    fn pick<T: Copy>(&self, ids: &[T], kind: RefKind) -> Result<Vec<T>, CommandError> {
        match self.index {
            None => Ok(ids.to_vec()),
            Some(i) => ids.get(i).map(|id| vec![*id]).ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "{self}: command {} made {} {}",
                        self.command,
                        ids.len(),
                        kind.word(ids.len())
                    ),
                )
            }),
        }
    }

    /// The ids command N made that a reference of this kind picks from: instances are the ones
    /// it left in the open context (`placed`; none when absent).
    fn candidates(&self, outcome: &Outcome, kind: Option<RefKind>) -> Vec<EntityId> {
        let top_level = |id: &EntityId| match id {
            EntityId::Instance(_) => outcome.placed.as_ref().is_some_and(|p| p.contains(id)),
            _ => true,
        };
        outcome
            .changes
            .created
            .iter()
            .filter(|id| match kind {
                None => !matches!(id, EntityId::Vertex(_)),
                Some(kind) => kind.matches(id),
            })
            .filter(|id| top_level(id))
            .copied()
            .collect()
    }

    /// The entity ids this reference stands for, in order. `context` says whether the open
    /// context still holds an id: `*` drops the ones it does not, an indexed reference to one
    /// is `not_found`.
    fn entities(
        &self,
        done: &[Outcome],
        context: &dyn OpenContext,
    ) -> Result<Vec<EntityId>, CommandError> {
        let found = self.candidates(self.outcome(done)?, self.kind);
        match (self.kind, self.index) {
            (Some(kind), Some(_)) => {
                let picked = self.pick(&found, kind)?;
                if let Some(gone) = picked.iter().find(|id| !context.holds(**id)) {
                    return Err(CommandError::new(
                        ErrorKind::NotFound,
                        format!("{self}: {}", context.missing_text(*gone)),
                    )
                    .with_ids(vec![*gone]));
                }
                Ok(picked)
            }
            _ => Ok(found.into_iter().filter(|id| context.holds(*id)).collect()),
        }
    }

    /// The definition this reference stands for.
    fn definition(&self, done: &[Outcome]) -> Result<DefinitionId, CommandError> {
        let made = self
            .outcome(done)?
            .definitions
            .as_deref()
            .unwrap_or_default();
        Ok(self.pick(made, RefKind::Definition)?[0])
    }

    /// The style this reference stands for.
    fn style(&self, done: &[Outcome]) -> Result<StyleId, CommandError> {
        let styles = &self.outcome(done)?.changes.styles;
        Ok(self.pick(styles, RefKind::Style)?[0])
    }

    /// The material this reference stands for.
    fn material(&self, done: &[Outcome]) -> Result<MaterialId, CommandError> {
        let materials = &self.outcome(done)?.changes.materials;
        Ok(self.pick(materials, RefKind::Material)?[0])
    }
}

/// "0" or digits without a leading zero.
fn canonical_number(digits: &str) -> Option<usize> {
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

/// A command of a batch, which may refer to ids earlier commands of the batch made.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchCommand {
    /// The command with stand-in ids in place of references.
    command: Command,
    /// The references, the stand-in `STAND_IN_BASE + k` holding the place of `refs[k]`.
    refs: Vec<BatchRef>,
}

impl From<Command> for BatchCommand {
    fn from(command: Command) -> Self {
        BatchCommand {
            command,
            refs: Vec::new(),
        }
    }
}

/// The top-level fields where references are recognised; `path` only in `open_path`,
/// `follow_me` and `copy_along_path`, `target` and `tools` only in `solid_subtract` and `solid_trim`, `a` and `b` also in `solid_split`, `instance` and `edges` only in `chamfer_edges`, and the anchors, edges and notes of the annotation commands only there.
fn is_reference_key(key: &str, command_type: Option<&str>) -> bool {
    matches!(key, "ids" | "face" | "id" | "material")
        || matches!(
            (key, command_type),
            ("item", Some("save_to_library"))
                | ("definition", Some("rename_definition" | "place_instance"))
                | ("path", Some("open_path" | "follow_me" | "copy_along_path"))
                | ("target" | "tools", Some("solid_subtract" | "solid_trim"))
                | ("instance" | "edges", Some("chamfer_edges"))
                | ("a" | "b", Some("add_dimension" | "solid_split"))
                | ("edge", Some("add_edge_dimension" | "add_radial_dimension"))
                | ("anchor", Some("add_note"))
                | ("note", Some("change_note"))
                | ("style", Some("change_style" | "delete_style" | "set_style"))
                | ("from", Some("add_style"))
                | (
                    "plane",
                    Some("flip_section_plane" | "activate_section_plane")
                )
        )
}

impl BatchCommand {
    /// Reads the command at position `index` of a batch. The error is the reason only, without
    /// the command's position.
    pub fn parse(mut value: Value, index: usize) -> Result<BatchCommand, String> {
        let mut refs = Vec::new();
        let mut literal_stand_in = false;
        if let Value::Object(fields) = &mut value {
            let command_type = fields
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            for (key, field) in fields.iter_mut() {
                if !is_reference_key(key, command_type.as_deref()) {
                    continue;
                }
                let material = if key == "material" {
                    Want::Material
                } else if key == "definition" {
                    Want::Definition
                } else if key == "style" || key == "from" {
                    Want::Style
                } else if key == "item" {
                    Want::Either
                } else {
                    Want::Entity
                };
                match field {
                    Value::String(text) => {
                        replace(
                            text,
                            false,
                            material,
                            index,
                            &mut refs,
                            &mut literal_stand_in,
                        )?;
                    }
                    Value::Array(items) => {
                        for item in items {
                            if let Value::String(text) = item {
                                replace(
                                    text,
                                    true,
                                    material,
                                    index,
                                    &mut refs,
                                    &mut literal_stand_in,
                                )?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if literal_stand_in && !refs.is_empty() {
            return Err(format!(
                "ids from {STAND_IN_BASE} up cannot be used with references"
            ));
        }
        let mut command: Command = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if refs.is_empty() {
            // Taken as written: a literal id however large fails at its turn, as without
            // references.
            return Ok(BatchCommand::from(command));
        }
        let mut stand_ins = 0;
        id_slots(&mut command, &mut |slot| {
            stand_ins += match slot {
                Slot::List(ids) | Slot::Path(ids) => {
                    ids.iter().filter(|id| is_stand_in(id)).count()
                }
                Slot::One(id) => usize::from(is_stand_in(id)),
                Slot::Material(m) => usize::from(m.0 >= STAND_IN_BASE),
                Slot::Definition(d) => usize::from(d.0 >= STAND_IN_BASE),
                Slot::Style(s) => usize::from(s.0 >= STAND_IN_BASE),
                Slot::Item(r) => usize::from(is_stand_in_ref(r)),
            }
        });
        if stand_ins != refs.len() {
            return Err("reference in a field that is not an id".to_string());
        }
        Ok(BatchCommand { command, refs })
    }

    /// The command with stand-in ids where its references are: only for checks before the
    /// batch runs (its type, its file paths). Stand-ins never reach a client.
    pub fn command(&self) -> &Command {
        &self.command
    }

    pub fn command_mut(&mut self) -> &mut Command {
        &mut self.command
    }

    pub fn has_references(&self) -> bool {
        !self.refs.is_empty()
    }

    /// The command to run, its references replaced by the ids they stand for. `done[n]` is
    /// command n's outcome.
    pub fn resolve(
        &self,
        done: &[Outcome],
        context: &dyn OpenContext,
    ) -> Result<Command, CommandError> {
        let mut command = self.command.clone();
        if self.refs.is_empty() {
            return Ok(command);
        }
        let mut error = None;
        id_slots(&mut command, &mut |slot| {
            if error.is_some() {
                return;
            }
            let result = match slot {
                Slot::List(ids) => self.resolve_list(ids, done, context),
                // The elements of a path are instances of nested contexts, not of the open one:
                // `open_path` checks them (as `chamfer_edges` checks a definition's edges).
                Slot::Path(ids) => self.resolve_list(ids, done, &AnyContext),
                Slot::One(id) => self.resolve_one(id, done, context),
                Slot::Material(m) => self.resolve_material(m, done),
                Slot::Definition(d) => self.resolve_definition(d, done),
                Slot::Style(s) => self.resolve_style(s, done),
                Slot::Item(r) => self.resolve_item(r, done, context),
            };
            if let Err(e) = result {
                error = Some(e);
            }
        });
        match error {
            Some(e) => Err(e),
            None => Ok(command),
        }
    }

    fn reference(&self, number: u64) -> &BatchRef {
        &self.refs[(number - STAND_IN_BASE) as usize]
    }

    fn resolve_list(
        &self,
        ids: &mut Vec<EntityId>,
        done: &[Outcome],
        context: &dyn OpenContext,
    ) -> Result<(), CommandError> {
        let mut resolved = Vec::with_capacity(ids.len());
        for id in ids.iter() {
            if is_stand_in(id) {
                resolved.extend(self.reference(number(id)).entities(done, context)?);
            } else {
                resolved.push(*id);
            }
        }
        *ids = resolved;
        Ok(())
    }

    fn resolve_one(
        &self,
        id: &mut EntityId,
        done: &[Outcome],
        context: &dyn OpenContext,
    ) -> Result<(), CommandError> {
        if is_stand_in(id) {
            // Scalar fields hold only single-id references, which resolve to one id or fail.
            *id = self.reference(number(id)).entities(done, context)?[0];
        }
        Ok(())
    }

    fn resolve_item(
        &self,
        r: &mut LibraryRef,
        done: &[Outcome],
        context: &dyn OpenContext,
    ) -> Result<(), CommandError> {
        match r {
            LibraryRef::Entity(id) => self.resolve_one(id, done, context),
            LibraryRef::Material(m) => self.resolve_material(m, done),
        }
    }

    fn resolve_definition(
        &self,
        d: &mut DefinitionId,
        done: &[Outcome],
    ) -> Result<(), CommandError> {
        if d.0 >= STAND_IN_BASE {
            *d = self.reference(d.0).definition(done)?;
        }
        Ok(())
    }

    fn resolve_style(&self, s: &mut StyleId, done: &[Outcome]) -> Result<(), CommandError> {
        if s.0 >= STAND_IN_BASE {
            *s = self.reference(s.0).style(done)?;
        }
        Ok(())
    }

    fn resolve_material(&self, m: &mut MaterialId, done: &[Outcome]) -> Result<(), CommandError> {
        if m.0 >= STAND_IN_BASE {
            *m = self.reference(m.0).material(done)?;
        }
        Ok(())
    }
}

/// What a reference field holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Entity,
    Material,
    Definition,
    Style,
    Either,
}

/// Replaces a reference in a reference field's string by a stand-in id, recording it in
/// `refs`. Other strings stay as they are.
fn replace(
    text: &mut String,
    in_list: bool,
    want: Want,
    index: usize,
    refs: &mut Vec<BatchRef>,
    literal_stand_in: &mut bool,
) -> Result<(), String> {
    if !text.starts_with('$') {
        let literal = match want {
            Want::Material => text.parse::<MaterialId>().map(|m| m.0).ok(),
            Want::Definition => text.parse::<DefinitionId>().map(|d| d.0).ok(),
            Want::Style => text.parse::<StyleId>().map(|s| s.0).ok(),
            Want::Entity => text.parse::<EntityId>().map(|id| number(&id)).ok(),
            Want::Either => text.parse::<LibraryRef>().ok().map(|r| match r {
                LibraryRef::Entity(id) => number(&id),
                LibraryRef::Material(m) => m.0,
            }),
        };
        if literal.is_some_and(|n| n >= STAND_IN_BASE) {
            *literal_stand_in = true;
        }
        return Ok(());
    }
    let r = BatchRef::parse(text).ok_or_else(|| format!("invalid reference {text:?}"))?;
    if r.command >= index {
        return Err(format!("{text:?} must refer to an earlier command"));
    }
    let is_style = r.kind == Some(RefKind::Style);
    if want == Want::Style && !is_style {
        return Err(format!("{text:?} is not a style"));
    }
    if want != Want::Style && is_style {
        return Err(format!("{text:?} is a style, not an id of that kind"));
    }
    let is_material = r.kind == Some(RefKind::Material);
    let is_definition = r.kind == Some(RefKind::Definition);
    if want == Want::Material && !is_material {
        return Err(format!("{text:?} is not a material"));
    }
    if want == Want::Definition && !is_definition {
        return Err(format!("{text:?} is not a definition"));
    }
    if want == Want::Entity && is_material {
        return Err(format!("{text:?} is a material, not an entity"));
    }
    if want != Want::Definition && is_definition {
        return Err(format!("{text:?} is a definition, not an entity"));
    }
    if r.index.is_none() && (is_material || is_definition || is_style) {
        return Err(format!(
            "{text:?} must name one {}",
            if is_material {
                "material"
            } else if is_style {
                "style"
            } else {
                "definition"
            }
        ));
    }
    if r.index.is_none() && !in_list {
        return Err(format!("{text:?} can only be used in a list of ids"));
    }
    *text = r.stand_in(refs.len());
    refs.push(r);
    Ok(())
}

fn number(id: &EntityId) -> u64 {
    match id {
        EntityId::Vertex(VertexId(n))
        | EntityId::Edge(EdgeId(n))
        | EntityId::Face(FaceId(n))
        | EntityId::Instance(InstanceId(n))
        | EntityId::Guide(GuideId(n))
        | EntityId::Dimension(DimensionId(n))
        | EntityId::Note(NoteId(n))
        | EntityId::Section(SectionId(n)) => *n,
    }
}

fn is_stand_in(id: &EntityId) -> bool {
    number(id) >= STAND_IN_BASE
}

fn is_stand_in_ref(r: &LibraryRef) -> bool {
    match r {
        LibraryRef::Entity(id) => is_stand_in(id),
        LibraryRef::Material(m) => m.0 >= STAND_IN_BASE,
    }
}

/// An id field of a command.
enum Slot<'a> {
    List(&'a mut Vec<EntityId>),
    /// Ids outside the open context, which the command checks: the instance path of
    /// `open_path`, the definition edges of `chamfer_edges` with `instance`.
    Path(&'a mut Vec<EntityId>),
    One(&'a mut EntityId),
    Material(&'a mut MaterialId),
    Definition(&'a mut DefinitionId),
    Style(&'a mut StyleId),
    /// An instance or a material (`save_to_library`'s `item`).
    Item(&'a mut LibraryRef),
}

/// Calls `f` on every id field of `command`. Every command is listed, so a new one must say
/// where its ids are.
fn id_slots(command: &mut Command, f: &mut dyn FnMut(Slot<'_>)) {
    match command {
        Command::PushPull { face, .. } => f(Slot::One(face)),
        Command::OpenInstance { id } => f(Slot::One(id)),
        Command::RenameDefinition { definition, .. } => f(Slot::Definition(definition)),
        Command::PlaceInstance { definition, .. } => f(Slot::Definition(definition)),
        Command::SetTransform { id, .. } => f(Slot::One(id)),
        Command::OrientFaces { id, ids } => {
            f(Slot::One(id));
            f(Slot::List(ids));
        }
        Command::Erase { ids }
        | Command::Move { ids, .. }
        | Command::Copy { ids, .. }
        | Command::Rotate { ids, .. }
        | Command::Scale { ids, .. }
        | Command::Offset { ids, .. }
        | Command::MakeGroup { ids, .. }
        | Command::MakeComponent { ids, .. }
        | Command::Explode { ids }
        | Command::Hide { ids }
        | Command::Unhide { ids }
        | Command::ReverseFaces { ids }
        | Command::SoftenEdges { ids, .. }
        | Command::SoftenByAngle { ids, .. }
        | Command::IntersectFaces { ids, .. }
        | Command::SolidUnion { ids }
        | Command::SolidIntersect { ids }
        | Command::OuterShell { ids }
        | Command::PositionTexture { ids, .. }
        | Command::SetTag { ids, .. }
        | Command::Select { ids, .. } => f(Slot::List(ids)),
        Command::MakeUnique { ids } | Command::Flip { ids, .. } => {
            if let Some(ids) = ids {
                f(Slot::List(ids));
            }
        }
        Command::OpenPath { path } => f(Slot::Path(path)),
        Command::SaveToLibrary { item, .. } => {
            if let Some(item) = item {
                f(Slot::Item(item));
            }
        }
        Command::AddDimension { a, b, .. } => {
            for anchor in [a, b] {
                if let AnchorInput::Vertex(id) = anchor {
                    f(Slot::One(id));
                }
            }
        }
        Command::AddNote { anchor, .. } => {
            if let AnchorInput::Vertex(id) = anchor {
                f(Slot::One(id));
            }
        }
        Command::AddEdgeDimension { edge, .. } | Command::AddRadialDimension { edge, .. } => {
            f(Slot::One(edge))
        }
        Command::ChangeNote { note, .. } => f(Slot::One(note)),
        Command::FlipSectionPlane { plane } | Command::ActivateSectionPlane { plane } => {
            f(Slot::One(plane))
        }
        Command::Revolve { face, .. } => f(Slot::One(face)),
        Command::SolidSubtract { target, tools } | Command::SolidTrim { target, tools } => {
            f(Slot::One(target));
            f(Slot::List(tools));
        }
        Command::SolidSplit { a, b } => {
            f(Slot::One(a));
            f(Slot::One(b));
        }
        Command::ChamferEdges {
            instance, edges, ..
        } => match instance {
            Some(instance) => {
                f(Slot::One(instance));
                // Edges of the instance's definition, not of the open context: the command
                // checks them.
                f(Slot::Path(edges));
            }
            None => f(Slot::List(edges)),
        },
        Command::CopyAlongPath { ids, path, .. } => {
            f(Slot::List(ids));
            f(Slot::List(path));
        }
        Command::FollowMe { face, path } => {
            f(Slot::One(face));
            f(Slot::List(path));
        }
        Command::ChangeMaterial { material, .. } | Command::DeleteMaterial { material } => {
            f(Slot::Material(material))
        }
        Command::Paint { ids, material, .. } => {
            f(Slot::List(ids));
            if let Some(material) = material {
                f(Slot::Material(material));
            }
        }
        Command::AddStyle { from, .. } => {
            if let Some(from) = from {
                f(Slot::Style(from));
            }
        }
        Command::ChangeStyle { style, .. } | Command::DeleteStyle { style } => {
            f(Slot::Style(style))
        }
        Command::SetStyle { style } => {
            if let Some(style) = style {
                f(Slot::Style(style));
            }
        }
        Command::AddBox { material, .. }
        | Command::AddCylinder { material, .. }
        | Command::AddTube { material, .. }
        | Command::AddCone { material, .. }
        | Command::AddPrism { material, .. }
        | Command::AddSphere { material, .. }
        | Command::AddTorus { material, .. } => {
            if let Some(material) = material {
                f(Slot::Material(material));
            }
        }
        Command::AddEdge { .. }
        | Command::AddRectangle { .. }
        | Command::AddCircle { .. }
        | Command::AddPolygon { .. }
        | Command::AddArc { .. }
        | Command::AddCurve { .. }
        | Command::AddHelix { .. }
        | Command::AddRotatedRectangle { .. }
        | Command::AddGuideLine { .. }
        | Command::AddGuidePoint { .. }
        | Command::DeleteGuides {}
        | Command::AddScreenNote { .. }
        | Command::AddSectionPlane { .. }
        | Command::DeactivateSection {}
        | Command::SetSectionDisplay { .. }
        | Command::Add3dText { .. }
        | Command::SetUnits { .. }
        | Command::CloseInstance {}
        | Command::PurgeDefinitions {}
        | Command::SetAxes { .. }
        | Command::ResetAxes {}
        | Command::UnhideLast {}
        | Command::UnhideAll {}
        | Command::AddMaterial { .. }
        | Command::AddTag { .. }
        | Command::ChangeTag { .. }
        | Command::DeleteTag { .. }
        | Command::AddScene { .. }
        | Command::UpdateScene { .. }
        | Command::ChangeScene { .. }
        | Command::MoveScene { .. }
        | Command::DeleteScene { .. }
        | Command::GoToScene { .. }
        | Command::New { .. }
        | Command::Open { .. }
        | Command::Save { .. }
        | Command::Export { .. }
        | Command::InsertLibraryItem { .. }
        | Command::Import { .. }
        | Command::Paste { .. }
        | Command::Undo { .. }
        | Command::Redo { .. }
        | Command::SetCamera { .. }
        | Command::Orbit { .. }
        | Command::Pan { .. }
        | Command::Zoom { .. }
        | Command::ZoomExtents {}
        | Command::StandardView { .. }
        | Command::SetProjection { .. }
        | Command::LookAround { .. }
        | Command::PositionCamera { .. }
        | Command::SetLighting { .. }
        | Command::SetColorByTag { .. }
        | Command::SetXRay { .. }
        | Command::SetHiddenGeometry { .. }
        | Command::SetShadows { .. }
        | Command::SetLocation { .. } => {}
    }
}

/// Refuses a reference across a `new` or `open` of the same batch: those restart the document,
/// so ids made before them name other things after. The error is the referring command's index
/// and the reason.
pub fn check_references(commands: &[BatchCommand]) -> Result<(), (usize, String)> {
    for (index, command) in commands.iter().enumerate() {
        for r in &command.refs {
            let replaced = (r.command..index).find(|&k| {
                matches!(
                    commands[k].command,
                    Command::New { .. } | Command::Open { .. }
                )
            });
            if let Some(k) = replaced {
                return Err((
                    index,
                    format!(
                        "\"{r}\" refers to command {}, before command {k} replaced the document",
                        r.command
                    ),
                ));
            }
        }
    }
    Ok(())
}
