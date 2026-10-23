//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod arc;
pub mod axes;
pub mod circle;
pub mod dimension;
pub mod erase;
pub mod follow_me;
pub mod freehand;
pub mod inference;
pub mod line;
pub mod move_tool;
pub mod offset;
pub mod overlay;
pub mod paint;
pub mod paste;
pub mod polygon;
pub mod probe;
pub mod protractor;
pub mod push_pull;
pub mod rectangle;
pub mod rotate;
pub mod rotated_rectangle;
pub mod scale;
pub mod scenery;
pub mod section;
pub mod select;
pub mod simplify;
pub mod tape;
#[cfg(test)]
mod testing;
pub mod text;
pub mod tool;
pub mod vcb;

pub use arc::ArcTool;
pub use axes::AxesTool;
pub use circle::CircleTool;
pub use dimension::DimensionTool;
pub use erase::EraseTool;
pub use follow_me::FollowMeTool;
pub use freehand::FreehandTool;
pub use inference::{
    Axis, Guides, Inference, Involved, Lock, SnapKind, annotation_under_where, edge_under,
    edge_under_in, edge_under_where, face_under, face_under_in, guide_under_where, infer,
    infer_detail, infer_in, infer_with, label_under,
};
pub use line::LineTool;
pub use move_tool::MoveTool;
pub use offset::OffsetTool;
pub use overlay::{Rgba, ScreenLine, Shape, Within};
pub use paint::PaintTool;
pub use paste::{PasteTool, TextSpec};
pub use polygon::PolygonTool;
pub use probe::{Hit, Reach, pick};
pub use protractor::ProtractorTool;
pub use push_pull::PushPullTool;
pub use rectangle::RectangleTool;
pub use rotate::RotateTool;
pub use rotated_rectangle::RotatedRectangleTool;
pub use scale::ScaleTool;
pub use scenery::{Label, Layout, Owner, Part, Placed, Scenery, Slab};
pub use section::SectionPlaneTool;
pub use select::{SelectTool, select_box};
pub use tape::TapeMeasureTool;
pub use text::TextTool;
pub use tool::{
    NoteRequest, Phase, Remembered, Solid, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput,
};
