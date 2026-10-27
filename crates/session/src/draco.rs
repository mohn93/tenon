//! Decoding the Draco streams of glTF's `KHR_draco_mesh_compression` with `draco-core`: the
//! triangles of a primitive and the values of the attributes its extension names.

use draco_core::{DataType, DecodeLimits, DecoderBuffer, Mesh, MeshDecoder};

use crate::import::{MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS};

/// One attribute to take from the stream: the unique id the extension gives it, how many
/// components a value has, and whether the glTF accessor reads integers as normalized.
pub(crate) struct Wanted {
    pub id: u32,
    pub width: usize,
    pub normalized: bool,
}

/// What a stream holds for a primitive.
pub(crate) struct Decoded {
    /// Corner indices, three per triangle.
    pub indices: Vec<usize>,
    /// For each wanted attribute, `points * width` numbers.
    pub values: Vec<Vec<f64>>,
}

/// The largest value an integer attribute type holds, for normalizing it to 0..1 or -1..1.
fn integer_max(t: DataType) -> Option<f64> {
    match t {
        DataType::Uint8 => Some(255.0),
        DataType::Int8 => Some(127.0),
        DataType::Uint16 => Some(65535.0),
        DataType::Int16 => Some(32767.0),
        _ => None,
    }
}

/// Decodes the triangle mesh in `bytes` and reads the `wanted` attributes. Fails, saying why,
/// for a stream that is not a triangle mesh, passes the importer's point and triangle limits
/// or lacks an attribute.
pub(crate) fn decode(bytes: &[u8], wanted: &[Wanted]) -> Result<Decoded, String> {
    let limits = DecodeLimits::default()
        .with_max_points(MAX_IMPORT_POINTS as u64)
        .with_max_faces(MAX_IMPORT_POLYGONS as u64)
        .with_max_decoded_bytes(256 << 20);
    let mut buffer = DecoderBuffer::new(bytes).with_limits(limits);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .map_err(|e| format!("{e}"))?;
    let points = mesh.num_points();
    if points > MAX_IMPORT_POINTS {
        return Err(format!("more than {MAX_IMPORT_POINTS} positions"));
    }
    if mesh.num_faces() > MAX_IMPORT_POLYGONS {
        return Err(format!("more than {MAX_IMPORT_POLYGONS} triangles"));
    }
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for f in mesh.faces() {
        for c in f {
            indices.push(c.0 as usize);
        }
    }
    let mut values = Vec::with_capacity(wanted.len());
    for w in wanted {
        let attribute = mesh
            .attribute_by_unique_id(w.id)
            .ok_or(format!("the stream has no attribute {}", w.id))?;
        if (attribute.num_components() as usize) < w.width {
            return Err(format!("attribute {} has too few components", w.id));
        }
        let scale = if attribute.normalized() || w.normalized {
            integer_max(attribute.data_type())
        } else {
            None
        };
        let read = attribute.read_f32s(points, w.width);
        values.push(
            read.into_iter()
                .map(|v| {
                    let v = f64::from(v);
                    match scale {
                        Some(max) => (v / max).max(-1.0),
                        None => v,
                    }
                })
                .collect(),
        );
    }
    Ok(Decoded { indices, values })
}
