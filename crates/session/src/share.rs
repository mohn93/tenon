//! Sharing a model with a phone: the encrypted envelope, the link fragment that carries the key,
//! and the QR matrix for the link.
//!
//! The blob is `b"SKAS1" || nonce(12) || ciphertext+tag` (ChaCha20-Poly1305, the associated data
//! is `b"SKAS1" || id`). The sealed plaintext is `name_len (u16 LE) || name (UTF-8, at most 256
//! bytes) || model bytes`, so the model's name is encrypted with it. Only the fragment of the
//! link holds the key, so the server that hands out the blob never sees it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use qrcode::{Color, EcLevel, QrCode};

use crate::{CommandError, ErrorKind};

const MAGIC: &[u8; 5] = b"SKAS1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// Largest model (and blob) accepted, as for `.tnn` files.
pub const MAX_BLOB_BYTES: usize = 256 * 1024 * 1024;
/// Longest model name carried, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 256;

/// A sealed model: the random `id` names the blob on the server, `key` stays in the link
/// fragment, `blob` is what the server hands out.
pub struct Share {
    pub id: [u8; 16],
    pub key: [u8; 32],
    pub blob: Vec<u8>,
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Share")
            .field("id", &self.id)
            .field("key", &"<hidden>")
            .field("blob_len", &self.blob.len())
            .finish()
    }
}

fn invalid(message: &str) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

fn damaged() -> CommandError {
    invalid("This link is damaged or expired")
}

fn aad(id: &[u8; 16]) -> Vec<u8> {
    let mut a = Vec::with_capacity(MAGIC.len() + id.len());
    a.extend_from_slice(MAGIC);
    a.extend_from_slice(id);
    a
}

/// `name` cut to at most [`MAX_NAME_BYTES`] bytes, at a character boundary: what a share carries.
pub fn clip_name(name: &str) -> &str {
    let mut end = name.len().min(MAX_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// Encrypts `model` and its `name` (cut to 256 bytes) under a fresh random key. A model over
/// 256 MiB is `invalid_input`.
///
/// # Errors
/// `invalid_input` for an oversize model or if the OS random source fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn seal(name: &str, model: &[u8]) -> Result<Share, CommandError> {
    let name = clip_name(name);
    let header = 2 + name.len();
    if model.len() > MAX_BLOB_BYTES - MAGIC.len() - NONCE_LEN - TAG_LEN - header {
        return Err(invalid("The model is too large to share (over 256 MiB)"));
    }
    let mut plain = Vec::with_capacity(header + model.len());
    plain.extend_from_slice(&(name.len() as u16).to_le_bytes());
    plain.extend_from_slice(name.as_bytes());
    plain.extend_from_slice(model);
    let mut id = [0u8; 16];
    let mut key = [0u8; 32];
    let mut nonce = [0u8; NONCE_LEN];
    for buf in [&mut id[..], &mut key[..], &mut nonce[..]] {
        getrandom::fill(buf)
            .map_err(|e| invalid(&format!("No secure random source is available: {e}")))?;
    }
    let cipher = ChaCha20Poly1305::new(&Key::from(key));
    let ct = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &plain,
                aad: &aad(&id),
            },
        )
        .map_err(|_| invalid("Encryption failed"))?;
    let mut blob = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ct);
    Ok(Share { id, key, blob })
}

/// Decrypts a blob made by [`seal`]: the model's name and the model's bytes.
///
/// # Errors
/// `invalid_input` ("This link is damaged or expired") for a wrong key or id, a changed or
/// truncated blob, a wrong prefix or a name that is too long or not UTF-8; a blob over 256 MiB
/// is refused as too large.
pub fn open(blob: &[u8], id: &[u8; 16], key: &[u8; 32]) -> Result<(String, Vec<u8>), CommandError> {
    if blob.len() > MAX_BLOB_BYTES {
        return Err(invalid("The shared model is too large (over 256 MiB)"));
    }
    if blob.len() < MAGIC.len() + NONCE_LEN + TAG_LEN || &blob[..MAGIC.len()] != MAGIC {
        return Err(damaged());
    }
    let nonce =
        Nonce::try_from(&blob[MAGIC.len()..MAGIC.len() + NONCE_LEN]).map_err(|_| damaged())?;
    let ct = &blob[MAGIC.len() + NONCE_LEN..];
    let mut plain = ChaCha20Poly1305::new(&Key::from(*key))
        .decrypt(
            &nonce,
            Payload {
                msg: ct,
                aad: &aad(id),
            },
        )
        .map_err(|_| damaged())?;
    let len = match plain.as_slice() {
        [a, b, ..] => usize::from(u16::from_le_bytes([*a, *b])),
        _ => return Err(damaged()),
    };
    if len > MAX_NAME_BYTES || plain.len() < 2 + len {
        return Err(damaged());
    }
    let name = std::str::from_utf8(&plain[2..2 + len])
        .map_err(|_| damaged())?
        .to_string();
    plain.drain(..2 + len);
    Ok((name, plain))
}

/// The link fragment (without `#`): `v1.<base64url id>.<base64url key>`, no padding.
#[must_use]
pub fn link_fragment(id: &[u8; 16], key: &[u8; 32]) -> String {
    format!(
        "v1.{}.{}",
        URL_SAFE_NO_PAD.encode(id),
        URL_SAFE_NO_PAD.encode(key)
    )
}

/// Parses a fragment made by [`link_fragment`]; anything else is `invalid_input`.
///
/// # Errors
/// `invalid_input` for a wrong version, part count, alphabet, padding or length.
pub fn parse_fragment(fragment: &str) -> Result<([u8; 16], [u8; 32]), CommandError> {
    let bad = || damaged();
    let mut parts = fragment.split('.');
    let (Some("v1"), Some(id), Some(key), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let id: [u8; 16] = URL_SAFE_NO_PAD
        .decode(id)
        .map_err(|_| bad())?
        .try_into()
        .map_err(|_| bad())?;
    let key: [u8; 32] = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| bad())?
        .try_into()
        .map_err(|_| bad())?;
    Ok((id, key))
}

/// The QR code for `text` (error correction M): its width in modules and the modules row by
/// row, `true` for dark. The quiet zone is not included.
///
/// # Errors
/// `invalid_input` if the text is too long for a QR code.
pub fn qr_modules(text: &str) -> Result<(usize, Vec<bool>), CommandError> {
    let code = QrCode::with_error_correction_level(text, EcLevel::M)
        .map_err(|_| invalid("The link is too long for a QR code"))?;
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| c == Color::Dark)
        .collect();
    Ok((code.width(), modules))
}
