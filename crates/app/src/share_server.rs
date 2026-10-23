//! The Open on Phone server: a small HTTP/1.1 server on the local network that serves the web
//! viewer's files and one encrypted share blob, nothing else. It exists only while the Open on
//! Phone sheet is open (and for at most [`LIFETIME`]); `std` only, no async runtime.
//!
//! What it serves: `GET` and `HEAD` of `/` (the viewer's `index.html`), `/<name>` for a regular
//! file directly inside the viewer directory (`[A-Za-z0-9._-]+`, no leading dot, no `..`, not
//! a link) and `/s/<blob id>` for the current blob. Everything else is 404 (query strings,
//! percent escapes and doubled slashes included) or 405. The decryption key is in the link's
//! fragment, which browsers never send, so the server never sees it.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a share lives at most.
pub const LIFETIME: Duration = Duration::from_secs(30 * 60);
/// Most connections served at once; more are answered 503.
pub const MAX_CONNECTIONS: usize = 16;
/// The most a connection lives in all.
pub const RESPONSE: Duration = Duration::from_secs(120);
/// How long a connection may sit idle (reading a request, or writing to a stalled reader).
pub const IDLE: Duration = Duration::from_secs(10);
/// The most a request's line and headers may take.
const MAX_HEAD: usize = 8 * 1024;
/// The most a request line may take.
const MAX_LINE: usize = 2 * 1024;
/// How a file is streamed.
const CHUNK: usize = 16 * 1024;
/// How often waiting loops look at the stop flag.
const TICK: Duration = Duration::from_millis(20);

/// What the sheet says when no viewer files are found.
pub const NOT_BUILT: &str = "The phone viewer is not built: run scripts/build-web.sh";
/// What the sheet says when the computer has no local network address.
pub const NO_NETWORK: &str = "Connect this computer to Wi-Fi to share";

/// What a server is made of.
pub struct Config {
    /// The viewer's files.
    pub dir: PathBuf,
    /// The blob served at `/s/<id>`.
    pub blob: Arc<Vec<u8>>,
    /// The blob's id, 32 lowercase hex digits.
    pub id: String,
    /// How long the server runs at most.
    pub lifetime: Duration,
    /// Idle timeout of a connection: no byte read or written for this long ends it.
    pub idle: Duration,
    /// The most a connection lives in all, however slowly its client reads.
    pub response: Duration,
    /// Most simultaneous connections.
    pub max_connections: usize,
}

impl Config {
    pub fn new(dir: PathBuf, blob: Vec<u8>, id: [u8; 16]) -> Self {
        Self {
            dir,
            blob: Arc::new(blob),
            id: hex(&id),
            lifetime: LIFETIME,
            idle: IDLE,
            response: RESPONSE,
            max_connections: MAX_CONNECTIONS,
        }
    }
}

/// `bytes` as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

struct Shared {
    stop: AtomicBool,
    active: AtomicUsize,
    downloads: AtomicUsize,
    started: Instant,
    config: Config,
}

impl Shared {
    /// The server was stopped or its lifetime is over: connections end.
    fn halted(&self) -> bool {
        self.stop.load(Ordering::SeqCst) || self.started.elapsed() >= self.config.lifetime
    }
}

/// A connection's output: every write checks the stop flag and lifetime, ends after the
/// connection's total time (`deadline`) and after `idle` without progress, so a client that
/// reads slowly (or not at all) cannot hold the server or the blob.
struct Out<'a> {
    stream: TcpStream,
    shared: &'a Shared,
    deadline: Instant,
    idle: Duration,
    progress: Instant,
}

impl<'a> Out<'a> {
    fn new(stream: TcpStream, shared: &'a Shared, total: Duration, idle: Duration) -> Self {
        let now = Instant::now();
        Out {
            stream,
            shared,
            deadline: now + total,
            idle,
            progress: now,
        }
    }
}

impl Write for Out<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            // Not `Interrupted`: `write_all` would retry it.
            if self.shared.halted() {
                return Err(ErrorKind::ConnectionAborted.into());
            }
            let now = Instant::now();
            if now >= self.deadline || now.duration_since(self.progress) >= self.idle {
                return Err(ErrorKind::TimedOut.into());
            }
            let left = self.deadline - now;
            let _ = self.stream.set_write_timeout(Some(left.min(TICK * 5)));
            match self.stream.write(buf) {
                Ok(n) => {
                    self.progress = Instant::now();
                    return Ok(n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

/// A running server. Dropping it stops it: the listener is closed before `drop` returns, and
/// connections still open end within a tick.
pub struct Server {
    shared: Arc<Shared>,
    port: u16,
    started: Instant,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Binds `0.0.0.0` on a free port and starts serving.
    pub fn start(config: Config) -> std::io::Result<Server> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let started = Instant::now();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            started,
            config,
        });
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("share-server".into())
                .spawn(move || accept_loop(listener, shared, started))?
        };
        Ok(Server {
            shared,
            port,
            started,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many times the whole blob was sent.
    pub fn downloads(&self) -> usize {
        self.shared.downloads.load(Ordering::Relaxed)
    }

    /// How long until the server stops by itself.
    pub fn remaining(&self) -> Duration {
        self.shared
            .config
            .lifetime
            .saturating_sub(self.started.elapsed())
    }

    /// Whether the listener is still open.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Closes the listener and ends the connections. Returns once the listener is closed.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, started: Instant) {
    while !shared.stop.load(Ordering::SeqCst) && started.elapsed() < shared.config.lifetime {
        match listener.accept() {
            Ok((stream, _)) => {
                // Some systems hand on the listener's non-blocking mode.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if shared.active.fetch_add(1, Ordering::SeqCst) >= shared.config.max_connections {
                    shared.active.fetch_sub(1, Ordering::SeqCst);
                    refuse(stream, &shared);
                    continue;
                }
                let shared = Arc::clone(&shared);
                let spawned = std::thread::Builder::new()
                    .name("share-conn".into())
                    .spawn({
                        let shared = Arc::clone(&shared);
                        move || {
                            let _guard = Active(Arc::clone(&shared));
                            serve(stream, &shared);
                        }
                    });
                if spawned.is_err() {
                    shared.active.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(_) => std::thread::sleep(TICK),
        }
    }
    // The listener drops here.
}

struct Active(Arc<Shared>);

impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Answers 503 and closes: all the connection slots are taken.
fn refuse(stream: TcpStream, shared: &Shared) {
    // Short, so a client that does not read cannot hold up accepting.
    let short = Duration::from_millis(100);
    let mut out = Out::new(stream, shared, short, short);
    let _ = send(
        &mut out,
        503,
        "Service Unavailable",
        &[],
        b"Busy
",
        false,
    );
}

enum Head {
    Ready(Vec<u8>),
    /// Status and reason to answer with.
    Bad(u16, &'static str),
    /// Closed, timed out or stopped: nothing to answer.
    Gone,
}

/// Reads the request line and headers: at most `MAX_HEAD` bytes, the line at most `MAX_LINE`,
/// all within the idle time (a client that dribbles bytes gets no longer).
fn read_head(stream: &mut TcpStream, shared: &Shared) -> Head {
    let deadline = Instant::now() + shared.config.idle;
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    loop {
        if shared.halted() {
            return Head::Gone;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Head::Gone;
        }
        let _ = stream.set_read_timeout(Some(left.min(TICK * 5)));
        match stream.read(&mut chunk) {
            Ok(0) => return Head::Gone,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Head::Gone,
        }
        let line_end = find(&buf, b"\r\n");
        if line_end.unwrap_or(buf.len()) > MAX_LINE {
            return Head::Bad(414, "URI Too Long");
        }
        match find(&buf, b"\r\n\r\n") {
            Some(end) if end + 4 > MAX_HEAD => {
                return Head::Bad(431, "Request Header Fields Too Large");
            }
            Some(end) => {
                buf.truncate(end);
                return Head::Ready(buf);
            }
            None if buf.len() >= MAX_HEAD => {
                return Head::Bad(431, "Request Header Fields Too Large");
            }
            None => {}
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// What a request resolves to.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    File(PathBuf),
    Blob,
    NotFound,
}

/// Whether `name` is a file name the server hands out.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whether `id` is 32 lowercase hex digits.
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where `target` (a request target) leads: the blob if it is `/s/<current id>`, else a regular
/// file directly inside `dir` (`/` is `index.html`), else nothing.
fn route(target: &str, dir: &Path, blob_id: &str) -> Route {
    if let Some(id) = target.strip_prefix("/s/") {
        return if valid_id(id) && id == blob_id {
            Route::Blob
        } else {
            Route::NotFound
        };
    }
    let name = match target {
        "/" => "index.html",
        t => match t.strip_prefix('/') {
            Some(n) => n,
            None => return Route::NotFound,
        },
    };
    if !valid_name(name) {
        return Route::NotFound;
    }
    let path = dir.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_file() => Route::File(path),
        _ => Route::NotFound,
    }
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("wasm") => "application/wasm",
        Some("js") => "text/javascript",
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    }
}

fn serve(stream: TcpStream, shared: &Shared) {
    let mut stream = Out::new(stream, shared, shared.config.response, shared.config.idle);
    let head = match read_head(&mut stream.stream, shared) {
        Head::Ready(h) => h,
        Head::Bad(code, reason) => {
            let _ = send(&mut stream, code, reason, &[], reason.as_bytes(), false);
            return;
        }
        Head::Gone => return,
    };
    let bad = |stream: &mut Out| {
        let _ = send(stream, 400, "Bad Request", &[], b"Bad Request\n", false);
    };
    let first = head.split(|&b| b == b'\r').next().unwrap_or(&[]);
    let Ok(line) = std::str::from_utf8(first) else {
        return bad(&mut stream);
    };
    let parts: Vec<&str> = line.split(' ').collect();
    let [method, target, version] = parts[..] else {
        return bad(&mut stream);
    };
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return bad(&mut stream);
    }
    let head_only = match method {
        "GET" => false,
        "HEAD" => true,
        _ => {
            let _ = send(
                &mut stream,
                405,
                "Method Not Allowed",
                &[("Allow", "GET, HEAD")],
                b"Method Not Allowed\n",
                false,
            );
            return;
        }
    };
    let cfg = &shared.config;
    match route(target, &cfg.dir, &cfg.id) {
        Route::NotFound => {
            let _ = send(
                &mut stream,
                404,
                "Not Found",
                &[],
                b"Not Found\n",
                head_only,
            );
        }
        Route::Blob => {
            let sent = send_stream(
                &mut stream,
                200,
                "OK",
                &[
                    ("Content-Type", "application/octet-stream"),
                    ("Cache-Control", "no-store"),
                ],
                &cfg.blob[..],
                cfg.blob.len() as u64,
                head_only,
            );
            if sent.is_ok() && !head_only {
                shared.downloads.fetch_add(1, Ordering::Relaxed);
            }
        }
        Route::File(path) => {
            let Ok(file) = File::open(&path) else {
                let _ = send(
                    &mut stream,
                    404,
                    "Not Found",
                    &[],
                    b"Not Found\n",
                    head_only,
                );
                return;
            };
            let Ok(meta) = file.metadata() else { return };
            if !meta.is_file() {
                let _ = send(
                    &mut stream,
                    404,
                    "Not Found",
                    &[],
                    b"Not Found\n",
                    head_only,
                );
                return;
            }
            let _ = send_file(&mut stream, file, meta.len(), mime(&path), head_only);
        }
    }
}

fn write_head(
    stream: &mut impl Write,
    code: u16,
    reason: &str,
    extra: &[(&str, &str)],
    length: u64,
    default_type: bool,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {code} {reason}\r\n");
    if default_type {
        head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {length}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
    ));
    stream.write_all(head.as_bytes())
}

/// A whole response held in memory (small bodies and the blob).
fn send(
    stream: &mut impl Write,
    code: u16,
    reason: &str,
    extra: &[(&str, &str)],
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let typed = extra.iter().any(|(k, _)| *k == "Content-Type");
    write_head(stream, code, reason, extra, body.len() as u64, !typed)?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()
}

/// A response whose body is streamed in chunks from `body` (`length` bytes); each chunk is
/// written through `Out`, which ends it when the server stops or the connection's time is up.
fn send_stream(
    stream: &mut impl Write,
    code: u16,
    reason: &str,
    extra: &[(&str, &str)],
    mut body: impl Read,
    length: u64,
    head_only: bool,
) -> std::io::Result<()> {
    write_head(stream, code, reason, extra, length, false)?;
    if head_only {
        return stream.flush();
    }
    let mut buf = vec![0u8; CHUNK];
    let mut left = length;
    while left > 0 {
        let want = buf.len().min(left as usize);
        let n = body.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
        left -= n as u64;
    }
    stream.flush()
}

fn send_file(
    stream: &mut impl Write,
    file: File,
    length: u64,
    content_type: &str,
    head_only: bool,
) -> std::io::Result<()> {
    send_stream(
        stream,
        200,
        "OK",
        &[("Content-Type", content_type)],
        file,
        length,
        head_only,
    )
}

/// The address to put in the link among `candidates` (the addresses the computer could be
/// reached at): the first IPv4 that is not loopback, link-local, unspecified or multicast.
pub fn pick_lan_ipv4(candidates: &[IpAddr]) -> Option<Ipv4Addr> {
    candidates.iter().find_map(|ip| match ip {
        IpAddr::V4(v4)
            if !v4.is_loopback()
                && !v4.is_link_local()
                && !v4.is_unspecified()
                && !v4.is_multicast()
                && !v4.is_broadcast() =>
        {
            Some(*v4)
        }
        _ => None,
    })
}

/// The computer's primary local IPv4: the address the system would send from towards a
/// non-routable address (a UDP "connect" sends no packet), if it is a usable one.
pub fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(SocketAddr::from(([192, 0, 2, 1], 9))).ok()?;
    pick_lan_ipv4(&[socket.local_addr().ok()?.ip()])
}

/// The folder with the web viewer's files: the first of `exe_dir/viewer`, `alt` and `source`
/// that holds an `index.html`.
pub fn resolve_viewer_dir(
    exe_dir: Option<&Path>,
    alt: Option<&Path>,
    source: &Path,
) -> Option<PathBuf> {
    exe_dir
        .map(|d| d.join("viewer"))
        .into_iter()
        .chain(alt.map(Path::to_path_buf))
        .chain(std::iter::once(source.to_path_buf()))
        .find(|d| d.join("index.html").is_file())
}

/// [`resolve_viewer_dir`] for this process: next to the executable, `$SKETCHUP_ALT_VIEWER`, and
/// the source tree's `web/dist` (development).
pub fn viewer_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let alt = std::env::var_os("SKETCHUP_ALT_VIEWER").map(PathBuf::from);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    resolve_viewer_dir(
        exe.as_deref().and_then(Path::parent),
        alt.as_deref(),
        &source,
    )
}

/// One share being served: the server, its link and QR code, and the model it holds.
pub struct Sharing {
    server: Server,
    url: String,
    qr_width: usize,
    qr: Vec<bool>,
    /// `Editor::documents` and the document's revision when the model was sealed.
    taken_at: (u64, u64),
}

impl Sharing {
    /// Seals `model` (the bytes of a `.skr` file) and its `name` and serves it from `dir`, advertised at `ip`.
    /// `taken_at` is what the model was taken from, for [`Sharing::stale`]. Err: what the sheet
    /// says.
    pub fn begin(
        dir: PathBuf,
        ip: Ipv4Addr,
        name: &str,
        model: &[u8],
        taken_at: (u64, u64),
    ) -> Result<Sharing, String> {
        let sealed = session::share::seal(name, model).map_err(|e| e.message)?;
        let server = Server::start(Config::new(dir, sealed.blob, sealed.id))
            .map_err(|e| format!("Could not start sharing: {e}"))?;
        let url = format!(
            "http://{ip}:{}/#{}",
            server.port(),
            session::share::link_fragment(&sealed.id, &sealed.key)
        );
        let (qr_width, qr) = session::share::qr_modules(&url).map_err(|e| e.message)?;
        Ok(Sharing {
            server,
            url,
            qr_width,
            qr,
            taken_at,
        })
    }

    /// [`Sharing::begin`] at this computer's address and viewer folder, or why it cannot be.
    pub fn begin_here(name: &str, model: &[u8], taken_at: (u64, u64)) -> Result<Sharing, String> {
        let dir = viewer_dir().ok_or_else(|| NOT_BUILT.to_string())?;
        let ip = lan_ipv4().ok_or_else(|| NO_NETWORK.to_string())?;
        Self::begin(dir, ip, name, model, taken_at)
    }

    pub fn running(&self) -> bool {
        self.server.running()
    }

    /// Whether the model is not the one that was sealed.
    pub fn stale(&self, now: (u64, u64)) -> bool {
        now != self.taken_at
    }

    /// What the sheet shows for the model as of `now` (`Editor::documents`, revision).
    pub fn view(&self, now: (u64, u64)) -> crate::ui::PhoneShare {
        crate::ui::PhoneShare {
            url: self.url.clone(),
            qr_width: self.qr_width,
            qr: self.qr.clone(),
            sent: self.server.downloads(),
            seconds_left: self.server.remaining().as_secs(),
            stale: self.stale(now),
        }
    }
}

#[cfg(test)]
mod tests;
