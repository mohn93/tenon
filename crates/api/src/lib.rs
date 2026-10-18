//! Agent API: JSON-RPC 2.0 over newline-delimited JSON, served on a Unix socket or stdio.
//! Every request runs through `session` commands and queries, the same path the UI uses.

mod dispatch;
mod editor;
mod headless;
mod hub;
mod input;
mod probe;
mod protocol;
mod roots;
mod transport;

pub use dispatch::{BatchResult, EditorQuery, ExecuteResult, Host, describe, handle};
pub use editor::{Dialog, EditorCommand, EditorState, MessageKind, PhaseName, ToolName};
pub use headless::HeadlessHost;
pub use hub::{ACTIVITY_LEN, Activity, Hub, Inbox};
pub use input::{
    Hover, Input, InputEvent, Issued, Key, Mods, MouseButton, Spot, ToolInput, ToolInputError,
    ToolInputParams, ToolInputResult, ToolInputStopped,
};
pub use probe::{
    AxisName, InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, ScreenBox, ScreenCommand,
    Snap, View, snap_name,
};
pub use protocol::{MAX_LINE_BYTES, codes};
pub use roots::ApiRoots;
pub use transport::{listen_unix, serve_stdio, serve_stream};
