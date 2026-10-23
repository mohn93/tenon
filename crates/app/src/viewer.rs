//! What the phone viewer adds to the window beyond looking: the parts of a model and their
//! sizes, and measuring between two points. Both are plain logic over the document, tested
//! natively; the panels that show them are in `ui::viewer_bar`.

pub mod measure;
pub mod parts;
