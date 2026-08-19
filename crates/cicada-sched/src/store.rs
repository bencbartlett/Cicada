//! The two-level persistent store (docs/12 §The store):
//!
//! 1. **Value store** — content-addressed blobs, `hash → zstd(postcard)`,
//!    deduplicated across nodes, solves, and time by construction. Lists
//!    reference children **by hash** (Merkle on disk, like in memory), so
//!    shared elements are stored once.
//! 2. **Memo table** — `NodeKey → output hashes`, plus cost samples per
//!    operation, persisted as an append-only framed log and replayed on
//!    open. A crash costs at most the in-flight records; a torn tail is
//!    detected and reported, never silently absorbed into wrong data.
//!
//! **Location**: the store lives in the user cache directory
//! ([`project_cache_dir`]) — NEVER inside the project folder by default;
//! project dirs are cloud-synced (DECISIONS.md cache row). The rule is
//! enforced here, not left to callers.
//!
//! Loads are **verified**: a blob is decoded, reconstructed through
//! [`HashedValue::new`] (the only door), and its recomputed hash must equal
//! the requested address — corruption is a loud, typed refusal.
//!
//! Lite scope, stated: the in-memory layer is a byte-budgeted LRU with a
//! fixed default (the doc-12 "25% of host RAM" default needs the server's
//! host probe, stage 5); disk-side LRU eviction of the 32 GB budget arrives
//! with v0.1 — the spike corpus is megabytes.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle};
use cicada_core::hash::ValueHash;
use cicada_core::scalar::{Color, Domain, IndexMap};
use cicada_core::spatial::{Plane, Point, Vector, Xform};
use cicada_core::value::{HashedValue, List, ValueData};
use serde::{Deserialize, Serialize};

use crate::cost::CostStats;
use crate::key::NodeKey;

/// Default in-memory value budget (bytes of encoded size). See the module
/// docs for why this is a constant in the spike.
pub const DEFAULT_MEM_BUDGET: usize = 2 * 1024 * 1024 * 1024;

/// Lists with at least this many fresh (unstored) children persist them
/// on the rayon pool; below it the fork costs more than the writes.
const PARALLEL_STORE_MIN: usize = 16;

/// Store failures — all loud, all typed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Filesystem trouble.
    #[error("store I/O at {path}: {source}")]
    Io {
        /// Where.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// A blob or record failed to decode.
    #[error("store decode at {path}: {message}")]
    Decode {
        /// Where.
        path: PathBuf,
        /// What.
        message: String,
    },
    /// A value blob decoded but re-hashed to a different address —
    /// corruption (or a self-referential blob cycle).
    #[error("corrupt value blob: wanted {expected}, reconstructed {got}")]
    CorruptValue {
        /// The requested address.
        expected: String,
        /// What the bytes actually hash to (or a cycle description).
        got: String,
    },
    /// A referenced value is not in the store.
    #[error("value {hash} is not in the store")]
    MissingValue {
        /// The absent address.
        hash: String,
    },
    /// A memo entry's output count disagrees with the node's declared
    /// outputs — a corrupt, stale, or foreign record. Refused loudly at
    /// hit time; trusting it would index out of bounds.
    #[error(
        "memo entry {key} has {got} outputs; node `{node}` declares {declared} — \
         corrupt or stale memo record"
    )]
    MemoArity {
        /// The offending key (hex).
        key: String,
        /// The node that hit it.
        node: String,
        /// Outputs in the record.
        got: usize,
        /// Outputs the node declares.
        declared: usize,
    },
    /// A loaded blob refused value construction (NaN in a blob is
    /// corruption — NaN never passes construction on the way in).
    #[error("stored bytes refused value construction: {message}")]
    ValueRejected {
        /// Why.
        message: String,
    },
    /// No user cache directory exists on this host.
    #[error("no user cache directory on this host")]
    NoCacheDir,
    /// The computed cache dir would land inside the project folder —
    /// forbidden (DECISIONS.md cache row); refusing beats syncing
    /// gigabytes of cache churn.
    #[error("cache dir {dir} is inside project {project} — forbidden (DECISIONS.md cache row)")]
    InsideProject {
        /// The offending dir.
        dir: PathBuf,
        /// The project.
        project: PathBuf,
    },
}

/// How a damaged memo log was recovered. Either way the log is TRUNCATED
/// at the damage point before reuse, so post-recovery appends land at a
/// replayable position — damage is a one-time reported recompute, never a
/// permanent silent loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogRecovery {
    /// The log ended mid-record — a crash mid-append. Costs at most the
    /// in-flight records.
    TornTail,
    /// A record BEFORE the end failed to decode — corruption. Everything
    /// from `offset` was dropped (it cannot be trusted once framing is
    /// broken); `bytes_dropped` of later completed work will recompute.
    CorruptRecord {
        /// Byte offset of the undecodable record.
        offset: usize,
        /// Bytes dropped from there to the end.
        bytes_dropped: usize,
    },
}

/// What `open` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenReport {
    /// Memo entries replayed from the log.
    pub memo_entries: usize,
    /// Cost-sample records replayed.
    pub sample_records: usize,
    /// Log damage found and recovered, if any.
    pub recovery: Option<LogRecovery>,
    /// Small-blob pack entries indexed.
    pub packed_values: usize,
    /// Pack damage found and recovered, if any (same truncate-at-damage
    /// semantics as the memo log; the dropped blobs recompute).
    pub pack_recovery: Option<LogRecovery>,
}

/// Compressed blobs up to this size live in the append-only pack file
/// (`values/pack.bin`) instead of one file each. A cold wall-scale solve
/// persists tens of thousands of small values — points, numbers, cells,
/// parts — and one blob create per value is syscall-bound (≈0.3 ms each
/// even in parallel on NTFS); an append is microseconds. Larger blobs stay
/// one file each so the pack never grows by gigabytes per generation.
pub const PACK_MAX_BYTES: usize = 256 * 1024;

/// Where a value's bytes live on disk (see [`DiskStore::locate_value`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobLocation {
    /// One file per blob (compressed size above [`PACK_MAX_BYTES`]).
    File(PathBuf),
    /// A frame in the small-blob pack: the payload starts at `offset`.
    Packed {
        /// The pack file.
        path: PathBuf,
        /// Byte offset of the compressed payload.
        offset: u64,
        /// Compressed payload length.
        len: u32,
    },
}

/// Where a packed blob lives: byte offset of its payload and its length.
#[derive(Debug, Clone, Copy)]
struct PackSlot {
    offset: u64,
    len: u32,
}

/// The small-blob pack: frames of `[u32 LE len][32-byte hash][zstd]`
/// (`len` covers hash + payload), append-only, indexed in memory at open.
/// A later frame for a hash supersedes an earlier one, so a re-store after
/// a quarantine heals the address in place.
struct Pack {
    index: HashMap<ValueHash, PackSlot>,
    /// Append handle (every frame is one `write_all` — atomic per
    /// append on both Windows and POSIX).
    write: fs::File,
    /// Read handle (positioned reads; never shares the writer's cursor).
    read: fs::File,
    /// Bytes in the file as this process knows them (= the next append
    /// offset — another process appending concurrently is invisible to
    /// this index, exactly like the memo log).
    len: u64,
}

/// The pack frame header: the length word plus the content hash.
const PACK_HEADER: usize = 4 + 32;

/// zstd level for value blobs.
const ZSTD_LEVEL: i32 = 3;

/// Compress with a thread-local reusable context. `zstd::stream::encode_all`
/// builds a fresh compression context per call — ~30 µs — which made
/// persisting 1,200 fresh scalars cost 38 ms per slider tick (stage-6
/// measurement); a reused bulk compressor is a microsecond or two.
fn compress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    thread_local! {
        static COMPRESSOR: std::cell::RefCell<Option<zstd::bulk::Compressor<'static>>> =
            const { std::cell::RefCell::new(None) };
    }
    COMPRESSOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(zstd::bulk::Compressor::new(ZSTD_LEVEL)?);
        }
        match slot.as_mut() {
            Some(compressor) => compressor.compress(bytes),
            None => Err(std::io::Error::other("zstd compressor unavailable")),
        }
    })
}

/// Bytes to pre-allocate at most from a frame's declared content size. The
/// size is UNTRUSTED — bit-rot or a torn write can set it to anything, and
/// the bulk decoder would `Vec::with_capacity` it verbatim, aborting the
/// process on a ~1 TiB header (adversarial review, stage 6). No single
/// value blob in this system is anywhere near this; a genuinely larger one
/// (or a corrupt header) falls through to the streaming decoder, which
/// grows from the ACTUAL output and errors loudly on corruption.
const DECOMPRESS_HINT_CAP: usize = 64 * 1024 * 1024;

/// Decompress with a thread-local reusable context, hinted by the frame's
/// content size but never trusting it for allocation: the hint is clamped,
/// and anything absent, implausibly large, or wrong falls back to
/// `zstd::stream::decode_all`, which sizes its buffer from the real output
/// (memory-safe) and returns a loud `Err` on a corrupt or truncated frame —
/// so the load path quarantines the bytes and the address self-heals,
/// instead of aborting the process.
fn decompress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    thread_local! {
        static DECOMPRESSOR: std::cell::RefCell<Option<zstd::bulk::Decompressor<'static>>> =
            const { std::cell::RefCell::new(None) };
    }
    let hint = zstd::zstd_safe::get_frame_content_size(bytes)
        .ok()
        .flatten()
        .and_then(|size| usize::try_from(size).ok())
        .filter(|&size| size <= DECOMPRESS_HINT_CAP);
    let Some(capacity) = hint else {
        return zstd::stream::decode_all(bytes);
    };
    let bulk = DECOMPRESSOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(zstd::bulk::Decompressor::new()?);
        }
        match slot.as_mut() {
            Some(decompressor) => decompressor.decompress(bytes, capacity),
            None => Err(std::io::Error::other("zstd decompressor unavailable")),
        }
    });
    // A wrong hint (the real output exceeds the clamp) or a corrupt frame
    // errors here; retry through the streaming decoder, which is bounded by
    // the actual output and is the sole arbiter of "corrupt" (a loud Err).
    bulk.or_else(|_| zstd::stream::decode_all(bytes))
}

/// One memo entry: the output value hashes of a completed computation, in
/// output-port order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoEntry {
    /// Output hashes, one per port.
    pub outputs: Vec<ValueHash>,
}

/// The user-cache-directory store root for one project, keyed by the
/// canonical project path: `<user cache>/cicada/projects/<hash16>`.
///
/// # Errors
///
/// [`StoreError::NoCacheDir`] when the host has no cache directory;
/// [`StoreError::Io`] when the project path cannot be canonicalized;
/// [`StoreError::InsideProject`] when the result would sit inside the
/// project (pathological setups only) — the never-in-the-project rule is
/// enforced, not assumed.
pub fn project_cache_dir(project: &Path) -> Result<PathBuf, StoreError> {
    let dirs = directories::ProjectDirs::from("", "", "cicada").ok_or(StoreError::NoCacheDir)?;
    let canonical = fs::canonicalize(project).map_err(|source| StoreError::Io {
        path: project.to_owned(),
        source,
    })?;
    let id = blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex();
    let dir = dirs.cache_dir().join("projects").join(&id.as_str()[..16]);
    // Compare with the `\\?\` verbatim prefix stripped: on Windows,
    // canonicalize returns verbatim paths while the cache dir is plain, so
    // a raw starts_with could never match and the guard would be dead.
    if without_verbatim(&dir).starts_with(without_verbatim(&canonical)) {
        return Err(StoreError::InsideProject {
            dir,
            project: canonical,
        });
    }
    Ok(dir)
}

/// Move a bad blob aside (`.zst.corrupt`) so a future `store_value` can
/// rewrite the address with good bytes. Best-effort — the load error
/// itself is the loud signal; rename (not delete) preserves the evidence.
fn quarantine_file(path: &Path) {
    let _ = fs::rename(path, path.with_extension("zst.corrupt"));
}

/// A path with Windows' `\\?\` verbatim prefix stripped, for comparisons
/// between canonicalized (verbatim) and plain paths.
fn without_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map_or_else(|| path.to_path_buf(), PathBuf::from)
}

// ------------------------------------------------------------ blob codec --

/// The on-disk value encoding: leaves inline, lists by child hash (Merkle
/// on disk). Serde-derived here so `cicada-core` stays serde-free.
/// Append-only enum (postcard encodes variant indices): never reorder or
/// renumber — old blobs must decode under new binaries.
#[derive(Serialize, Deserialize)]
enum StoredValue {
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Text(String),
    Color([f64; 4]),
    Domain([f64; 2]),
    IndexMap(Vec<u64>),
    Point([f64; 3]),
    Vector([f64; 3]),
    Plane([[f64; 3]; 3]),
    Xform([f64; 12]),
    List {
        axis: Option<String>,
        slots: Vec<Option<[u8; 32]>>,
    },
    Nothing,
    Curve(StoredCurve),
    Mesh {
        positions: Vec<f64>,
        indices: Vec<u32>,
    },
}

/// On-disk curve encoding, one variant per analytic curve kind. Same
/// append-only contract as [`StoredValue`].
#[derive(Serialize, Deserialize)]
enum StoredCurve {
    Line {
        a: [f64; 3],
        b: [f64; 3],
    },
    Polyline {
        vertices: Vec<[f64; 3]>,
        closed: bool,
    },
    Circle {
        plane: [[f64; 3]; 3],
        radius: f64,
    },
    Rectangle {
        plane: [[f64; 3]; 3],
        x: [f64; 2],
        y: [f64; 2],
    },
}

fn to_stored(data: &ValueData) -> StoredValue {
    match data {
        ValueData::Number(x) => StoredValue::Number(*x),
        ValueData::Integer(i) => StoredValue::Integer(*i),
        ValueData::Boolean(b) => StoredValue::Boolean(*b),
        ValueData::Text(s) => StoredValue::Text(s.as_ref().to_owned()),
        ValueData::Color(c) => StoredValue::Color([c.r, c.g, c.b, c.a]),
        ValueData::Domain(d) => StoredValue::Domain([d.start, d.end]),
        ValueData::IndexMap(m) => StoredValue::IndexMap(m.0.clone()),
        ValueData::Point(p) => StoredValue::Point([p.0.x, p.0.y, p.0.z]),
        ValueData::Vector(v) => StoredValue::Vector([v.0.x, v.0.y, v.0.z]),
        ValueData::Plane(p) => StoredValue::Plane([
            [p.origin.0.x, p.origin.0.y, p.origin.0.z],
            [p.x.0.x, p.x.0.y, p.x.0.z],
            [p.y.0.x, p.y.0.y, p.y.0.z],
        ]),
        ValueData::Xform(x) => StoredValue::Xform(x.coefficients()),
        ValueData::Curve(curve) => StoredValue::Curve(match curve {
            Curve::Line(line) => StoredCurve::Line {
                a: point_triplet(line.a),
                b: point_triplet(line.b),
            },
            Curve::Polyline(polyline) => StoredCurve::Polyline {
                vertices: polyline
                    .vertices
                    .iter()
                    .map(|&v| point_triplet(v))
                    .collect(),
                closed: polyline.closed,
            },
            Curve::Circle(circle) => StoredCurve::Circle {
                plane: plane_triplets(&circle.plane),
                radius: circle.radius,
            },
            Curve::Rectangle(rectangle) => StoredCurve::Rectangle {
                plane: plane_triplets(&rectangle.plane),
                x: [rectangle.x.start, rectangle.x.end],
                y: [rectangle.y.start, rectangle.y.end],
            },
        }),
        ValueData::Mesh(mesh) => StoredValue::Mesh {
            positions: mesh.positions().to_vec(),
            indices: mesh.indices().to_vec(),
        },
        ValueData::List(list) => StoredValue::List {
            axis: list.axis.as_ref().map(|axis| axis.as_ref().to_owned()),
            slots: list
                .slots
                .iter()
                .map(|slot| slot.as_ref().map(|element| *element.hash().as_bytes()))
                .collect(),
        },
        ValueData::Nothing => StoredValue::Nothing,
    }
}

fn point_triplet(point: Point) -> [f64; 3] {
    [point.0.x, point.0.y, point.0.z]
}

fn plane_triplets(plane: &Plane) -> [[f64; 3]; 3] {
    [
        point_triplet(plane.origin),
        [plane.x.0.x, plane.x.0.y, plane.x.0.z],
        [plane.y.0.x, plane.y.0.y, plane.y.0.z],
    ]
}

fn triplet_point(t: [f64; 3]) -> Point {
    Point::new(t[0], t[1], t[2])
}

fn triplets_plane(t: [[f64; 3]; 3]) -> Plane {
    Plane {
        origin: triplet_point(t[0]),
        x: Vector::new(t[1][0], t[1][1], t[1][2]),
        y: Vector::new(t[2][0], t[2][1], t[2][2]),
    }
}

// -------------------------------------------------------------- memo log --

/// One framed record of the append-only log. Append-only enum: never
/// renumber variants — old logs must replay under new binaries.
#[derive(Serialize, Deserialize)]
enum LogRecord {
    Memo {
        key: [u8; 32],
        outputs: Vec<[u8; 32]>,
    },
    Sample {
        op: String,
        elements: u64,
        nanos: u64,
    },
    /// Tombstone: a memo entry whose promised outputs failed to load
    /// (quarantined blob). Replay removes the entry, so the next solve
    /// recomputes and re-stores — the self-heal path.
    Unmemo { key: [u8; 32] },
}

// --------------------------------------------------------------- the store --

struct MemEntry {
    value: Arc<HashedValue>,
    bytes: usize,
    last_use: u64,
}

struct MemCache {
    entries: HashMap<ValueHash, MemEntry>,
    total_bytes: usize,
    budget: usize,
}

impl MemCache {
    fn get(&mut self, hash: &ValueHash, tick: u64) -> Option<Arc<HashedValue>> {
        self.entries.get_mut(hash).map(|entry| {
            entry.last_use = tick;
            Arc::clone(&entry.value)
        })
    }

    fn insert(&mut self, value: Arc<HashedValue>, bytes: usize, tick: u64) {
        let hash = value.hash();
        if let Some(existing) = self.entries.get_mut(&hash) {
            // Refresh recency — re-storing a value is a use.
            existing.last_use = tick;
            return;
        }
        self.total_bytes += bytes;
        self.entries.insert(
            hash,
            MemEntry {
                value,
                bytes,
                last_use: tick,
            },
        );
        if self.total_bytes > self.budget {
            self.evict();
        }
    }

    /// Evict least-recently-used entries until back under budget.
    /// O(n log n) when it runs; it runs rarely (budget crossings).
    fn evict(&mut self) {
        let mut by_age: Vec<(u64, ValueHash, usize)> = self
            .entries
            .iter()
            .map(|(hash, entry)| (entry.last_use, *hash, entry.bytes))
            .collect();
        by_age.sort_unstable();
        for (_, hash, bytes) in by_age {
            if self.total_bytes <= self.budget {
                break;
            }
            self.entries.remove(&hash);
            self.total_bytes -= bytes;
        }
    }
}

impl Pack {
    /// Open (creating if absent) the pack at `path`, indexing its frames.
    /// A torn tail or an unframeable record is truncated away and reported,
    /// exactly like the memo log — dropped blobs recompute; nothing is
    /// silently trusted.
    fn open(path: &Path) -> Result<(Self, Option<LogRecovery>), StoreError> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| StoreError::Io { path, source }
        };
        let mut index = HashMap::new();
        let mut recovery = None;
        let mut len = 0u64;
        if path.exists() {
            let bytes = fs::read(path).map_err(io(path))?;
            let mut offset = 0usize;
            while offset < bytes.len() {
                let Some(header) = bytes.get(offset..offset + 4) else {
                    recovery = Some(LogRecovery::TornTail);
                    break;
                };
                let frame_len = header
                    .try_into()
                    .map(u32::from_le_bytes)
                    .unwrap_or_default() as usize;
                if frame_len < 32 {
                    recovery = Some(LogRecovery::CorruptRecord {
                        offset,
                        bytes_dropped: bytes.len() - offset,
                    });
                    break;
                }
                let Some(frame) = bytes.get(offset + 4..offset + 4 + frame_len) else {
                    recovery = Some(LogRecovery::TornTail);
                    break;
                };
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&frame[..32]);
                // Later frames supersede earlier ones (a healed re-store).
                index.insert(
                    ValueHash::from_bytes(hash),
                    PackSlot {
                        offset: (offset + PACK_HEADER) as u64,
                        len: u32::try_from(frame_len - 32).unwrap_or(u32::MAX),
                    },
                );
                offset += 4 + frame_len;
            }
            if recovery.is_some() {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .map_err(io(path))?;
                file.set_len(offset as u64).map_err(io(path))?;
            }
            len = offset as u64;
        }
        let write = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io(path))?;
        let read = fs::OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(io(path))?;
        Ok((
            Self {
                index,
                write,
                read,
                len,
            },
            recovery,
        ))
    }

    /// Append one blob frame; returns nothing, indexes it.
    fn append(&mut self, path: &Path, hash: &ValueHash, payload: &[u8]) -> Result<(), StoreError> {
        let frame_len = u32::try_from(32 + payload.len()).map_err(|_| StoreError::Decode {
            path: path.to_owned(),
            message: "pack frame over 4 GiB".to_owned(),
        })?;
        let mut framed = Vec::with_capacity(PACK_HEADER + payload.len());
        framed.extend_from_slice(&frame_len.to_le_bytes());
        framed.extend_from_slice(hash.as_bytes());
        framed.extend_from_slice(payload);
        self.write
            .write_all(&framed)
            .map_err(|source| StoreError::Io {
                path: path.to_owned(),
                source,
            })?;
        self.write.flush().map_err(|source| StoreError::Io {
            path: path.to_owned(),
            source,
        })?;
        let offset = self.len + PACK_HEADER as u64;
        self.len += framed.len() as u64;
        self.index.insert(
            *hash,
            PackSlot {
                offset,
                len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
            },
        );
        Ok(())
    }

    /// Append many blob frames in ONE write, indexing each.
    fn append_many(&mut self, path: &Path, items: &[(ValueHash, &[u8])]) -> Result<(), StoreError> {
        if items.is_empty() {
            return Ok(());
        }
        let total: usize = items
            .iter()
            .map(|(_, payload)| PACK_HEADER + payload.len())
            .sum();
        let mut framed = Vec::with_capacity(total);
        let mut slots = Vec::with_capacity(items.len());
        for (hash, payload) in items {
            let frame_len = u32::try_from(32 + payload.len()).map_err(|_| StoreError::Decode {
                path: path.to_owned(),
                message: "pack frame over 4 GiB".to_owned(),
            })?;
            let offset = self.len + framed.len() as u64 + PACK_HEADER as u64;
            framed.extend_from_slice(&frame_len.to_le_bytes());
            framed.extend_from_slice(hash.as_bytes());
            framed.extend_from_slice(payload);
            slots.push((
                *hash,
                PackSlot {
                    offset,
                    len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
                },
            ));
        }
        self.write
            .write_all(&framed)
            .map_err(|source| StoreError::Io {
                path: path.to_owned(),
                source,
            })?;
        self.write.flush().map_err(|source| StoreError::Io {
            path: path.to_owned(),
            source,
        })?;
        self.len += framed.len() as u64;
        for (hash, slot) in slots {
            self.index.insert(hash, slot);
        }
        Ok(())
    }

    /// Read a packed blob's compressed bytes, if indexed.
    fn read(&mut self, path: &Path, hash: &ValueHash) -> Result<Option<Vec<u8>>, StoreError> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let Some(slot) = self.index.get(hash).copied() else {
            return Ok(None);
        };
        let io = |source| StoreError::Io {
            path: path.to_owned(),
            source,
        };
        self.read.seek(SeekFrom::Start(slot.offset)).map_err(io)?;
        let mut bytes = vec![0u8; slot.len as usize];
        self.read.read_exact(&mut bytes).map_err(io)?;
        Ok(Some(bytes))
    }

    /// Forget a packed blob (its bytes failed verification); the next
    /// store appends a fresh frame that supersedes it on replay.
    fn forget(&mut self, hash: &ValueHash) {
        self.index.remove(hash);
    }
}

/// The persistent store. Thread-safe: the executor's parallel chunks write
/// through it concurrently.
pub struct DiskStore {
    root: PathBuf,
    memo: RwLock<HashMap<NodeKey, MemoEntry>>,
    samples: RwLock<HashMap<String, CostStats>>,
    log: Mutex<fs::File>,
    mem: Mutex<MemCache>,
    /// The small-blob pack (see [`PACK_MAX_BYTES`]).
    pack: Mutex<Pack>,
    /// Monotonic LRU tick (coarse, cross-thread).
    tick: AtomicU64,
    /// Uniquifier for temp blob files.
    temp_counter: AtomicU64,
}

impl DiskStore {
    /// Open (creating if absent) the store at `root`, replaying the memo
    /// log.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on I/O or on a decode failure before the log tail.
    pub fn open(root: &Path) -> Result<(Self, OpenReport), StoreError> {
        Self::open_with_budget(root, DEFAULT_MEM_BUDGET)
    }

    /// [`Self::open`] with an explicit in-memory budget (tests use small
    /// ones to exercise eviction).
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_with_budget(
        root: &Path,
        mem_budget: usize,
    ) -> Result<(Self, OpenReport), StoreError> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| StoreError::Io { path, source }
        };
        fs::create_dir_all(root.join("values")).map_err(io(root))?;
        let log_path = root.join("memo.log");

        let mut memo = HashMap::new();
        let mut samples: HashMap<String, CostStats> = HashMap::new();
        let mut memo_entries = 0;
        let mut sample_records = 0;
        let mut recovery = None;
        if log_path.exists() {
            let bytes = fs::read(&log_path).map_err(io(&log_path))?;
            let mut offset = 0;
            while offset < bytes.len() {
                let Some(header) = bytes.get(offset..offset + 4) else {
                    recovery = Some(LogRecovery::TornTail);
                    break;
                };
                // Header slice is exactly 4 bytes by construction.
                let len = header
                    .try_into()
                    .map(u32::from_le_bytes)
                    .unwrap_or_default() as usize;
                let Some(body) = bytes.get(offset + 4..offset + 4 + len) else {
                    recovery = Some(LogRecovery::TornTail);
                    break;
                };
                let Ok(record) = postcard::from_bytes::<LogRecord>(body) else {
                    // An undecodable frame BEFORE the end is corruption,
                    // not a tear; everything after is unframeable either
                    // way. Reported with counts, then truncated below.
                    recovery = Some(LogRecovery::CorruptRecord {
                        offset,
                        bytes_dropped: bytes.len() - offset,
                    });
                    break;
                };
                match record {
                    LogRecord::Memo { key, outputs } => {
                        memo_entries += 1;
                        memo.insert(
                            NodeKey::from_hash(ValueHash::from_bytes(key)),
                            MemoEntry {
                                outputs: outputs.into_iter().map(ValueHash::from_bytes).collect(),
                            },
                        );
                    }
                    LogRecord::Sample {
                        op,
                        elements,
                        nanos,
                    } => {
                        sample_records += 1;
                        samples.entry(op).or_default().record(elements, nanos);
                    }
                    LogRecord::Unmemo { key } => {
                        memo.remove(&NodeKey::from_hash(ValueHash::from_bytes(key)));
                    }
                }
                offset += 4 + len;
            }
            // TRUNCATE at the damage point before reuse. Without this,
            // every later append would land BEHIND the bad bytes —
            // permanently unreplayable, and the broken frame would
            // mis-frame genuine later records into garbage. Truncation
            // failing is a loud error, never a proceed.
            if recovery.is_some() {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(&log_path)
                    .map_err(io(&log_path))?;
                file.set_len(offset as u64).map_err(io(&log_path))?;
            }
        }

        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(io(&log_path))?;

        let (pack, pack_recovery) = Pack::open(&root.join("values").join("pack.bin"))?;
        let packed_values = pack.index.len();

        Ok((
            Self {
                root: root.to_owned(),
                memo: RwLock::new(memo),
                samples: RwLock::new(samples),
                log: Mutex::new(log),
                mem: Mutex::new(MemCache {
                    entries: HashMap::new(),
                    total_bytes: 0,
                    budget: mem_budget,
                }),
                pack: Mutex::new(pack),
                tick: AtomicU64::new(0),
                temp_counter: AtomicU64::new(0),
            },
            OpenReport {
                memo_entries,
                sample_records,
                recovery,
                packed_values,
                pack_recovery,
            },
        ))
    }

    /// Where a value's bytes live on disk, if this store knows of them
    /// (diagnostics and tests — corruption tests damage the real bytes).
    #[must_use]
    pub fn locate_value(&self, hash: &ValueHash) -> Option<BlobLocation> {
        let slot = self
            .pack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .index
            .get(hash)
            .copied();
        if let Some(slot) = slot {
            return Some(BlobLocation::Packed {
                path: self.pack_path(),
                offset: slot.offset,
                len: slot.len,
            });
        }
        let path = self.blob_path(hash);
        path.exists().then_some(BlobLocation::File(path))
    }

    /// How many values the small-blob pack holds (tests and diagnostics).
    #[must_use]
    pub fn packed_values(&self) -> usize {
        self.pack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .index
            .len()
    }

    /// The store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Look up a memo entry.
    #[must_use]
    pub fn memo(&self, key: &NodeKey) -> Option<MemoEntry> {
        self.memo
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    /// Record a completed computation. Call only after every output blob is
    /// stored — a memo entry PROMISES its outputs are loadable.
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`] when the log append fails.
    pub fn record_memo(&self, key: NodeKey, outputs: &[ValueHash]) -> Result<(), StoreError> {
        self.append(&LogRecord::Memo {
            key: *key.as_hash().as_bytes(),
            outputs: outputs.iter().map(|hash| *hash.as_bytes()).collect(),
        })?;
        self.memo
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                key,
                MemoEntry {
                    outputs: outputs.to_vec(),
                },
            );
        Ok(())
    }

    /// Record a cost sample for an operation (docs/12: samples feed chunk
    /// sizing, ETA, threading, routing — the naive estimator is enough for
    /// stage 3, the recording format is the contract).
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`] when the log append fails.
    pub fn record_sample(&self, op: &str, elements: u64, nanos: u64) -> Result<(), StoreError> {
        self.append(&LogRecord::Sample {
            op: op.to_owned(),
            elements,
            nanos,
        })?;
        self.samples
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(op.to_owned())
            .or_default()
            .record(elements, nanos);
        Ok(())
    }

    /// Invalidate a memo entry whose promise broke (its outputs failed to
    /// load): removed from the map and tombstoned in the log, so the next
    /// solve recomputes and re-stores instead of re-hitting a dead entry
    /// forever.
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`] when the tombstone append fails.
    pub fn invalidate_memo(&self, key: NodeKey) -> Result<(), StoreError> {
        self.append(&LogRecord::Unmemo {
            key: *key.as_hash().as_bytes(),
        })?;
        self.memo
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key);
        Ok(())
    }

    /// Accumulated cost stats for an operation.
    #[must_use]
    pub fn stats(&self, op: &str) -> Option<CostStats> {
        self.samples
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(op)
            .copied()
    }

    fn append(&self, record: &LogRecord) -> Result<(), StoreError> {
        let log_path = self.root.join("memo.log");
        let body = postcard::to_allocvec(record).map_err(|error| StoreError::Decode {
            path: log_path.clone(),
            message: error.to_string(),
        })?;
        let mut framed = Vec::with_capacity(4 + body.len());
        framed.extend_from_slice(
            &u32::try_from(body.len())
                .map_err(|_| StoreError::Decode {
                    path: log_path.clone(),
                    message: "record over 4 GiB".to_owned(),
                })?
                .to_le_bytes(),
        );
        framed.extend_from_slice(&body);
        let mut log = self
            .log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        log.write_all(&framed).map_err(|source| StoreError::Io {
            path: log_path.clone(),
            source,
        })?;
        log.flush().map_err(|source| StoreError::Io {
            path: log_path,
            source,
        })
    }

    fn blob_path(&self, hash: &ValueHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root
            .join("values")
            .join(&hex[..2])
            .join(format!("{hex}.zst"))
    }

    /// True when the in-memory layer or the pack index holds the value —
    /// which implies it is on disk: every `mem` insert happens after a
    /// successful blob write (`store_value`) or a verified blob read
    /// (`load_inner`), and the pack index is the pack file. No stat.
    fn known_stored(&self, hash: &ValueHash) -> bool {
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        if self
            .mem
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(hash, tick)
            .is_some()
        {
            return true;
        }
        self.pack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .index
            .contains_key(hash)
    }

    /// True when a value blob exists (memory, pack, or disk).
    #[must_use]
    pub fn contains_value(&self, hash: &ValueHash) -> bool {
        self.known_stored(hash) || self.blob_path(hash).exists()
    }

    fn pack_path(&self) -> PathBuf {
        self.root.join("values").join("pack.bin")
    }

    /// Persist leaf values (no children): encode in parallel, then append
    /// every small blob to the pack under one lock in one write; the rare
    /// big leaf takes the file path.
    fn store_leaves(&self, leaves: &[&Arc<HashedValue>]) -> Result<(), StoreError> {
        if leaves.is_empty() {
            return Ok(());
        }
        let encode = |value: &Arc<HashedValue>| -> Result<(Arc<HashedValue>, Vec<u8>), StoreError> {
            let path = self.blob_path(&value.hash());
            let encoded = postcard::to_allocvec(&to_stored(value.data())).map_err(|error| {
                StoreError::Decode {
                    path: path.clone(),
                    message: error.to_string(),
                }
            })?;
            let compressed =
                compress(&encoded).map_err(|source| StoreError::Io { path, source })?;
            Ok((Arc::clone(value), compressed))
        };
        let encoded: Vec<(Arc<HashedValue>, Vec<u8>)> = if leaves.len() >= PARALLEL_STORE_MIN {
            use rayon::prelude::*;
            leaves
                .par_iter()
                .map(|value| encode(value))
                .collect::<Result<_, _>>()?
        } else {
            leaves
                .iter()
                .map(|value| encode(value))
                .collect::<Result<_, _>>()?
        };
        let (small, big): (Vec<_>, Vec<_>) = encoded
            .into_iter()
            .partition(|(_, compressed)| compressed.len() <= PACK_MAX_BYTES);
        for (value, _) in &big {
            self.store_value(value)?;
        }
        if small.is_empty() {
            return Ok(());
        }
        let pack_path = self.pack_path();
        {
            let mut pack = self
                .pack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let items: Vec<(ValueHash, &[u8])> = small
                .iter()
                .filter(|(value, _)| !pack.index.contains_key(&value.hash()))
                .map(|(value, compressed)| (value.hash(), compressed.as_slice()))
                .collect();
            pack.append_many(&pack_path, &items)?;
        }
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        let mut mem = self
            .mem
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (value, compressed) in small {
            mem.insert(value, compressed.len(), tick);
        }
        Ok(())
    }

    /// Persist a value (children first — a stored list PROMISES its
    /// elements are loadable). Content-addressed: storing an
    /// already-present value is a cheap no-op, which is exactly what makes
    /// warming and re-solves trivially incremental (docs/12).
    ///
    /// A list's fresh children are written **in parallel** (rayon): a cold
    /// 1,500-element fan-out is one blob create per element, and blob
    /// creates are syscall-bound (the stage-5 probe measured a cold
    /// `construct_point ×1500` at seconds — all of it file creation, none
    /// of it compute). Content addressing makes the parallel writes
    /// trivially safe: two writers of one hash race to identical bytes.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on I/O or encoding failure.
    pub fn store_value(&self, value: &Arc<HashedValue>) -> Result<(), StoreError> {
        if let ValueData::List(list) = value.data() {
            // Persist the children that are not already known. Leaf
            // children are encoded in parallel and appended to the pack in
            // ONE write (a slider tick over a 1,200-element list used to
            // cost 1,200 append syscalls — stage-6 measurement); nested
            // lists recurse, in parallel once there are enough.
            let fresh: Vec<&Arc<HashedValue>> = list
                .slots
                .iter()
                .flatten()
                .filter(|element| !self.known_stored(&element.hash()))
                .collect();
            let (leaves, lists): (Vec<_>, Vec<_>) = fresh
                .into_iter()
                .partition(|element| !matches!(element.data(), ValueData::List(_)));
            if lists.len() >= PARALLEL_STORE_MIN {
                use rayon::prelude::*;
                lists
                    .par_iter()
                    .try_for_each(|element| self.store_value(element))?;
            } else {
                for element in lists {
                    self.store_value(element)?;
                }
            }
            self.store_leaves(&leaves)?;
        }
        if self.known_stored(&value.hash()) {
            return Ok(());
        }
        let path = self.blob_path(&value.hash());
        let encoded = postcard::to_allocvec(&to_stored(value.data())).map_err(|error| {
            StoreError::Decode {
                path: path.clone(),
                message: error.to_string(),
            }
        })?;
        let compressed = compress(&encoded).map_err(|source| StoreError::Io {
            path: path.clone(),
            source,
        })?;
        if compressed.len() <= PACK_MAX_BYTES {
            // Small blob: one append under the pack lock. A concurrent
            // store of the same hash (parallel children of two lists)
            // appends a second identical frame — harmless, and the index
            // check under the lock keeps it rare.
            let pack_path = self.pack_path();
            let mut pack = self
                .pack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !pack.index.contains_key(&value.hash()) {
                pack.append(&pack_path, &value.hash(), &compressed)?;
            }
            drop(pack);
            let tick = self.tick.fetch_add(1, Ordering::Relaxed);
            self.mem
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(Arc::clone(value), compressed.len(), tick);
            return Ok(());
        }
        if path.exists() {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&self.root).to_owned();
        // Write-then-rename so a crash never leaves a torn blob under its
        // content address. The temp name carries the PROCESS id: two
        // processes storing the same fresh value must not share a temp
        // path (fs::write truncates — a shared path could tear). With
        // unique temps, whichever rename lands second atomically REPLACES
        // the destination with identical bytes (Windows rename replaces;
        // that is the desired semantics, not a failure).
        let temp = parent.join(format!(
            "{}.tmp-{}-{}",
            value.hash().to_hex(),
            std::process::id(),
            self.temp_counter.fetch_add(1, Ordering::Relaxed)
        ));
        // The shard directory is created on demand — on the first miss,
        // not with a `create_dir_all` stat per blob (one syscall of the
        // five a cold element used to cost).
        if let Err(source) = fs::write(&temp, &compressed) {
            if source.kind() != std::io::ErrorKind::NotFound {
                return Err(StoreError::Io { path: temp, source });
            }
            fs::create_dir_all(&parent).map_err(|source| StoreError::Io {
                path: parent.clone(),
                source,
            })?;
            fs::write(&temp, &compressed).map_err(|source| StoreError::Io {
                path: temp.clone(),
                source,
            })?;
        }
        if let Err(source) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            if !path.exists() {
                return Err(StoreError::Io { path, source });
            }
        }
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        self.mem
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(Arc::clone(value), compressed.len(), tick);
        Ok(())
    }

    /// Load a value by address, verifying it: the reconstructed value must
    /// re-hash to exactly the requested address.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when absent, undecodable, corrupt, or cyclic.
    pub fn load_value(&self, hash: &ValueHash) -> Result<Arc<HashedValue>, StoreError> {
        let mut visiting = Vec::new();
        self.load_inner(hash, &mut visiting)
    }

    fn load_inner(
        &self,
        hash: &ValueHash,
        visiting: &mut Vec<ValueHash>,
    ) -> Result<Arc<HashedValue>, StoreError> {
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        if let Some(value) = self
            .mem
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(hash, tick)
        {
            return Ok(value);
        }
        if visiting.contains(hash) {
            // A blob claiming itself (or an ancestor) as a child: only
            // corruption can produce this — content addressing is acyclic.
            return Err(StoreError::CorruptValue {
                expected: hash.to_hex(),
                got: "a reference cycle".to_owned(),
            });
        }
        visiting.push(*hash);

        // Where the bytes live: the pack (small blobs) or a file.
        let path = self.blob_path(hash);
        let packed = {
            let pack_path = self.pack_path();
            self.pack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .read(&pack_path, hash)?
        };
        let from_pack = packed.is_some();
        let compressed = match packed {
            Some(bytes) => bytes,
            None => fs::read(&path).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    StoreError::MissingValue {
                        hash: hash.to_hex(),
                    }
                } else {
                    StoreError::Io {
                        path: path.clone(),
                        source,
                    }
                }
            })?,
        };
        // Bad bytes are moved aside (file) or forgotten (pack) so a
        // re-store heals the address — never "Ok forever on store, error
        // forever on load".
        let quarantine = |this: &Self| {
            if from_pack {
                this.pack
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .forget(hash);
            } else {
                quarantine_file(&path);
            }
        };
        let Ok(encoded) = decompress(&compressed) else {
            quarantine(self);
            return Err(StoreError::Decode {
                path,
                message: "zstd decode failed".to_owned(),
            });
        };
        let stored: StoredValue = match postcard::from_bytes(&encoded) {
            Ok(stored) => stored,
            Err(error) => {
                quarantine(self);
                return Err(StoreError::Decode {
                    path,
                    message: error.to_string(),
                });
            }
        };
        let data = match self.hydrate_stored(stored, visiting) {
            Ok(data) => data,
            // A structurally invalid payload (bad mesh buffers) is THIS
            // blob's corruption — quarantine it like undecodable bytes.
            // Child-load errors pass through untouched (the child is the
            // problem, and it quarantined itself if corrupt).
            Err(error @ StoreError::ValueRejected { .. }) => {
                quarantine(self);
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let value = match HashedValue::new(data) {
            Ok(value) => value,
            Err(error) => {
                quarantine(self);
                return Err(StoreError::ValueRejected {
                    message: error.to_string(),
                });
            }
        };
        if value.hash() != *hash {
            quarantine(self);
            return Err(StoreError::CorruptValue {
                expected: hash.to_hex(),
                got: value.hash().to_hex(),
            });
        }
        visiting.pop();
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        self.mem
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(Arc::clone(&value), compressed.len(), tick);
        Ok(value)
    }

    fn hydrate_stored(
        &self,
        stored: StoredValue,
        visiting: &mut Vec<ValueHash>,
    ) -> Result<ValueData, StoreError> {
        Ok(match stored {
            StoredValue::Number(x) => ValueData::Number(x),
            StoredValue::Integer(i) => ValueData::Integer(i),
            StoredValue::Boolean(b) => ValueData::Boolean(b),
            StoredValue::Text(s) => ValueData::Text(Arc::from(s.as_str())),
            StoredValue::Color([r, g, b, a]) => ValueData::Color(Color::new(r, g, b, a)),
            StoredValue::Domain([start, end]) => ValueData::Domain(Domain::new(start, end)),
            StoredValue::IndexMap(indices) => ValueData::IndexMap(IndexMap(indices)),
            StoredValue::Point([x, y, z]) => ValueData::Point(Point::new(x, y, z)),
            StoredValue::Vector([x, y, z]) => ValueData::Vector(Vector::new(x, y, z)),
            StoredValue::Plane([o, x, y]) => ValueData::Plane(Plane {
                origin: Point::new(o[0], o[1], o[2]),
                x: Vector::new(x[0], x[1], x[2]),
                y: Vector::new(y[0], y[1], y[2]),
            }),
            StoredValue::Xform(c) => ValueData::Xform(Xform::from_coefficients(c)),
            StoredValue::Curve(stored) => ValueData::Curve(match stored {
                StoredCurve::Line { a, b } => Curve::Line(Line {
                    a: triplet_point(a),
                    b: triplet_point(b),
                }),
                StoredCurve::Polyline { vertices, closed } => Curve::Polyline(Polyline {
                    vertices: vertices.into_iter().map(triplet_point).collect(),
                    closed,
                }),
                StoredCurve::Circle { plane, radius } => Curve::Circle(Circle {
                    plane: triplets_plane(plane),
                    radius,
                }),
                StoredCurve::Rectangle { plane, x, y } => Curve::Rectangle(Rectangle {
                    plane: triplets_plane(plane),
                    x: Domain::new(x[0], x[1]),
                    y: Domain::new(y[0], y[1]),
                }),
            }),
            StoredValue::Mesh { positions, indices } => {
                // Structural validation on load: a blob failing Mesh::new is
                // corruption (nothing invalid passes construction on the
                // way in) — surfaced as ValueRejected, quarantined by the
                // caller like every bad blob.
                ValueData::Mesh(Mesh::new(positions, indices).map_err(|error| {
                    StoreError::ValueRejected {
                        message: error.to_string(),
                    }
                })?)
            }
            StoredValue::List { axis, slots } => {
                let mut loaded = Vec::with_capacity(slots.len());
                for slot in slots {
                    loaded.push(match slot {
                        None => None,
                        Some(child) => {
                            Some(self.load_inner(&ValueHash::from_bytes(child), visiting)?)
                        }
                    });
                }
                ValueData::List(List {
                    axis: axis.map(|axis| Arc::from(axis.as_str())),
                    slots: loaded,
                })
            }
            StoredValue::Nothing => ValueData::Nothing,
        })
    }
}

#[cfg(test)]
mod decompress_tests {
    use super::{compress, decompress};

    /// A valid zstd frame with a raw single-byte block and an explicit
    /// 8-byte content size — `fcs` is written verbatim so a test can corrupt
    /// it. Layout: magic, FHD 0xE0 (8-byte FCS, single segment), FCS, then a
    /// raw last block of one byte.
    fn frame_with_fcs(fcs: u64, content: u8) -> Vec<u8> {
        let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0xE0];
        f.extend_from_slice(&fcs.to_le_bytes());
        f.extend_from_slice(&[0x09, 0x00, 0x00]); // raw block, size 1, last
        f.push(content);
        f
    }

    #[test]
    fn round_trips_a_bulk_blob() {
        let bytes = b"the wall's field magnitudes, packed and reloaded";
        let compressed = compress(bytes).unwrap();
        assert_eq!(decompress(&compressed).unwrap(), bytes);
    }

    #[test]
    fn a_corrupt_content_size_header_refuses_loudly_and_never_aborts() {
        // Regression (adversarial review, stage 6): a frame whose declared
        // content size is corrupted to an enormous value must NOT drive a
        // `Vec::with_capacity(that)` — that aborts the process before any
        // Err can be returned, bypassing the load path's quarantine + heal.
        // A plausible small FCS decodes; an implausible one falls to the
        // streaming decoder, which errors on the size mismatch. Neither
        // panics.
        assert_eq!(decompress(&frame_with_fcs(1, 0x41)).unwrap(), vec![0x41]);
        // > isize::MAX: the old code's `Vec::with_capacity` panicked
        // "capacity overflow"; now it is a clamped-out Err.
        assert!(decompress(&frame_with_fcs(u64::MAX, 0x41)).is_err());
        // ~1 TiB: the old code attempted a real 1 TiB allocation (abort);
        // now clamped out to the streaming path, which rejects the lie.
        assert!(decompress(&frame_with_fcs(1 << 40, 0x41)).is_err());
    }
}
