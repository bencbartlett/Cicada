//! Binary geometry frames (docs/13 §Binary frame format) — typed arrays
//! ready for GPU upload, zero parsing on the client beyond the header. This
//! module IS the byte-exact spec (docs/13 points here); the client mirrors
//! it in `web/src/protocol/frames.ts`, and the decoder below keeps the two
//! honest through round-trip tests.
//!
//! All integers and floats are **little-endian**. Every array starts at a
//! 4-byte-aligned offset, so the client builds `Float32Array` /
//! `Uint32Array` views straight over the received `ArrayBuffer`.
//!
//! ```text
//! Header (32 bytes, every frame)
//!   0  u32  magic          = 0x4643_4943  (the bytes "CICF")
//!   4  u16  version        = 1
//!   6  u16  kind           1 mesh · 2 curve · 3 point · 4 clear · 5 mesh_blob · 6 instances
//!   8  u64  generation     solve generation (the client drops older frames per node)
//!  16  u32  node           view-model node ref
//!  20  u32  output         output port index
//!  24  u32  element_start  first element index this frame covers
//!  28  u32  element_count  elements covered (spike: always the whole output)
//!
//! Batch body (kinds 1–3)
//!  32  u32  E   element-table entries
//!  36  u32  V   vertices
//!  40  u32  I   indices (mesh: 3 per triangle · curve: 2 per segment · point: 0)
//!  44  E × { u32 element_index, u32 pick_id, u32 vertex_start, u32 vertex_count,
//!            u32 index_start, u32 index_count }                          (24 bytes each)
//!      f32  positions[3·V]
//!      u32  indices[I]          (frame-global vertex indices)
//!      u32  pick_ids[V]         (per vertex — the ID-buffer attribute)
//!
//! Clear (kind 4): no body — the output draws nothing this generation.
//!
//! Mesh blob (kind 5): a content-addressed mesh, referenced by instances
//!  32  [32] hash (blake3)
//!  64  u32  V
//!  68  u32  I
//!  72  f32  positions[3·V] · u32 indices[I]
//!
//! Instances (kind 6): every element of this output sharing one mesh hash
//!  32  [32] hash
//!  64  u32  N
//!  68  N × { u32 element_index, u32 pick_id, f32 transform[12] (3×4 row-major) }  (56 bytes each)
//! ```
//!
//! Normals are NOT transmitted: the viewport shades flat via screen-space
//! derivatives (CAD-correct hard edges for boxes and carves, no vertex
//! duplication, half the mesh bandwidth); smooth normals arrive with the
//! display-cost work if a use case needs them. Instance transforms are the
//! identity in the spike — instancing is hash-driven (identical mesh hashes
//! across elements arrive once), and detecting rigid copies with different
//! hashes is v0.1 interner work.

use cicada_core::hash::ValueHash;

/// The header magic, `b"CICF"` read as a little-endian `u32`.
pub const MAGIC: u32 = 0x4643_4943;
/// Frame format version.
pub const VERSION: u16 = 1;
/// Header size in bytes.
pub const HEADER_LEN: usize = 32;

/// Frame kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum FrameKind {
    /// Triangle meshes.
    Mesh = 1,
    /// Polylines as line segments.
    Curve = 2,
    /// Points.
    Point = 3,
    /// Nothing to draw for this output.
    Clear = 4,
    /// Content-addressed mesh referenced by instances.
    MeshBlob = 5,
    /// Instances of a mesh blob.
    Instances = 6,
}

impl FrameKind {
    fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::Mesh,
            2 => Self::Curve,
            3 => Self::Point,
            4 => Self::Clear,
            5 => Self::MeshBlob,
            6 => Self::Instances,
            _ => return None,
        })
    }
}

/// The header of every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Frame kind.
    pub kind: FrameKind,
    /// Solve generation.
    pub generation: u64,
    /// View-model node ref.
    pub node: u32,
    /// Output port index.
    pub output: u32,
    /// First element covered.
    pub element_start: u32,
    /// Elements covered.
    pub element_count: u32,
}

/// One element-table entry of a batch frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementEntry {
    /// The element's index within the output value (top-level slot).
    pub element_index: u32,
    /// Pick id (0 = unpickable).
    pub pick_id: u32,
    /// First vertex.
    pub vertex_start: u32,
    /// Vertex count.
    pub vertex_count: u32,
    /// First index.
    pub index_start: u32,
    /// Index count.
    pub index_count: u32,
}

/// A batch frame's body (mesh / curve / point).
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// Element table.
    pub elements: Vec<ElementEntry>,
    /// Positions, `xyz` interleaved.
    pub positions: Vec<f32>,
    /// Indices (frame-global).
    pub indices: Vec<u32>,
    /// Per-vertex pick ids.
    pub pick_ids: Vec<u32>,
}

impl Batch {
    /// An empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            positions: Vec::new(),
            indices: Vec::new(),
            pick_ids: Vec::new(),
        }
    }

    /// Append one element's geometry: `positions` (f64 xyz), local
    /// `indices`, one pick id for every vertex.
    ///
    /// # Panics
    ///
    /// When `positions.len()` is not a multiple of 3, or the batch would
    /// exceed `u32` vertex/index counts — invariant violations, surfaced.
    pub fn push_element(
        &mut self,
        element_index: u32,
        pick_id: u32,
        positions: &[f64],
        indices: &[u32],
    ) {
        assert!(
            positions.len().is_multiple_of(3),
            "positions are xyz triples"
        );
        let vertex_start = u32::try_from(self.positions.len() / 3)
            .unwrap_or_else(|_| panic!("frame vertex count exceeds u32"));
        let index_start = u32::try_from(self.indices.len())
            .unwrap_or_else(|_| panic!("frame index count exceeds u32"));
        let vertex_count = u32::try_from(positions.len() / 3)
            .unwrap_or_else(|_| panic!("element vertex count exceeds u32"));
        let index_count = u32::try_from(indices.len())
            .unwrap_or_else(|_| panic!("element index count exceeds u32"));
        // f64 → f32 at the display boundary (DECISIONS.md numeric row);
        // origin rebasing for large coordinates arrives with the display
        // work that needs it.
        #[allow(clippy::cast_possible_truncation)]
        self.positions.extend(positions.iter().map(|&x| x as f32));
        self.indices
            .extend(indices.iter().map(|&i| i.saturating_add(vertex_start)));
        self.pick_ids
            .extend(std::iter::repeat_n(pick_id, vertex_count as usize));
        self.elements.push(ElementEntry {
            element_index,
            pick_id,
            vertex_start,
            vertex_count,
            index_start,
            index_count,
        });
    }

    /// True when nothing was pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}

/// One instance of a mesh blob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    /// The element's index within the output value.
    pub element_index: u32,
    /// Pick id.
    pub pick_id: u32,
    /// 3×4 row-major affine transform.
    pub transform: [f32; 12],
}

/// The identity instance transform.
pub const IDENTITY: [f32; 12] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0,
];

/// A decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Kinds 1–3.
    Batch {
        /// Header.
        header: Header,
        /// Body.
        batch: Batch,
    },
    /// Kind 4.
    Clear {
        /// Header.
        header: Header,
    },
    /// Kind 5.
    MeshBlob {
        /// Header.
        header: Header,
        /// Content hash.
        hash: ValueHash,
        /// Positions.
        positions: Vec<f32>,
        /// Triangle indices.
        indices: Vec<u32>,
    },
    /// Kind 6.
    Instances {
        /// Header.
        header: Header,
        /// The blob's hash.
        hash: ValueHash,
        /// The instances.
        instances: Vec<Instance>,
    },
}

impl Frame {
    /// The header.
    #[must_use]
    pub fn header(&self) -> &Header {
        match self {
            Self::Batch { header, .. }
            | Self::Clear { header }
            | Self::MeshBlob { header, .. }
            | Self::Instances { header, .. } => header,
        }
    }
}

/// Decoding failures (tests + the headless client library).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// Fewer bytes than the header.
    #[error("frame shorter than its header ({0} bytes)")]
    Short(usize),
    /// Wrong magic.
    #[error("bad frame magic {0:#x}")]
    Magic(u32),
    /// Unknown version.
    #[error("unsupported frame version {0}")]
    Version(u16),
    /// Unknown kind.
    #[error("unknown frame kind {0}")]
    Kind(u16),
    /// The body does not match its counts.
    #[error("truncated frame body: needed {needed} bytes, had {had}")]
    Truncated {
        /// Bytes needed.
        needed: usize,
        /// Bytes available.
        had: usize,
    },
}

struct Writer(Vec<u8>);

impl Writer {
    fn with_header(header: &Header, kind: FrameKind, capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(HEADER_LEN + capacity);
        buf.extend_from_slice(&MAGIC.to_le_bytes());
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&(kind as u16).to_le_bytes());
        buf.extend_from_slice(&header.generation.to_le_bytes());
        buf.extend_from_slice(&header.node.to_le_bytes());
        buf.extend_from_slice(&header.output.to_le_bytes());
        buf.extend_from_slice(&header.element_start.to_le_bytes());
        buf.extend_from_slice(&header.element_count.to_le_bytes());
        Self(buf)
    }
    fn u32(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn f32s(&mut self, xs: &[f32]) {
        for x in xs {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn u32s(&mut self, xs: &[u32]) {
        for x in xs {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
}

/// Byte length of a batch body for `(E, V, I)`.
fn batch_body_len(e: usize, v: usize, i: usize) -> usize {
    12 + 24 * e + 12 * v + 4 * i + 4 * v
}

/// Encode a batch frame (`kind` must be Mesh/Curve/Point).
///
/// # Panics
///
/// When `kind` is not a batch kind or the batch's counts overflow `u32`.
#[must_use]
pub fn encode_batch(header: &Header, kind: FrameKind, batch: &Batch) -> Vec<u8> {
    assert!(
        matches!(kind, FrameKind::Mesh | FrameKind::Curve | FrameKind::Point),
        "encode_batch takes a batch kind"
    );
    let vertex_count = batch.positions.len() / 3;
    let count = |n: usize| u32::try_from(n).unwrap_or_else(|_| panic!("frame count exceeds u32"));
    let mut w = Writer::with_header(
        header,
        kind,
        batch_body_len(batch.elements.len(), vertex_count, batch.indices.len()),
    );
    w.u32(count(batch.elements.len()));
    w.u32(count(vertex_count));
    w.u32(count(batch.indices.len()));
    for entry in &batch.elements {
        w.u32(entry.element_index);
        w.u32(entry.pick_id);
        w.u32(entry.vertex_start);
        w.u32(entry.vertex_count);
        w.u32(entry.index_start);
        w.u32(entry.index_count);
    }
    w.f32s(&batch.positions);
    w.u32s(&batch.indices);
    w.u32s(&batch.pick_ids);
    w.0
}

/// Encode a clear frame.
#[must_use]
pub fn encode_clear(header: &Header) -> Vec<u8> {
    Writer::with_header(header, FrameKind::Clear, 0).0
}

/// Encode a mesh blob.
///
/// # Panics
///
/// When counts overflow `u32`.
#[must_use]
pub fn encode_mesh_blob(
    header: &Header,
    hash: &ValueHash,
    positions: &[f32],
    indices: &[u32],
) -> Vec<u8> {
    let count = |n: usize| u32::try_from(n).unwrap_or_else(|_| panic!("frame count exceeds u32"));
    let mut w = Writer::with_header(
        header,
        FrameKind::MeshBlob,
        40 + 4 * positions.len() + 4 * indices.len(),
    );
    w.0.extend_from_slice(hash.as_bytes());
    w.u32(count(positions.len() / 3));
    w.u32(count(indices.len()));
    w.f32s(positions);
    w.u32s(indices);
    w.0
}

/// Encode an instances frame.
///
/// # Panics
///
/// When counts overflow `u32`.
#[must_use]
pub fn encode_instances(header: &Header, hash: &ValueHash, instances: &[Instance]) -> Vec<u8> {
    let count = |n: usize| u32::try_from(n).unwrap_or_else(|_| panic!("frame count exceeds u32"));
    let mut w = Writer::with_header(header, FrameKind::Instances, 36 + 56 * instances.len());
    w.0.extend_from_slice(hash.as_bytes());
    w.u32(count(instances.len()));
    for instance in instances {
        w.u32(instance.element_index);
        w.u32(instance.pick_id);
        w.f32s(&instance.transform);
    }
    w.0
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn need(&self, n: usize) -> Result<(), FrameError> {
        if self.at + n > self.bytes.len() {
            return Err(FrameError::Truncated {
                needed: self.at + n,
                had: self.bytes.len(),
            });
        }
        Ok(())
    }
    fn u16(&mut self) -> Result<u16, FrameError> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.bytes[self.at], self.bytes[self.at + 1]]);
        self.at += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, FrameError> {
        self.need(4)?;
        let mut b = [0u8; 4];
        b.copy_from_slice(&self.bytes[self.at..self.at + 4]);
        self.at += 4;
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> Result<u64, FrameError> {
        self.need(8)?;
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.bytes[self.at..self.at + 8]);
        self.at += 8;
        Ok(u64::from_le_bytes(b))
    }
    fn f32(&mut self) -> Result<f32, FrameError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn hash(&mut self) -> Result<ValueHash, FrameError> {
        self.need(32)?;
        let mut b = [0u8; 32];
        b.copy_from_slice(&self.bytes[self.at..self.at + 32]);
        self.at += 32;
        Ok(ValueHash::from_bytes(b))
    }
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, FrameError> {
        self.need(4 * n)?;
        (0..n).map(|_| self.f32()).collect()
    }
    fn u32s(&mut self, n: usize) -> Result<Vec<u32>, FrameError> {
        self.need(4 * n)?;
        (0..n).map(|_| self.u32()).collect()
    }
}

/// Decode a frame (tests, the headless client library, `/debug`).
///
/// # Errors
///
/// [`FrameError`] on malformed bytes.
pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
    if bytes.len() < HEADER_LEN {
        return Err(FrameError::Short(bytes.len()));
    }
    let mut r = Reader { bytes, at: 0 };
    let magic = r.u32()?;
    if magic != MAGIC {
        return Err(FrameError::Magic(magic));
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(FrameError::Version(version));
    }
    let kind_raw = r.u16()?;
    let kind = FrameKind::from_u16(kind_raw).ok_or(FrameError::Kind(kind_raw))?;
    let header = Header {
        kind,
        generation: r.u64()?,
        node: r.u32()?,
        output: r.u32()?,
        element_start: r.u32()?,
        element_count: r.u32()?,
    };
    match kind {
        FrameKind::Mesh | FrameKind::Curve | FrameKind::Point => {
            let e = r.u32()? as usize;
            let v = r.u32()? as usize;
            let i = r.u32()? as usize;
            r.need(batch_body_len(e, v, i) - 12)?;
            let mut elements = Vec::with_capacity(e);
            for _ in 0..e {
                elements.push(ElementEntry {
                    element_index: r.u32()?,
                    pick_id: r.u32()?,
                    vertex_start: r.u32()?,
                    vertex_count: r.u32()?,
                    index_start: r.u32()?,
                    index_count: r.u32()?,
                });
            }
            let positions = r.f32s(3 * v)?;
            let indices = r.u32s(i)?;
            let pick_ids = r.u32s(v)?;
            Ok(Frame::Batch {
                header,
                batch: Batch {
                    elements,
                    positions,
                    indices,
                    pick_ids,
                },
            })
        }
        FrameKind::Clear => Ok(Frame::Clear { header }),
        FrameKind::MeshBlob => {
            let hash = r.hash()?;
            let v = r.u32()? as usize;
            let i = r.u32()? as usize;
            let positions = r.f32s(3 * v)?;
            let indices = r.u32s(i)?;
            Ok(Frame::MeshBlob {
                header,
                hash,
                positions,
                indices,
            })
        }
        FrameKind::Instances => {
            let hash = r.hash()?;
            let n = r.u32()? as usize;
            r.need(56 * n)?;
            let mut instances = Vec::with_capacity(n);
            for _ in 0..n {
                let element_index = r.u32()?;
                let pick_id = r.u32()?;
                let mut transform = [0f32; 12];
                for slot in &mut transform {
                    *slot = r.f32()?;
                }
                instances.push(Instance {
                    element_index,
                    pick_id,
                    transform,
                });
            }
            Ok(Frame::Instances {
                header,
                hash,
                instances,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: FrameKind) -> Header {
        Header {
            kind,
            generation: 7,
            node: 3,
            output: 1,
            element_start: 0,
            element_count: 2,
        }
    }

    #[test]
    fn batch_round_trips_and_arrays_are_aligned() {
        let mut batch = Batch::new();
        batch.push_element(
            0,
            11,
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &[0, 1, 2],
        );
        batch.push_element(
            1,
            12,
            &[5.0, 5.0, 5.0, 6.0, 5.0, 5.0, 5.0, 6.0, 5.0],
            &[0, 1, 2],
        );
        assert_eq!(
            batch.indices,
            vec![0, 1, 2, 3, 4, 5],
            "indices are frame-global"
        );
        assert_eq!(batch.pick_ids, vec![11, 11, 11, 12, 12, 12]);
        let bytes = encode_batch(&header(FrameKind::Mesh), FrameKind::Mesh, &batch);
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(&bytes[0..4], b"CICF");
        // positions start after header + counts + table
        let positions_at = HEADER_LEN + 12 + 24 * 2;
        assert_eq!(positions_at % 4, 0);
        assert_eq!(bytes.len(), positions_at + 12 * 6 + 4 * 6 + 4 * 6);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(
            decoded,
            Frame::Batch {
                header: header(FrameKind::Mesh),
                batch
            }
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // bit-exact round trip of the identity
    fn clear_blob_and_instances_round_trip() {
        let h = header(FrameKind::Clear);
        assert_eq!(
            decode(&encode_clear(&h)).unwrap(),
            Frame::Clear { header: h }
        );
        let hash = ValueHash::from_bytes([9u8; 32]);
        let blob = encode_mesh_blob(
            &header(FrameKind::MeshBlob),
            &hash,
            &[1.0, 2.0, 3.0],
            &[0, 0, 0],
        );
        match decode(&blob).unwrap() {
            Frame::MeshBlob {
                hash: h,
                positions,
                indices,
                ..
            } => {
                assert_eq!(h, hash);
                assert_eq!(positions, vec![1.0, 2.0, 3.0]);
                assert_eq!(indices, vec![0, 0, 0]);
            }
            other => panic!("{other:?}"),
        }
        let inst = encode_instances(
            &header(FrameKind::Instances),
            &hash,
            &[Instance {
                element_index: 4,
                pick_id: 40,
                transform: IDENTITY,
            }],
        );
        assert_eq!(inst.len(), HEADER_LEN + 36 + 56);
        match decode(&inst).unwrap() {
            Frame::Instances { instances, .. } => {
                assert_eq!(instances.len(), 1);
                assert_eq!(instances[0].transform, IDENTITY);
                assert_eq!(instances[0].pick_id, 40);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malformed_frames_are_refused_loudly() {
        assert_eq!(decode(&[0u8; 8]).unwrap_err(), FrameError::Short(8));
        let mut bad = encode_clear(&header(FrameKind::Clear));
        bad[0] = b'X';
        assert!(matches!(decode(&bad).unwrap_err(), FrameError::Magic(_)));
        let mut batch = Batch::new();
        batch.push_element(0, 1, &[0.0; 9], &[0, 1, 2]);
        let mut truncated = encode_batch(&header(FrameKind::Curve), FrameKind::Curve, &batch);
        truncated.truncate(truncated.len() - 3);
        assert!(matches!(
            decode(&truncated).unwrap_err(),
            FrameError::Truncated { .. }
        ));
        let mut kind = encode_clear(&header(FrameKind::Clear));
        kind[6] = 99;
        assert_eq!(decode(&kind).unwrap_err(), FrameError::Kind(99));
    }
}
