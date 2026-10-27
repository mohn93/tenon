//! Decoding the buffer views of glTF's `EXT_meshopt_compression`: attributes (mode 0),
//! triangles (mode 1) and indices (mode 2), with the octahedral, quaternion and exponential
//! filters. Written from the extension's published bitstream text. Every read is bounds
//! checked, and a stream that is cut short, begins with the wrong byte or leaves bytes over
//! is an error.

/// How a compressed view's bytes are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Attributes,
    Triangles,
    Indices,
}

/// What is done to each element after decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    None,
    Octahedral,
    Quaternion,
    Exponential,
}

const CUT_SHORT: &str = "a compressed buffer view is cut short";
const BYTES_LEFT: &str = "a compressed buffer view has bytes left over";

/// The `count` elements of `stride` bytes each that `data` holds, filtered. `stride` is
/// 1-256 and a multiple of 4 for attributes, 2 or 4 for the index modes.
pub(crate) fn decode(
    mode: Mode,
    filter: Filter,
    data: &[u8],
    count: usize,
    stride: usize,
) -> Result<Vec<u8>, String> {
    let size = count
        .checked_mul(stride)
        .ok_or("a compressed buffer view is too large")?;
    let mut out = match mode {
        Mode::Attributes => {
            if stride == 0 || stride > 256 || !stride.is_multiple_of(4) {
                return Err("a compressed attribute view's stride is not a multiple of 4".into());
            }
            attributes(data, count, stride)?
        }
        Mode::Triangles => {
            if !matches!(stride, 2 | 4) || !count.is_multiple_of(3) {
                return Err("a compressed triangle view's stride or count is wrong".into());
            }
            triangles(data, count, stride)?
        }
        Mode::Indices => {
            if !matches!(stride, 2 | 4) {
                return Err("a compressed index view's stride is not 2 or 4".into());
            }
            indices(data, count, stride)?
        }
    };
    debug_assert_eq!(out.len(), size);
    match filter {
        Filter::None => {}
        _ if mode != Mode::Attributes => {
            return Err("a filter is set on a compressed view that is not attributes".into());
        }
        Filter::Octahedral => octahedral(&mut out, stride)?,
        Filter::Quaternion => quaternion(&mut out, stride)?,
        Filter::Exponential => exponential(&mut out),
    }
    Ok(out)
}

fn zigzag8(v: u8) -> u8 {
    if v & 1 != 0 { !(v >> 1) } else { v >> 1 }
}

/// A byte stream read from the front.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len());
        let end = end.ok_or(CUT_SHORT)?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    /// An unsigned LEB128 value of at most 32 bits.
    fn varint(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.byte()?;
            v |= u32::from(b & 0x7f)
                .checked_shl(shift)
                .ok_or("a compressed buffer view has a number that is too large")?;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("a compressed buffer view has a number that is too large".into())
    }

    fn done(&self) -> Result<(), String> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(BYTES_LEFT.into())
        }
    }
}

/// Mode 0: deinterleaved byte deltas in blocks of groups of 16 elements.
fn attributes(data: &[u8], count: usize, stride: usize) -> Result<Vec<u8>, String> {
    let tail = stride.max(32);
    if data.len() < 1 + tail {
        return Err(CUT_SHORT.into());
    }
    if data[0] != 0xa0 {
        return Err("a compressed attribute view does not begin with 0xa0".into());
    }
    let body_end = data.len() - tail;
    let mut r = Reader {
        bytes: &data[..body_end],
        at: 1,
    };
    // The tail is the baseline element, zero padded in front to 32 bytes.
    let mut prev: Vec<u8> = data[data.len() - stride..].to_vec();
    let mut out = vec![0u8; count * stride];
    let max_block = ((8192 / stride) & !15).min(256);
    let mut first = 0;
    while first < count {
        let block = (count - first).min(max_block);
        let groups = block.div_ceil(16);
        let header_bytes = groups.div_ceil(4);
        for k in 0..stride {
            let header = r.take(header_bytes)?;
            let mut element = first;
            for g in 0..groups {
                let mode = (header[g / 4] >> ((g % 4) * 2)) & 3;
                let mut deltas = [0u8; 16];
                match mode {
                    0 => {}
                    1 | 2 => {
                        let (bits, sentinel) = if mode == 1 { (2, 3) } else { (4, 15) };
                        let packed = r.take(if mode == 1 { 4 } else { 8 })?;
                        for (i, d) in deltas.iter_mut().enumerate() {
                            let per_byte = 8 / bits;
                            let shift = 8 - bits * (i % per_byte + 1);
                            let v = (packed[i / per_byte] >> shift) & sentinel;
                            *d = if v == sentinel { r.byte()? } else { v };
                        }
                    }
                    _ => deltas.copy_from_slice(r.take(16)?),
                }
                for d in deltas {
                    if element < first + block {
                        let v = prev[k].wrapping_add(zigzag8(d));
                        out[element * stride + k] = v;
                        prev[k] = v;
                    }
                    element += 1;
                }
            }
        }
        first += block;
    }
    r.done()?;
    Ok(out)
}

/// A FIFO of the 16 latest pushes, the latest at index 0.
struct Fifo<T> {
    items: [T; 16],
    next: usize,
}

impl<T: Copy + Default> Fifo<T> {
    fn new() -> Self {
        Fifo {
            items: [T::default(); 16],
            next: 0,
        }
    }

    fn push(&mut self, v: T) {
        self.items[self.next & 15] = v;
        self.next = self.next.wrapping_add(1);
    }

    fn get(&self, i: usize) -> T {
        self.items[self.next.wrapping_sub(1).wrapping_sub(i) & 15]
    }
}

fn put_index(out: &mut Vec<u8>, stride: usize, v: u32) -> Result<(), String> {
    if stride == 2 {
        let v = u16::try_from(v).map_err(|_| "a compressed index does not fit 16 bits")?;
        out.extend_from_slice(&v.to_le_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(())
}

/// Mode 1: a code byte per triangle over edge and vertex FIFOs.
fn triangles(data: &[u8], count: usize, stride: usize) -> Result<Vec<u8>, String> {
    let tris = count / 3;
    if data.len() < 1 + tris + 16 {
        return Err(CUT_SHORT.into());
    }
    if data[0] != 0xe1 {
        return Err("a compressed triangle view does not begin with 0xe1".into());
    }
    let codes = &data[1..1 + tris];
    let aux = &data[data.len() - 16..];
    let mut r = Reader {
        bytes: &data[1 + tris..data.len() - 16],
        at: 0,
    };
    let (mut next, mut last) = (0u32, 0u32);
    let mut edges: Fifo<(u32, u32)> = Fifo::new();
    let mut vertices: Fifo<u32> = Fifo::new();
    let mut out = Vec::with_capacity(count * stride);
    fn read_index(r: &mut Reader, last: &mut u32) -> Result<u32, String> {
        let v = r.varint()?;
        let delta = if v & 1 != 0 { !(v >> 1) } else { v >> 1 };
        *last = last.wrapping_add(delta);
        Ok(*last)
    }
    for &code in codes {
        let (x, y) = ((code >> 4) as usize, (code & 15) as usize);
        let (a, b, c);
        if code < 0xf0 {
            let (ea, eb) = edges.get(x);
            a = ea;
            b = eb;
            c = match y {
                0 => {
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                }
                1..=12 => vertices.get(y),
                13 => {
                    last = last.wrapping_sub(1);
                    last
                }
                14 => {
                    last = last.wrapping_add(1);
                    last
                }
                _ => read_index(&mut r, &mut last)?,
            };
            edges.push((c, b));
            edges.push((a, c));
            if y == 0 || y >= 13 {
                vertices.push(c);
            }
        } else {
            let (mut push_b, mut push_c) = (false, false);
            if y < 14 {
                let zw = aux[y];
                let (z, w) = ((zw >> 4) as usize, (zw & 15) as usize);
                a = next;
                next = next.wrapping_add(1);
                b = if z == 0 {
                    push_b = true;
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                } else {
                    vertices.get(z - 1)
                };
                c = if w == 0 {
                    push_c = true;
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                } else {
                    vertices.get(w - 1)
                };
            } else {
                let zw = r.byte()?;
                if zw == 0 {
                    next = 0;
                }
                let (z, w) = ((zw >> 4) as usize, (zw & 15) as usize);
                a = if y == 14 {
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                } else {
                    read_index(&mut r, &mut last)?
                };
                b = match z {
                    0 => {
                        push_b = true;
                        next = next.wrapping_add(1);
                        next.wrapping_sub(1)
                    }
                    15 => {
                        push_b = true;
                        read_index(&mut r, &mut last)?
                    }
                    _ => vertices.get(z - 1),
                };
                c = match w {
                    0 => {
                        push_c = true;
                        next = next.wrapping_add(1);
                        next.wrapping_sub(1)
                    }
                    15 => {
                        push_c = true;
                        read_index(&mut r, &mut last)?
                    }
                    _ => vertices.get(w - 1),
                };
            }
            edges.push((b, a));
            edges.push((c, b));
            edges.push((a, c));
            vertices.push(a);
            if push_b {
                vertices.push(b);
            }
            if push_c {
                vertices.push(c);
            }
        }
        for v in [a, b, c] {
            put_index(&mut out, stride, v)?;
        }
    }
    r.done()?;
    Ok(out)
}

/// Mode 2: deltas against two running baselines.
fn indices(data: &[u8], count: usize, stride: usize) -> Result<Vec<u8>, String> {
    if data.len() < 1 + 4 {
        return Err(CUT_SHORT.into());
    }
    if data[0] != 0xd1 {
        return Err("a compressed index view does not begin with 0xd1".into());
    }
    let mut r = Reader {
        bytes: &data[..data.len() - 4],
        at: 1,
    };
    let mut last = [0u32; 2];
    let mut out = Vec::with_capacity(count * stride);
    for _ in 0..count {
        let v = r.varint()?;
        let base = (v & 1) as usize;
        let delta = if v & 2 != 0 { !(v >> 2) } else { v >> 2 };
        last[base] = last[base].wrapping_add(delta);
        put_index(&mut out, stride, last[base])?;
    }
    r.done()?;
    Ok(out)
}

/// Filter 1: unit vectors in octahedral form, as 8-bit (stride 4) or 16-bit (stride 8)
/// normalized components; the fourth component is kept.
fn octahedral(out: &mut [u8], stride: usize) -> Result<(), String> {
    let decode = |input: [i32; 3], max: f32| -> [i32; 3] {
        let one = input[2] as f32;
        if one <= 0.0 {
            return [0; 3];
        }
        let mut x = input[0] as f32 / one;
        let mut y = input[1] as f32 / one;
        let z = 1.0 - x.abs() - y.abs();
        let t = z.min(0.0);
        x -= t.copysign(x);
        y -= t.copysign(y);
        let len = (x * x + y * y + z * z).sqrt();
        [x / len, y / len, z / len].map(|v| (v * max).round() as i32)
    };
    match stride {
        4 => {
            for e in out.as_chunks_mut::<4>().0 {
                let r = decode([e[0] as i8, e[1] as i8, e[2] as i8].map(i32::from), 127.0);
                for i in 0..3 {
                    e[i] = r[i] as i8 as u8;
                }
            }
        }
        8 => {
            for e in out.as_chunks_mut::<8>().0 {
                let v = |i: usize| i32::from(i16::from_le_bytes([e[2 * i], e[2 * i + 1]]));
                let r = decode([v(0), v(1), v(2)], 32767.0);
                for i in 0..3 {
                    e[2 * i..2 * i + 2].copy_from_slice(&(r[i] as i16).to_le_bytes());
                }
            }
        }
        _ => return Err("an octahedral filter needs a stride of 4 or 8".into()),
    }
    Ok(())
}

/// Filter 2: unit quaternions as three components and the index of the omitted largest.
fn quaternion(out: &mut [u8], stride: usize) -> Result<(), String> {
    if stride != 8 {
        return Err("a quaternion filter needs a stride of 8".into());
    }
    let range = std::f32::consts::FRAC_1_SQRT_2;
    for e in out.as_chunks_mut::<8>().0 {
        let v = |i: usize| i16::from_le_bytes([e[2 * i], e[2 * i + 1]]);
        let input = [v(0), v(1), v(2), v(3)];
        let one = f32::from(input[3] | 3);
        let [x, y, z] = [0, 1, 2].map(|i| f32::from(input[i]) / one * range);
        let w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
        let max = (input[3] & 3) as usize;
        let mut res = [0i16; 4];
        res[(max + 1) % 4] = (x * 32767.0).round() as i16;
        res[(max + 2) % 4] = (y * 32767.0).round() as i16;
        res[(max + 3) % 4] = (z * 32767.0).round() as i16;
        res[max] = (w * 32767.0).round() as i16;
        for (i, r) in res.iter().enumerate() {
            e[2 * i..2 * i + 2].copy_from_slice(&r.to_le_bytes());
        }
    }
    Ok(())
}

/// Filter 3: 32-bit floats as an 8-bit exponent and 24-bit mantissa.
fn exponential(out: &mut [u8]) {
    for e in out.as_chunks_mut::<4>().0 {
        let v = i32::from_le_bytes([e[0], e[1], e[2], e[3]]);
        let (exp, mantissa) = (v >> 24, (v << 8) >> 8);
        // ldexp: exact in f64 for every exponent, then rounded once.
        let f = (f64::from(mantissa) * 2f64.powi(exp)) as f32;
        e.copy_from_slice(&f.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshopt_rs::index::IndexEncodingVersion;
    use meshopt_rs::vertex::VertexEncodingVersion;

    fn encode_vertices(bytes: &[u8], stride: usize) -> Vec<u8> {
        let count = bytes.len() / stride;
        let mut buf =
            vec![0u8; meshopt_rs::vertex::buffer::encode_vertex_buffer_bound(count, stride)];
        // The encoder is generic over a `Vertex` of a fixed size: use arrays.
        macro_rules! go {
            ($n:literal) => {{
                let v: Vec<[u8; $n]> = bytes
                    .chunks_exact($n)
                    .map(|c| c.try_into().unwrap())
                    .collect();
                meshopt_rs::vertex::buffer::encode_vertex_buffer(
                    &mut buf,
                    &v,
                    VertexEncodingVersion::V0,
                )
                .unwrap()
            }};
        }
        let n = match stride {
            4 => go!(4),
            16 => go!(16),
            36 => go!(36),
            64 => go!(64),
            _ => unreachable!(),
        };
        buf.truncate(n);
        buf
    }

    /// Deterministic bytes with runs, small steps and noise, so every group mode appears.
    fn sample(count: usize, stride: usize) -> Vec<u8> {
        let mut s = 12345u32;
        let mut out = Vec::new();
        for i in 0..count {
            for k in 0..stride {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (s >> 24) as u8;
                out.push(match k % 4 {
                    0 => (i / 7) as u8,
                    1 => (i * 3) as u8,
                    2 => (i as u8).wrapping_add(noise & 3),
                    _ => noise,
                });
            }
        }
        out
    }

    #[test]
    fn attributes_round_trip_across_block_sizes() {
        for (count, stride) in [
            (1, 4),
            (16, 4),
            (17, 16),
            (300, 4),
            (500, 16),
            (200, 64),
            (45, 36),
        ] {
            let bytes = sample(count, stride);
            let packed = encode_vertices(&bytes, stride);
            let out = decode(Mode::Attributes, Filter::None, &packed, count, stride).unwrap();
            assert_eq!(out, bytes, "{count} x {stride}");
        }
    }

    #[test]
    fn the_spec_example_group_decodes() {
        // One 4-bit sentinel group, from the extension text, as a lone byte column over 16
        // elements (stride 4, so three more all-zero columns); baseline 0.
        let mut data = vec![0xa0u8];
        data.push(0b10); // column 0: header, group 0 in mode 2
        data.extend([0x17, 0x5f, 0xf0, 0xbc, 0x77, 0xa9, 0x21, 0x00, 0x34, 0xb5]);
        data.extend([0, 0, 0]); // columns 1-3: headers, all groups zero
        data.extend([0u8; 32]);
        let out = decode(Mode::Attributes, Filter::None, &data, 16, 4).unwrap();
        let deltas = [-1i32, -4, -3, 26, -91, 0, -6, 6, -4, -4, 5, -5, 1, -1, 0, 0];
        let mut v = 0i32;
        for (i, d) in deltas.iter().enumerate() {
            v += d;
            assert_eq!(out[i * 4], v as u8, "{i}");
        }
    }

    #[test]
    fn triangles_round_trip() {
        // A strip-like grid, then a stream with fans and an explicit far index.
        let mut tris: Vec<u32> = Vec::new();
        for i in 0..40u32 {
            tris.extend([i, i + 1, i + 2]);
        }
        tris.extend([100, 3, 250, 250, 3, 17, 7, 100, 9]);
        let mut buf =
            vec![0u8; meshopt_rs::index::buffer::encode_index_buffer_bound(tris.len(), 300)];
        let n = meshopt_rs::index::buffer::encode_index_buffer(
            &mut buf,
            &tris,
            IndexEncodingVersion::V1,
        )
        .unwrap();
        for stride in [2usize, 4] {
            let out = decode(Mode::Triangles, Filter::None, &buf[..n], tris.len(), stride).unwrap();
            let got: Vec<u32> = out
                .chunks_exact(stride)
                .map(|c| {
                    let mut b = [0u8; 4];
                    b[..stride].copy_from_slice(c);
                    u32::from_le_bytes(b)
                })
                .collect();
            // The encoder may rotate a triangle's corners; compare as sets of rotations.
            for (g, w) in got.chunks(3).zip(tris.chunks(3)) {
                let rotations = [[w[0], w[1], w[2]], [w[1], w[2], w[0]], [w[2], w[0], w[1]]];
                assert!(rotations.contains(&[g[0], g[1], g[2]]), "{g:?} {w:?}");
            }
        }
    }

    #[test]
    fn indices_decode_with_two_baselines() {
        // Encode by hand: values alternating between two ascending runs.
        let values = [0u32, 1000, 1, 1001, 2, 1002, 3, 900];
        let mut data = vec![0xd1u8];
        let mut last = [0u32; 2];
        for (i, v) in values.iter().enumerate() {
            let base = i % 2;
            let delta = (i64::from(*v) - i64::from(last[base])) as i32;
            last[base] = *v;
            let (neg, mag) = if delta < 0 {
                (2u32, !(delta as u32))
            } else {
                (0, delta as u32)
            };
            let mut x = (mag << 2) | neg | base as u32;
            loop {
                let b = (x & 0x7f) as u8;
                x >>= 7;
                if x == 0 {
                    data.push(b);
                    break;
                }
                data.push(b | 0x80);
            }
        }
        data.extend([0u8; 4]);
        let out = decode(Mode::Indices, Filter::None, &data, values.len(), 4).unwrap();
        let got: Vec<u32> = out
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        assert_eq!(got, values);
    }

    #[test]
    fn filters_decode_as_the_text_says() {
        // Octahedral, 8 bit, K = 8: (127, 0, 127) is (1, 0, 0) after the fixup-free path.
        let mut v = vec![127u8, 0, 127, 5];
        octahedral(&mut v, 4).unwrap();
        assert_eq!(v, [127, 0, 0, 5]);
        // Straight up: x = y = 0 is z = 1.
        let mut v = vec![0u8, 0, 127, 0];
        octahedral(&mut v, 4).unwrap();
        assert_eq!(v, [0, 0, 127, 0]);
        // 16 bit with K = 12 (one is 2047): (1023.5, 0) is half way along x, so z = 0.5.
        let mut v = Vec::new();
        for c in [1024i16, 0, 2047, 9] {
            v.extend(c.to_le_bytes());
        }
        octahedral(&mut v, 8).unwrap();
        let c = |i: usize| f32::from(i16::from_le_bytes([v[2 * i], v[2 * i + 1]])) / 32767.0;
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((c(0) - h).abs() < 1e-3 && c(1).abs() < 1e-3 && (c(2) - h).abs() < 1e-3);
        assert_eq!(i16::from_le_bytes([v[6], v[7]]), 9);
        // The bottom corner folds: (2047, 2047) is (0, 0, -1).
        let mut v = Vec::new();
        for c in [2047i16, 2047, 2047, 0] {
            v.extend(c.to_le_bytes());
        }
        octahedral(&mut v, 8).unwrap();
        assert_eq!(i16::from_le_bytes([v[4], v[5]]), -32767);
        // Quaternion: the identity with w the largest (index 3), K = 16.
        let mut v = Vec::new();
        for c in [0i16, 0, 0, 32767 & !3 | 3] {
            v.extend(c.to_le_bytes());
        }
        quaternion(&mut v, 8).unwrap();
        let q: Vec<i16> = v
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert_eq!(q, [0, 0, 0, 32767]);
        // Exponential: exponent -2, mantissa 12 is 3.0; exponent 3, mantissa -1 is -8.0;
        // exponent 120, mantissa 1 is 2^120, and exponent -128 mantissa 4 is 2^-126.
        let mut v = Vec::new();
        for (e, m) in [(-2i32, 12i32), (3, -1), (120, 1), (-128, 4)] {
            v.extend((((e << 24) as u32) | (m as u32 & 0xff_ffff)).to_le_bytes());
        }
        exponential(&mut v);
        let f: Vec<f32> = v
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(f, [3.0, -8.0, 2f32.powi(120), 2f32.powi(-126)]);
    }

    #[test]
    fn broken_streams_are_errors_not_panics() {
        let bytes = sample(100, 16);
        let packed = encode_vertices(&bytes, 16);
        for cut in [0, 1, 20, packed.len() / 2, packed.len() - 1] {
            assert!(decode(Mode::Attributes, Filter::None, &packed[..cut], 100, 16).is_err());
        }
        let mut wrong = packed.clone();
        wrong[0] = 0;
        assert!(decode(Mode::Attributes, Filter::None, &wrong, 100, 16).is_err());
        let mut extra = packed.clone();
        extra.insert(packed.len() - 32, 0);
        assert!(decode(Mode::Attributes, Filter::None, &extra, 100, 16).is_err());
        // Garbage for every mode, from every byte value.
        for b in 0..=255u8 {
            let junk = vec![b; 64];
            for mode in [Mode::Attributes, Mode::Triangles, Mode::Indices] {
                let _ = decode(mode, Filter::None, &junk, 12, 4);
            }
        }
        assert!(decode(Mode::Triangles, Filter::None, &[0xe1; 3], 3, 4).is_err());
        assert!(decode(Mode::Attributes, Filter::Octahedral, &packed, 100, 16).is_err());
    }
}
