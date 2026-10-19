//! Batch commands that refer to ids made by earlier commands of the same batch.
//!
//! In a batch, an id field may hold a reference instead of an id: `"$N:KI"` is the I-th id of
//! kind K (`v e f i g` from `changes.created`, `m` from `changes.materials`) in command N's
//! outcome, `"$N:K*"` all of that kind and `"$N:*"` every edge, face, instance and guide
//! command N created (not vertices: `"$N:v*"` names those). `*` references expand in place in
//! id lists only.
//!
//! A parsed command holds stand-in ids (numbers from `STAND_IN_BASE` up) where its references
//! were; they are replaced by real ids when the command's turn comes.

use std::fmt;

use kernel::{EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId};
use serde_json::Value;

use crate::command::{Command, CommandError, ErrorKind, Outcome};

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
    Material,
}

impl RefKind {
    fn from_letter(c: char) -> Option<RefKind> {
        Some(match c {
            'v' => RefKind::Vertex,
            'e' => RefKind::Edge,
            'f' => RefKind::Face,
            'i' => RefKind::Instance,
            'g' => RefKind::Guide,
            'm' => RefKind::Material,
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
            RefKind::Material => 'm',
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
            RefKind::Material if one => "material",
            RefKind::Material => "materials",
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
        )
    }
}

/// A reference to ids an earlier command of the batch made: `$<command>:<kind><index>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchRef {
    pub command: usize,
    /// None: `*`, every edge, face, instance and guide the command created (not vertices).
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
    /// Reads `^\$(0|[1-9][0-9]*):(([vefigm])(0|[1-9][0-9]*|\*)|\*)$`.
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

    /// The entity ids this reference stands for, in order.
    fn entities(&self, done: &[Outcome]) -> Result<Vec<EntityId>, CommandError> {
        let created = &self.outcome(done)?.changes.created;
        match self.kind {
            None => Ok(created
                .iter()
                .filter(|id| !matches!(id, EntityId::Vertex(_)))
                .copied()
                .collect()),
            Some(kind) => {
                let of_kind: Vec<EntityId> = created
                    .iter()
                    .filter(|id| kind.matches(id))
                    .copied()
                    .collect();
                self.pick(&of_kind, kind)
            }
        }
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

/// The top-level fields where references are recognised; `path` only in `open_path`.
fn is_reference_key(key: &str, command_type: Option<&str>) -> bool {
    matches!(key, "ids" | "face" | "id" | "material")
        || (key == "path" && command_type == Some("open_path"))
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
                let material = key == "material";
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
                Slot::List(ids) => ids.iter().filter(|id| is_stand_in(id)).count(),
                Slot::One(id) => usize::from(is_stand_in(id)),
                Slot::Material(m) => usize::from(m.0 >= STAND_IN_BASE),
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
    pub fn resolve(&self, done: &[Outcome]) -> Result<Command, CommandError> {
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
                Slot::List(ids) => self.resolve_list(ids, done),
                Slot::One(id) => self.resolve_one(id, done),
                Slot::Material(m) => self.resolve_material(m, done),
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

    fn resolve_list(&self, ids: &mut Vec<EntityId>, done: &[Outcome]) -> Result<(), CommandError> {
        let mut resolved = Vec::with_capacity(ids.len());
        for id in ids.iter() {
            if is_stand_in(id) {
                resolved.extend(self.reference(number(id)).entities(done)?);
            } else {
                resolved.push(*id);
            }
        }
        *ids = resolved;
        Ok(())
    }

    fn resolve_one(&self, id: &mut EntityId, done: &[Outcome]) -> Result<(), CommandError> {
        if is_stand_in(id) {
            // Scalar fields hold only single-id references, which resolve to one id or fail.
            *id = self.reference(number(id)).entities(done)?[0];
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

/// Replaces a reference in a reference field's string by a stand-in id, recording it in
/// `refs`. Other strings stay as they are.
fn replace(
    text: &mut String,
    in_list: bool,
    material: bool,
    index: usize,
    refs: &mut Vec<BatchRef>,
    literal_stand_in: &mut bool,
) -> Result<(), String> {
    if !text.starts_with('$') {
        let literal = if material {
            text.parse::<MaterialId>().map(|m| m.0).ok()
        } else {
            text.parse::<EntityId>().map(|id| number(&id)).ok()
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
    let is_material = r.kind == Some(RefKind::Material);
    if material && !is_material {
        return Err(format!("{text:?} is not a material"));
    }
    if !material && is_material {
        return Err(format!("{text:?} is a material, not an entity"));
    }
    if r.index.is_none() && is_material {
        return Err(format!("{text:?} must name one material"));
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
        | EntityId::Guide(GuideId(n)) => *n,
    }
}

fn is_stand_in(id: &EntityId) -> bool {
    number(id) >= STAND_IN_BASE
}

/// An id field of a command.
enum Slot<'a> {
    List(&'a mut Vec<EntityId>),
    One(&'a mut EntityId),
    Material(&'a mut MaterialId),
}

/// Calls `f` on every id field of `command`. Every command is listed, so a new one must say
/// where its ids are.
fn id_slots(command: &mut Command, f: &mut dyn FnMut(Slot<'_>)) {
    match command {
        Command::PushPull { face, .. } => f(Slot::One(face)),
        Command::OpenInstance { id } => f(Slot::One(id)),
        Command::Erase { ids }
        | Command::Move { ids, .. }
        | Command::Copy { ids, .. }
        | Command::Rotate { ids, .. }
        | Command::Scale { ids, .. }
        | Command::MakeGroup { ids }
        | Command::MakeComponent { ids, .. }
        | Command::Explode { ids }
        | Command::SoftenEdges { ids, .. }
        | Command::SoftenByAngle { ids, .. }
        | Command::PositionTexture { ids, .. }
        | Command::Select { ids, .. } => f(Slot::List(ids)),
        Command::OpenPath { path } => f(Slot::List(path)),
        Command::ChangeMaterial { material, .. } | Command::DeleteMaterial { material } => {
            f(Slot::Material(material))
        }
        Command::Paint { ids, material, .. } => {
            f(Slot::List(ids));
            if let Some(material) = material {
                f(Slot::Material(material));
            }
        }
        Command::AddEdge { .. }
        | Command::AddRectangle { .. }
        | Command::AddCircle { .. }
        | Command::AddPolygon { .. }
        | Command::AddArc { .. }
        | Command::AddGuideLine { .. }
        | Command::AddGuidePoint { .. }
        | Command::DeleteGuides {}
        | Command::CloseInstance {}
        | Command::RenameDefinition { .. }
        | Command::PurgeDefinitions {}
        | Command::AddMaterial { .. }
        | Command::New { .. }
        | Command::Open { .. }
        | Command::Save { .. }
        | Command::Export { .. }
        | Command::Import { .. }
        | Command::Undo { .. }
        | Command::Redo { .. }
        | Command::SetCamera { .. }
        | Command::Orbit { .. }
        | Command::Pan { .. }
        | Command::Zoom { .. }
        | Command::ZoomExtents {}
        | Command::StandardView { .. }
        | Command::SetProjection { .. }
        | Command::SetLighting { .. } => {}
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
