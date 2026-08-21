//! Content hashing (docs/12 §Values): every value carries a blake3 hash
//! computed once at construction. Hash inputs are versioned, domain-separated
//! byte encodings — little-endian everywhere, so hashes are identical across
//! platforms (golden hex constants in tests lock the format, one per kind).

use core::fmt;

/// Bump when the hash byte format changes. Deliberately invalidates every
/// cached value everywhere — that is the point; never reuse a version.
pub const HASH_FORMAT_VERSION: u8 = 1;

/// Domain-separation tag hashed first for each value kind. Append-only:
/// never renumber, never reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KindTag {
    /// `Number` (f64)
    Number = 1,
    /// `Integer` (i64)
    Integer = 2,
    /// `Boolean`
    Boolean = 3,
    /// `Text`
    Text = 4,
    /// `Color` (linear RGBA, f64)
    Color = 5,
    /// `Domain` (interval)
    Domain = 6,
    /// `IndexMap` (provenance indices)
    IndexMap = 7,
    /// `Point`
    Point = 8,
    /// `Vector`
    Vector = 9,
    /// `Plane`
    Plane = 10,
    /// `Xform`
    Xform = 11,
    /// `List` (Merkle: hashes of element hashes + axis)
    List = 12,
    /// `Nothing` — the absent case of a standalone `T?`
    Nothing = 13,
    /// `ProjectConfig` tolerances (participates in `NodeKey`s —
    /// DECISIONS.md tolerance row)
    ProjectConfig = 14,
    /// A scheduler cache key (docs/12 §Cache keys) — not a value; tagged
    /// here so every hash in the system shares one versioned format.
    NodeKey = 15,
    /// Normalized expression-node IR (docs/12: expression `node_version` =
    /// "hash of the normalized expression IR").
    ExprIr = 16,
    /// `Curve` (analytic sum type, stage 4)
    Curve = 17,
    /// `Mesh` (`SoA` triangle mesh, stage 4)
    Mesh = 18,
    /// Script-node source bytes (docs/12 cache keys: script
    /// `node_version` = hash of the source file) — not a value; tagged
    /// here so every hash shares one versioned format.
    ScriptSource = 19,
    /// `Solid` (B-rep, v0.1 item 3): the hash is over the kernel's
    /// canonical bytes (DECISIONS.md row 42, revised 2026-08-20).
    Solid = 20,
}

/// A 32-byte blake3 content hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueHash([u8; 32]);

impl ValueHash {
    /// The raw hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Rebuild from raw bytes — the persistence loading path (stage 3's
    /// memo log and value store address values by hash on disk).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Lowercase hex, 64 chars.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use core::fmt::Write as _;
            // write! to a String is infallible.
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

impl fmt::Debug for ValueHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValueHash({})", self.to_hex())
    }
}

impl fmt::Display for ValueHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Incremental hasher with the version + kind-tag prologue already applied.
/// The only way to produce a [`ValueHash`] — keeps every hash site on the
/// same format. Builder-style by value: [`Self::finish`] consumes the
/// hasher, so one prologue yields exactly one hash.
pub struct ValueHasher(blake3::Hasher);

impl ValueHasher {
    /// Start a hash for one value kind.
    #[must_use]
    pub fn new(tag: KindTag) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[HASH_FORMAT_VERSION, tag as u8]);
        Self(hasher)
    }

    /// Hash an f64 by its little-endian bits, self-enforcing the canonical
    /// form: `-0.0` collapses to `0.0` here too (harmless for
    /// already-canonical callers, safety net for future hash sites), and
    /// NaN is a debug-build panic — NaN must be refused long before any
    /// hash is taken (docs/12).
    #[must_use]
    pub fn f64(mut self, x: f64) -> Self {
        debug_assert!(
            !x.is_nan(),
            "NaN reached a hash site — refuse it at construction"
        );
        let canonical = if x == 0.0 { 0.0 } else { x };
        self.0.update(&canonical.to_le_bytes());
        self
    }

    /// Hash an i64, little-endian.
    #[must_use]
    pub fn i64(mut self, x: i64) -> Self {
        self.0.update(&x.to_le_bytes());
        self
    }

    /// Hash a u64, little-endian.
    #[must_use]
    pub fn u64(mut self, x: u64) -> Self {
        self.0.update(&x.to_le_bytes());
        self
    }

    /// Hash one byte.
    #[must_use]
    pub fn byte(mut self, b: u8) -> Self {
        self.0.update(&[b]);
        self
    }

    /// Hash raw bytes with a u64 length prefix (prevents concatenation
    /// ambiguity between adjacent variable-length fields).
    #[must_use]
    pub fn bytes(mut self, bytes: &[u8]) -> Self {
        self.0.update(&(bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    /// Hash a whole f64 buffer with a u64 length prefix — the mesh-scale
    /// bulk path (per-call [`Self::f64`] on a million-vertex buffer would
    /// pay one hasher update per component). Chunked little-endian
    /// encoding; the same canonical form as [`Self::f64`] (`-0.0`
    /// collapses, NaN is a debug panic), so bulk and per-component hashing
    /// of equal data agree.
    #[must_use]
    pub fn f64s(mut self, values: &[f64]) -> Self {
        self.0.update(&(values.len() as u64).to_le_bytes());
        let mut buffer = [0_u8; 8 * 512];
        for chunk in values.chunks(512) {
            for (index, &x) in chunk.iter().enumerate() {
                debug_assert!(
                    !x.is_nan(),
                    "NaN reached a hash site — refuse it at construction"
                );
                let canonical = if x == 0.0 { 0.0 } else { x };
                buffer[index * 8..index * 8 + 8].copy_from_slice(&canonical.to_le_bytes());
            }
            self.0.update(&buffer[..chunk.len() * 8]);
        }
        self
    }

    /// Hash a whole u32 buffer with a u64 length prefix (mesh indices).
    #[must_use]
    pub fn u32s(mut self, values: &[u32]) -> Self {
        self.0.update(&(values.len() as u64).to_le_bytes());
        let mut buffer = [0_u8; 4 * 512];
        for chunk in values.chunks(512) {
            for (index, &x) in chunk.iter().enumerate() {
                buffer[index * 4..index * 4 + 4].copy_from_slice(&x.to_le_bytes());
            }
            self.0.update(&buffer[..chunk.len() * 4]);
        }
        self
    }

    /// Hash a child value's hash (Merkle edge).
    #[must_use]
    pub fn child(mut self, hash: &ValueHash) -> Self {
        self.0.update(hash.as_bytes());
        self
    }

    /// Finish, consuming the hasher.
    #[must_use]
    pub fn finish(self) -> ValueHash {
        ValueHash(*self.0.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_shape() {
        let hash = ValueHasher::new(KindTag::Number).f64(1.0).finish();
        let hex = hash.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_kind_tags_separate_domains() {
        // Same payload bytes, different kind → different hash.
        let a = ValueHasher::new(KindTag::Number).i64(1).finish();
        let b = ValueHasher::new(KindTag::Integer).i64(1).finish();
        assert_ne!(a, b);
    }

    #[test]
    fn length_prefix_prevents_concat_ambiguity() {
        let a = ValueHasher::new(KindTag::Text)
            .bytes(b"ab")
            .bytes(b"c")
            .finish();
        let b = ValueHasher::new(KindTag::Text)
            .bytes(b"a")
            .bytes(b"bc")
            .finish();
        assert_ne!(a, b);
    }

    #[test]
    fn f64_is_self_canonicalizing_for_negative_zero() {
        let neg = ValueHasher::new(KindTag::Number).f64(-0.0).finish();
        let pos = ValueHasher::new(KindTag::Number).f64(0.0).finish();
        assert_eq!(neg, pos);
    }

    #[test]
    fn bulk_f64s_matches_length_prefix_plus_components() {
        // The bulk path must be byte-equivalent to a u64 length prefix
        // followed by per-component f64 hashing — including across the
        // 512-element chunk boundary and for -0.0 canonicalization.
        let values: Vec<f64> = (0..1300)
            .map(|i| if i % 7 == 0 { -0.0 } else { f64::from(i) * 0.5 })
            .collect();
        let bulk = ValueHasher::new(KindTag::Mesh).f64s(&values).finish();
        let mut per = ValueHasher::new(KindTag::Mesh).u64(values.len() as u64);
        for &x in &values {
            per = per.f64(x);
        }
        assert_eq!(bulk, per.finish());
    }

    #[test]
    fn bulk_u32s_is_length_prefixed_and_chunk_stable() {
        let values: Vec<u32> = (0..1300).collect();
        let a = ValueHasher::new(KindTag::Mesh).u32s(&values).finish();
        let b = ValueHasher::new(KindTag::Mesh).u32s(&values).finish();
        assert_eq!(a, b);
        // Length prefix prevents concat ambiguity between adjacent buffers.
        let ab = ValueHasher::new(KindTag::Mesh)
            .u32s(&[1, 2])
            .u32s(&[3])
            .finish();
        let ba = ValueHasher::new(KindTag::Mesh)
            .u32s(&[1])
            .u32s(&[2, 3])
            .finish();
        assert_ne!(ab, ba);
    }
}
