//! The phone viewer's parts that do not touch the browser: telling from the page's URL fragment
//! whether to start the viewer, where the model is fetched from, how much of it is accepted and
//! what is said when something goes wrong. They are plain Rust so the native gate tests them.
//!
//! The fragment holds the key (`v1.<id>.<key>`, see `session::share`): it never leaves the
//! browser in a request, is not logged and is taken out of the address bar once the model is
//! open.

use session::share::{MAX_BLOB_BYTES, parse_fragment};

/// What is said when the link is wrong (the app's own text, so the two cannot differ), the model is not what the link names, or it has expired
/// (the server no longer has it).
pub const DAMAGED: &str = app::app::DAMAGED_LINK;

/// What is said when the computer that shared the model cannot be reached.
pub const UNREACHABLE: &str = "Could not reach the computer sharing this model";

/// The name the viewer's top bar shows: the link carries no name.
pub const MODEL_NAME: &str = "Shared model";

/// What the page's fragment asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// No share: the editor.
    Editor,
    /// A share fragment that is not valid.
    Damaged,
    /// The viewer, for this share.
    Viewer(Link),
}

impl Start {
    /// Whether the fragment must leave the address bar before anything else happens: it holds
    /// a key (or something shaped like one), valid or not.
    pub fn forgets_fragment(&self) -> bool {
        !matches!(self, Start::Editor)
    }
}

/// Where a shared model is and what opens it. The key is never shown by `Debug`.
#[derive(PartialEq, Eq)]
pub struct Link {
    pub id: [u8; 16],
    pub key: [u8; 32],
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("id", &blob_path(&self.id))
            .field("key", &"<hidden>")
            .finish()
    }
}

/// What to start for `hash`, the page's `location.hash` (with or without its `#`). Anything
/// that starts `v1.` is a share, valid or not; anything else is the editor.
pub fn classify(hash: &str) -> Start {
    let fragment = hash.strip_prefix('#').unwrap_or(hash);
    if !fragment.starts_with("v1.") {
        return Start::Editor;
    }
    match parse_fragment(fragment) {
        Ok((id, key)) => Start::Viewer(Link { id, key }),
        Err(_) => Start::Damaged,
    }
}

/// The path the desktop's server hands the blob out at: `/s/` and the id in 32 lowercase hex
/// digits.
pub fn blob_path(id: &[u8; 16]) -> String {
    use std::fmt::Write;
    let mut path = String::from("/s/");
    for byte in id {
        let _ = write!(path, "{byte:02x}");
    }
    path
}

/// Why a model could not be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The link, the blob or the key is wrong, the blob is too big, or the server no longer has
    /// it.
    Damaged,
    /// The request did not reach the server.
    Unreachable,
}

impl Failure {
    /// What the page says.
    pub fn message(self) -> &'static str {
        match self {
            Failure::Damaged => DAMAGED,
            Failure::Unreachable => UNREACHABLE,
        }
    }
}

/// Refuses a response that says it is over the size cap, before any of it is read.
pub fn check_length(content_length: Option<u64>) -> Result<(), Failure> {
    match content_length {
        Some(n) if n > MAX_BLOB_BYTES as u64 => Err(Failure::Damaged),
        _ => Ok(()),
    }
}

/// The body as it arrives, never holding more than the size cap whatever the response said.
#[derive(Debug)]
pub struct Collector {
    bytes: Vec<u8>,
    cap: u64,
}

impl Default for Collector {
    fn default() -> Self {
        Self::with_cap(MAX_BLOB_BYTES as u64)
    }
}

impl Collector {
    /// A collector that takes at most `cap` bytes.
    pub fn with_cap(cap: u64) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }

    /// Takes the next chunk; refused (and nothing kept of it) when it would pass the cap.
    pub fn extend(&mut self, chunk: &[u8]) -> Result<(), Failure> {
        if !self.fits(chunk.len() as u64) {
            return Err(Failure::Damaged);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    /// Whether a chunk of `len` bytes would still be within the cap: asked before the chunk is
    /// copied out of the browser.
    pub fn fits(&self, len: u64) -> bool {
        (self.bytes.len() as u64).saturating_add(len) <= self.cap
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::share::{link_fragment, seal};

    #[test]
    fn a_share_fragment_starts_the_viewer() {
        let share = seal(b"model").unwrap();
        let fragment = link_fragment(&share.id, &share.key);
        for hash in [format!("#{fragment}"), fragment.clone()] {
            let Start::Viewer(link) = classify(&hash) else {
                panic!("not a viewer for {hash}");
            };
            assert_eq!(link.id, share.id);
            assert_eq!(link.key, share.key);
        }
    }

    #[test]
    fn other_fragments_start_the_editor() {
        for hash in ["", "#", "#top", "#v2.a.b", "#V1.a.b", "#xv1.a.b", "v1"] {
            assert_eq!(classify(hash), Start::Editor, "{hash:?}");
        }
    }

    #[test]
    fn a_broken_share_fragment_is_damaged() {
        let share = seal(b"model").unwrap();
        let good = link_fragment(&share.id, &share.key);
        let cut = &good[..good.len() - 1];
        let (id, _) = good.rsplit_once('.').unwrap();
        for hash in [
            "#v1.".to_string(),
            "#v1.x.y".to_string(),
            "#v1.a.b.c".to_string(),
            format!("#{cut}"),
            format!("#{good}.extra"),
            format!("#{id}.short"),
            format!("#{id}."),
        ] {
            assert_eq!(classify(&hash), Start::Damaged, "{hash:?}");
        }
    }

    #[test]
    fn the_blob_path_is_32_lowercase_hex_digits() {
        let id = [
            0x00, 0x01, 0xab, 0xff, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xcd,
        ];
        assert_eq!(blob_path(&id), "/s/0001abff02030405060708090a0b0ccd");
        let path = blob_path(&[0x5a; 16]);
        assert_eq!(path.len(), 3 + 32);
        assert!(
            path[3..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn the_key_is_not_in_debug_output() {
        let share = seal(b"model").unwrap();
        let link = Link {
            id: share.id,
            key: share.key,
        };
        let text = format!("{link:?}");
        let fragment = link_fragment(&share.id, &share.key);
        let key_text = fragment.rsplit('.').next().unwrap();
        assert!(!text.contains(key_text));
        assert!(!text.contains(&format!("{:?}", share.key)));
        assert!(text.contains("hidden"));
    }

    #[test]
    fn a_response_over_the_cap_is_refused_before_it_is_read() {
        assert_eq!(check_length(None), Ok(()));
        assert_eq!(check_length(Some(0)), Ok(()));
        assert_eq!(check_length(Some(MAX_BLOB_BYTES as u64)), Ok(()));
        assert_eq!(
            check_length(Some(MAX_BLOB_BYTES as u64 + 1)),
            Err(Failure::Damaged)
        );
        assert_eq!(check_length(Some(u64::MAX)), Err(Failure::Damaged));
    }

    #[test]
    fn the_collector_stops_at_its_cap_whatever_the_response_said() {
        let mut c = Collector::with_cap(100);
        c.extend(&[7u8; 60]).unwrap();
        assert!(c.fits(40));
        c.extend(&[7u8; 40]).unwrap();
        assert!(!c.fits(1));
        assert_eq!(c.extend(&[1]), Err(Failure::Damaged));
        assert_eq!(c.into_bytes().len(), 100);
        let c = Collector::with_cap(10);
        assert!(!c.fits(u64::MAX));
        assert_eq!(Collector::default().cap, MAX_BLOB_BYTES as u64);
    }

    #[test]
    fn a_share_fragment_is_taken_out_of_the_address_before_any_request() {
        let share = seal(b"model").unwrap();
        let good = format!("#{}", link_fragment(&share.id, &share.key));
        assert!(classify(&good).forgets_fragment());
        assert!(classify("#v1.x.y").forgets_fragment(), "damaged too");
        assert!(!classify("#top").forgets_fragment());
        assert!(!classify("").forgets_fragment());
    }

    #[test]
    fn the_damaged_message_is_the_apps() {
        assert_eq!(DAMAGED, app::app::DAMAGED_LINK);
    }

    #[test]
    fn the_messages_are_the_ones_the_page_shows() {
        assert_eq!(
            Failure::Damaged.message(),
            "This link is damaged or expired"
        );
        assert_eq!(
            Failure::Unreachable.message(),
            "Could not reach the computer sharing this model"
        );
    }
}
