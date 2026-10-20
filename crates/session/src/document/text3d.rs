//! `add_3d_text`: a text's letters as a new group of faces with holes, optionally extruded.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, ImportEdge, ImportFace, ImportGeometry, ImportTree, MeshId,
    SceneChange, check_note_text,
};

use super::{Document, MAX_COORDINATE, check_coordinates};
use crate::command::{CommandError, ErrorKind};
use crate::text3d::{MIN_TEXT_HEIGHT, Text3dReport, TextShape, shape};

/// The most characters of the text the group's name shows.
const NAME_CHARS: usize = 32;

impl Document {
    /// Draws `text` as a new group at `position` of the open context, selected; the report is
    /// kept for the outcome. Nothing changes on failure.
    pub(super) fn add_3d_text_changes(
        &mut self,
        text: &str,
        position: [f64; 3],
        height: f64,
        depth: f64,
        filled: bool,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
        check_note_text(text).map_err(|e| invalid(e.replace("a note's text", "the text")))?;
        check_coordinates(&[position])?;
        if !(height.is_finite() && (MIN_TEXT_HEIGHT..=MAX_COORDINATE).contains(&height)) {
            return Err(invalid(format!(
                "height must be from {MIN_TEXT_HEIGHT} to {MAX_COORDINATE} m"
            )));
        }
        if !(depth.is_finite() && (0.0..=MAX_COORDINATE).contains(&depth)) {
            return Err(invalid(format!(
                "depth must be from 0 (flat) to {MAX_COORDINATE} m"
            )));
        }
        if !filled && depth > 0.0 {
            return Err(invalid(
                "3D text that is not filled is flat: depth must be 0".into(),
            ));
        }
        let shape = shape(text, height);
        let Some([lo, hi]) = shape.bounds() else {
            return Err(invalid(
                "the text has no letters to draw (only spaces, or characters the font lacks)"
                    .into(),
            ));
        };
        let at = DVec3::from(position);
        let corners = [at + lo.extend(0.0), at + hi.extend(depth)];
        if corners
            .iter()
            .any(|c| !c.is_finite() || c.abs().max_element() > MAX_COORDINATE)
        {
            return Err(invalid(format!(
                "the text would reach beyond +/-{MAX_COORDINATE} m"
            )));
        }

        let mesh = self.active();
        let tree = ImportTree {
            definitions: Vec::new(),
            root: geometry(&shape, filled, depth),
        };
        let (instance, _, built) = self
            .scene
            .import_tree(mesh, &tree, DefinitionKind::Group, &group_name(text))
            .map_err(|e| invalid(e.to_string()))?;
        let definition = self
            .model()
            .instance(instance)
            .expect("the group was placed")
            .definition;
        let inner = MeshId::Definition(definition);
        let mut change = built;
        if at != DVec3::ZERO {
            match self
                .scene
                .edit(mesh, |m| Ok(m.move_instances(&[instance], at)))
            {
                Ok(moved) => change = SceneChange::join([change, moved]),
                Err(e) => {
                    self.scene.apply(&change.inverse());
                    return Err(invalid(e.to_string()));
                }
            }
        }
        let faces = self.scene.mesh(inner).map_or(0, |m| m.faces().count());
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        self.text_report = Some(Text3dReport {
            instance: EntityId::Instance(instance),
            definition,
            faces,
            skipped: shape.skipped,
        });
        Ok(change)
    }
}

/// The group's contents: each letter face with its holes, or (not `filled`) only the loops'
/// edges. With `depth` above 0 each face becomes the solid pushing it up by `depth` makes: the
/// face turned over as the base, a copy `depth` higher as the top, and a side for each side of
/// each loop, all facing out (built at once: pushing a thousand letters one by one would look
/// through the whole group each time).
fn geometry(shape: &TextShape, filled: bool, depth: f64) -> ImportGeometry {
    let mut g = ImportGeometry::default();
    let face = |loops: Vec<Vec<usize>>| ImportFace {
        loops,
        front: None,
        back: None,
        tag: None,
    };
    for letter in &shape.faces {
        let rings: Vec<&Vec<DVec2>> = std::iter::once(&letter.outer)
            .chain(&letter.holes)
            .collect();
        let mut loops = Vec::new();
        for ring in &rings {
            let start = g.points.len();
            g.points.extend(ring.iter().map(|p| p.extend(0.0)));
            let ids: Vec<usize> = (start..g.points.len()).collect();
            if !filled {
                g.edges.extend((0..ids.len()).map(|i| ImportEdge {
                    a: ids[i],
                    b: ids[(i + 1) % ids.len()],
                    soft: false,
                    tag: None,
                }));
            }
            loops.push(ids);
        }
        if !filled {
            continue;
        }
        if depth <= 0.0 {
            g.faces.push(face(loops));
            continue;
        }
        let top: Vec<Vec<usize>> = rings
            .iter()
            .map(|ring| {
                let start = g.points.len();
                g.points.extend(ring.iter().map(|p| p.extend(depth)));
                (start..g.points.len()).collect()
            })
            .collect();
        // Outer loops wind counter-clockwise and holes clockwise seen from above, so a side
        // a-b, up to the top and back, faces away from the letter.
        for (bottom, up) in loops.iter().zip(&top) {
            let n = bottom.len();
            for i in 0..n {
                let j = (i + 1) % n;
                g.faces
                    .push(face(vec![vec![bottom[i], bottom[j], up[j], up[i]]]));
            }
        }
        let base = loops
            .into_iter()
            .map(|mut l| {
                l.reverse();
                l
            })
            .collect();
        g.faces.push(face(base));
        g.faces.push(face(top));
    }
    g
}

/// "3D Text: " and the text's first `NAME_CHARS` characters, its line breaks as spaces.
fn group_name(text: &str) -> String {
    let start: String = text
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(NAME_CHARS)
        .collect();
    format!("3D Text: {}", start.trim_end())
}
