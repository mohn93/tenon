//! Connections, request routing and change notifications.
//!
//! Transports (socket, stdio, tests) feed lines into an `Inbox` from any thread. The program
//! that owns the document calls `Hub::process` on its own thread; that is the only place
//! requests touch the document.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::json;

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
}

impl Inbox {
    /// Registers a connection. Replies and notifications for it arrive on the returned receiver.
    pub fn open(&self) -> (u64, Receiver<String>) {
        let conn = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let (out, rx) = channel();
        self.send(Event::Opened { conn, out });
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

    fn send(&self, event: Event) {
        if self.tx.send(event).is_ok() {
            (self.wake)();
        }
    }
}

pub struct Hub {
    rx: Receiver<Event>,
    inbox: Inbox,
    outs: HashMap<u64, Sender<String>>,
    subscribers: HashSet<u64>,
    /// Last revision subscribers were told about.
    notified: Option<u64>,
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
            },
            outs: HashMap::new(),
            subscribers: HashSet::new(),
            notified: None,
        }
    }

    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
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
        for conn in &self.subscribers {
            if let Some(out) = self.outs.get(conn) {
                for line in &lines {
                    let _ = out.send(line.clone());
                }
            }
        }
        self.notified = Some(doc.revision());
    }

    fn on_event(&mut self, host: &mut dyn Host, event: Event) {
        match event {
            Event::Opened { conn, out } => {
                self.outs.insert(conn, out);
            }
            Event::Closed { conn } => {
                self.outs.remove(&conn);
                self.subscribers.remove(&conn);
            }
            Event::Reply { conn, line } => {
                if let Some(out) = self.outs.get(&conn) {
                    let _ = out.send(line);
                }
            }
            Event::Line { conn, line } => {
                let reply = match parse(&line) {
                    Ok(req) if req.method == "subscribe" => {
                        // Notify about anything earlier before this connection joins.
                        self.notify(host);
                        self.subscribers.insert(conn);
                        req.id
                            .map(|id| success(&id, json!({"revision": host.document().revision()})))
                    }
                    _ => handle(host, &line),
                };
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
