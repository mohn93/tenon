//! Clip text: what Copy puts on the clipboard and Paste reads back. The header line
//! [`HEADER`], then base64 (standard alphabet) of a zip archive holding `model.json`, the
//! clip as a `.skr` file writes its document (with textures as entries, as in `.skr`), and
//! `clip.json` `{ "base": [x, y, z], "source": "<token>" }`: the base point (the minimum corner
//! of the clip's bounding box, in world coordinates) and the token of the document it was
//! copied from.

use base64::Engine;
use glam::{DAffine3, DVec3};
use std::collections::HashMap;

use kernel::{DefinitionId, MeshId, Scene};
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::document::MAX_COORDINATE;
use crate::file::{self, ViewLook};

/// The first line of every clip text.
pub const HEADER: &str = "sketchup-alt clip 1";
/// The most bytes a clip's archive may hold once its base64 is decoded.
pub const MAX_CLIP_BYTES: usize = 64 * 1024 * 1024;
/// The most segments [`outline`] returns.
pub const MAX_OUTLINE_SEGMENTS: usize = 20_000;
/// The most instances a clip may place, counting a nested instance once for every placement
/// of what holds it (a component of 10 placed 10 times counts 110). Larger clips are refused,
/// so walking a clip's placements stays bounded whatever text is pasted.
pub const MAX_CLIP_INSTANCES: u64 = 100_000;
/// The most levels of instances a clip may nest, as for imports. Deeper clips are refused,
/// so walking a clip's nesting stays shallow whatever text is pasted.
pub const MAX_CLIP_DEPTH: usize = kernel::MAX_IMPORT_DEPTH;
/// The most faces a clip may show, counting a nested face once for every placement of what
/// holds it, as for imports.
pub const MAX_CLIP_PLACED_FACES: u64 = kernel::MAX_IMPORT_PLACED_FACES;
/// The most instances [`outline`] walks into.
const MAX_OUTLINE_VISITS: usize = 100_000;
/// The archive entry holding the clip's model, as `.skr` writes its document.
const MODEL_ENTRY: &str = "model.json";
/// The archive entry holding the base point and the source token.
const CLIP_ENTRY: &str = "clip.json";
/// The largest `clip.json` read.
const MAX_INFO_BYTES: u64 = 64 * 1024;
/// The longest source token read.
const MAX_TOKEN_LEN: usize = 256;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Info {
    base: [f64; 3],
    source: String,
}

/// A clip read back: its scene (in world coordinates), base point and source token.
pub(crate) struct Clip {
    pub scene: Scene,
    pub base: DVec3,
    pub source: String,
}

/// What the Paste tool draws while it follows the cursor: the clip's edges in world
/// coordinates (the root's, and those inside its instances, at most
/// [`MAX_OUTLINE_SEGMENTS`]) and its base point, which the cursor carries.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipOutline {
    pub base: DVec3,
    pub segments: Vec<[DVec3; 2]>,
}

/// The clip text of `scene` (a clip in world coordinates) copied from the document with
/// `token`. The camera and look only fill the `.skr` document's view; paste ignores them.
pub(crate) fn encode(
    scene: &Scene,
    token: &str,
    camera: &Camera,
    look: ViewLook,
) -> Result<String, String> {
    let info = Info {
        base: base_point(scene).to_array(),
        source: token.to_string(),
    };
    let info = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
    let bytes = file::write_bytes(scene, camera, look, MODEL_ENTRY, &[(CLIP_ENTRY, &info)])?;
    if bytes.len() > MAX_CLIP_BYTES {
        return Err(format!(
            "the selection is too large to copy: over {MAX_CLIP_BYTES} bytes"
        ));
    }
    Ok(format!(
        "{HEADER}\n{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Reads clip text, with every check opening a `.skr` file makes. Anything wrong is
/// `invalid_input`.
pub(crate) fn decode(text: &str) -> Result<Clip, CommandError> {
    let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
    let body = text
        .strip_prefix(HEADER)
        .and_then(|rest| {
            rest.strip_prefix('\n')
                .or_else(|| rest.strip_prefix("\r\n"))
        })
        .ok_or_else(|| invalid("not a sketchup-alt clip".into()))?;
    // Base64 takes 4 characters for 3 bytes; line breaks a clipboard may add are allowed.
    let encoded_limit = MAX_CLIP_BYTES.div_ceil(3) * 4;
    let too_large = || {
        invalid(format!(
            "the clip is too large: over {MAX_CLIP_BYTES} bytes"
        ))
    };
    if body.len() > encoded_limit * 2 {
        return Err(too_large());
    }
    // Counted before the copy is made, so an over-long body is never copied.
    let length = body.bytes().filter(|b| !b.is_ascii_whitespace()).count();
    if length > encoded_limit {
        return Err(too_large());
    }
    let mut compact = Vec::with_capacity(length);
    compact.extend(body.bytes().filter(|b| !b.is_ascii_whitespace()));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&compact)
        .map_err(|_| invalid("the clip is damaged: its body is not valid base64".into()))?;
    if bytes.len() > MAX_CLIP_BYTES {
        return Err(too_large());
    }
    let damaged = |e: String| invalid(format!("the clip is damaged: {e}"));
    let (scene, _, extras) =
        file::read_bytes(&bytes, MODEL_ENTRY, &[(CLIP_ENTRY, MAX_INFO_BYTES)]).map_err(damaged)?;
    let info: Info = serde_json::from_slice(&extras[0])
        .map_err(|_| damaged(format!("{CLIP_ENTRY} is not valid")))?;
    let base = DVec3::from(info.base);
    if !base.is_finite() || base.abs().max_element() > MAX_COORDINATE {
        return Err(damaged(format!(
            "its base point is not finite or beyond +/-{MAX_COORDINATE} m"
        )));
    }
    check_caps(&scene).map_err(|e| match e {
        Refused::Damaged(m) => damaged(m),
        Refused::Invalid(m) => invalid(m),
    })?;
    if info.source.len() > MAX_TOKEN_LEN {
        return Err(damaged("its source token is too long".into()));
    }
    Ok(Clip {
        scene,
        base,
        source: info.source,
    })
}

/// Why a scene cannot be a clip.
enum Refused {
    Damaged(String),
    Invalid(String),
}

/// The checks every clip passes before it is pasted: its definitions form a tree of
/// placements, and it nests, places and shows no more than the caps allow.
fn check_caps(scene: &Scene) -> Result<(), Refused> {
    let tally = tally(scene).ok_or_else(|| {
        Refused::Damaged("its definitions do not form a tree of placements".into())
    })?;
    if tally.depth > MAX_CLIP_DEPTH {
        return Err(Refused::Invalid(format!(
            "the clip's instances are nested more than {MAX_CLIP_DEPTH} deep"
        )));
    }
    if tally.instances > MAX_CLIP_INSTANCES {
        return Err(Refused::Invalid(format!(
            "the clip places more than {MAX_CLIP_INSTANCES} instances, nested ones counted \
             once per placement"
        )));
    }
    if tally.faces > MAX_CLIP_PLACED_FACES {
        return Err(Refused::Invalid(format!(
            "the clip shows more than {MAX_CLIP_PLACED_FACES} faces, nested ones counted once \
             per placement"
        )));
    }
    Ok(())
}

/// The clip caps (nesting depth, expanded instances, placed faces) for a library item's scene;
/// every walk of an item's placements comes after this.
pub(crate) fn check_item_caps(scene: &Scene) -> Result<(), CommandError> {
    check_caps(scene).map_err(|e| match e {
        Refused::Damaged(m) => {
            CommandError::new(ErrorKind::InvalidInput, format!("the item is damaged: {m}"))
        }
        Refused::Invalid(m) => {
            CommandError::new(ErrorKind::InvalidInput, m.replace("the clip", "the item"))
        }
    })
}

/// A clip of `scene` (in world coordinates) taken as from the document `source`, with the
/// caps clip text meets; the base point is the minimum corner of its box. A library item's
/// scene is made a clip this way, with a token no document has, so pasting it always makes
/// new definitions.
pub(crate) fn from_scene(scene: Scene, source: String) -> Result<Clip, CommandError> {
    check_item_caps(&scene)?;
    let base = base_point(&scene);
    if !base.is_finite() || base.abs().max_element() > MAX_COORDINATE {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("the item is beyond +/-{MAX_COORDINATE} m"),
        ));
    }
    Ok(Clip {
        scene,
        base,
        source,
    })
}

/// The Paste tool's preview of clip text: its base point and edges. Fails as paste would.
pub fn outline(text: &str) -> Result<ClipOutline, CommandError> {
    Ok(outline_of(&decode(text)?))
}

/// The preview of `clip`.
pub(crate) fn outline_of(clip: &Clip) -> ClipOutline {
    let mut segments = Vec::new();
    add_segments(&clip.scene, MeshId::Root, DAffine3::IDENTITY, &mut segments);
    ClipOutline {
        base: clip.base,
        segments,
    }
}

/// The edges of mesh `mesh` and of the instances in it, placed by `to_world`: at most
/// [`MAX_OUTLINE_SEGMENTS`], after visiting at most [`MAX_OUTLINE_VISITS`] instances (a clip
/// nesting many empty instances stops there).
fn add_segments(scene: &Scene, mesh: MeshId, to_world: DAffine3, out: &mut Vec<[DVec3; 2]>) {
    let mut stack = vec![(mesh, to_world)];
    let mut visits = 0;
    while let Some((mesh, to_world)) = stack.pop() {
        let Some(m) = scene.mesh(mesh) else { continue };
        for (_, e) in m.edges() {
            if out.len() >= MAX_OUTLINE_SEGMENTS {
                return;
            }
            if let (Some(a), Some(b)) = (m.position(e.v[0]), m.position(e.v[1])) {
                out.push([to_world.transform_point3(a), to_world.transform_point3(b)]);
            }
        }
        for (_, i) in m.instances() {
            visits += 1;
            if visits > MAX_OUTLINE_VISITS {
                return;
            }
            stack.push((MeshId::Definition(i.definition), to_world * i.transform));
        }
    }
}

/// The minimum corner of the box around the root's vertices, guides' anchors, annotation points
/// and instances (their definitions' boxes placed, or their positions when empty); the origin
/// for an empty root.
fn base_point(scene: &Scene) -> DVec3 {
    let root = scene.mesh(MeshId::Root).expect("a scene has a root");
    mesh_box(root, &definition_boxes(scene)).map_or(DVec3::ZERO, |[lo, _]| lo)
}

/// The clip's definitions, each after every definition it places; None when one places
/// itself (directly or deeper) or places a definition the clip lacks.
fn children_first(scene: &Scene) -> Option<Vec<DefinitionId>> {
    // 0: not seen, 1: on the path, 2: done.
    let mut state: HashMap<DefinitionId, u8> = HashMap::new();
    let mut order = Vec::new();
    for (start, _) in scene.definitions() {
        if state.contains_key(&start) {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((id, children_done)) = stack.pop() {
            if children_done {
                state.insert(id, 2);
                order.push(id);
                continue;
            }
            match state.get(&id) {
                Some(2) => continue,
                Some(_) => return None,
                None => {}
            }
            state.insert(id, 1);
            stack.push((id, true));
            for (_, i) in scene.definition(id)?.mesh.instances() {
                match state.get(&i.definition) {
                    Some(2) => {}
                    Some(_) => return None,
                    None => stack.push((i.definition, false)),
                }
            }
        }
    }
    Some(order)
}

/// What a clip places, nested placements counted.
struct Tally {
    /// Instances placed, nested ones counted once per placement (saturating).
    instances: u64,
    /// Faces shown, nested ones counted once per placement (saturating).
    faces: u64,
    /// The most levels of instances nested from the root (0 when it places none).
    depth: usize,
}

/// The clip's [`Tally`], worked out once per definition; None when its definitions do not
/// form a tree of placements.
fn tally(scene: &Scene) -> Option<Tally> {
    let mut tallies: HashMap<DefinitionId, Tally> = HashMap::new();
    let of = |m: &kernel::Model, tallies: &HashMap<DefinitionId, Tally>| {
        let mut t = Tally {
            instances: 0,
            faces: m.faces().count() as u64,
            depth: 0,
        };
        for (_, i) in m.instances() {
            let below = tallies.get(&i.definition);
            t.instances = t
                .instances
                .saturating_add(1)
                .saturating_add(below.map_or(0, |b| b.instances));
            t.faces = t.faces.saturating_add(below.map_or(0, |b| b.faces));
            t.depth = t.depth.max(1 + below.map_or(0, |b| b.depth));
        }
        t
    };
    for id in children_first(scene)? {
        let t = of(&scene.definition(id)?.mesh, &tallies);
        tallies.insert(id, t);
    }
    Some(of(scene.mesh(MeshId::Root)?, &tallies))
}

/// The box around each definition's vertices, guides' anchors, annotation points and placed
/// instances' boxes,
/// in its own coordinates (None when it holds none of these), worked out once per definition.
/// Boxes of instances are the boxes around their definitions' boxes placed, as the kernel's
/// `bounds` takes them.
pub(crate) fn definition_boxes(scene: &Scene) -> HashMap<DefinitionId, Option<[DVec3; 2]>> {
    let mut boxes = HashMap::new();
    for id in children_first(scene).unwrap_or_default() {
        let Some(d) = scene.definition(id) else {
            continue;
        };
        let b = mesh_box(&d.mesh, &boxes);
        boxes.insert(id, b);
    }
    boxes
}

/// The box around `m`'s vertices, guides' anchors, annotation points and instances (by
/// `boxes`, or their positions when empty).
fn mesh_box(
    m: &kernel::Model,
    boxes: &HashMap<DefinitionId, Option<[DVec3; 2]>>,
) -> Option<[DVec3; 2]> {
    let mut points: Vec<DVec3> = m.vertices().map(|(_, v)| v.pos).collect();
    points.extend(m.guides().map(|(_, g)| g.anchor()));
    points.extend(m.annotation_points(DAffine3::IDENTITY));
    points.extend(m.sections().map(|(_, p)| p.point));
    for (_, i) in m.instances() {
        match boxes.get(&i.definition).copied().flatten() {
            Some(b) => points.extend(corners(i.transform, b)),
            None => points.push(i.transform.translation),
        }
    }
    let lo = points.iter().copied().reduce(DVec3::min)?;
    let hi = points.iter().copied().reduce(DVec3::max)?;
    Some([lo, hi])
}

/// The eight corners of box `b` placed by `t`.
pub(crate) fn corners(t: DAffine3, [lo, hi]: [DVec3; 2]) -> [DVec3; 8] {
    std::array::from_fn(|k| {
        let pick = |bit: usize, axis: usize| if k & bit == 0 { lo[axis] } else { hi[axis] };
        t.transform_point3(DVec3::new(pick(1, 0), pick(2, 1), pick(4, 2)))
    })
}
