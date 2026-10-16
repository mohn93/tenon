//! Connections, request routing and change notifications.
//!
//! Transports (socket, stdio, tests) feed lines into an `Inbox` from any thread. The program
//! that owns the document calls `Hub::process` on its own thread; that is the only place
//! requests touch the document.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kernel::EntityId;
use serde_json::{Value, json};
use session::Query;

use crate::dispatch::{Host, handle};
use crate::protocol::{notification, parse, success};

enum Event {
    Opened { conn: u64, out: Sender<String> },
    Line { conn: u64, line: String },
    Closed { conn: u64 },
    Reply { conn: u64, line: String },
}

/// The sending side used by transports. Cheap to clone; usable from any thread.
#[derive(Clone)]
pub struct Inbox {
    tx: Sender<Event>,
    next_conn: Arc<AtomicU64>,
    wake: Arc<dyn Fn() + Send + Sync>,
    writers: Arc<Writers>,
}

/// How many transport threads are writing replies, so `Hub::close` can wait for them.
#[derive(Default)]
struct Writers {
    state: Mutex<WritersState>,
    done: Condvar,
}

#[derive(Default)]
struct WritersState {
    running: usize,
    /// Set by `Hub::close`: connections opened after it are dropped at once.
    closed: bool,
}

impl Writers {
    fn lock(&self) -> std::sync::MutexGuard<'_, WritersState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Held by a transport's writer thread while it runs.
pub(crate) struct Writing(Arc<Writers>);

impl Drop for Writing {
    fn drop(&mut self) {
        self.0.lock().running -= 1;
        self.0.done.notify_all();
    }
}

impl Inbox {
    /// Registers a connection. Replies and notifications for it arrive on the returned receiver.
    /// Once the hub is closed the receiver gets nothing and ends at once.
    pub fn open(&self) -> (u64, Receiver<String>) {
        let conn = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let (out, rx) = channel();
        // Checked and sent under the lock, so `Hub::close` never misses a connection's sender.
        let writers = self.writers.lock();
        if !writers.closed {
            self.send(Event::Opened { conn, out });
        }
        drop(writers);
        (conn, rx)
    }

    pub fn send_line(&self, conn: u64, line: String) {
        self.send(Event::Line { conn, line });
    }

    pub fn close(&self, conn: u64) {
        self.send(Event::Closed { conn });
    }

    /// Sends a ready-made line straight back to the connection (e.g. a transport-level error).
    pub fn reply(&self, conn: u64, line: String) {
        self.send(Event::Reply { conn, line });
    }

    /// Counts a writer thread as running until the returned guard is dropped.
    pub(crate) fn writing(&self) -> Writing {
        self.writers.lock().running += 1;
        Writing(self.writers.clone())
    }

    fn send(&self, event: Event) {
        if self.tx.send(event).is_ok() {
            (self.wake)();
        }
    }
}

/// How many entries the activity log keeps.
pub const ACTIVITY_LEN: usize = 100;

/// One line of the activity log: a connection opening or closing, or a request handled.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// When it happened (the last time, for a repeated request).
    pub at: Instant,
    pub connection: u64,
    /// `connected`, `disconnected`, or the request: method, command or query type, and the
    /// path of a file command (`execute save /a/house.skr`), at most 160 characters.
    pub request: String,
    /// The error message when the reply was an error.
    pub error: Option<String>,
    /// How many times in a row it came with the same result (1 at first).
    pub repeats: u32,
}

pub struct Hub {
    rx: Receiver<Event>,
    inbox: Inbox,
    outs: HashMap<u64, Sender<String>>,
    subscribers: HashSet<u64>,
    /// Last revision subscribers were told about.
    notified: Option<u64>,
    /// The selection, camera and editor subscribers were last told about (None while nobody
    /// subscribes).
    seen: Option<Seen>,
    activity: VecDeque<Activity>,
    activity_revision: u64,
}

/// The state `selection`, `camera` and `editor` notifications report.
struct Seen {
    selection: BTreeSet<EntityId>,
    /// The `camera` query's result without its `type`.
    camera: Value,
    editor: Option<Value>,
}

impl Seen {
    fn of(host: &dyn Host) -> Seen {
        let doc = host.document();
        let mut camera = json!(
            doc.query(&Query::Camera {})
                .expect("the camera query cannot fail")
        );
        if let Some(fields) = camera.as_object_mut() {
            fields.remove("type");
        }
        Seen {
            selection: doc.selection().clone(),
            camera,
            editor: host.editor().map(|e| json!(e)),
        }
    }
}

impl Hub {
    /// `wake` is called (from transport threads) whenever input arrives, so a window event loop
    /// can schedule `process`. Pass a no-op when blocking in `process_blocking`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = channel();
        Self {
            rx,
            inbox: Inbox {
                tx,
                next_conn: Arc::new(AtomicU64::new(1)),
                wake: Arc::new(wake),
                writers: Arc::default(),
            },
            outs: HashMap::new(),
            subscribers: HashSet::new(),
            notified: None,
            seen: None,
            activity: VecDeque::new(),
            activity_revision: 0,
        }
    }

    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    /// The log, oldest first.
    pub fn activity(&self) -> &VecDeque<Activity> {
        &self.activity
    }

    /// Goes up whenever the log changes, so a window knows to draw it again.
    pub fn activity_revision(&self) -> u64 {
        self.activity_revision
    }

    /// Adds a line to the activity log, or counts it again when it repeats the last one.
    fn log(&mut self, connection: u64, request: String, error: Option<String>) {
        self.activity_revision += 1;
        let now = Instant::now();
        if let Some(last) = self.activity.back_mut()
            && last.connection == connection
            && last.request == request
            && last.error == error
        {
            last.repeats += 1;
            last.at = now;
            return;
        }
        if self.activity.len() == ACTIVITY_LEN {
            self.activity.pop_front();
        }
        self.activity.push_back(Activity {
            at: now,
            connection,
            request,
            error,
            repeats: 1,
        });
    }

    pub fn open_connections(&self) -> usize {
        self.outs.len()
    }

    /// Handles everything waiting without blocking, then notifies subscribers.
    pub fn process(&mut self, host: &mut dyn Host) {
        while let Ok(event) = self.rx.try_recv() {
            self.on_event(host, event);
        }
        self.notify(host);
    }

    /// Waits for at least one event, then handles everything waiting. The hub holds its own
    /// inbox, so this returns true for as long as the hub exists; callers decide when to stop
    /// (e.g. when `open_connections()` drops to zero).
    pub fn process_blocking(&mut self, host: &mut dyn Host) -> bool {
        match self.rx.recv() {
            Ok(event) => {
                self.on_event(host, event);
                self.process(host);
                true
            }
            Err(_) => false,
        }
    }

    /// Sends `changed` notifications for model changes subscribers have not seen yet (including
    /// changes made through the UI). Call after any edit outside `process`.
    pub fn notify(&mut self, host: &dyn Host) {
        let doc = host.document();
        let Some(since) = self.notified else {
            self.notified = Some(doc.revision());
            return;
        };
        if doc.revision() == since {
            return;
        }
        let (events, truncated) = doc.events_since(since);
        let lines: Vec<String> = if truncated {
            vec![notification("resync", json!({"revision": doc.revision()}))]
        } else {
            events
                .iter()
                .map(|e| notification("changed", json!(e)))
                .collect()
        };
        self.broadcast(&lines);
        self.notified = Some(doc.revision());
    }

    /// Sends `selection`, `camera` and `editor` notifications for those that changed since
    /// subscribers were last told, whoever changed them: at most one of each per call. Call it
    /// once a frame (the window) or after each round of requests (headless).
    pub fn notify_state(&mut self, host: &dyn Host) {
        if self.subscribers.is_empty() {
            self.seen = None;
            return;
        }
        let now = Seen::of(host);
        let Some(before) = self.seen.replace(now) else {
            return;
        };
        let now = self.seen.as_ref().expect("just set");
        let mut lines = Vec::new();
        if now.selection != before.selection {
            let ids: Vec<EntityId> = now.selection.iter().copied().collect();
            let revision = host.document().revision();
            lines.push(notification(
                "selection",
                json!({"ids": ids, "revision": revision}),
            ));
        }
        if now.camera != before.camera {
            lines.push(notification("camera", now.camera.clone()));
        }
        if now.editor != before.editor
            && let Some(editor) = &now.editor
        {
            lines.push(notification("editor", editor.clone()));
        }
        self.broadcast(&lines);
    }

    /// Sends `lines` to every subscriber.
    fn broadcast(&self, lines: &[String]) {
        for conn in &self.subscribers {
            if let Some(out) = self.outs.get(conn) {
                for line in lines {
                    let _ = out.send(line.clone());
                }
            }
        }
    }

    /// Stops serving: drops requests not yet handled and every connection's sending side, so
    /// each transport's writer thread ends once the lines queued for it are written, and waits
    /// up to `wait` for them. True if they all finished. Connections opened from now on are
    /// dropped at once. Call it before the program exits, so the last replies (such as a
    /// `quit`'s) reach their clients.
    pub fn close(&mut self, wait: Duration) -> bool {
        let writers = &self.inbox.writers;
        let mut state = writers.lock();
        state.closed = true;
        // Under the lock: no connection can be registered between this and the wait.
        while self.rx.try_recv().is_ok() {}
        self.outs.clear();
        self.subscribers.clear();
        let (state, _) = writers
            .done
            .wait_timeout_while(state, wait, |s| s.running > 0)
            .unwrap_or_else(|e| e.into_inner());
        state.running == 0
    }

    fn on_event(&mut self, host: &mut dyn Host, event: Event) {
        match event {
            Event::Opened { conn, out } => {
                self.outs.insert(conn, out);
                self.log(conn, "connected".into(), None);
            }
            Event::Closed { conn } => {
                self.subscribers.remove(&conn);
                if self.outs.remove(&conn).is_some() {
                    self.log(conn, "disconnected".into(), None);
                }
            }
            Event::Reply { conn, line } => {
                if let Some(out) = self.outs.get(&conn) {
                    let _ = out.send(line);
                }
            }
            Event::Line { conn, line } => {
                let reply = match parse(&line) {
                    Ok(req) if req.method == "subscribe" && !host.quitting() => {
                        // Notify about anything earlier before this connection joins.
                        self.notify(host);
                        self.subscribers.insert(conn);
                        if self.seen.is_none() {
                            self.seen = Some(Seen::of(host));
                        }
                        req.id
                            .map(|id| success(&id, json!({"revision": host.document().revision()})))
                    }
                    _ => handle(host, &line),
                };
                let error = reply
                    .as_deref()
                    .and_then(|r| serde_json::from_str::<Value>(r).ok())
                    .and_then(|r| r["error"]["message"].as_str().map(str::to_string));
                self.log(conn, summary(&line), error);
                if let (Some(reply), Some(out)) = (reply, self.outs.get(&conn)) {
                    let _ = out.send(reply);
                }
                // Notify right after each request: a connection may close in the same batch of
                // events, and its notifications must go out before it is dropped.
                self.notify(host);
            }
        }
    }
}

/// How the activity log names a request line: method, command or query type, and the path
/// of a file command; at most 160 characters.
fn summary(line: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return "(not JSON)".into();
    };
    let method = v["method"].as_str().unwrap_or("?");
    let params = &v["params"];
    let mut s = method.to_string();
    match method {
        "batch" => {
            let n = params["commands"].as_array().map_or(0, Vec::len);
            s += &format!(" ({n} commands)");
        }
        "tool_input" => {
            let n = params["events"].as_array().map_or(0, Vec::len);
            s += &format!(" ({n} events)");
        }
        _ => {
            if let Some(t) = params["type"].as_str() {
                s += &format!(" {t}");
            }
            if let Some(p) = params["path"].as_str() {
                s += &format!(" {p}");
            }
        }
    }
    if s.chars().count() > 160 {
        s = s.chars().take(159).collect::<String>() + "\u{2026}";
    }
    s
}
