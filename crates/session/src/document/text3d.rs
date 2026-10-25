//! `add_3d_text`: a text's letters as a new group of faces with holes, optionally extruded.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, ImportDefinition, ImportEdge, ImportFace, ImportGeometry,
    ImportInstance, ImportTree, InstanceId, KernelError, MeshId, SceneChange, TreeReport,
    check_note_text,
};

use super::{Document, MAX_COORDINATE, check_coordinates};
use crate::command::{CommandError, ErrorKind};
use crate::text3d::{MIN_TEXT_HEIGHT, SkippedCharacter, Text3dReport, TextFace, TextShape, shape};

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
        let (instance, built, faces, left_out) =
            self.import_text(mesh, &shape, filled, depth, &group_name(text))?;
        let definition = self
            .model()
            .instance(instance)
            .expect("the group was placed")
            .definition;
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
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        self.text_report = Some(Text3dReport {
            instance: EntityId::Instance(instance),
            definition,
            faces,
            skipped: shape.skipped.into_iter().chain(left_out).collect(),
        });
        self.text_report
            .as_mut()
            .expect("just set")
            .skipped
            .sort_by_key(|k| k.index);
        Ok(change)
    }

    /// Builds the shape's letters into a new group of `mesh` named `name`: the instance, the
    /// change, the faces made and the letters left out. A letter comes out whole or not at
    /// all: when the kernel skips, splits or strips a face of the import, the import is taken
    /// back, each letter is tried alone, and the group is built again without those that
    /// failed (reported with why). Fails, changing nothing, when no letter can be built.
    fn import_text(
        &mut self,
        mesh: MeshId,
        shape: &TextShape,
        filled: bool,
        depth: f64,
        name: &str,
    ) -> Result<(InstanceId, SceneChange, usize, Vec<SkippedCharacter>), CommandError> {
        let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
        let letters: BTreeSet<usize> = shape.faces.iter().map(|f| f.index).collect();
        let first = self.build_text(mesh, shape, &BTreeSet::new(), filled, depth, name);
        let mut bad = BTreeSet::new();
        match first {
            Ok((instance, report, change)) if whole(&report) => {
                return Ok((instance, change, report.faces, Vec::new()));
            }
            Ok((_, _, change)) => self.scene.apply(&change.inverse()),
            Err(e) => return Err(invalid(e.to_string())),
        }
        for &letter in &letters {
            let others = letters.iter().copied().filter(|&l| l != letter).collect();
            match self.build_text(mesh, shape, &others, filled, depth, name) {
                Ok((_, report, change)) => {
                    self.scene.apply(&change.inverse());
                    if !whole(&report) {
                        bad.insert(letter);
                    }
                }
                Err(_) => {
                    bad.insert(letter);
                }
            }
        }
        let built = if bad.len() < letters.len() {
            self.build_text(mesh, shape, &bad, filled, depth, name)
        } else {
            Err(KernelError::NothingToImport)
        };
        match built {
            Ok((instance, report, change)) if whole(&report) => {
                let left_out = bad
                    .iter()
                    .filter_map(|&i| shape.faces.iter().find(|f| f.index == i))
                    .map(|f| SkippedCharacter {
                        character: f.character.to_string(),
                        index: f.index,
                        reason: "its faces could not be built (a crossing or degenerate outline)"
                            .into(),
                    })
                    .collect();
                Ok((instance, change, report.faces, left_out))
            }
            Ok((_, _, change)) => {
                self.scene.apply(&change.inverse());
                Err(invalid("the letters could not be built whole".into()))
            }
            Err(_) => Err(invalid(
                "none of the letters could be built as faces".into(),
            )),
        }
    }

    /// Imports the shape's letters but those of `exclude`: letters that touch another letter
    /// each in a group of their own (so no points are welded across letters), the rest in the
    /// main group.
    fn build_text(
        &mut self,
        mesh: MeshId,
        shape: &TextShape,
        exclude: &BTreeSet<usize>,
        filled: bool,
        depth: f64,
        name: &str,
    ) -> Result<(InstanceId, TreeReport, SceneChange), KernelError> {
        let mut tree = ImportTree::default();
        let mut together: Vec<&TextFace> = Vec::new();
        let mut rest = shape.faces.iter().filter(|f| !exclude.contains(&f.index));
        let mut next = rest.next();
        while let Some(first) = next {
            let mut letter = vec![first];
            next = rest.next();
            while let Some(f) = next.filter(|f| f.index == first.index) {
                letter.push(f);
                next = rest.next();
            }
            if shape.touching.contains(&first.index) {
                tree.root.instances.push(ImportInstance {
                    definition: tree.definitions.len(),
                    transform: glam::DAffine3::IDENTITY,
                    material: None,
                    tag: None,
                    hidden: false,
                });
                tree.definitions.push(ImportDefinition {
                    name: "Letter".into(),
                    kind: DefinitionKind::Group,
                    geometry: geometry(&letter, filled, depth),
                });
            } else {
                together.extend(letter);
            }
        }
        let main = geometry(&together, filled, depth);
        tree.root.points = main.points;
        tree.root.faces = main.faces;
        tree.root.edges = main.edges;
        self.scene
            .import_tree(mesh, &tree, DefinitionKind::Group, name)
    }
}

/// True when an import made every face as given.
fn whole(report: &TreeReport) -> bool {
    report.skipped == 0 && report.split == 0 && report.holes_dropped == 0
}

/// The group's contents: each letter face with its holes, or (not `filled`) only the loops'
/// edges. With `depth` above 0 each face becomes the solid pushing it up by `depth` makes: the
/// face turned over as the base, a copy `depth` higher as the top, and a side for each side of
/// each loop, all facing out (built at once: pushing a thousand letters one by one would look
/// through the whole group each time).
fn geometry(letters: &[&TextFace], filled: bool, depth: f64) -> ImportGeometry {
    let mut g = ImportGeometry::default();
    let face = |loops: Vec<Vec<usize>>| ImportFace {
        loops,
        front: None,
        back: None,
        texture: None,
        tag: None,
        hidden: false,
    };
    for letter in letters {
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
                    hidden: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text3d::TextFace;

    fn square(x: f64) -> Vec<DVec2> {
        vec![
            DVec2::new(x, 0.0),
            DVec2::new(x + 1.0, 0.0),
            DVec2::new(x + 1.0, 1.0),
            DVec2::new(x, 1.0),
        ]
    }

    fn face(index: usize, outer: Vec<DVec2>) -> TextFace {
        TextFace {
            index,
            character: 'x',
            outer,
            holes: Vec::new(),
        }
    }

    /// A glyph whose outline crosses itself is left out whole and reported; the good glyph
    /// stays and nothing of the bad one is left in the scene.
    #[test]
    fn a_glyph_the_kernel_cannot_build_whole_is_left_out_and_reported() {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        let bowtie = vec![
            DVec2::new(3.0, 0.0),
            DVec2::new(4.0, 1.0),
            DVec2::new(4.0, 0.0),
            DVec2::new(3.0, 1.0),
        ];
        let shape = TextShape {
            faces: vec![face(0, square(0.0)), face(1, bowtie)],
            touching: BTreeSet::new(),
            skipped: Vec::new(),
        };
        let mesh = d.active();
        let (_, change, faces, skipped) = d
            .import_text(mesh, &shape, true, 0.0, "t")
            .expect("the good glyph is built");
        assert_eq!(faces, 1);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].index, 1);
        assert!(!change.is_empty());
        d.scene.validate().unwrap();
        let defs = d.scene.definitions().count();
        assert_eq!(defs, 1, "only the good glyph's group is left");
        let only_bad = TextShape {
            faces: vec![face(0, shape.faces[1].outer.clone())],
            touching: BTreeSet::new(),
            skipped: Vec::new(),
        };
        let before = d.scene.definitions().count();
        assert!(d.import_text(mesh, &only_bad, true, 0.0, "t").is_err());
        assert_eq!(d.scene.definitions().count(), before, "nothing half-built");
    }
}
