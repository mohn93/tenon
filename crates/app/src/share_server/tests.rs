use super::*;
use std::sync::atomic::AtomicU32;

const ID: [u8; 16] = [0xab; 16];

struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "sk-share-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(name), bytes).unwrap();
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A viewer folder with a few files, and a file outside it.
fn viewer() -> Dir {
    let d = Dir::new();
    d.file("index.html", b"<html>viewer</html>");
    d.file("web.js", b"let x;");
    d.file("web_bg.wasm", &vec![7u8; 300_000]);
    d.file("style.css", b"a{}");
    d.file("data.bin", b"bin");
    d.file(".hidden", b"secret");
    std::fs::create_dir(d.0.join("sub")).unwrap();
    d.file("sub/inner.js", b"inner");
    std::fs::write(d.0.parent().unwrap().join("outside.txt"), b"outside").ok();
    d
}

fn config(dir: &Dir) -> Config {
    let mut c = Config::new(dir.0.clone(), b"BLOB-BYTES".to_vec(), ID);
    c.idle = Duration::from_millis(600);
    c
}

fn start(dir: &Dir) -> Server {
    Server::start(config(dir)).unwrap()
}

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(": ")?;
            k.eq_ignore_ascii_case(name).then_some(v)
        })
    }
}

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

fn read_reply(mut s: TcpStream) -> Reply {
    let mut raw = Vec::new();
    // Keep what arrived even if the read then fails (a reset after the head is fine).
    let read = s.read_to_end(&mut raw);
    let at = find(&raw, b"\r\n\r\n").unwrap_or_else(|| {
        panic!(
            "a response head; read result {read:?}, got {} bytes: {:?}",
            raw.len(),
            String::from_utf8_lossy(&raw)
        )
    });
    let head = String::from_utf8(raw[..at].to_vec()).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Reply {
        status,
        head,
        body: raw[at + 4..].to_vec(),
    }
}

fn raw(port: u16, bytes: &[u8]) -> Reply {
    let mut s = connect(port);
    s.write_all(bytes).unwrap();
    read_reply(s)
}

fn request(port: u16, method: &str, target: &str) -> Reply {
    raw(
        port,
        format!("{method} {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes(),
    )
}

fn get(port: u16, target: &str) -> Reply {
    request(port, "GET", target)
}

#[test]
fn index_and_files_are_served_with_their_types() {
    let d = viewer();
    let s = start(&d);
    let r = get(s.port(), "/");
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"<html>viewer</html>");
    assert_eq!(r.header("Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(r.header("Connection"), Some("close"));
    assert_eq!(r.header("Content-Length"), Some("19"));
    for (name, kind) in [
        ("web.js", "text/javascript"),
        ("web_bg.wasm", "application/wasm"),
        ("style.css", "text/css"),
        ("data.bin", "application/octet-stream"),
        ("index.html", "text/html; charset=utf-8"),
    ] {
        let r = get(s.port(), &format!("/{name}"));
        assert_eq!(r.status, 200, "{name}");
        assert_eq!(r.header("Content-Type"), Some(kind), "{name}");
    }
}

fn get_with(port: u16, target: &str, headers: &str) -> Reply {
    raw(
        port,
        format!("GET {target} HTTP/1.1\r\nHost: x\r\n{headers}\r\n").as_bytes(),
    )
}

#[test]
fn a_gzip_copy_is_sent_to_browsers_that_take_gzip() {
    let d = viewer();
    d.file("web_bg.wasm.gz", b"GZIPPED");
    let s = start(&d);
    for accept in [
        "Accept-Encoding: gzip, deflate, br\r\n",
        "accept-encoding: br;q=1.0, GZIP;q=0.5\r\n",
        "Accept-Encoding: *\r\n",
    ] {
        let r = get_with(s.port(), "/web_bg.wasm", accept);
        assert_eq!(r.status, 200, "{accept}");
        assert_eq!(r.body, b"GZIPPED", "{accept}");
        assert_eq!(r.header("Content-Encoding"), Some("gzip"), "{accept}");
        assert_eq!(r.header("Content-Type"), Some("application/wasm"));
        assert_eq!(r.header("Content-Length"), Some("7"));
        assert_eq!(r.header("Vary"), Some("Accept-Encoding"));
    }
    // Without gzip (or refusing it) the file itself.
    for accept in [
        "",
        "Accept-Encoding: br\r\n",
        "Accept-Encoding: gzip;q=0\r\n",
        "Accept-Encoding: gzip;q=0, *\r\n",
        "Accept-Encoding: *, gzip; q=0\r\n",
        "Accept-Encoding: *;q=0\r\n",
    ] {
        let r = get_with(s.port(), "/web_bg.wasm", accept);
        assert_eq!(r.body.len(), 300_000, "{accept:?}");
        assert_eq!(r.header("Content-Encoding"), None, "{accept:?}");
    }
    // A file with no copy is sent as it is.
    let r = get_with(s.port(), "/web.js", "Accept-Encoding: gzip\r\n");
    assert_eq!(r.body, b"let x;");
    assert_eq!(r.header("Content-Encoding"), None);
}

#[test]
fn a_gzip_copy_that_is_a_link_is_not_sent() {
    #[cfg(unix)]
    {
        let d = viewer();
        std::os::unix::fs::symlink(d.0.join("data.bin"), d.0.join("web.js.gz")).unwrap();
        let s = start(&d);
        let r = get_with(s.port(), "/web.js", "Accept-Encoding: gzip\r\n");
        assert_eq!(r.body, b"let x;");
        assert_eq!(r.header("Content-Encoding"), None);
    }
}

#[test]
fn a_gzip_copy_older_than_its_file_is_not_sent() {
    let d = viewer();
    d.file("web_bg.wasm.gz", b"GZIPPED");
    let set_time = |name: &str, t: std::time::SystemTime| {
        File::options()
            .write(true)
            .open(d.0.join(name))
            .unwrap()
            .set_modified(t)
            .unwrap();
    };
    let now = std::time::SystemTime::now();
    set_time("web_bg.wasm", now);
    set_time("web_bg.wasm.gz", now - Duration::from_secs(3600));
    let s = start(&d);
    let r = get_with(s.port(), "/web_bg.wasm", "Accept-Encoding: gzip\r\n");
    assert_eq!(
        r.body.len(),
        300_000,
        "the rebuilt file, not the stale copy"
    );
    assert_eq!(r.header("Content-Encoding"), None);
    // A copy as new as its file is sent, also when its time lost the file's nanoseconds
    // (gzip -k keeps the time to the microsecond).
    set_time("web_bg.wasm.gz", now - Duration::from_nanos(999));
    let r = get_with(s.port(), "/web_bg.wasm", "Accept-Encoding: gzip\r\n");
    assert_eq!(r.body, b"GZIPPED");
}

#[test]
fn a_gzip_copy_that_cannot_be_opened_falls_back_to_the_file() {
    let d = viewer();
    std::fs::create_dir(d.0.join("web_bg.wasm.gz")).unwrap();
    let s = start(&d);
    let r = get_with(s.port(), "/web_bg.wasm", "Accept-Encoding: gzip\r\n");
    assert_eq!(r.status, 200);
    assert_eq!(r.body.len(), 300_000);
    assert_eq!(r.header("Content-Encoding"), None);
}

#[cfg(unix)]
#[test]
fn files_are_opened_without_following_links() {
    let d = viewer();
    std::os::unix::fs::symlink(d.0.join("data.bin"), d.0.join("linked.bin")).unwrap();
    assert!(open_regular(&d.0.join("linked.bin")).is_none());
    assert!(open_regular(&d.0.join("data.bin")).is_some());
    assert!(open_regular(&d.0.join("sub")).is_none(), "a folder");
}

#[test]
fn viewer_files_are_revalidated_by_their_etag() {
    let d = viewer();
    d.file("web_bg.wasm.gz", b"GZIPPED");
    let s = start(&d);
    let first = get(s.port(), "/web_bg.wasm");
    assert_eq!(first.header("Cache-Control"), Some("no-cache"));
    let tag = first.header("ETag").expect("an etag").to_string();
    assert!(tag.starts_with('"') && tag.ends_with('"'));
    let again = get_with(
        s.port(),
        "/web_bg.wasm",
        &format!("If-None-Match: {tag}\r\n"),
    );
    assert_eq!(again.status, 304);
    assert!(again.body.is_empty());
    assert_eq!(
        again.header("Content-Length"),
        Some("300000"),
        "a 200's length"
    );
    assert_eq!(again.header("ETag"), Some(tag.as_str()));
    let weak = get_with(
        s.port(),
        "/web_bg.wasm",
        &format!("If-None-Match: \"other\", W/{tag}\r\n"),
    );
    assert_eq!(weak.status, 304);
    // The gzip copy is another file with its own tag.
    let gz = get_with(s.port(), "/web_bg.wasm", "Accept-Encoding: gzip\r\n");
    assert_ne!(gz.header("ETag"), Some(tag.as_str()));
    let stale = get_with(s.port(), "/web_bg.wasm", "If-None-Match: \"other\"\r\n");
    assert_eq!(stale.status, 200);
    assert_eq!(stale.body.len(), 300_000);
    // The blob is never kept.
    let blob = get_with(
        s.port(),
        &format!("/s/{}", hex(&ID)),
        "If-None-Match: *\r\n",
    );
    assert_eq!(blob.status, 200);
    assert_eq!(blob.header("Cache-Control"), Some("no-store"));
    assert_eq!(blob.header("ETag"), None);
}

#[test]
fn request_headers_are_read_by_name() {
    let head = b"GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding:  gzip \r\nX-Odd:no-space";
    assert_eq!(header(head, "accept-encoding"), Some("gzip"));
    assert_eq!(header(head, "x-odd"), Some("no-space"));
    assert_eq!(header(head, "if-none-match"), None);
    // The request line is not a header.
    assert_eq!(header(b"GET /a:b HTTP/1.1", "get /a"), None);
    assert!(accepts_gzip(head));
    assert!(!accepts_gzip(
        b"GET / HTTP/1.1\r\nAccept-Encoding: gzipx, identity"
    ));
    assert!(!accepts_gzip(
        b"GET / HTTP/1.1\r\nAccept-Encoding: gzip; q=0.0"
    ));
    assert!(accepts_gzip(
        b"GET / HTTP/1.1\r\nAccept-Encoding: gzip; q=0.001"
    ));
    assert!(!accepts_gzip(
        b"GET / HTTP/1.1\r\nAccept-Encoding: gzip;q=0, *"
    ));
    assert!(accepts_gzip(b"GET / HTTP/1.1\r\nAccept-Encoding: br, *"));
    assert!(!accepts_gzip(b"GET / HTTP/1.1\r\nHost: x"));
}

#[test]
fn a_large_file_arrives_whole() {
    let d = viewer();
    let s = start(&d);
    let r = get(s.port(), "/web_bg.wasm");
    assert_eq!(r.body.len(), 300_000);
    assert!(r.body.iter().all(|&b| b == 7));
    assert_eq!(r.header("Content-Length"), Some("300000"));
}

#[test]
fn the_blob_is_served_only_under_its_own_id() {
    let d = viewer();
    let s = start(&d);
    let id = hex(&ID);
    let r = get(s.port(), &format!("/s/{id}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"BLOB-BYTES");
    assert_eq!(r.header("Cache-Control"), Some("no-store"));
    assert_eq!(r.header("Content-Type"), Some("application/octet-stream"));
    for target in [
        format!("/s/{}", hex(&[0xcd; 16])),
        format!("/s/{}", id.to_uppercase()),
        format!("/s/{}0", id),
        format!("/s/{}", &id[..31]),
        "/s/".to_string(),
        "/s".to_string(),
        format!("/s/{id}/"),
        format!("/s/{id}?x=1"),
        format!("//s/{id}"),
    ] {
        assert_eq!(get(s.port(), &target).status, 404, "{target}");
    }
}

#[test]
fn other_paths_are_404() {
    let d = viewer();
    let s = start(&d);
    for target in [
        "/missing.js",
        "/sub",
        "/sub/inner.js",
        "/.hidden",
        "/..",
        "/../outside.txt",
        "/..%2foutside.txt",
        "/%2e%2e/outside.txt",
        "/web.js?x=1",
        "/web.js#frag",
        "//web.js",
        "//etc/passwd",
        "/web%2ejs",
        "/a/b",
        "/web.js/",
        "/index.html/..",
        "web.js",
        "*",
        "http://host/web.js",
        "/\\web.js",
        "/a..b",
    ] {
        let r = get(s.port(), target);
        assert_eq!(r.status, 404, "{target}");
        assert!(!String::from_utf8_lossy(&r.body).contains("outside"));
    }
}

#[test]
fn a_linked_file_is_not_served() {
    #[cfg(unix)]
    {
        let d = viewer();
        std::os::unix::fs::symlink(d.0.join("web.js"), d.0.join("alias.js")).unwrap();
        let s = start(&d);
        assert_eq!(get(s.port(), "/alias.js").status, 404);
        assert_eq!(get(s.port(), "/web.js").status, 200);
    }
}

#[test]
fn only_get_and_head_are_allowed() {
    let d = viewer();
    let s = start(&d);
    for method in ["POST", "PUT", "DELETE", "OPTIONS", "PATCH", "TRACE", "get"] {
        let r = request(s.port(), method, "/");
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("Allow"), Some("GET, HEAD"));
    }
    let r = raw(
        s.port(),
        b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello",
    );
    assert_eq!(r.status, 405);
}

#[test]
fn head_has_the_length_and_no_body() {
    let d = viewer();
    let s = start(&d);
    let r = request(s.port(), "HEAD", "/web.js");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Length"), Some("6"));
    assert!(r.body.is_empty());
    let id = hex(&ID);
    let r = request(s.port(), "HEAD", &format!("/s/{id}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Length"), Some("10"));
    assert!(r.body.is_empty());
    assert_eq!(s.downloads(), 0);
    let r = request(s.port(), "HEAD", "/nope");
    assert_eq!(r.status, 404);
    assert!(r.body.is_empty());
}

#[test]
fn malformed_requests_are_refused() {
    let d = viewer();
    let s = start(&d);
    assert_eq!(raw(s.port(), b"GET /\r\n\r\n").status, 400);
    assert_eq!(raw(s.port(), b"GET / HTTP/2\r\n\r\n").status, 400);
    assert_eq!(raw(s.port(), b"GET  / HTTP/1.1\r\n\r\n").status, 400);
    assert_eq!(raw(s.port(), b"\xff\xfe / HTTP/1.1\r\n\r\n").status, 400);
}

#[test]
fn the_request_line_and_head_are_capped() {
    let d = viewer();
    let s = start(&d);
    let long = "a".repeat(3000);
    let r = raw(s.port(), format!("GET /{long} HTTP/1.1\r\n\r\n").as_bytes());
    assert_eq!(r.status, 414);
    // A line that never ends is cut at the cap too.
    let r = raw(s.port(), format!("GET /{long}").as_bytes());
    assert_eq!(r.status, 414);
    let mut big = String::from("GET / HTTP/1.1\r\n");
    while big.len() < 9000 {
        big.push_str("X-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
    }
    big.push_str("\r\n");
    assert_eq!(raw(s.port(), big.as_bytes()).status, 431);
    // A head under the cap is fine.
    let mut ok = String::from("GET / HTTP/1.1\r\n");
    while ok.len() < 7000 {
        ok.push_str("X-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
    }
    ok.push_str("\r\n");
    assert_eq!(raw(s.port(), ok.as_bytes()).status, 200);
}

#[test]
fn an_idle_connection_is_closed_after_the_timeout() {
    let d = viewer();
    let s = start(&d);
    let mut c = connect(s.port());
    let t = Instant::now();
    let mut buf = [0u8; 16];
    // The server closes without a word.
    assert_eq!(c.read(&mut buf).unwrap(), 0);
    assert!(t.elapsed() >= Duration::from_millis(500));
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_client_that_dribbles_bytes_gets_no_extra_time() {
    let d = viewer();
    let s = start(&d);
    let mut c = connect(s.port());
    let t = Instant::now();
    // Set while the socket is open: macOS refuses it once the server has closed its end.
    c.set_read_timeout(Some(Duration::from_millis(1))).unwrap();
    let mut closed = false;
    while t.elapsed() < Duration::from_secs(4) {
        if c.write_all(b"G").is_err() {
            closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        let mut b = [0u8; 4];
        match c.read(&mut b) {
            Ok(0) => closed = true,
            // A reset is the server closing too; a timeout is it still waiting.
            Err(e) if !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                closed = true;
            }
            _ => {}
        }
        if closed {
            break;
        }
    }
    assert!(closed);
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn extra_connections_get_503_and_slots_come_back() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.max_connections = 2;
    cfg.idle = Duration::from_secs(5);
    let s = Server::start(cfg).unwrap();
    let a = connect(s.port());
    let b = connect(s.port());
    // Let the server take both before the third arrives.
    std::thread::sleep(Duration::from_millis(200));
    let r = get(s.port(), "/");
    assert_eq!(r.status, 503);
    assert_eq!(r.header("Connection"), Some("close"));
    drop(a);
    drop(b);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(get(s.port(), "/").status, 200);
}

#[test]
fn the_default_cap_is_sixteen() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.max_connections = MAX_CONNECTIONS;
    cfg.idle = Duration::from_secs(5);
    let s = Server::start(cfg).unwrap();
    let held: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| connect(s.port())).collect();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(get(s.port(), "/").status, 503);
    drop(held);
}

#[test]
fn completed_blob_downloads_are_counted() {
    let d = viewer();
    let s = start(&d);
    let id = hex(&ID);
    assert_eq!(s.downloads(), 0);
    get(s.port(), &format!("/s/{id}"));
    get(s.port(), &format!("/s/{id}"));
    get(s.port(), "/web.js");
    get(s.port(), &format!("/s/{}", hex(&[1; 16])));
    assert_eq!(s.downloads(), 2);
}

#[test]
fn stopping_closes_the_listener() {
    let d = viewer();
    let mut s = start(&d);
    let port = s.port();
    assert!(s.running());
    assert_eq!(get(port, "/").status, 200);
    s.stop();
    assert!(!s.running());
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn dropping_stops_too() {
    let d = viewer();
    let s = start(&d);
    let port = s.port();
    drop(s);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn stopping_ends_open_connections() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.idle = Duration::from_secs(30);
    let mut s = Server::start(cfg).unwrap();
    let mut c = connect(s.port());
    std::thread::sleep(Duration::from_millis(100));
    s.stop();
    let mut buf = [0u8; 8];
    let t = Instant::now();
    assert!(matches!(c.read(&mut buf), Ok(0) | Err(_)));
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn the_server_stops_by_itself_after_its_lifetime() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.lifetime = Duration::from_millis(300);
    let s = Server::start(cfg).unwrap();
    let port = s.port();
    assert!(s.remaining() <= Duration::from_millis(300));
    std::thread::sleep(Duration::from_millis(800));
    assert!(!s.running());
    assert_eq!(s.remaining(), Duration::ZERO);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn the_default_lifetime_is_thirty_minutes() {
    assert_eq!(LIFETIME, Duration::from_secs(1800));
    assert_eq!(IDLE, Duration::from_secs(10));
}

#[test]
fn the_server_binds_all_interfaces() {
    let d = viewer();
    let s = start(&d);
    // 0.0.0.0 answers on loopback; the port is the advertised one.
    assert_ne!(s.port(), 0);
    assert_eq!(get(s.port(), "/").status, 200);
}

#[test]
fn lan_address_skips_loopback_and_link_local() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    assert_eq!(
        pick_lan_ipv4(&[ip("127.0.0.1"), ip("169.254.3.4"), ip("192.168.1.20")]),
        Some(Ipv4Addr::new(192, 168, 1, 20))
    );
    assert_eq!(
        pick_lan_ipv4(&[ip("10.0.0.5"), ip("192.168.1.20")]),
        Some(Ipv4Addr::new(10, 0, 0, 5))
    );
    assert_eq!(pick_lan_ipv4(&[ip("127.0.0.1")]), None);
    assert_eq!(pick_lan_ipv4(&[ip("169.254.1.1")]), None);
    assert_eq!(
        pick_lan_ipv4(&[ip("0.0.0.0"), ip("::1"), ip("fe80::1")]),
        None
    );
    assert_eq!(
        pick_lan_ipv4(&[ip("224.0.0.1"), ip("255.255.255.255")]),
        None
    );
    assert_eq!(pick_lan_ipv4(&[]), None);
}

#[test]
fn the_machine_address_is_usable_or_absent() {
    // Whatever the machine has, it is never one of the unusable kinds.
    if let Some(ip) = lan_ipv4() {
        assert!(!ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified());
    }
}

#[test]
fn viewer_dir_takes_the_first_with_an_index() {
    let exe = Dir::new();
    std::fs::create_dir(exe.0.join("viewer")).unwrap();
    let alt = Dir::new();
    let src = Dir::new();
    // Nothing has an index.
    assert_eq!(resolve_viewer_dir(Some(&exe.0), Some(&alt.0), &src.0), None);
    src.file("index.html", b"s");
    assert_eq!(
        resolve_viewer_dir(Some(&exe.0), Some(&alt.0), &src.0),
        Some(src.0.clone())
    );
    alt.file("index.html", b"a");
    assert_eq!(
        resolve_viewer_dir(Some(&exe.0), Some(&alt.0), &src.0),
        Some(alt.0.clone())
    );
    std::fs::write(exe.0.join("viewer/index.html"), b"e").unwrap();
    assert_eq!(
        resolve_viewer_dir(Some(&exe.0), Some(&alt.0), &src.0),
        Some(exe.0.join("viewer"))
    );
    assert_eq!(resolve_viewer_dir(None, None, &src.0), Some(src.0.clone()));
}

#[test]
fn an_index_that_is_a_folder_does_not_count() {
    let src = Dir::new();
    std::fs::create_dir(src.0.join("index.html")).unwrap();
    assert_eq!(resolve_viewer_dir(None, None, &src.0), None);
}

#[test]
fn route_names_are_strict() {
    for ok in ["a", "web_bg.wasm", "a-b.c_d", "A1"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in ["", ".a", "a..b", "a/b", "a%20", "a b", "a?b", "é", "a\0"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
    assert!(valid_id(&"0123456789abcdef".repeat(2)));
    assert!(!valid_id(&"0123456789ABCDEF".repeat(2)));
}

#[test]
fn a_share_serves_a_blob_the_link_opens() {
    let d = viewer();
    let s = Sharing::begin(
        d.0.clone(),
        Ipv4Addr::new(192, 168, 1, 20),
        "House",
        b"MODEL",
        (3, 9),
    )
    .unwrap();
    let (_, rest) = s.url.split_once("://").unwrap();
    assert!(rest.starts_with("192.168.1.20:"));
    let (host, fragment) = rest.split_once("/#").unwrap();
    let port: u16 = host.split_once(':').unwrap().1.parse().unwrap();
    assert_eq!(port, s.server.port());
    let (id, key) = session::share::parse_fragment(fragment).unwrap();
    let r = get(port, &format!("/s/{}", hex(&id)));
    assert_eq!(r.status, 200);
    assert_eq!(
        session::share::open(&r.body, &id, &key).unwrap(),
        ("House".to_string(), b"MODEL".to_vec())
    );
    // The key is never in anything the server could be sent or sends.
    assert!(!s.url.split('#').next().unwrap().contains("v1."));
    assert_eq!(s.view((3, 9)).sent, 1);
    assert!(!s.view((3, 9)).stale);
    assert!(s.view((3, 10)).stale);
    assert!(s.view((4, 9)).stale);
    assert!(s.view((3, 9)).seconds_left <= 1800);
    assert!(s.qr_width >= 21);
}

#[test]
fn replacing_a_share_stops_the_old_one() {
    let d = viewer();
    let ip = Ipv4Addr::new(10, 0, 0, 2);
    let old = Sharing::begin(d.0.clone(), ip, "A", b"A", (1, 1)).unwrap();
    let old_port = old.server.port();
    let new = Sharing::begin(d.0.clone(), ip, "B", b"B", (1, 2)).unwrap();
    drop(old);
    assert!(TcpStream::connect(("127.0.0.1", old_port)).is_err());
    assert_eq!(get(new.server.port(), "/").status, 200);
}

#[test]
fn a_missing_viewer_or_network_gives_the_sheets_words() {
    assert_eq!(
        NOT_BUILT,
        "The phone viewer is not built: run scripts/build-web.sh"
    );
    assert_eq!(NO_NETWORK, "Connect this computer to Wi-Fi to share");
}

/// A server whose blob is 32 MiB (more than the sockets buffer), and the test's handle on it.
fn big_blob(d: &Dir, tweak: impl FnOnce(&mut Config)) -> (Server, Arc<Vec<u8>>) {
    let blob = Arc::new(vec![5u8; 32 << 20]);
    let mut cfg = config(d);
    cfg.blob = Arc::clone(&blob);
    cfg.idle = Duration::from_secs(30);
    tweak(&mut cfg);
    (Server::start(cfg).unwrap(), blob)
}

fn blob_request(port: u16) -> TcpStream {
    let mut c = connect(port);
    c.write_all(format!("GET /s/{} HTTP/1.1\r\n\r\n", hex(&ID)).as_bytes())
        .unwrap();
    c
}

/// Reads until the connection ends; how many bytes came and how long it took.
fn drain(c: &mut TcpStream, limit: Duration) -> (usize, Duration) {
    let t = Instant::now();
    let mut total = 0;
    let mut buf = vec![0u8; 64 * 1024];
    while t.elapsed() < limit {
        match c.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => total += n,
        }
    }
    (total, t.elapsed())
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < Duration::from_secs(3), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stopping_cuts_off_a_blob_download_and_frees_the_blob() {
    let d = viewer();
    let (mut s, blob) = big_blob(&d, |_| {});
    let mut c = blob_request(s.port());
    let mut first = [0u8; 4096];
    c.read_exact(&mut first).unwrap();
    // The server is now blocked writing to a client that reads slowly.
    std::thread::sleep(Duration::from_millis(300));
    s.stop();
    let (got, took) = drain(&mut c, Duration::from_secs(5));
    assert!(
        got < blob.len() / 2,
        "the download went on after stop: {got} bytes"
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
    drop(s);
    wait_for("the blob is released", || Arc::strong_count(&blob) == 1);
}

#[test]
fn a_download_has_a_total_deadline() {
    let d = viewer();
    let (s, blob) = big_blob(&d, |c| c.response = Duration::from_millis(500));
    let mut c = blob_request(s.port());
    let mut first = [0u8; 4096];
    c.read_exact(&mut first).unwrap();
    // The client reads a little now and then, so it is never idle for long.
    let t = Instant::now();
    let mut buf = [0u8; 4096];
    while Arc::strong_count(&blob) > 2 && t.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
        let _ = c.set_read_timeout(Some(Duration::from_millis(10)));
        let _ = c.read(&mut buf);
    }
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(s.downloads(), 0);
}

#[test]
fn the_lifetime_ends_a_download_in_progress() {
    let d = viewer();
    let (s, blob) = big_blob(&d, |c| c.lifetime = Duration::from_millis(600));
    let mut c = blob_request(s.port());
    let mut first = [0u8; 4096];
    c.read_exact(&mut first).unwrap();
    // The connection thread lets go of the blob once the lifetime is up, though the client
    // has not read it.
    wait_for("the blob is released", || Arc::strong_count(&blob) == 2);
    assert_eq!(s.downloads(), 0);
}

#[test]
fn a_client_that_never_reads_the_refusal_does_not_stall_accepting() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.max_connections = 1;
    cfg.idle = Duration::from_secs(5);
    let s = Server::start(cfg).unwrap();
    let _held = connect(s.port());
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    for _ in 0..20 {
        let _c = connect(s.port());
    }
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn the_preferred_port_is_used_when_free_and_a_free_one_otherwise() {
    let d = viewer();
    let free = {
        let l = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let mut cfg = config(&d);
    cfg.preferred_port = Some(free);
    let first = Server::start(cfg).unwrap();
    assert_eq!(first.port(), free);
    let mut cfg = config(&d);
    cfg.preferred_port = Some(free);
    let second = Server::start(cfg).unwrap();
    assert_ne!(second.port(), free, "taken: the OS chose another");
    assert_eq!(get(second.port(), "/").status, 200);
    drop(first);
    // Freed by the first share's end, the next one gets it again.
    let mut cfg = config(&d);
    cfg.preferred_port = Some(free);
    assert_eq!(Server::start(cfg).unwrap().port(), free);
    assert_eq!(PREFERRED_PORT, 47240);
}

#[test]
fn silent_refused_clients_do_not_stall_accepting() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.max_connections = 1;
    cfg.idle = Duration::from_secs(5);
    let s = Server::start(cfg).unwrap();
    let _held = connect(s.port());
    std::thread::sleep(Duration::from_millis(200));
    // Over the cap and silent, but held open.
    let silent: Vec<TcpStream> = (0..20).map(|_| connect(s.port())).collect();
    let t = Instant::now();
    assert_eq!(get(s.port(), "/").status, 503);
    assert!(
        t.elapsed() < Duration::from_millis(700),
        "the refusal took {:?}",
        t.elapsed()
    );
    drop(silent);
}

#[test]
fn a_refused_client_that_sends_late_still_reads_the_503() {
    let d = viewer();
    let mut cfg = config(&d);
    cfg.max_connections = 1;
    cfg.idle = Duration::from_secs(5);
    let s = Server::start(cfg).unwrap();
    let _held = connect(s.port());
    std::thread::sleep(Duration::from_millis(200));
    let mut c = connect(s.port());
    std::thread::sleep(Duration::from_millis(250));
    c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert_eq!(read_reply(c).status, 503);
}
