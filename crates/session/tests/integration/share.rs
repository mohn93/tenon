use session::ErrorKind;
use session::share::{Share, clip_name, link_fragment, open, parse_fragment, qr_modules, seal};

fn sample() -> Vec<u8> {
    (0..5000u32).map(|i| (i * 7 % 251) as u8).collect()
}

fn sealed() -> (Vec<u8>, Share) {
    let m = sample();
    let s = seal("House", &m).unwrap();
    (m, s)
}

#[test]
fn round_trip() {
    let (m, s) = sealed();
    assert!(s.blob.starts_with(b"SKAS1"));
    assert_eq!(s.blob.len(), 5 + 12 + 2 + "House".len() + m.len() + 16);
    assert_eq!(
        open(&s.blob, &s.id, &s.key).unwrap(),
        ("House".to_string(), m.clone())
    );
    let s2 = seal("House", &m).unwrap();
    assert_ne!(s.id, s2.id);
    assert_ne!(s.key, s2.key);
    assert_ne!(s.blob, s2.blob);
}

#[test]
fn empty_model_round_trips() {
    let s = seal("", &[]).unwrap();
    assert_eq!(
        open(&s.blob, &s.id, &s.key).unwrap(),
        (String::new(), Vec::<u8>::new())
    );
}

#[test]
fn the_name_is_inside_the_encryption() {
    let s = seal("Secret villa", &sample()).unwrap();
    let hay = &s.blob;
    assert!(!hay.windows(6).any(|w| w == b"Secret"));
    let (name, _) = open(&s.blob, &s.id, &s.key).unwrap();
    assert_eq!(name, "Secret villa");
}

#[test]
fn names_keep_their_text_and_are_cut_at_256_bytes() {
    for name in ["Haus am See", "Maison d'été", "家", "a b"] {
        let s = seal(name, b"M").unwrap();
        assert_eq!(open(&s.blob, &s.id, &s.key).unwrap().0, name);
    }
    let long = "x".repeat(300);
    let s = seal(&long, b"M").unwrap();
    let (name, model) = open(&s.blob, &s.id, &s.key).unwrap();
    assert_eq!(name, "x".repeat(256));
    assert_eq!(model, b"M");
    // A cut never splits a character: 3-byte characters, 256 is not a multiple of 3.
    let wide = "家".repeat(100);
    let (name, model) = {
        let s = seal(&wide, b"M").unwrap();
        open(&s.blob, &s.id, &s.key).unwrap()
    };
    assert_eq!(name, "家".repeat(85));
    assert_eq!(model, b"M");
}

/// A blob sealed by hand with `plain` as the plaintext, as a damaged or hostile sender might.
fn forged(plain: &[u8]) -> (Vec<u8>, [u8; 16], [u8; 32]) {
    use chacha20poly1305::aead::{Aead, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
    let (id, key, nonce) = ([5u8; 16], [6u8; 32], [7u8; 12]);
    let mut aad = b"SKAS1".to_vec();
    aad.extend_from_slice(&id);
    let ct = ChaCha20Poly1305::new(&Key::from(key))
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .unwrap();
    let mut blob = b"SKAS1".to_vec();
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ct);
    (blob, id, key)
}

#[test]
fn a_bad_name_header_is_refused() {
    let mut cases: Vec<Vec<u8>> = vec![
        vec![],
        vec![1],
        vec![5, 0, b'a'],
        vec![0, 1],
        vec![1, 1, b'a'],
    ];
    let mut too_long = 257u16.to_le_bytes().to_vec();
    too_long.extend(std::iter::repeat_n(b'a', 257));
    cases.push(too_long);
    cases.push(vec![2, 0, 0xff, 0xfe, b'M']);
    for plain in cases {
        let (blob, id, key) = forged(&plain);
        refused(open(&blob, &id, &key));
    }
    // And a good hand-made one opens.
    let (blob, id, key) = forged(&[2, 0, b'h', b'i', b'M']);
    assert_eq!(
        open(&blob, &id, &key).unwrap(),
        ("hi".to_string(), b"M".to_vec())
    );
}

fn refused(r: Result<(String, Vec<u8>), session::CommandError>) {
    let e = r.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(e.message.contains("damaged or expired"), "{}", e.message);
}

#[test]
fn every_flipped_byte_is_refused() {
    let (_, s) = sealed();
    for i in (0..s.blob.len()).step_by(37).chain([s.blob.len() - 1]) {
        let mut b = s.blob.clone();
        b[i] ^= 1;
        refused(open(&b, &s.id, &s.key));
    }
}

#[test]
fn truncation_is_refused() {
    let (_, s) = sealed();
    for n in [0, 4, 5, 10, 16, 17, 20, s.blob.len() - 1] {
        refused(open(&s.blob[..n], &s.id, &s.key));
    }
}

#[test]
fn wrong_key_id_and_prefix_are_refused() {
    let (_, s) = sealed();
    let mut k = s.key;
    k[0] ^= 1;
    refused(open(&s.blob, &s.id, &k));
    let mut id = s.id;
    id[15] ^= 1;
    refused(open(&s.blob, &id, &s.key));
    let mut b = s.blob.clone();
    b[..5].copy_from_slice(b"SKAS2");
    refused(open(&b, &s.id, &s.key));
}

#[test]
fn oversize_is_refused() {
    let big = vec![0u8; 256 * 1024 * 1024 + 1];
    assert_eq!(seal("", &big).unwrap_err().kind, ErrorKind::InvalidInput);
    let blob = vec![0u8; 256 * 1024 * 1024 + 64];
    assert_eq!(
        open(&blob, &[0; 16], &[0; 32]).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn fragment_round_trip() {
    let (_, s) = sealed();
    let f = link_fragment(&s.id, &s.key);
    assert!(f.starts_with("v1."));
    assert_eq!(f.split('.').count(), 3);
    assert!(!f.contains('=') && !f.contains('+') && !f.contains('/'));
    assert_eq!(parse_fragment(&f).unwrap(), (s.id, s.key));
}

#[test]
fn malformed_fragments_are_refused() {
    let id = [3u8; 16];
    let key = [9u8; 32];
    let good = link_fragment(&id, &key);
    let parts: Vec<&str> = good.split('.').collect();
    let (a, b) = (parts[1], parts[2]);
    let bad = [
        String::new(),
        "v1".into(),
        "v1..".into(),
        format!("v2.{a}.{b}"),
        format!("{a}.{b}"),
        format!("v1.{a}"),
        format!("v1.{a}.{b}.x"),
        format!("#{good}"),
        format!("v1.{b}.{a}"),
        format!("v1.{a}.{}", &b[..b.len() - 1]),
        format!("v1.{a}={}.{b}", ""),
        format!("v1.{a}+.{b}"),
        format!("v1.{a}.{b}="),
        format!("V1.{a}.{b}"),
        format!("v1.{}.{b}", &a[..a.len() - 1]),
    ];
    for f in bad {
        let e = parse_fragment(&f).expect_err(&f);
        assert_eq!(e.kind, ErrorKind::InvalidInput, "{f}");
    }
}

#[test]
fn qr_width_for_a_url_is_deterministic() {
    let url = "http://192.168.1.20:54321/s/0123456789abcdef0123456789abcdef0123456789a";
    assert!((68..=72).contains(&url.len()), "{}", url.len());
    let (w, m) = qr_modules(url).unwrap();
    assert!(w == 33 || w == 37, "{w}");
    assert_eq!(m.len(), w * w);
    assert!(m.iter().any(|&x| x) && m.iter().any(|&x| !x));
    // finder pattern corner: top-left module is dark
    assert!(m[0]);
    assert_eq!(qr_modules(url).unwrap(), (w, m));
}

#[test]
fn qr_for_a_real_share_link_fits_a_phone_camera() {
    let (id, key) = ([1u8; 16], [2u8; 32]);
    let url = format!("http://192.168.100.200:54321/#{}", link_fragment(&id, &key));
    let (w, m) = qr_modules(&url).unwrap();
    assert!(w <= 45, "{w}");
    assert_eq!(m.len(), w * w);
}

#[test]
fn clip_name_cuts_on_a_character() {
    assert_eq!(clip_name("House"), "House");
    assert_eq!(clip_name(&"x".repeat(300)).len(), 256);
    assert_eq!(clip_name(&"家".repeat(100)), "家".repeat(85));
}
