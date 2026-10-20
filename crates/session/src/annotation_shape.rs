//! Where a dimension or a note is drawn: the points its lines join and where its label sits.
//! The renderer adds arrows and the window the label text; tools pick by the same lines.

use glam::{DAffine3, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::{Circle, Dimension, EntityId, Model, Note};

use crate::units::{Units, format_length_in};

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
        let mut out = Vec::with_capacity(3);
        self.for_each_segment(|s| out.push(s));
        out
    }

    /// `segments`, each given to `f` as it is worked out (nothing is collected).
    pub fn for_each_segment(&self, mut f: impl FnMut([DVec3; 2])) {
        let far = |p: DVec3, q: DVec3| p.distance(q) > LENGTH_TOL;
        match *self {
            AnnotationShape::Linear { a, b, line } => {
                if far(a, line[0]) {
                    f([a, line[0]]);
                }
                if far(b, line[1]) {
                    f([b, line[1]]);
                }
                f(line);
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
                f([start, near]);
                // The label beyond the circle (measured in its plane, as `near` is): the line
                // goes on to it.
                let v = label - center;
                let across = (v - circle.normal * v.dot(circle.normal)).length();
                if across > circle.radius + LENGTH_TOL {
                    f([near, label]);
                }
            }
            AnnotationShape::Leader { anchor, position } => f([anchor, position]),
            AnnotationShape::Screen { .. } => {}
        }
    }

    /// A dimension's label as the shape is placed, in `units`: its length ("1.5 m"), or for a
    /// radial dimension "R 1.5 m" (radius) or "Ø 3 m" (diameter). A dimension inside a scaled
    /// group measures what is drawn, not its own coordinates. None for notes.
    pub fn dimension_label(&self, units: &Units) -> Option<String> {
        match *self {
            AnnotationShape::Linear { a, b, .. } => Some(format_length_in(a.distance(b), units)),
            AnnotationShape::Radial {
                circle, diameter, ..
            } => {
                let value = if diameter {
                    2.0 * circle.radius
                } else {
                    circle.radius
                };
                let prefix = if diameter { "Ø" } else { "R" };
                Some(format!("{prefix} {}", format_length_in(value, units)))
            }
            AnnotationShape::Leader { .. } | AnnotationShape::Screen { .. } => None,
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

    #[test]
    fn a_label_off_the_circles_plane_but_within_its_radius_ends_the_line_at_the_circle() {
        let circle = Circle {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            radius: 1.0,
        };
        // 0.5 across the plane, 2 above it: inside the circle as seen along its normal.
        let r = AnnotationShape::Radial {
            circle,
            label: DVec3::new(0.5, 0.0, 2.0),
            diameter: false,
        };
        assert_eq!(r.segments(), vec![[DVec3::ZERO, DVec3::X]]);
    }

    #[test]
    fn dimension_labels_measure_the_placed_shape() {
        let units = Units::default();
        let s = AnnotationShape::Linear {
            a: DVec3::ZERO,
            b: DVec3::X,
            line: [DVec3::ZERO, DVec3::X],
        };
        let scaled = s.transformed(DAffine3::from_scale(DVec3::splat(2.0)));
        assert_eq!(scaled.dimension_label(&units).as_deref(), Some("2 m"));
        let circle = Circle {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            radius: 1.5,
        };
        let r = AnnotationShape::Radial {
            circle,
            label: DVec3::new(3.0, 0.0, 0.0),
            diameter: true,
        };
        assert_eq!(r.dimension_label(&units).as_deref(), Some("Ø 3 m"));
        let leader = AnnotationShape::Leader {
            anchor: DVec3::ZERO,
            position: DVec3::X,
        };
        assert_eq!(leader.dimension_label(&units), None);
    }
}
