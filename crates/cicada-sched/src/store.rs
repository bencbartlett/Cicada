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
fn quarantine(path: &Path) {
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

/// The persistent store. Thread-safe: the executor's parallel chunks write
/// through it concurrently.
pub struct DiskStore {
    root: PathBuf,
    memo: RwLock<HashMap<NodeKey, MemoEntry>>,
    samples: RwLock<HashMap<String, CostStats>>,
    log: Mutex<fs::File>,
    mem: Mutex<MemCache>,
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
                tick: AtomicU64::new(0),
                temp_counter: AtomicU64::new(0),
            },
            OpenReport {
                memo_entries,
                sample_records,
                recovery,
            },
        ))
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

    /// True when a value blob exists (memory or disk).
    #[must_use]
    pub fn contains_value(&self, hash: &ValueHash) -> bool {
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
        self.blob_path(hash).exists()
    }

    /// Persist a value (children first — a stored list PROMISES its
    /// elements are loadable). Content-addressed: storing an
    /// already-present value is a cheap no-op, which is exactly what makes
    /// warming and re-solves trivially incremental (docs/12).
    ///
    /// # Errors
    ///
    /// [`StoreError`] on I/O or encoding failure.
    pub fn store_value(&self, value: &Arc<HashedValue>) -> Result<(), StoreError> {
        if let ValueData::List(list) = value.data() {
            for element in list.slots.iter().flatten() {
                self.store_value(element)?;
            }
        }
        let path = self.blob_path(&value.hash());
        if path.exists() {
            return Ok(());
        }
        let encoded = postcard::to_allocvec(&to_stored(value.data())).map_err(|error| {
            StoreError::Decode {
                path: path.clone(),
                message: error.to_string(),
            }
        })?;
        let compressed =
            zstd::stream::encode_all(encoded.as_slice(), 3).map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;
        let parent = path.parent().unwrap_or(&self.root).to_owned();
        fs::create_dir_all(&parent).map_err(|source| StoreError::Io {
            path: parent.clone(),
            source,
        })?;
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
        fs::write(&temp, &compressed).map_err(|source| StoreError::Io {
            path: temp.clone(),
            source,
        })?;
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

        let path = self.blob_path(hash);
        let compressed = fs::read(&path).map_err(|source| {
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
        })?;
        let Ok(encoded) = zstd::stream::decode_all(compressed.as_slice()) else {
            quarantine(&path);
            return Err(StoreError::Decode {
                path,
                message: "zstd decode failed".to_owned(),
            });
        };
        let stored: StoredValue = match postcard::from_bytes(&encoded) {
            Ok(stored) => stored,
            Err(error) => {
                quarantine(&path);
                return Err(StoreError::Decode {
                    path,
                    message: error.to_string(),
                });
            }
        };
        let data = self.hydrate_stored(stored, visiting)?;
        let value = match HashedValue::new(data) {
            Ok(value) => value,
            Err(error) => {
                quarantine(&path);
                return Err(StoreError::ValueRejected {
                    message: error.to_string(),
                });
            }
        };
        if value.hash() != *hash {
            quarantine(&path);
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
