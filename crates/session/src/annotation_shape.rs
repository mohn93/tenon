//! Where a dimension or a note is drawn: the points its lines join and where its label sits.
//! The renderer adds arrows and the window the label text; tools pick by the same lines.

use glam::{DAffine3, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::{Circle, Dimension, EntityId, Model, Note};

/// An annotation placed in some coordinates (a mesh's own, or another by `transformed`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnnotationShape {
    /// A linear dimension: the measured points `a` and `b`, and its dimension line from
    /// `line[0]` (on `a`'s side) to `line[1]`.
    Linear {
        a: DVec3,
        b: DVec3,
        line: [DVec3; 2],
    },
    /// A radial dimension: the circle measured and the label's point; a diameter is drawn
    /// across the whole circle.
    Radial {
        circle: Circle,
        label: DVec3,
        diameter: bool,
    },
    /// A leader note: the line from the anchored point to the text's position.
    Leader { anchor: DVec3, position: DVec3 },
    /// A screen note, `at` fractions of the view's width and height from its top left.
    Screen { at: [f64; 2] },
}

impl AnnotationShape {
    /// The shape of the dimension or note `id` of `model`, in the model's coordinates. None for
    /// other ids, and for an annotation that is missing or whose anchors are.
    pub fn of(model: &Model, id: EntityId) -> Option<AnnotationShape> {
        match id {
            EntityId::Dimension(d) => match *model.dimension(d)? {
                Dimension::Linear { .. } => {
                    let (a, b) = model.dimension_points(d)?;
                    let (l0, l1) = model.dimension_line(d)?;
                    Some(AnnotationShape::Linear {
                        a,
                        b,
                        line: [l0, l1],
                    })
                }
                Dimension::Radial {
                    diameter, leader, ..
                } => {
                    let circle = model.radial_circle(d)?;
                    Some(AnnotationShape::Radial {
                        circle,
                        label: circle.center + leader,
                        diameter,
                    })
                }
            },
            EntityId::Note(n) => match model.note(n)? {
                Note::Leader { position, .. } => Some(AnnotationShape::Leader {
                    anchor: model.note_point(n)?,
                    position: *position,
                }),
                Note::Screen { at, .. } => Some(AnnotationShape::Screen { at: *at }),
            },
            EntityId::Vertex(_)
            | EntityId::Edge(_)
            | EntityId::Face(_)
            | EntityId::Instance(_)
            | EntityId::Guide(_) => None,
        }
    }

    /// The shape placed by `t`. A screen note stays where it is on the view.
    pub fn transformed(self, t: DAffine3) -> AnnotationShape {
        let at = |p: DVec3| t.transform_point3(p);
        match self {
            AnnotationShape::Linear { a, b, line } => AnnotationShape::Linear {
                a: at(a),
                b: at(b),
                line: line.map(at),
            },
            AnnotationShape::Radial {
                circle,
                label,
                diameter,
            } => AnnotationShape::Radial {
                circle: circle.transformed(t),
                label: at(label),
                diameter,
            },
            AnnotationShape::Leader { anchor, position } => AnnotationShape::Leader {
                anchor: at(anchor),
                position: at(position),
            },
            AnnotationShape::Screen { .. } => self,
        }
    }

    /// Where the label goes: the middle of a dimension line, a radial dimension's label point,
    /// a leader note's position. None for a screen note, which is placed on the view.
    pub fn label_point(&self) -> Option<DVec3> {
        match *self {
            AnnotationShape::Linear { line, .. } => Some(line[0].midpoint(line[1])),
            AnnotationShape::Radial { label, .. } => Some(label),
            AnnotationShape::Leader { position, .. } => Some(position),
            AnnotationShape::Screen { .. } => None,
        }
    }

    /// For a radial dimension: where its line meets the circle, toward the label.
    pub fn radial_point(circle: &Circle, label: DVec3) -> DVec3 {
        let n = circle.normal;
        let v = label - circle.center;
        let u = (v - n * v.dot(n))
            .try_normalize()
            .unwrap_or_else(|| n.any_orthonormal_vector());
        circle.center + u * circle.radius
    }

    /// The segments drawn for the shape, without arrows or the extension lines' overshoot:
    /// a linear dimension's extension lines (where it is offset) and dimension line; a radial
    /// dimension's line from its start through the circle to the label; a leader note's
    /// leader. None for a screen note.
    pub fn segments(&self) -> Vec<[DVec3; 2]> {
        let far = |p: DVec3, q: DVec3| p.distance(q) > LENGTH_TOL;
        match *self {
            AnnotationShape::Linear { a, b, line } => {
                let mut out = Vec::with_capacity(3);
                if far(a, line[0]) {
                    out.push([a, line[0]]);
                }
                if far(b, line[1]) {
                    out.push([b, line[1]]);
                }
                out.push(line);
                out
            }
            AnnotationShape::Radial {
                circle,
                label,
                diameter,
            } => {
                let center = circle.center;
                let near = Self::radial_point(&circle, label);
                let start = if diameter {
                    center - (near - center)
                } else {
                    center
                };
                let mut out = vec![[start, near]];
                // The label beyond the circle: the line goes on to it.
                if (label - center).length() > circle.radius + LENGTH_TOL {
                    out.push([near, label]);
                }
                out
            }
            AnnotationShape::Leader { anchor, position } => vec![[anchor, position]],
            AnnotationShape::Screen { .. } => Vec::new(),
        }
    }

    /// The points that must lie in a selection box for the shape to be inside it: the ends of
    /// its segments, and its label point.
    pub fn points(&self) -> Vec<DVec3> {
        self.segments()
            .into_iter()
            .flatten()
            .chain(self.label_point())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radial_line_runs_from_the_center_through_the_circle_to_the_label() {
        let circle = Circle {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            radius: 1.0,
        };
        let label = DVec3::new(3.0, 0.0, 2.0);
        let r = AnnotationShape::Radial {
            circle,
            label,
            diameter: false,
        };
        assert_eq!(
            r.segments(),
            vec![[DVec3::ZERO, DVec3::X], [DVec3::X, label]]
        );
        let d = AnnotationShape::Radial {
            circle,
            label: DVec3::new(0.5, 0.0, 0.0),
            diameter: true,
        };
        // A label inside the circle: just the diameter.
        assert_eq!(d.segments(), vec![[-DVec3::X, DVec3::X]]);
        let moved = r.transformed(DAffine3::from_translation(DVec3::Y));
        assert_eq!(moved.label_point(), Some(label + DVec3::Y));
    }

    #[test]
    fn a_linear_dimension_without_offset_draws_only_its_line() {
        let s = AnnotationShape::Linear {
            a: DVec3::ZERO,
            b: DVec3::X,
            line: [DVec3::ZERO, DVec3::X],
        };
        assert_eq!(s.segments(), vec![[DVec3::ZERO, DVec3::X]]);
        assert_eq!(s.label_point(), Some(DVec3::new(0.5, 0.0, 0.0)));
        assert_eq!(AnnotationShape::Screen { at: [0.5, 0.5] }.points(), vec![]);
    }
}
