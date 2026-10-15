//! Agent API: JSON-RPC 2.0 over newline-delimited JSON, served on a Unix socket or stdio.
//! Every request runs through `session` commands and queries, the same path the UI uses.

mod dispatch;
mod headless;
mod hub;
mod probe;
mod protocol;
mod transport;

pub use dispatch::{EditorQuery, Host, describe, handle};
pub use headless::HeadlessHost;
pub use hub::{Hub, Inbox};
pub use probe::{InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, Snap, View};
pub use protocol::{MAX_LINE_BYTES, codes};
pub use transport::{listen_unix, serve_stdio, serve_stream};
