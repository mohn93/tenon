//! Clip text: what Copy puts on the clipboard and Paste reads back. The header line
//! [`HEADER`], then base64 (standard alphabet) of a zip archive holding `model.json`, the
//! clip as a `.skr` file writes its document (with textures as entries, as in `.skr`), and
//! `clip.json` `{ "base": [x, y, z], "source": "<token>" }`: the base point (the minimum corner
//! of the clip's bounding box, in world coordinates) and the token of the document it was
//! copied from.

use base64::Engine;
use glam::{DAffine3, DVec3};
use kernel::{MeshId, Scene};
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
    let compact: Vec<u8> = body.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if compact.len() > encoded_limit {
        return Err(too_large());
    }
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
    if info.source.len() > MAX_TOKEN_LEN {
        return Err(damaged("its source token is too long".into()));
    }
    Ok(Clip {
        scene,
        base,
        source: info.source,
    })
}

/// The Paste tool's preview of clip text: its base point and edges. Fails as paste would.
pub fn outline(text: &str) -> Result<ClipOutline, CommandError> {
    let clip = decode(text)?;
    let mut segments = Vec::new();
    add_segments(&clip.scene, MeshId::Root, DAffine3::IDENTITY, &mut segments);
    Ok(ClipOutline {
        base: clip.base,
        segments,
    })
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

/// The minimum corner of the box around the root's vertices, instances (or their positions,
/// when empty) and guides' anchors; the origin for an empty root.
fn base_point(scene: &Scene) -> DVec3 {
    let root = scene.mesh(MeshId::Root).expect("a scene has a root");
    let mut points: Vec<DVec3> = root.vertices().map(|(_, v)| v.pos).collect();
    for (_, i) in root.instances() {
        match scene.instance_bounds(i) {
            Some([lo, hi]) => points.extend([lo, hi]),
            None => points.push(i.transform.translation),
        }
        points.extend(scene.instance_guide_anchors(i));
    }
    points.extend(root.guides().map(|(_, g)| g.anchor()));
    points
        .into_iter()
        .reduce(|a, b| a.min(b))
        .unwrap_or(DVec3::ZERO)
}
