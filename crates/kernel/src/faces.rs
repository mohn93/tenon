//! Automatic face creation (implemented in Task 5).

use crate::ids::EdgeId;
use crate::model::Model;

impl Model {
    /// Creates faces for closed coplanar loops that use at least one of `new_edges`.
    pub(crate) fn create_faces(&mut self, _new_edges: &[EdgeId]) {}
}
