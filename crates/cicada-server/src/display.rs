//! Values → display: binary frames for the viewport (hash-driven
//! instancing, pick ids) and compact [`ValueSummary`]s for the inspector,
//! wire hover, and the closest zoom tier's port previews (docs/16). Both
//! read cached values only — display never re-solves anything.
//!
//! A `Solid` (v0.1 item 3 WP-B) draws through tessellation — the kernel
//! meshes its canonical bytes at the generation's display TIER
//! ([`DisplayTier`]: the preview deflection for a slider drag's
//! generations, the fine one for structural generations and the viewport
//! at rest; `cicada_geom::solid::Deflection`, docs/03) — cached by the
//! solid's VALUE hash + the tier's deflection in the session's
//! [`SolidCache`] (docs/12 §Display cache): a hit is a map lookup, a miss
//! is one kernel call. The session warms the cache for a generation's
//! distinct solids on the solve loop's workers BEFORE taking its lock
//! ([`distinct_solids`] + `Scheduler::map_parallel`), so the broadcast
//! under the lock only hits. The frames are the ordinary mesh frames
//! (`frames.rs` is unchanged) keyed by the DISPLAY MESH's own value hash
//! — content-addressed, so a solid drawn at two deflections travels as two
//! blobs and identical solids at one deflection travel once. A mesh the
//! kernel could not close still draws (a green Solid never vanishes from
//! the viewport) and says so in [`DisplayStats::warnings`] and the summary's
//! `watertight` fact; a solid that cannot be tessellated at all — bytes the
//! kernel refuses — draws nothing and says why in [`DisplayStats::errors`]
//! and its summary; never a silent skip.
//!
//! The display edge is **bounded** (v0.1 wave 5 D1, docs/12 §Display —
//! the triangle budget): a structural generation asks for the fine tier,
//! but an output whose distinct solids would exceed
//! [`DISPLAY_TRIANGLE_BUDGET`] triangles at that tier is drawn at the
//! preview tier instead, and one that exceeds it even there is drawn at
//! preview anyway and marked `over_budget` ([`choose_tier`] →
//! [`BudgetStats`], recorded in [`DisplayStats::budget`]). The decision is
//! a pure function of the value set and the budget: the fine tally stops
//! at the budget (the work wasted before a "too many" verdict is bounded
//! by the budget itself, and the tessellations it did are cache entries),
//! and a verdict never depends on timing or on the order the solids were
//! meshed in. The cache is **resizable** ([`SolidCache::set_budget`]) and
//! **watched** ([`SolidCache::watch`]): the session asks it to count the
//! evictions of the entries the previous complete generation displayed,
//! which is the `thrash` flag of the `caches` view (docs/13).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Mesh, Solid};
use cicada_core::hash::ValueHash;
use cicada_core::value::{HashedValue, ValueData};
use cicada_geom::curve::tessellate_closed;
use cicada_geom::solid::{self as solids, Deflection};

use crate::frames::{
    Batch, FrameKind, Header, IDENTITY, Instance, encode_batch, encode_clear, encode_instances,
    encode_mesh_blob,
};
use crate::protocol::ValueSummary;

/// Segments per full circle for display tessellation of analytic circles.
pub const CIRCLE_SEGMENTS: i64 = 64;

/// Stable pick ids for `(node ref, output, element)` triples — backward
/// picking's currency (docs/04). Ids never repeat within a session, so a
/// pick made against an older frame still resolves to the right element.
#[derive(Debug, Default)]
pub struct PickTable {
    next: u32,
    ids: HashMap<(u32, u32, u32), u32>,
    back: HashMap<u32, (u32, u32, u32)>,
    encodes: u64,
}

impl PickTable {
    /// The pick id for a triple, allocated on first sight (ids start at 1;
    /// 0 = nothing).
    pub fn id_for(&mut self, node: u32, output: u32, element: u32) -> u32 {
        if let Some(&id) = self.ids.get(&(node, output, element)) {
            return id;
        }
        self.next += 1;
        self.ids.insert((node, output, element), self.next);
        self.back.insert(self.next, (node, output, element));
        self.next
    }

    /// The ids of one output's elements, in one call — what
    /// [`frames_for_value`] asks its [`PickIds`] for exactly once, before
    /// any encoding, so the table's lock is held for this call and not for
    /// the encode. Counts the call: [`Self::encodes`].
    pub fn ids_for(&mut self, node: u32, output: u32, elements: &[u32]) -> Vec<u32> {
        self.encodes += 1;
        elements
            .iter()
            .map(|&element| self.id_for(node, output, element))
            .collect()
    }

    /// How many outputs have been encoded against this table
    /// ([`Self::ids_for`] calls) — the `/debug/state` counter a test reads
    /// to know whether a restream paid for an output.
    #[must_use]
    pub fn encodes(&self) -> u64 {
        self.encodes
    }

    /// Distinct pick ids allocated so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// No id allocated yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Resolve a pick id back to `(node ref, output, element)`.
    #[must_use]
    pub fn resolve(&self, id: u32) -> Option<(u32, u32, u32)> {
        self.back.get(&id).copied()
    }
}

/// How [`frames_for_value`] gets its pick ids: ONE call with every element
/// index the output draws (ascending, deduplicated — nested lists share
/// their outer slot), answered with the id of each, in order — normally
/// [`PickTable::ids_for`] under whatever mutex guards the table, held for
/// that call only. The encode that follows (the wall's largest output:
/// most of a second on a debug engine) runs outside it, so a joiner's
/// restream never holds the table against the live path, which encodes
/// under the session lock (docs/13 §Two lanes, one socket).
pub type PickIds<'a> = &'a mut dyn FnMut(&[u32]) -> Vec<u32>;

/// Default byte budget of a [`SolidCache`]: the welded display meshes it
/// may hold before evicting least-recently-used entries (positions + index
/// buffers, as uploaded). 1 GiB since v0.1 wave 5 (DECISIONS.md row
/// 2026-08-25; it was 256 MiB): two value sets of 1,000 fine spheres —
/// the undo/redo flip of docs/17 §Measurement U30 — are 2 × 192 MB, and a
/// cache that holds one set thrashes on every flip. A budget, not a
/// correctness boundary — eviction only costs a re-tessellation — and
/// resizable at run time (`cicada serve --solid-cache-mib`, the
/// `set_display_cache` intent; [`SolidCache::set_budget`]).
pub const SOLID_CACHE_BUDGET: usize = 1024 * 1024 * 1024;

/// The smallest display cache a client may ask for through
/// `set_display_cache {mib}` (docs/13): 64 MiB — below it one fine-tier
/// output evicts itself on every redraw.
pub const SOLID_CACHE_MIN_MIB: u64 = 64;
/// The largest: 64 GiB — above it the number is a typo, not a budget.
pub const SOLID_CACHE_MAX_MIB: u64 = 65_536;

/// The triangle budget of one displayed output per generation (docs/12
/// §Display; DECISIONS.md row 2026-08-25): an output whose distinct solids
/// would exceed it at the tier the generation asks for is drawn at the
/// preview tier instead ([`choose_tier`]). A million triangles is ~36 MB
/// of frame and ~0.3 s of client decode + upload; 1,000 fine-tier spheres
/// (docs/17 §Measurement U30) are eight times that and drop to preview —
/// 866,000 triangles, 17 MB, 9× less tessellation. Per output, so a
/// pipeline of many modest outputs is not penalised for their sum. The
/// session's default; `SessionConfig::display_triangle_budget` lets a test
/// lower it.
pub const DISPLAY_TRIANGLE_BUDGET: u64 = 1_000_000;

/// Which deflection a display pass tessellates solids at (docs/03 §Display
/// tessellation): `Preview` for the generations of a slider drag — coarse,
/// what a drag can afford — and `Fine` for structural generations, the
/// release, a joining client and the inspector. Ordered: a fine drawing
/// satisfies a preview request, never the reverse, so an output drawn at
/// `Preview` is redrawn by the next `Fine` generation of the same value.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTier {
    /// The coarse tier (`Deflection::preview`).
    Preview,
    /// The fine tier (`Deflection::display`).
    Fine,
}

impl DisplayTier {
    /// The tier's deflection for a project.
    #[must_use]
    pub fn deflection(self, config: &ProjectConfig) -> Deflection {
        match self {
            Self::Preview => Deflection::preview(config),
            Self::Fine => Deflection::display(config),
        }
    }
}

/// What the session passes the display path: the project configuration
/// (tolerance for curve tessellation, tolerance + unit for the solid
/// display deflection), the solid tessellation cache, the tier this pass
/// draws at, and — for an output the triangle budget judged — its verdict,
/// recorded in the output's [`DisplayStats::budget`].
#[derive(Clone, Copy)]
pub struct DisplayContext<'a> {
    /// The project's configuration.
    pub config: &'a ProjectConfig,
    /// The session's tessellation cache.
    pub solids: &'a SolidCache,
    /// The tier of this pass — for a live emission, the tier the budget
    /// CHOSE for the output ([`BudgetStats::drawn`]).
    pub tier: DisplayTier,
    /// The budget's verdict for this output, when one was taken (a live
    /// emission); `None` for a restream (which redraws at the tier on
    /// record) and for summaries.
    pub budget: Option<BudgetStats>,
    /// The display meshes the pass's warm-up fetched for its outputs at
    /// the tier each is drawn at, PINNED for the encode (a live emission;
    /// docs/12 §Display): a solid found here is drawn from it and the
    /// cache is not asked, so the encode under the session lock never
    /// tessellates — however the cache's eviction went between the
    /// warm-up and the encode (a working set larger than the budget
    /// evicts the warm-up's own first entries; review finding
    /// 2026-08-25). `None` for a restream and for summaries, which read
    /// the cache.
    pub pinned: Option<&'a PinnedMeshes>,
}

/// The meshes a display pass pinned for its encode ([`DisplayContext::
/// pinned`]), by the cache's own key — a solid drawn at two tiers by two
/// outputs of one pass is two entries.
pub type PinnedMeshes = HashMap<TessellationKey, Arc<DisplayMesh>>;

/// One value's distinct solids' display meshes at one tier, as the tally
/// fetched them through the cache ([`Chosen::meshes`], [`fetch_meshes`]).
pub type DrawnMeshes = Vec<(ValueHash, Arc<DisplayMesh>)>;

impl DisplayContext<'_> {
    /// The display deflection of this pass (the tier's, for this project;
    /// docs/03 formula — the relative term is applied per solid below it).
    #[must_use]
    pub fn deflection(&self) -> Deflection {
        self.tier.deflection(self.config)
    }
}

/// The key of a cached tessellation: the solid's value hash plus the tier
/// deflection it was meshed at (bit patterns — the deflection is a pure
/// function of the project configuration and the tier, and a configuration
/// change is exactly what must miss; the per-solid relative term is a
/// function of the solid, so it needs no place in the key). Public so the
/// session can name the entries a generation displayed
/// ([`SolidCache::watch`]); opaque otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TessellationKey {
    hash: ValueHash,
    linear_bits: u64,
    angular_bits: u64,
}

impl TessellationKey {
    /// The key of `hash` meshed at `deflection`.
    #[must_use]
    pub fn new(hash: ValueHash, deflection: Deflection) -> Self {
        Self {
            hash,
            linear_bits: deflection.linear().to_bits(),
            angular_bits: deflection.angular().to_bits(),
        }
    }
}

/// One solid's display mesh, as the cache holds it and the frames draw it.
#[derive(Debug)]
pub struct DisplayMesh {
    /// The welded display mesh, sealed as a value: its hash is the frames'
    /// content key (the blob a group of identical solids shares), so two
    /// deflections of one solid are two blobs and a mesh-valued twin of the
    /// tessellation would share one.
    sealed: Arc<HashedValue>,
    /// B-rep faces in the solid.
    pub faces: usize,
    /// Did the kernel's mesh close? `false` draws all the same and is
    /// reported (the summary's `watertight`, the stats' `warnings`).
    pub watertight: bool,
    /// The deflection the mesher ran at (the tier's, raised by the relative
    /// term for this solid's extent).
    pub deflection: Deflection,
}

impl DisplayMesh {
    fn new(tessellation: solids::DisplayTessellation) -> Result<Self, String> {
        let solids::DisplayTessellation {
            mesh,
            watertight,
            faces,
            deflection,
        } = tessellation;
        let sealed = HashedValue::new(ValueData::Mesh(mesh))
            .map_err(|error| format!("display mesh could not be sealed as a value: {error}"))?;
        Ok(Self {
            sealed,
            faces,
            watertight,
            deflection,
        })
    }

    /// The mesh.
    #[must_use]
    pub fn mesh(&self) -> &Mesh {
        match self.sealed.data() {
            ValueData::Mesh(mesh) => mesh,
            other => unreachable!(
                "DisplayMesh seals a Mesh by construction, found {}",
                other.kind_name()
            ),
        }
    }

    /// The content hash of the mesh — the frames' blob key.
    #[must_use]
    pub fn hash(&self) -> ValueHash {
        self.sealed.hash()
    }
}

/// A cache entry: the display mesh, or the kernel's refusal of these bytes
/// at this deflection (kept so an undrawable solid is not re-meshed on every
/// redraw — docs/12 §Display cache).
#[derive(Debug)]
enum Cached {
    Mesh(Arc<DisplayMesh>),
    Refused(Arc<str>),
}

/// One cached entry and its place in the recency order.
struct Entry {
    cached: Cached,
    /// Its footprint, as counted against the budget.
    size: usize,
    /// Its stamp in `CacheState::recency` (the key there).
    touched: u64,
}

struct CacheState {
    entries: HashMap<TessellationKey, Entry>,
    /// The recency index: touch stamp → key, least recently used first.
    /// Stamps come from `clock`, strictly increasing, so every entry holds
    /// a distinct one. A touch moves one stamp (two `BTreeMap` operations,
    /// O(log n)); eviction pops the first. Nothing here is linear in the
    /// number of entries — a display pass over N distinct solids costs
    /// O(N log entries), not O(N × entries), however full the cache.
    recency: BTreeMap<u64, TessellationKey>,
    clock: u64,
    bytes: usize,
    /// Refusals held (a subset of `entries`).
    refusals: usize,
    /// The byte budget — under the lock, so a resize and the eviction it
    /// forces are one step ([`SolidCache::set_budget`]).
    budget: usize,
    /// The entries whose eviction is worth counting: what the previous
    /// complete generation displayed ([`SolidCache::watch`]). An eviction
    /// of one of them is the `thrash` signal — the cache could not hold
    /// the last picture and this one together.
    watched: HashSet<TessellationKey>,
    /// Evictions of watched entries since the watch was set.
    watched_evictions: u64,
}

impl CacheState {
    fn stamp(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Evict least-recently-used entries until `bytes + incoming` fits the
    /// budget (`incoming` = 0 for a resize). Counts evictions of watched
    /// entries.
    fn make_room(&mut self, incoming: usize, evictions: &AtomicU64) {
        while self.bytes + incoming > self.budget {
            let Some((_, oldest)) = self.recency.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.bytes -= evicted.size;
                if matches!(evicted.cached, Cached::Refused(_)) {
                    self.refusals -= 1;
                }
                if self.watched.contains(&oldest) {
                    self.watched_evictions += 1;
                }
                evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The hash-keyed solid tessellation cache (docs/12 §Display cache; DECISIONS.md
/// row 42: "display tessellates Solids through a hash-keyed cache").
/// Internally synchronized so the solve loop's workers (warming it in
/// parallel), the frame path and the summary path share one instance
/// behind `&`; bounded by bytes, evicted least-recently-used in O(log n)
/// per touch; hit/miss/eviction counts are observable in `/debug/state`
/// (additive). Refusals are cached too, as small negative entries under
/// the same key and the same eviction: a solid whose bytes the kernel
/// refuses, or whose mesher fails after doing its work, is refused from the
/// cache on the next pass instead of re-paying the kernel call — a
/// corrected value is a new hash and misses as it should. A tessellation
/// larger than the whole budget is served but never kept (`oversized`
/// counts them): keeping it would evict everything else for one entry the
/// budget cannot hold anyway. Resizable ([`Self::set_budget`]: shrinking
/// evicts at once) and watchable ([`Self::watch`]).
pub struct SolidCache {
    state: std::sync::Mutex<CacheState>,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    oversized: AtomicU64,
}

/// The cache's counters, as `/debug/state` → `display_cache` reports them
/// and the `caches` view carries them (flattened beside its flags).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SolidCacheStats {
    /// Entries held (meshes and refusals).
    pub entries: usize,
    /// Bytes held (mesh buffers as uploaded; a refusal counts its text) —
    /// never above `budget`.
    pub bytes: usize,
    /// The byte budget.
    pub budget: usize,
    /// Lookups served from the cache (a cached refusal is a hit too).
    pub hits: u64,
    /// Lookups that called the kernel.
    pub misses: u64,
    /// Entries evicted to stay within budget.
    pub evictions: u64,
    /// Tessellations larger than the whole budget: served to the caller,
    /// never kept (each is a miss every time it is drawn).
    pub oversized: u64,
    /// Refusals held (negative entries; a subset of `entries`).
    pub refusals: usize,
}

impl Default for SolidCache {
    fn default() -> Self {
        Self::new(SOLID_CACHE_BUDGET)
    }
}

/// How [`SolidCache::tessellation_served`] answered: from the cache, or by
/// calling the kernel (and caching the result — a refusal included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    /// A cache hit (a cached refusal is a hit too).
    Hit,
    /// A kernel call.
    Miss,
}

impl SolidCache {
    /// An empty cache with a byte budget.
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            state: std::sync::Mutex::new(CacheState {
                entries: HashMap::new(),
                recency: BTreeMap::new(),
                clock: 0,
                bytes: 0,
                refusals: 0,
                budget,
                watched: HashSet::new(),
                watched_evictions: 0,
            }),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            oversized: AtomicU64::new(0),
        }
    }

    /// The byte budget.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.lock().budget
    }

    /// Resize the budget (`set_display_cache`, docs/13): shrinking evicts
    /// least-recently-used entries AT ONCE until the held bytes fit —
    /// counted in `evictions` (and `watched_evictions` when they were
    /// watched) like any eviction; growing evicts nothing. Returns how many
    /// entries the resize evicted.
    pub fn set_budget(&self, budget: usize) -> u64 {
        let mut state = self.lock();
        let before = self.evictions.load(Ordering::Relaxed);
        state.budget = budget;
        state.make_room(0, &self.evictions);
        self.evictions.load(Ordering::Relaxed) - before
    }

    /// Watch `keys` — the picture on screen: from now on an eviction of
    /// any of them counts in [`Self::watched_evictions`] (the previous
    /// count is reset), and each of them held is TOUCHED, so the on-screen
    /// picture is the newest thing in the cache and the least recently
    /// used entries — past value sets nobody draws, the fine meshes of an
    /// output the budget dropped to preview — go first. Without the touch
    /// a stable output's meshes, never looked up again once displayed,
    /// were the OLDEST entries and left before any garbage, and the flag
    /// called it thrash although the picture fit with room to spare
    /// (review finding 2026-08-25). The session sets it after every pass
    /// and after a resize, and reads the count after the next pass — the
    /// `thrash` flag (docs/12 §Display): now exactly "the previous
    /// picture and this one do not fit together".
    pub fn watch(&self, keys: impl IntoIterator<Item = TessellationKey>) {
        let mut state = self.lock();
        let watched: HashSet<TessellationKey> = keys.into_iter().collect();
        for key in &watched {
            let stamp = state.stamp();
            if let Some(entry) = state.entries.get_mut(key) {
                let previous = std::mem::replace(&mut entry.touched, stamp);
                state.recency.remove(&previous);
                state.recency.insert(stamp, *key);
            }
        }
        state.watched = watched;
        state.watched_evictions = 0;
    }

    /// Evictions of watched entries since the watch was set.
    #[must_use]
    pub fn watched_evictions(&self) -> u64 {
        self.lock().watched_evictions
    }

    /// Is `key` held right now? A read that touches no recency (a test
    /// oracle and the session's bookkeeping, never the display path).
    #[must_use]
    pub fn contains(&self, key: TessellationKey) -> bool {
        self.lock().entries.contains_key(&key)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The display mesh of `solid` (whose sealed value hash is `hash`) at a
    /// tier's `deflection`: the cached one, or the kernel's, which is then
    /// cached — and so is a refusal. The error is the kernel's reason,
    /// rendered for the stats and the summary — the caller attaches the
    /// output and element.
    ///
    /// # Errors
    ///
    /// The `GeomError` of `cicada_geom::solid::tessellate_display`,
    /// rendered: `KernelUnavailable` in a build without `occt`,
    /// `Serialization` for bytes the kernel cannot read, the mesher's
    /// failures. Never "not watertight" — closure is reported on the mesh.
    pub fn tessellation(
        &self,
        hash: ValueHash,
        solid: &Solid,
        deflection: Deflection,
    ) -> Result<Arc<DisplayMesh>, String> {
        self.tessellation_served(hash, solid, deflection).0
    }

    /// [`Self::tessellation`] saying how it was answered — a cache hit or
    /// a kernel call — so a display pass can attribute its lookups to the
    /// output it made them for (the profiler's `cache_hits` /
    /// `cache_misses` per display row, v0.1 wave 5 P1) without reading the
    /// cache-wide counters, which a restream or an inspector summary on
    /// another thread moves too.
    pub fn tessellation_served(
        &self,
        hash: ValueHash,
        solid: &Solid,
        deflection: Deflection,
    ) -> (Result<Arc<DisplayMesh>, String>, Served) {
        let key = TessellationKey::new(hash, deflection);
        if let Some(found) = self.lookup(key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            let found = match found {
                Cached::Mesh(mesh) => Ok(mesh),
                Cached::Refused(reason) => Err(reason.to_string()),
            };
            return (found, Served::Hit);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let result = solids::tessellate_display(solid, deflection)
            .map_err(|error| error.to_string())
            .and_then(DisplayMesh::new);
        let result = match result {
            Ok(mesh) => {
                let mesh = Arc::new(mesh);
                self.insert(key, Cached::Mesh(Arc::clone(&mesh)));
                Ok(mesh)
            }
            Err(reason) => {
                self.insert(key, Cached::Refused(Arc::from(reason.as_str())));
                Err(reason)
            }
        };
        (result, Served::Miss)
    }

    /// The display mesh a SUMMARY reads: whatever is cached for this solid
    /// at either tier (the fine one preferred) — a read, NEVER a kernel
    /// call. `None` when no display pass has meshed the solid (its preview
    /// is off, or its output is not yet drawn): the summary says so instead
    /// of meshing it at the fine tier under the session lock. The first cut
    /// computed the fine mesh on a miss, and a consumer's `inspect` of a
    /// hidden 1,001-solid list — hidden with the eye exactly to spare that
    /// work — stalled the session 41 s (wave 5 N1 review CR-2, 2026-09-19).
    /// What is tessellated is the display path's decision alone.
    ///
    /// # Errors
    ///
    /// A cached refusal is `Some(Err(reason))`, as [`SolidCache::tessellation`]
    /// reports it.
    pub fn tessellation_for_summary(
        &self,
        hash: ValueHash,
        config: &ProjectConfig,
    ) -> Option<Result<Arc<DisplayMesh>, String>> {
        for tier in [DisplayTier::Fine, DisplayTier::Preview] {
            let key = TessellationKey::new(hash, tier.deflection(config));
            if let Some(found) = self.lookup(key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(match found {
                    Cached::Mesh(mesh) => Ok(mesh),
                    Cached::Refused(reason) => Err(reason.to_string()),
                });
            }
        }
        None
    }

    fn lookup(&self, key: TessellationKey) -> Option<Cached> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stamp = state.stamp();
        let entry = state.entries.get_mut(&key)?;
        let found = match &entry.cached {
            Cached::Mesh(mesh) => Cached::Mesh(Arc::clone(mesh)),
            Cached::Refused(reason) => Cached::Refused(Arc::clone(reason)),
        };
        let previous = std::mem::replace(&mut entry.touched, stamp);
        state.recency.remove(&previous);
        state.recency.insert(stamp, key);
        Some(found)
    }

    fn insert(&self, key: TessellationKey, cached: Cached) {
        let size = match &cached {
            Cached::Mesh(mesh) => mesh_bytes(mesh.mesh()),
            Cached::Refused(reason) => reason.len(),
        };
        let mut state = self.lock();
        if size > state.budget {
            // Nothing in the cache could make room for this; evicting
            // everything for an entry that still does not fit would only
            // cost the other solids their hits.
            self.oversized.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // A concurrent miss on the same key may have inserted first: keep
        // the one that is there (identical content), count nothing twice.
        if state.entries.contains_key(&key) {
            return;
        }
        state.make_room(size, &self.evictions);
        if matches!(cached, Cached::Refused(_)) {
            state.refusals += 1;
        }
        let touched = state.stamp();
        state.entries.insert(
            key,
            Entry {
                cached,
                size,
                touched,
            },
        );
        state.recency.insert(touched, key);
        state.bytes += size;
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> SolidCacheStats {
        let state = self.lock();
        SolidCacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            budget: state.budget,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            oversized: self.oversized.load(Ordering::Relaxed),
            refusals: state.refusals,
        }
    }
}

/// A mesh's footprint as the cache and the frames see it: f64 positions
/// and u32 indices.
fn mesh_bytes(mesh: &Mesh) -> usize {
    std::mem::size_of_val(mesh.positions()) + std::mem::size_of_val(mesh.indices())
}

/// Every distinct `Solid` inside `values` (bare or in lists, by value
/// hash), for the session to warm the cache with on the solve loop's
/// workers before it takes its lock: `Scheduler::map_parallel` over this
/// list, each item one `SolidCache::tessellation` at the pass's tier. A
/// `Solid` is an `Arc` over its bytes, so the clones are cheap.
#[must_use]
pub fn distinct_solids(values: &[Arc<HashedValue>]) -> Vec<(ValueHash, Solid)> {
    let mut seen: BTreeMap<ValueHash, Solid> = BTreeMap::new();
    for value in values {
        let (mut points, mut curves, mut meshes, mut solids) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        collect(
            value,
            None,
            &mut points,
            &mut curves,
            &mut meshes,
            &mut solids,
        );
        for (_, solid_value) in solids {
            if let ValueData::Solid(solid) = solid_value.data() {
                seen.entry(solid_value.hash())
                    .or_insert_with(|| solid.clone());
            }
        }
    }
    seen.into_iter().collect()
}

/// What one output's frames contained (the `/debug/state` display report
/// and the "geometry changed" oracle Playwright asserts on).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct DisplayStats {
    /// Frame kinds emitted (`mesh`, `curve`, `point`, `instances`, `clear`).
    pub kinds: Vec<&'static str>,
    /// Elements drawn.
    pub elements: usize,
    /// Vertices transmitted.
    pub vertices: usize,
    /// Triangles.
    pub triangles: usize,
    /// Line segments.
    pub segments: usize,
    /// Points.
    pub points: usize,
    /// Instanced elements (drawn from a shared blob).
    pub instanced: usize,
    /// World bounds of everything drawn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[[f64; 3]; 2]>,
    /// Bytes on the wire.
    pub bytes: usize,
    /// Solids drawn through tessellation (counted in `elements` and
    /// `triangles` too; additive, v0.1 item 3 WP-B).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub solids: usize,
    /// The tier the solids were tessellated at (additive; present when a
    /// solid was drawn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<DisplayTier>,
    /// Elements that could not be drawn, with the reason (a solid in a
    /// build without the kernel, bytes the kernel refused). Additive;
    /// empty means everything drawable was drawn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Elements drawn with a caveat: a solid whose kernel mesh did not
    /// close (drawn as is). Additive; empty means every solid's mesh closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The triangle budget's verdict for this output (additive, v0.1 wave
    /// 5 D1; docs/12 §Display): present when a solid was drawn by a live
    /// emission — the tier the generation asked for, the tier the budget
    /// chose, the distinct solids' triangles at that tier, the limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetStats>,
}

/// The triangle budget's verdict for one output ([`choose_tier`]; docs/12
/// §Display): a pure function of the output's distinct solids, the tier
/// the generation asked for and the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BudgetStats {
    /// The budget: triangles per output per generation.
    pub limit: u64,
    /// The tier the generation asked for.
    pub requested: DisplayTier,
    /// The tier the output is drawn at: `requested`, or `Preview` when
    /// the request would exceed the limit.
    pub drawn: DisplayTier,
    /// The distinct solids' triangles at the drawn tier (exact).
    pub triangles: u64,
    /// Even the preview tier exceeds the limit: drawn at preview anyway,
    /// and said so. Omitted when false. (The TRIANGLE budget's flag — the
    /// `caches` view's `over_budget` is the cache's: the working set's
    /// bytes against the cache budget; the two are different budgets and
    /// the profiler will show both.)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub over_budget: bool,
}

/// [`choose_tier`]'s answer: the verdict, and the drawn tier's display
/// meshes as the tally fetched them through the cache — for the pass to
/// pin ([`DisplayContext::pinned`]). A refused solid has no mesh here; the
/// emit reports it.
pub struct Chosen {
    /// The verdict.
    pub stats: BudgetStats,
    /// The distinct solids' meshes at `stats.drawn`.
    pub meshes: DrawnMeshes,
    /// Cache lookups the decision made that the cache answered (both
    /// tiers' tallies when the fine one exceeded the limit).
    pub hits: u64,
    /// Cache lookups the decision made that called the kernel.
    pub misses: u64,
}

/// [`fetch_meshes`]'s answer: the meshes, and how the cache served them.
#[derive(Debug, Default)]
pub struct Fetched {
    /// The distinct solids' meshes at the asked tier (a refused solid is
    /// absent).
    pub meshes: DrawnMeshes,
    /// Lookups the cache answered.
    pub hits: u64,
    /// Lookups that called the kernel (an evicted mesh, meshed again).
    pub misses: u64,
}

/// What one [`tally`] did: the triangle total, whether it passed the limit,
/// the meshes it fetched, and how the cache served its lookups.
struct Tallied {
    triangles: u64,
    exceeded: bool,
    meshes: DrawnMeshes,
    hits: u64,
    misses: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The caller's parallel map for [`choose_tier`]'s per-solid tessellations
/// — the scheduler's worker pool in the session (`Scheduler::map_parallel`),
/// a serial loop in tests. Results in input order.
pub type ParallelMap<'a> = &'a (
        dyn Fn(
    Vec<(ValueHash, Solid)>,
    &(dyn Fn((ValueHash, Solid)) -> Option<u64> + Sync),
) -> Vec<Option<u64>>
            + Sync
    );

/// Tessellate `solids` at `deflection` through the cache, summing their
/// triangles; with a `limit`, stop meshing once the running total is past
/// it (the verdict "exceeds" is already certain, and the work wasted
/// before it is bounded by the limit plus one solid per worker). A refusal
/// counts no triangles — the emit reports it. Returns the total, whether
/// the limit was exceeded, the meshes fetched (every solid's when nothing
/// was skipped) and how the cache served the lookups: when the true total
/// fits, nothing is skipped and the total is exact; when it does not, the
/// total is a partial sum already past the limit — either way the verdict
/// is the value set's, never the meshing order's.
fn tally(
    solids: &[(ValueHash, Solid)],
    deflection: Deflection,
    cache: &SolidCache,
    limit: Option<u64>,
    map: ParallelMap<'_>,
) -> Tallied {
    let total = AtomicU64::new(0);
    let hits = AtomicU64::new(0);
    let misses = AtomicU64::new(0);
    let fetched = std::sync::Mutex::new(Vec::with_capacity(solids.len()));
    let _ = map(solids.to_vec(), &|(hash, solid): (ValueHash, Solid)| {
        if limit.is_some_and(|limit| total.load(Ordering::Relaxed) > limit) {
            return None;
        }
        let (result, served) = cache.tessellation_served(hash, &solid, deflection);
        match served {
            Served::Hit => hits.fetch_add(1, Ordering::Relaxed),
            Served::Miss => misses.fetch_add(1, Ordering::Relaxed),
        };
        let triangles = match result {
            Ok(mesh) => {
                let triangles = u64::try_from(mesh.mesh().triangle_count()).unwrap_or(u64::MAX);
                fetched
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((hash, mesh));
                triangles
            }
            Err(_) => 0,
        };
        total.fetch_add(triangles, Ordering::Relaxed);
        Some(triangles)
    });
    let triangles = total.load(Ordering::Relaxed);
    let meshes = fetched
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Tallied {
        triangles,
        exceeded: limit.is_some_and(|limit| triangles > limit),
        meshes,
        hits: hits.load(Ordering::Relaxed),
        misses: misses.load(Ordering::Relaxed),
    }
}

/// The display meshes of `solids` at `tier`, through the cache on the
/// caller's pool — a hash lookup each when warm, the kernel for an
/// evicted one: what a pass fetches and pins for an output whose verdict
/// the memo already knew (the verdict says which tier; the meshes may
/// have left the cache since — an undo/redo flip after a shrink, or
/// enough other value sets through it; review finding 2026-08-25). A
/// refused solid is absent; the hits and misses say how the cache served
/// the lookups (the profiler's per-output counters).
#[must_use]
pub fn fetch_meshes(
    solids: &[(ValueHash, Solid)],
    tier: DisplayTier,
    config: &ProjectConfig,
    cache: &SolidCache,
    map: ParallelMap<'_>,
) -> Fetched {
    let tallied = tally(solids, tier.deflection(config), cache, None, map);
    Fetched {
        meshes: tallied.meshes,
        hits: tallied.hits,
        misses: tallied.misses,
    }
}

/// The triangle budget's decision for one output (docs/12 §Display; the
/// D1 contract): `requested` is the generation's tier; when the output's
/// distinct `solids` would exceed `limit` triangles at it, the output is
/// drawn at [`DisplayTier::Preview`] instead, and when even the preview
/// tier exceeds the limit it is drawn at preview anyway and marked
/// `over_budget`. Every tessellation goes through `cache` — the decision
/// IS the warm-up — and the drawn tier's meshes come back with the
/// verdict for the pass to pin, so the emit that follows tessellates
/// nothing. A value without solids is drawn as asked (meshes and curves
/// have no tier and no budget — `triangles` is 0 and nothing is measured).
#[must_use]
pub fn choose_tier(
    solids: &[(ValueHash, Solid)],
    requested: DisplayTier,
    limit: u64,
    config: &ProjectConfig,
    cache: &SolidCache,
    map: ParallelMap<'_>,
) -> Chosen {
    let verdict = |drawn: DisplayTier, triangles: u64| BudgetStats {
        limit,
        requested,
        drawn,
        triangles,
        over_budget: triangles > limit,
    };
    if solids.is_empty() {
        return Chosen {
            stats: verdict(requested, 0),
            meshes: Vec::new(),
            hits: 0,
            misses: 0,
        };
    }
    let (mut hits, mut misses) = (0, 0);
    if requested == DisplayTier::Fine {
        let fine = tally(
            solids,
            DisplayTier::Fine.deflection(config),
            cache,
            Some(limit),
            map,
        );
        hits += fine.hits;
        misses += fine.misses;
        if !fine.exceeded {
            return Chosen {
                stats: verdict(DisplayTier::Fine, fine.triangles),
                meshes: fine.meshes,
                hits,
                misses,
            };
        }
    }
    let preview = tally(
        solids,
        DisplayTier::Preview.deflection(config),
        cache,
        None,
        map,
    );
    Chosen {
        stats: verdict(DisplayTier::Preview, preview.triangles),
        meshes: preview.meshes,
        hits: hits + preview.hits,
        misses: misses + preview.misses,
    }
}

/// A serial [`ParallelMap`] (tests, and any caller without a pool): pass
/// `&serial_map`.
pub fn serial_map(
    items: Vec<(ValueHash, Solid)>,
    f: &(dyn Fn((ValueHash, Solid)) -> Option<u64> + Sync),
) -> Vec<Option<u64>> {
    items.into_iter().map(f).collect()
}

impl DisplayStats {
    fn grow(&mut self, positions: &[f64]) {
        // A trailing partial triple is ignored, as `chunks_exact(3)` did.
        let (triples, _) = positions.as_chunks::<3>();
        for &point in triples {
            match &mut self.bounds {
                None => self.bounds = Some([point, point]),
                Some([lo, hi]) => {
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(point[axis]);
                        hi[axis] = hi[axis].max(point[axis]);
                    }
                }
            }
        }
    }
}

/// The frames for one output value: encoded bytes + stats. An empty result
/// (nothing drawable) yields exactly one `clear` frame.
pub struct DisplayFrames {
    /// Encoded frames, in send order.
    pub frames: Vec<Vec<u8>>,
    /// What they hold.
    pub stats: DisplayStats,
    /// The distinct solids drawn — each one's value hash and its display
    /// mesh's bytes as the cache counts them — at the context's tier: the
    /// output's share of the cache's working set (docs/12 §Display, the
    /// `over_budget` / `thrash` flags). Empty without solids.
    pub solids: Vec<(ValueHash, usize)>,
}

/// Does this value hold anything the viewport can draw?
#[must_use]
pub fn is_drawable(value: &HashedValue) -> bool {
    match value.data() {
        ValueData::Point(_) | ValueData::Curve(_) | ValueData::Mesh(_) | ValueData::Solid(_) => {
            true
        }
        ValueData::List(list) => list
            .slots
            .iter()
            .flatten()
            .any(|element| is_drawable(element)),
        _ => false,
    }
}

/// Encode the display frames of `value` for `(node, output)` at
/// `generation`. The context's tolerance feeds curve tessellation; its
/// configuration and cache draw solids. `picks` is asked once, up front,
/// for every drawn element's id ([`PickIds`]); nothing below it holds a
/// lock.
///
/// # Panics
///
/// When `picks` answers a different number of ids than elements it was
/// asked for — a caller bug ([`PickTable::ids_for`] never does), never a
/// data condition.
#[must_use]
#[allow(clippy::too_many_lines)] // one pass over points, curves, meshes, solids — splitting hides the frame order
pub fn frames_for_value(
    value: &HashedValue,
    generation: u64,
    node: u32,
    output: u32,
    picks: PickIds<'_>,
    context: &DisplayContext<'_>,
) -> DisplayFrames {
    let tolerance = context.config.tol();
    let mut points: Vec<(u32, [f64; 3])> = Vec::new();
    let mut curves: Vec<(u32, &Curve)> = Vec::new();
    let mut meshes: Vec<(u32, &HashedValue)> = Vec::new();
    let mut solids: Vec<(u32, &HashedValue)> = Vec::new();
    collect(
        value,
        None,
        &mut points,
        &mut curves,
        &mut meshes,
        &mut solids,
    );
    // The pick ids, in ONE ask — before the tessellation and the encoding
    // below, which the table's lock must not outlast.
    let mut elements: Vec<u32> = points
        .iter()
        .map(|(element, _)| *element)
        .chain(curves.iter().map(|(element, _)| *element))
        .chain(meshes.iter().map(|(element, _)| *element))
        .chain(solids.iter().map(|(element, _)| *element))
        .collect();
    elements.sort_unstable();
    elements.dedup();
    let ids = picks(&elements);
    assert_eq!(
        ids.len(),
        elements.len(),
        "PickIds answered {} ids for {} elements of ({node}, {output})",
        ids.len(),
        elements.len()
    );
    let pick_of: HashMap<u32, u32> = elements.iter().copied().zip(ids).collect();
    let pick = |element: u32| -> u32 {
        // Every element below came from the same collect — absent here
        // would be a bug in this function, never a data condition.
        pick_of.get(&element).copied().unwrap_or_else(|| {
            unreachable!("element {element} of ({node}, {output}) has no pick id")
        })
    };
    let element_count = u32::try_from(match value.data() {
        ValueData::List(list) => list.slots.len(),
        _ => 1,
    })
    .unwrap_or(u32::MAX);
    let header = |kind: FrameKind| Header {
        kind,
        generation,
        node,
        output,
        element_start: 0,
        element_count,
    };
    let mut out = DisplayFrames {
        frames: Vec::new(),
        stats: DisplayStats::default(),
        solids: Vec::new(),
    };

    // Points: one batch.
    if !points.is_empty() {
        let mut batch = Batch::new();
        for (element, xyz) in &points {
            batch.push_element(*element, pick(*element), xyz, &[]);
            out.stats.grow(xyz);
        }
        out.stats.points += points.len();
        out.stats.vertices += points.len();
        out.stats.elements += points.len();
        out.stats.kinds.push("point");
        out.frames.push(encode_batch(
            &header(FrameKind::Point),
            FrameKind::Point,
            &batch,
        ));
    }

    // Curves: tessellated to segment pairs, one batch.
    if !curves.is_empty() {
        let mut batch = Batch::new();
        for (element, curve) in &curves {
            let (positions, indices) = tessellate_curve(curve, tolerance);
            if positions.is_empty() {
                continue;
            }
            out.stats.grow(&positions);
            out.stats.segments += indices.len() / 2;
            out.stats.vertices += positions.len() / 3;
            out.stats.elements += 1;
            batch.push_element(*element, pick(*element), &positions, &indices);
        }
        if !batch.is_empty() {
            out.stats.kinds.push("curve");
            out.frames.push(encode_batch(
                &header(FrameKind::Curve),
                FrameKind::Curve,
                &batch,
            ));
        }
    }

    // Solids: the mesh the pass pinned for this solid at this tier (a live
    // emission: the warm-up fetched it on the solve loop's workers — the
    // encode under the lock never tessellates), else the cache's (a
    // restream, a pass whose graph changed under an intent between its
    // warm-up and its encode), then drawn as meshes under the DISPLAY
    // MESH's value hash — identical
    // solids at one deflection instance like identical meshes, and the same
    // solid at another deflection is another blob. A mesh that did not
    // close draws all the same, with a warning on record; a solid that
    // cannot be tessellated is reported, not drawn.
    let mut tessellated: Vec<(u32, Arc<DisplayMesh>)> = Vec::new();
    if !solids.is_empty() {
        let deflection = context.deflection();
        // The working set this output holds in the cache: one entry per
        // DISTINCT solid (by value hash), sized as the cache counts it.
        let mut distinct: BTreeMap<ValueHash, usize> = BTreeMap::new();
        for (element, value) in &solids {
            let ValueData::Solid(solid) = value.data() else {
                continue;
            };
            let pinned = context
                .pinned
                .and_then(|pinned| pinned.get(&TessellationKey::new(value.hash(), deflection)))
                .map(Arc::clone);
            let tessellated_mesh = match pinned {
                Some(mesh) => Ok(mesh),
                None => context.solids.tessellation(value.hash(), solid, deflection),
            };
            match tessellated_mesh {
                Ok(mesh) => {
                    if !mesh.watertight {
                        out.stats.warnings.push(format!(
                            "element {element} (Solid): the kernel's mesh does not close at \
                             this deflection; drawn as is"
                        ));
                    }
                    distinct
                        .entry(value.hash())
                        .or_insert_with(|| mesh_bytes(mesh.mesh()));
                    tessellated.push((*element, mesh));
                }
                Err(reason) => out
                    .stats
                    .errors
                    .push(format!("element {element} (Solid): {reason}")),
            }
        }
        out.stats.solids = tessellated.len();
        if !tessellated.is_empty() {
            out.stats.tier = Some(context.tier);
            out.stats.budget = context.budget;
        }
        out.solids = distinct.into_iter().collect();
    }

    // Meshes: hash-driven instancing — a hash seen once goes inline; a
    // hash shared by several elements travels once as a blob plus an
    // instances frame (identity transforms in the spike).
    if !meshes.is_empty() || !tessellated.is_empty() {
        // The element's VALUE hash is the interning key (docs/12) — no
        // re-hashing: list slots are already-hashed values, and a display
        // mesh is sealed once when it is tessellated.
        let mut by_hash: BTreeMap<ValueHash, Vec<(u32, &Mesh)>> = BTreeMap::new();
        for (element, value) in &meshes {
            if let ValueData::Mesh(mesh) = value.data() {
                by_hash
                    .entry(value.hash())
                    .or_default()
                    .push((*element, mesh));
            }
        }
        for (element, display) in &tessellated {
            by_hash
                .entry(display.hash())
                .or_default()
                .push((*element, display.mesh()));
        }
        let mut batch = Batch::new();
        for (hash, group) in &by_hash {
            let mesh = group[0].1;
            if group.len() == 1 {
                let (element, mesh) = group[0];
                batch.push_element(element, pick(element), mesh.positions(), mesh.indices());
                out.stats.grow(mesh.positions());
                out.stats.triangles += mesh.triangle_count();
                out.stats.vertices += mesh.vertex_count();
                out.stats.elements += 1;
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let positions: Vec<f32> = mesh.positions().iter().map(|&x| x as f32).collect();
            out.frames.push(encode_mesh_blob(
                &header(FrameKind::MeshBlob),
                hash,
                &positions,
                mesh.indices(),
            ));
            let instances: Vec<Instance> = group
                .iter()
                .map(|&(element, _)| Instance {
                    element_index: element,
                    pick_id: pick(element),
                    transform: IDENTITY,
                })
                .collect();
            out.frames.push(encode_instances(
                &header(FrameKind::Instances),
                hash,
                &instances,
            ));
            out.stats.grow(mesh.positions());
            out.stats.triangles += mesh.triangle_count() * group.len();
            out.stats.vertices += mesh.vertex_count();
            out.stats.elements += group.len();
            out.stats.instanced += group.len();
            if !out.stats.kinds.contains(&"instances") {
                out.stats.kinds.push("instances");
            }
        }
        if !batch.is_empty() {
            out.stats.kinds.push("mesh");
            out.frames.push(encode_batch(
                &header(FrameKind::Mesh),
                FrameKind::Mesh,
                &batch,
            ));
        }
    }

    if out.frames.is_empty() {
        out.stats.kinds.push("clear");
        out.frames.push(encode_clear(&header(FrameKind::Clear)));
    }
    out.stats.bytes = out.frames.iter().map(Vec::len).sum();
    out
}

/// A clear frame for an output that no longer draws (red, gone, scalar).
#[must_use]
pub fn clear_frame(generation: u64, node: u32, output: u32) -> Vec<u8> {
    encode_clear(&Header {
        kind: FrameKind::Clear,
        generation,
        node,
        output,
        element_start: 0,
        element_count: 0,
    })
}

/// Walk a value collecting drawables; nested lists inherit the top-level
/// element index (provenance is the outer slot).
fn collect<'v>(
    value: &'v HashedValue,
    element: Option<u32>,
    points: &mut Vec<(u32, [f64; 3])>,
    curves: &mut Vec<(u32, &'v Curve)>,
    meshes: &mut Vec<(u32, &'v HashedValue)>,
    solids: &mut Vec<(u32, &'v HashedValue)>,
) {
    let index = element.unwrap_or(0);
    match value.data() {
        ValueData::Point(p) => points.push((index, [p.0.x, p.0.y, p.0.z])),
        ValueData::Curve(curve) => curves.push((index, curve)),
        ValueData::Mesh(_) => meshes.push((index, value)),
        ValueData::Solid(_) => solids.push((index, value)),
        ValueData::List(list) => {
            for (slot, item) in list.slots.iter().enumerate() {
                if let Some(item) = item {
                    let own = element.unwrap_or(u32::try_from(slot).unwrap_or(u32::MAX));
                    collect(item, Some(own), points, curves, meshes, solids);
                }
            }
        }
        _ => {}
    }
}

/// Display tessellation: `(positions xyz, segment index pairs)`.
fn tessellate_curve(curve: &Curve, tolerance: f64) -> (Vec<f64>, Vec<u32>) {
    let chain: Vec<[f64; 3]> = match curve {
        Curve::Line(line) => vec![
            [line.a.0.x, line.a.0.y, line.a.0.z],
            [line.b.0.x, line.b.0.y, line.b.0.z],
        ],
        Curve::Polyline(polyline) => polyline
            .vertices
            .iter()
            .map(|p| [p.0.x, p.0.y, p.0.z])
            .collect(),
        Curve::Circle(_) | Curve::Rectangle(_) => {
            match tessellate_closed(curve, CIRCLE_SEGMENTS, tolerance) {
                Ok(points) => points.iter().map(|p| [p.0.x, p.0.y, p.0.z]).collect(),
                // A degenerate analytic curve (which the node would have
                // refused) draws nothing rather than lying.
                Err(_) => Vec::new(),
            }
        }
    };
    if chain.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let closed = curve.is_closed();
    let mut positions = Vec::with_capacity(chain.len() * 3);
    for p in &chain {
        positions.extend_from_slice(p);
    }
    let n = u32::try_from(chain.len()).unwrap_or(u32::MAX);
    let mut indices = Vec::with_capacity((chain.len() * 2) + 2);
    for i in 0..n - 1 {
        indices.push(i);
        indices.push(i + 1);
    }
    if closed && n > 2 {
        indices.push(n - 1);
        indices.push(0);
    }
    (positions, indices)
}

// -------------------------------------------------------------- summaries --

/// A compact summary of a value (inspector / hover / port preview). The
/// context is what a `Solid` needs for its facts and bounds (the display
/// tessellation — a cache hit when the value is displayed).
#[must_use]
pub fn summarize(value: &HashedValue, context: &DisplayContext<'_>) -> ValueSummary {
    let mut summary = ValueSummary {
        kind: value.data().kind_name().to_owned(),
        hash: value.hash().to_hex(),
        count: None,
        absent: None,
        axis: None,
        bounds: None,
        samples: Vec::new(),
        facts: BTreeMap::new(),
    };
    match value.data() {
        ValueData::List(list) => summarize_list(value, list, &mut summary, context),
        ValueData::Solid(solid) => summarize_solid(value, solid, &mut summary, context),
        ValueData::Mesh(mesh) => {
            summary.samples = vec![render(value)];
            summary.facts.insert(
                "vertices".to_owned(),
                serde_json::json!(mesh.vertex_count()),
            );
            summary.facts.insert(
                "triangles".to_owned(),
                serde_json::json!(mesh.triangle_count()),
            );
            summary.facts.insert(
                "watertight".to_owned(),
                serde_json::json!(mesh.is_watertight()),
            );
            let mut stats = DisplayStats::default();
            stats.grow(mesh.positions());
            summary.bounds = stats.bounds;
        }
        ValueData::Curve(curve) => {
            summary.samples = vec![render(value)];
            summary.facts.insert(
                "variant".to_owned(),
                serde_json::json!(curve.variant_name()),
            );
            summary
                .facts
                .insert("closed".to_owned(), serde_json::json!(curve.is_closed()));
            let (positions, _) = tessellate_curve(curve, 1e-6);
            let mut stats = DisplayStats::default();
            stats.grow(&positions);
            summary.bounds = stats.bounds;
        }
        ValueData::Point(p) => {
            summary.samples = vec![render(value)];
            let xyz = [p.0.x, p.0.y, p.0.z];
            summary.bounds = Some([xyz, xyz]);
        }
        _ => summary.samples = vec![render(value)],
    }
    summary
}

/// The list arm of [`summarize`]: counts, samples, element kind, and the
/// bounds of everything drawable inside (solids through the cache).
fn summarize_list(
    value: &HashedValue,
    list: &cicada_core::value::List,
    summary: &mut ValueSummary,
    context: &DisplayContext<'_>,
) {
    summary.count = Some(list.slots.len());
    summary.absent = Some(list.slots.iter().filter(|slot| slot.is_none()).count());
    summary.axis = list.axis.as_ref().map(std::string::ToString::to_string);
    summary.samples = list
        .slots
        .iter()
        .take(8)
        .map(|slot| slot.as_ref().map_or_else(|| "∅".to_owned(), |v| render(v)))
        .collect();
    if let Some(kind) = list
        .slots
        .iter()
        .flatten()
        .map(|v| v.data().kind_name())
        .next()
    {
        summary
            .facts
            .insert("element_kind".to_owned(), serde_json::json!(kind));
    }
    let mut stats = DisplayStats::default();
    let mut points = Vec::new();
    let mut curves = Vec::new();
    let mut meshes = Vec::new();
    let mut solids = Vec::new();
    collect(
        value,
        None,
        &mut points,
        &mut curves,
        &mut meshes,
        &mut solids,
    );
    for (_, xyz) in &points {
        stats.grow(xyz);
    }
    for (_, curve) in &curves {
        let (positions, _) = tessellate_curve(curve, 1e-6);
        stats.grow(&positions);
    }
    let mut triangles = 0;
    for (_, value) in &meshes {
        if let ValueData::Mesh(mesh) = value.data() {
            stats.grow(mesh.positions());
            triangles += mesh.triangle_count();
        }
    }
    let mut errors = Vec::new();
    let drawn = listed_solid_facts(&solids, context, &mut stats, &mut errors);
    triangles += drawn.triangles;
    if triangles > 0 {
        summary
            .facts
            .insert("triangles".to_owned(), serde_json::json!(triangles));
    }
    if !solids.is_empty() {
        summary
            .facts
            .insert("solids".to_owned(), serde_json::json!(solids.len()));
        // The mesh-derived facts (`faces`, `triangles`, the bounds) cover the
        // displayed solids; `not_displayed` says how many they leave out.
        if drawn.not_displayed < solids.len() {
            summary
                .facts
                .insert("faces".to_owned(), serde_json::json!(drawn.faces));
        }
        if drawn.not_displayed > 0 {
            summary.facts.insert(
                "not_displayed".to_owned(),
                serde_json::json!(drawn.not_displayed),
            );
        }
        if drawn.unclosed > 0 {
            summary
                .facts
                .insert("unclosed".to_owned(), serde_json::json!(drawn.unclosed));
        }
    }
    if !errors.is_empty() {
        summary
            .facts
            .insert("error".to_owned(), serde_json::json!(errors.join("; ")));
    }
    summary.bounds = stats.bounds;
}

/// What a list's solids contribute to its summary, read off the display
/// cache ([`SolidCache::tessellation_for_summary`]): the drawn ones grow
/// `stats` and count their triangles, faces and unclosed meshes; a cached
/// refusal is an `errors` line; a solid no pass has meshed is counted in
/// `not_displayed` and never meshed here.
#[derive(Default)]
struct ListedSolidFacts {
    triangles: usize,
    faces: usize,
    unclosed: usize,
    not_displayed: usize,
}

fn listed_solid_facts(
    solids: &[(u32, &HashedValue)],
    context: &DisplayContext<'_>,
    stats: &mut DisplayStats,
    errors: &mut Vec<String>,
) -> ListedSolidFacts {
    let mut facts = ListedSolidFacts::default();
    for (element, value) in solids {
        if !matches!(value.data(), ValueData::Solid(_)) {
            continue;
        }
        match context
            .solids
            .tessellation_for_summary(value.hash(), context.config)
        {
            Some(Ok(display)) => {
                stats.grow(display.mesh().positions());
                facts.triangles += display.mesh().triangle_count();
                facts.faces += display.faces;
                if !display.watertight {
                    facts.unclosed += 1;
                }
            }
            Some(Err(reason)) => errors.push(format!("element {element} (Solid): {reason}")),
            None => facts.not_displayed += 1,
        }
    }
    facts
}

/// The solid arm of [`summarize`] — "Solid, N faces, bbox": the facts come
/// from the display tessellation (a cache hit when the value is on screen,
/// at whichever tier drew it); a solid the kernel cannot tessellate says
/// why instead, one whose mesh did not close says `watertight: false`, and
/// one no display pass has meshed says `tessellation: "not displayed"` —
/// the summary never meshes anything itself.
fn summarize_solid(
    value: &HashedValue,
    solid: &Solid,
    summary: &mut ValueSummary,
    context: &DisplayContext<'_>,
) {
    summary.samples = vec![render(value)];
    summary
        .facts
        .insert("bytes".to_owned(), serde_json::json!(solid.bytes().len()));
    match context
        .solids
        .tessellation_for_summary(value.hash(), context.config)
    {
        Some(Ok(display)) => {
            summary
                .facts
                .insert("faces".to_owned(), serde_json::json!(display.faces));
            summary.facts.insert(
                "triangles".to_owned(),
                serde_json::json!(display.mesh().triangle_count()),
            );
            summary.facts.insert(
                "watertight".to_owned(),
                serde_json::json!(display.watertight),
            );
            let mut stats = DisplayStats::default();
            stats.grow(display.mesh().positions());
            summary.bounds = stats.bounds;
        }
        Some(Err(reason)) => {
            summary
                .facts
                .insert("error".to_owned(), serde_json::json!(reason));
        }
        None => {
            summary
                .facts
                .insert("tessellation".to_owned(), serde_json::json!(NOT_DISPLAYED));
        }
    }
}

/// The `tessellation` fact of a solid summary no display pass has meshed
/// (and the `not_displayed` count of a list's): the mesh-derived facts and
/// bounds are missing because nothing drew the value, not because it has
/// none. A summary carrying either is not memoized by the session — the
/// next read may find the mesh drawn.
pub const NOT_DISPLAYED: &str = "not displayed";

/// Does `summary` read solids no display pass has meshed yet?
#[must_use]
pub fn reads_undisplayed_solids(summary: &ValueSummary) -> bool {
    summary.facts.contains_key("not_displayed")
        || summary.facts.get("tessellation").and_then(|v| v.as_str()) == Some(NOT_DISPLAYED)
}

/// Compact human rendering of a value (the inspector's sample text; the
/// same shapes `cicada run` prints).
#[must_use]
pub fn render(value: &HashedValue) -> String {
    match value.data() {
        ValueData::Number(x) => format!("{x}"),
        ValueData::Integer(i) => format!("{i}"),
        ValueData::Boolean(b) => format!("{b}"),
        ValueData::Text(s) => format!("{s:?}"),
        ValueData::Color(c) => format!("Color({}, {}, {}, {})", c.r, c.g, c.b, c.a),
        ValueData::Domain(d) => format!("{}..{}", d.start, d.end),
        ValueData::IndexMap(m) => format!("IndexMap(×{})", m.0.len()),
        ValueData::Point(p) => format!("({}, {}, {})", p.0.x, p.0.y, p.0.z),
        ValueData::Vector(v) => format!("({}, {}, {})", v.0.x, v.0.y, v.0.z),
        ValueData::Plane(plane) => format!(
            "Plane(origin ({}, {}, {}))",
            plane.origin.0.x, plane.origin.0.y, plane.origin.0.z
        ),
        ValueData::Xform(_) => "Xform".to_owned(),
        ValueData::Curve(curve) => match curve {
            Curve::Line(line) => format!(
                "Line(({}, {}, {}) → ({}, {}, {}))",
                line.a.0.x, line.a.0.y, line.a.0.z, line.b.0.x, line.b.0.y, line.b.0.z
            ),
            Curve::Polyline(p) => format!(
                "Polyline(×{}{})",
                p.vertices.len(),
                if p.closed { ", closed" } else { "" }
            ),
            Curve::Circle(c) => format!(
                "Circle(center ({}, {}, {}), r {})",
                c.plane.origin.0.x, c.plane.origin.0.y, c.plane.origin.0.z, c.radius
            ),
            Curve::Rectangle(r) => format!(
                "Rectangle({}..{} × {}..{})",
                r.x.start, r.x.end, r.y.start, r.y.end
            ),
        },
        ValueData::Mesh(mesh) => format!(
            "Mesh({} vertices, {} triangles)",
            mesh.vertex_count(),
            mesh.triangle_count()
        ),
        ValueData::Solid(solid) => format!("Solid({} bytes)", solid.bytes().len()),
        ValueData::List(list) => {
            let shown: Vec<String> = list
                .slots
                .iter()
                .take(4)
                .map(|slot| slot.as_ref().map_or_else(|| "∅".to_owned(), |v| render(v)))
                .collect();
            let ellipsis = if list.slots.len() > 4 { ", …" } else { "" };
            let axis = list
                .axis
                .as_ref()
                .map_or_else(String::new, |axis| format!("{axis}: "));
            format!(
                "[{axis}{}{ellipsis}] ×{}",
                shown.join(", "),
                list.slots.len()
            )
        }
        ValueData::Nothing => "Nothing".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{Frame, decode};
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::List;
    use std::sync::Arc;

    fn point(x: f64) -> Arc<HashedValue> {
        HashedValue::new(ValueData::Point(Point::new(x, 0.0, 0.0))).unwrap()
    }

    /// A default project and a fresh cache — what the session hands in —
    /// at a tier.
    struct TestContext {
        config: ProjectConfig,
        solids: SolidCache,
    }

    impl TestContext {
        fn new() -> Self {
            Self {
                config: ProjectConfig::default(),
                solids: SolidCache::default(),
            }
        }

        fn with_budget(budget: usize) -> Self {
            Self {
                config: ProjectConfig::default(),
                solids: SolidCache::new(budget),
            }
        }

        fn context(&self) -> DisplayContext<'_> {
            self.at(DisplayTier::Fine)
        }

        fn at(&self, tier: DisplayTier) -> DisplayContext<'_> {
            DisplayContext {
                config: &self.config,
                solids: &self.solids,
                tier,
                budget: None,
                pinned: None,
            }
        }
    }

    /// The probe's 10 × 20 × 30 box at the origin: real canonical bytes
    /// (WP-A's golden `e220198a…`), committed so these tests draw a real
    /// solid.
    fn probe_box() -> Arc<HashedValue> {
        let bytes = include_bytes!("../tests/fixtures/box-10x20x30.brep.bin");
        assert_eq!(bytes.len(), 4494);
        HashedValue::new(ValueData::Solid(
            Solid::from_canonical_bytes(bytes.to_vec()).unwrap(),
        ))
        .unwrap()
    }

    /// A curved solid — radius 1, height 2 — whose display mesh depends on
    /// the deflection (a box's does not).
    fn cylinder() -> Arc<HashedValue> {
        let solid = solids::cylinder(&Plane::world_xy(), 1.0, 2.0, 1e-6).unwrap();
        HashedValue::new(ValueData::Solid(solid)).unwrap()
    }

    fn solid_of(value: &HashedValue) -> &Solid {
        let ValueData::Solid(solid) = value.data() else {
            panic!("solid")
        };
        solid
    }

    fn tetra(offset: f64) -> Mesh {
        Mesh::new(
            vec![
                offset,
                0.0,
                0.0, //
                offset + 1.0,
                0.0,
                0.0, //
                offset,
                1.0,
                0.0, //
                offset,
                0.0,
                1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
        )
        .unwrap()
    }

    #[test]
    fn the_server_tests_run_in_the_kernel_world() {
        // cicada-server depends on cicada-geom with its default features,
        // and `occt` is a default feature since WP-C: every solid test below
        // draws through the real kernel. A build that turned it off would
        // make them vacuous — this says so instead of passing quietly.
        assert!(
            solids::kernel_available(),
            "cicada-server's display tests need the OCCT kernel (cicada-geom feature `occt`)"
        );
    }

    #[test]
    fn a_point_list_becomes_one_point_batch_with_stable_picks() {
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(point(0.0)), None, Some(point(2.0))],
        }))
        .unwrap();
        let test = TestContext::new();
        let mut picks = PickTable::default();
        let first = frames_for_value(
            &list,
            1,
            5,
            0,
            &mut |e| picks.ids_for(5, 0, e),
            &test.context(),
        );
        assert_eq!(first.stats.kinds, vec!["point"]);
        assert_eq!(first.stats.points, 2);
        assert_eq!(first.stats.bounds, Some([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]));
        let Frame::Batch { header, batch } = decode(&first.frames[0]).unwrap() else {
            panic!("point batch")
        };
        assert_eq!(
            (header.node, header.output, header.element_count),
            (5, 0, 3)
        );
        assert_eq!(
            batch.elements[1].element_index, 2,
            "absent slot skipped, index kept"
        );
        let pick_of_third = batch.elements[1].pick_id;
        assert_eq!(picks.resolve(pick_of_third), Some((5, 0, 2)));
        // Same triple next generation → same pick id.
        let second = frames_for_value(
            &list,
            2,
            5,
            0,
            &mut |e| picks.ids_for(5, 0, e),
            &test.context(),
        );
        let Frame::Batch { batch, .. } = decode(&second.frames[0]).unwrap() else {
            panic!("point batch")
        };
        assert_eq!(batch.elements[1].pick_id, pick_of_third);
    }

    /// The pick ids are asked for ONCE, before any encoding, for every
    /// element the output draws — the contract that lets the session hold
    /// the pick table's mutex for that ask alone (review 2026-08-21: the
    /// encoder used to take an id per element while it encoded, so a
    /// joiner's restream held the table across a 94 MB encode and the live
    /// path, which takes it under the session lock, waited with every
    /// intent behind it). Points, curves and meshes of one list, a nested
    /// list sharing its outer slot, an absent slot: one ask, the distinct
    /// slots ascending, and the frames carry exactly the ids answered.
    #[test]
    fn pick_ids_are_asked_for_once_up_front_for_every_drawn_element() {
        let test = TestContext::new();
        let nested = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(point(7.0)), Some(point(8.0))],
        }))
        .unwrap();
        let circle = HashedValue::new(ValueData::Curve(Curve::Circle(
            cicada_core::geometry::Circle {
                plane: cicada_core::spatial::Plane::world_xy(),
                radius: 2.0,
            },
        )))
        .unwrap();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![
                Some(HashedValue::new(ValueData::Mesh(tetra(0.0))).unwrap()),
                None,
                Some(circle),
                Some(nested),
                Some(point(1.0)),
            ],
        }))
        .unwrap();
        let mut asks: Vec<Vec<u32>> = Vec::new();
        let out = frames_for_value(
            &list,
            1,
            9,
            0,
            &mut |elements: &[u32]| {
                asks.push(elements.to_vec());
                // Ids of the test's choosing: element + 100.
                elements.iter().map(|e| e + 100).collect()
            },
            &test.context(),
        );
        assert_eq!(
            asks,
            vec![vec![0, 2, 3, 4]],
            "one ask, the distinct drawn slots ascending (the absent slot 1 is no element)"
        );
        let mut carried: Vec<(u32, u32)> = Vec::new();
        for bytes in &out.frames {
            if let Frame::Batch { batch, .. } = decode(bytes).unwrap() {
                for element in &batch.elements {
                    carried.push((element.element_index, element.pick_id));
                }
            }
        }
        carried.sort_unstable();
        assert_eq!(
            carried,
            vec![(0, 100), (2, 102), (3, 103), (3, 103), (4, 104)],
            "every element carries the id answered for its slot — the nested list's two points share slot 3"
        );
        // A scalar draws nothing and still makes its one (empty) ask: the
        // ask count is the encode count the session's `picks.encodes` reads.
        let number = HashedValue::new(ValueData::Number(1.0)).unwrap();
        let mut table = PickTable::default();
        let before = table.encodes();
        let _ = frames_for_value(
            &number,
            1,
            9,
            0,
            &mut |e| table.ids_for(9, 0, e),
            &test.context(),
        );
        assert_eq!(table.encodes(), before + 1);
        assert!(table.is_empty(), "no element, no id");
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact bounds from exact inputs
    fn repeated_mesh_hashes_instance_and_singles_go_inline() {
        let shared = HashedValue::new(ValueData::Mesh(tetra(0.0))).unwrap();
        let single = HashedValue::new(ValueData::Mesh(tetra(5.0))).unwrap();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(shared.clone()), Some(single), Some(shared)],
        }))
        .unwrap();
        let test = TestContext::new();
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &list,
            3,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.context(),
        );
        assert_eq!(out.stats.instanced, 2);
        assert_eq!(out.stats.elements, 3);
        assert_eq!(out.stats.triangles, 12);
        let kinds: Vec<FrameKind> = out
            .frames
            .iter()
            .map(|bytes| decode(bytes).unwrap().header().kind)
            .collect();
        assert_eq!(
            kinds,
            vec![FrameKind::MeshBlob, FrameKind::Instances, FrameKind::Mesh]
        );
        let Frame::Instances { instances, .. } = decode(&out.frames[1]).unwrap() else {
            panic!("instances")
        };
        assert_eq!(
            instances
                .iter()
                .map(|i| i.element_index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(out.stats.bounds.unwrap()[1][0], 6.0);
    }

    #[test]
    fn scalars_clear_and_curves_tessellate() {
        let test = TestContext::new();
        let number = HashedValue::new(ValueData::Number(1.0)).unwrap();
        assert!(!is_drawable(&number));
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &number,
            1,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.context(),
        );
        assert_eq!(out.stats.kinds, vec!["clear"]);
        let circle = HashedValue::new(ValueData::Curve(Curve::Circle(
            cicada_core::geometry::Circle {
                plane: cicada_core::spatial::Plane::world_xy(),
                radius: 2.0,
            },
        )))
        .unwrap();
        let out = frames_for_value(
            &circle,
            1,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.context(),
        );
        assert_eq!(out.stats.kinds, vec!["curve"]);
        assert_eq!(
            out.stats.segments,
            usize::try_from(CIRCLE_SEGMENTS).unwrap(),
            "closed loop"
        );
        let bounds = out.stats.bounds.unwrap();
        assert!((bounds[1][0] - 2.0).abs() < 1e-9);
        let summary = summarize(&circle, &test.context());
        assert_eq!(summary.kind, "Curve");
        assert_eq!(summary.facts["closed"], true);
    }

    // ------------------------------------------------------------ solids --

    /// The summary READS the display cache and never calls the kernel
    /// (wave 5 N1 review CR-2, 2026-09-19): a solid no pass has drawn says
    /// `tessellation: "not displayed"` — no mesh facts, no bounds, no miss
    /// — and a list counts such elements as `not_displayed`, reporting the
    /// mesh facts of the drawn ones alone; once the display has drawn the
    /// solid the same summary carries its facts, and the session's memo
    /// rule (`reads_undisplayed_solids`) tells the two apart.
    #[test]
    fn a_summary_reads_the_cache_and_never_meshes() {
        let test = TestContext::new();
        let solid = probe_box();
        let summary = summarize(&solid, &test.context());
        assert_eq!(summary.kind, "Solid");
        assert_eq!(summary.facts["tessellation"], NOT_DISPLAYED);
        assert!(!summary.facts.contains_key("faces"), "{:?}", summary.facts);
        assert!(!summary.facts.contains_key("triangles"));
        assert!(summary.bounds.is_none());
        assert!(reads_undisplayed_solids(&summary));
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(solid.clone()), Some(solid.clone())],
        }))
        .unwrap();
        let summary = summarize(&list, &test.context());
        assert_eq!(summary.facts["solids"], 2);
        assert_eq!(summary.facts["not_displayed"], 2);
        assert!(!summary.facts.contains_key("faces"), "{:?}", summary.facts);
        assert!(summary.bounds.is_none());
        assert!(reads_undisplayed_solids(&summary));
        let stats = test.solids.stats();
        assert_eq!(
            (stats.misses, stats.entries),
            (0, 0),
            "no summary called the kernel: {stats:?}"
        );
        // Drawn once: every summary reads the one entry.
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &solid,
            1,
            7,
            0,
            &mut |e| picks.ids_for(7, 0, e),
            &test.context(),
        );
        assert_eq!(out.stats.solids, 1);
        let summary = summarize(&solid, &test.context());
        assert_eq!(summary.facts["triangles"], 12);
        assert!(summary.facts["faces"].is_number());
        assert!(!summary.facts.contains_key("tessellation"));
        assert!(summary.bounds.is_some());
        assert!(!reads_undisplayed_solids(&summary));
        let summary = summarize(&list, &test.context());
        assert!(!summary.facts.contains_key("not_displayed"));
        assert_eq!(summary.facts["triangles"], 24);
        assert!(!reads_undisplayed_solids(&summary));
        assert_eq!(test.solids.stats().misses, 1, "the one draw");
    }

    /// One bare solid: the box as the display path reports it — a real
    /// cube through the kernel, one miss for the frames, hits after.
    #[test]
    #[allow(clippy::float_cmp)] // exact bounds from exact planar geometry
    fn a_solid_draws_as_mesh_frames_through_the_cache() {
        let test = TestContext::new();
        let solid = probe_box();
        assert!(is_drawable(&solid));
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &solid,
            1,
            7,
            0,
            &mut |e| picks.ids_for(7, 0, e),
            &test.context(),
        );
        let summary = summarize(&solid, &test.context());
        assert_eq!(summary.kind, "Solid");
        assert_eq!(summary.facts["bytes"], 4494);
        assert_eq!(summary.samples, vec!["Solid(4494 bytes)"]);
        assert_eq!(out.stats.kinds, vec!["mesh"]);
        assert_eq!(out.stats.solids, 1);
        assert_eq!(out.stats.tier, Some(DisplayTier::Fine));
        assert_eq!(out.stats.elements, 1);
        assert_eq!(out.stats.triangles, 12);
        assert_eq!(out.stats.vertices, 8);
        assert!(out.stats.errors.is_empty(), "{:?}", out.stats.errors);
        assert!(out.stats.warnings.is_empty(), "{:?}", out.stats.warnings);
        assert_eq!(
            out.stats.bounds,
            Some([[0.0, 0.0, 0.0], [10.0, 20.0, 30.0]])
        );
        let Frame::Batch { header, batch } = decode(&out.frames[0]).unwrap() else {
            panic!("mesh batch")
        };
        assert_eq!(header.kind, FrameKind::Mesh);
        assert_eq!((header.node, header.output), (7, 0));
        assert_eq!(batch.elements.len(), 1);
        assert_eq!(picks.resolve(batch.elements[0].pick_id), Some((7, 0, 0)));
        // The summary: "Solid, N faces, bbox", closed.
        assert_eq!(summary.facts["faces"], 6);
        assert_eq!(summary.facts["triangles"], 12);
        assert_eq!(summary.facts["watertight"], true);
        assert_eq!(summary.bounds, Some([[0.0, 0.0, 0.0], [10.0, 20.0, 30.0]]));
        assert!(!summary.facts.contains_key("error"));
        // One miss for the frames, one hit for the summary.
        let stats = test.solids.stats();
        assert_eq!((stats.misses, stats.hits, stats.entries), (1, 1, 1));
        assert_eq!(stats.bytes, 8 * 3 * 8 + 12 * 3 * 4);
        assert_eq!(stats.refusals, 0);
        // Drawing it again is a hit, not a kernel call.
        let again = frames_for_value(
            &solid,
            2,
            7,
            0,
            &mut |e| picks.ids_for(7, 0, e),
            &test.context(),
        );
        assert_eq!(again.stats.triangles, 12);
        assert_eq!(test.solids.stats().hits, 2);
        assert_eq!(test.solids.stats().misses, 1);
        // The JSON the debug state carries: `tier` is a lowercase word,
        // `warnings` is omitted when empty.
        let json = serde_json::to_value(&out.stats).unwrap();
        assert_eq!(json["tier"], "fine");
        assert!(json.get("warnings").is_none());
        assert!(json.get("errors").is_none());
    }

    #[test]
    fn identical_solids_in_a_list_instance_under_the_display_meshs_hash() {
        let test = TestContext::new();
        let solid = probe_box();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(solid.clone()), None, Some(solid.clone())],
        }))
        .unwrap();
        assert!(is_drawable(&list));
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &list,
            1,
            2,
            0,
            &mut |e| picks.ids_for(2, 0, e),
            &test.context(),
        );
        let summary = summarize(&list, &test.context());
        assert_eq!(summary.count, Some(3));
        assert_eq!(summary.facts["element_kind"], "Solid");
        assert_eq!(summary.facts["solids"], 2);
        assert!(!summary.facts.contains_key("unclosed"));
        assert_eq!(out.stats.solids, 2);
        assert_eq!(out.stats.instanced, 2);
        assert_eq!(out.stats.triangles, 24);
        let kinds: Vec<FrameKind> = out
            .frames
            .iter()
            .map(|bytes| decode(bytes).unwrap().header().kind)
            .collect();
        assert_eq!(kinds, vec![FrameKind::MeshBlob, FrameKind::Instances]);
        let Frame::MeshBlob { hash, .. } = decode(&out.frames[0]).unwrap() else {
            panic!("blob")
        };
        // The blob is keyed by the DISPLAY MESH's content hash — the hash
        // the same mesh would have as a Mesh value — not by the solid's
        // (whose content is independent of the deflection).
        assert_ne!(hash, solid.hash(), "not the Solid's hash");
        let cached = test
            .solids
            .tessellation(solid.hash(), solid_of(&solid), test.context().deflection())
            .unwrap();
        assert_eq!(hash, cached.hash());
        let as_mesh_value = HashedValue::new(ValueData::Mesh(cached.mesh().clone())).unwrap();
        assert_eq!(hash, as_mesh_value.hash(), "content-addressed like a Mesh");
        let Frame::Instances { instances, .. } = decode(&out.frames[1]).unwrap() else {
            panic!("instances")
        };
        assert_eq!(
            instances
                .iter()
                .map(|i| i.element_index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(summary.facts["faces"], 12);
        assert_eq!(summary.facts["triangles"], 24);
        // The two elements share one value: one miss, then hits.
        let stats = test.solids.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
    }

    /// The review's protocol finding: the client caches blobs by hash
    /// forever, so a blob's hash must BE its content. A curved solid drawn
    /// at the preview tier and again at the fine tier is two meshes → two
    /// blob hashes; a box's mesh is the same at any deflection → one.
    #[test]
    fn two_deflections_of_one_solid_are_two_blobs() {
        let test = TestContext::new();
        let round = cylinder();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(round.clone()), Some(round.clone())],
        }))
        .unwrap();
        let mut picks = PickTable::default();
        let blob_hash = |out: &DisplayFrames| {
            let Frame::MeshBlob { hash, .. } = decode(&out.frames[0]).unwrap() else {
                panic!("blob")
            };
            hash
        };
        let preview = frames_for_value(
            &list,
            1,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.at(DisplayTier::Preview),
        );
        let fine = frames_for_value(
            &list,
            2,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.at(DisplayTier::Fine),
        );
        assert_eq!(preview.stats.tier, Some(DisplayTier::Preview));
        assert_eq!(fine.stats.tier, Some(DisplayTier::Fine));
        assert!(
            preview.stats.triangles < fine.stats.triangles,
            "preview {} vs fine {} triangles",
            preview.stats.triangles,
            fine.stats.triangles
        );
        assert_ne!(
            blob_hash(&preview),
            blob_hash(&fine),
            "two deflections, two blobs"
        );
        assert_eq!(test.solids.stats().entries, 2, "two keys in the cache");
        // A second fine pass is the same blob: content-addressed both ways.
        let fine_again = frames_for_value(
            &list,
            3,
            1,
            0,
            &mut |e| picks.ids_for(1, 0, e),
            &test.at(DisplayTier::Fine),
        );
        assert_eq!(blob_hash(&fine), blob_hash(&fine_again));
        // The box: deflection-independent mesh, one blob hash at both tiers.
        let flat = probe_box();
        let boxes = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(flat.clone()), Some(flat)],
        }))
        .unwrap();
        let a = frames_for_value(
            &boxes,
            4,
            2,
            0,
            &mut |e| picks.ids_for(2, 0, e),
            &test.at(DisplayTier::Preview),
        );
        let b = frames_for_value(
            &boxes,
            5,
            2,
            0,
            &mut |e| picks.ids_for(2, 0, e),
            &test.at(DisplayTier::Fine),
        );
        assert_eq!(blob_hash(&a), blob_hash(&b));
        // The summary reads whatever tier is cached, the fine one first.
        let before = test.solids.stats();
        let summary = summarize(&round, &test.context());
        assert_eq!(summary.facts["triangles"], fine.stats.triangles / 2);
        assert_eq!(test.solids.stats().misses, before.misses, "no kernel call");
    }

    /// The display deflection's relative term: a solid 4 m long is meshed
    /// at 1/1000 of its extent (4 mm), not at 0.02 mm — the deflection the
    /// cache entry records says so.
    #[test]
    fn giant_solids_are_meshed_at_the_relative_deflection() {
        let test = TestContext::new();
        let bar = solids::box_at(
            Point::origin(),
            cicada_core::spatial::Vector::new(4000.0, 40.0, 10.0),
        )
        .unwrap();
        let value = HashedValue::new(ValueData::Solid(bar.clone())).unwrap();
        let cached = test
            .solids
            .tessellation(value.hash(), &bar, test.context().deflection())
            .unwrap();
        assert!((cached.deflection.linear() - 4.0).abs() < 1e-12);
        assert!((cached.deflection.angular() - 0.1).abs() < 1e-12);
        // The probe box is 30 long: 0.03, just above the physical floor.
        let medium = probe_box();
        let cached = test
            .solids
            .tessellation(
                medium.hash(),
                solid_of(&medium),
                test.context().deflection(),
            )
            .unwrap();
        assert!((cached.deflection.linear() - 0.03).abs() < 1e-12);
        // A small part keeps the physical floor (0.02 mm in a mm document).
        let small = solids::box_at(
            Point::origin(),
            cicada_core::spatial::Vector::new(5.0, 5.0, 5.0),
        )
        .unwrap();
        let value = HashedValue::new(ValueData::Solid(small.clone())).unwrap();
        let cached = test
            .solids
            .tessellation(value.hash(), &small, test.context().deflection())
            .unwrap();
        assert!((cached.deflection.linear() - 0.02).abs() < 1e-12);
        // And the preview tier's floor is 0.1 mm / 0.3 rad.
        let cached = test
            .solids
            .tessellation(
                value.hash(),
                &small,
                test.at(DisplayTier::Preview).deflection(),
            )
            .unwrap();
        assert!((cached.deflection.linear() - 0.1).abs() < 1e-12);
        assert!((cached.deflection.angular() - 0.3).abs() < 1e-12);
    }

    /// An open display mesh — the tetrahedron minus one face — inserted
    /// under a solid's key: the display path draws it (a green Solid never
    /// vanishes), the stats warn, the summary says `watertight: false`.
    #[test]
    fn an_unclosed_mesh_still_draws_with_a_warning() {
        let test = TestContext::new();
        let solid = probe_box();
        let context = test.context();
        let open = Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 0, 3, 2],
        )
        .unwrap();
        assert!(!open.is_watertight());
        test.solids.insert(
            TessellationKey::new(solid.hash(), context.deflection()),
            Cached::Mesh(Arc::new(
                DisplayMesh::new(solids::DisplayTessellation {
                    mesh: open,
                    watertight: false,
                    faces: 6,
                    deflection: context.deflection(),
                })
                .unwrap(),
            )),
        );
        let out = frames_for_value(
            &solid,
            1,
            3,
            0,
            &mut |e| PickTable::default().ids_for(3, 0, e),
            &context,
        );
        assert_eq!(out.stats.kinds, vec!["mesh"], "drawn");
        assert_eq!(out.stats.solids, 1);
        assert_eq!(out.stats.triangles, 3);
        assert!(out.stats.errors.is_empty());
        assert_eq!(
            out.stats.warnings,
            vec![
                "element 0 (Solid): the kernel's mesh does not close at this deflection; drawn \
                 as is"
            ]
        );
        let summary = summarize(&solid, &context);
        assert_eq!(summary.facts["watertight"], false);
        assert_eq!(summary.facts["triangles"], 3);
        assert!(!summary.facts.contains_key("error"));
        // In a list the caveat is a count.
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(solid.clone()), Some(solid)],
        }))
        .unwrap();
        let out = frames_for_value(
            &list,
            2,
            3,
            0,
            &mut |e| PickTable::default().ids_for(3, 0, e),
            &context,
        );
        assert_eq!(out.stats.warnings.len(), 2);
        assert_eq!(out.stats.instanced, 2);
        let summary = summarize(&list, &context);
        assert_eq!(summary.facts["unclosed"], 2);
        assert_eq!(test.solids.stats().misses, 0, "every read was a hit");
    }

    #[test]
    fn refusals_are_cached_as_negative_entries() {
        // Core accepts the header alone; the kernel does not. The refusal
        // is on record, drawn as a clear frame — and cached, so the next
        // pass over the same bytes does not re-pay the kernel call.
        let test = TestContext::new();
        let pseudo = HashedValue::new(ValueData::Solid(
            Solid::from_canonical_bytes(cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec())
                .unwrap(),
        ))
        .unwrap();
        let out = frames_for_value(
            &pseudo,
            1,
            1,
            0,
            &mut |e| PickTable::default().ids_for(1, 0, e),
            &test.context(),
        );
        assert_eq!(out.stats.kinds, vec!["clear"]);
        assert_eq!(out.stats.errors.len(), 1);
        assert!(out.stats.errors[0].starts_with("element 0 (Solid): "));
        assert!(out.stats.warnings.is_empty());
        let summary = summarize(&pseudo, &test.context());
        let error = summary.facts["error"].as_str().unwrap().to_owned();
        assert!(error.contains("OCCT"), "{error}");
        let stats = test.solids.stats();
        assert_eq!(
            (stats.misses, stats.hits),
            (1, 1),
            "the summary hit the refusal"
        );
        assert_eq!((stats.entries, stats.refusals), (1, 1));
        assert_eq!(stats.bytes, error.len(), "a refusal counts its text");
        // Drawn again: a hit, the same text.
        let again = frames_for_value(
            &pseudo,
            2,
            1,
            0,
            &mut |e| PickTable::default().ids_for(1, 0, e),
            &test.context(),
        );
        assert_eq!(again.stats.errors, out.stats.errors);
        assert_eq!(test.solids.stats().misses, 1);
        // Evicted like any entry: a budget too small for the text keeps
        // nothing (oversized), and the refusal is re-derived each time.
        let tiny = TestContext::with_budget(8);
        let _ = frames_for_value(
            &pseudo,
            1,
            1,
            0,
            &mut |e| PickTable::default().ids_for(1, 0, e),
            &tiny.context(),
        );
        let _ = frames_for_value(
            &pseudo,
            2,
            1,
            0,
            &mut |e| PickTable::default().ids_for(1, 0, e),
            &tiny.context(),
        );
        let stats = tiny.solids.stats();
        assert_eq!((stats.entries, stats.refusals, stats.oversized), (0, 0, 2));
        assert_eq!(stats.misses, 2);
    }

    #[test]
    fn distinct_solids_dedups_by_hash_through_nested_lists() {
        let solid = probe_box();
        let round = cylinder();
        let inner = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(round.clone()), Some(solid.clone()), None],
        }))
        .unwrap();
        let outer = HashedValue::new(ValueData::List(List {
            axis: Some(Arc::from("part")),
            slots: vec![Some(solid.clone()), Some(inner), Some(point(1.0))],
        }))
        .unwrap();
        let distinct = distinct_solids(&[outer, solid.clone(), point(2.0)]);
        let hashes: Vec<ValueHash> = distinct.iter().map(|(hash, _)| *hash).collect();
        let mut expected = vec![solid.hash(), round.hash()];
        expected.sort();
        assert_eq!(hashes, expected, "each solid once, in hash order");
        assert_eq!(
            distinct[0].1.bytes().len() + distinct[1].1.bytes().len(),
            solid_of(&solid).bytes().len() + solid_of(&round).bytes().len()
        );
        assert!(distinct_solids(&[point(0.0)]).is_empty());
    }

    #[test]
    fn the_cache_evicts_least_recently_used_within_its_budget() {
        let solid = probe_box();
        let bytes = solid_of(&solid);
        // A cube's display mesh is 8 vertices + 12 triangles = 336 bytes.
        let test = TestContext::with_budget(700);
        let context = test.context();
        let deflection = context.deflection();
        let coarser = Deflection::new(deflection.linear() * 2.0, deflection.angular()).unwrap();
        let finer = Deflection::new(deflection.linear() / 2.0, deflection.angular()).unwrap();
        // Three distinct keys (one value, three deflections) at 336 B each
        // against a 700 B budget: the third insert evicts the oldest.
        test.solids
            .tessellation(solid.hash(), bytes, deflection)
            .unwrap();
        test.solids
            .tessellation(solid.hash(), bytes, coarser)
            .unwrap();
        assert_eq!(test.solids.stats().entries, 2);
        // Touch the first so the SECOND is the least recently used.
        test.solids
            .tessellation(solid.hash(), bytes, deflection)
            .unwrap();
        test.solids
            .tessellation(solid.hash(), bytes, finer)
            .unwrap();
        let stats = test.solids.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.evictions, 1);
        assert_eq!(stats.bytes, 2 * 336);
        assert_eq!(stats.budget, 700);
        // The first key survived (hit), the second was evicted (miss).
        let before = test.solids.stats();
        test.solids
            .tessellation(solid.hash(), bytes, deflection)
            .unwrap();
        assert_eq!(test.solids.stats().hits, before.hits + 1);
        test.solids
            .tessellation(solid.hash(), bytes, coarser)
            .unwrap();
        assert_eq!(test.solids.stats().misses, before.misses + 1);
    }

    /// A synthetic display mesh the tests can key and size without the
    /// kernel: a tetrahedron (4 vertices × 24 B + 4 triangles × 12 B =
    /// 144 B as the cache counts it) with `faces` set to `tag` so entries
    /// are distinguishable.
    fn synthetic(tag: usize) -> Cached {
        let mesh = Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
        )
        .unwrap();
        assert!(mesh.is_watertight());
        assert_eq!(mesh_bytes(&mesh), 144);
        Cached::Mesh(Arc::new(
            DisplayMesh::new(solids::DisplayTessellation {
                mesh,
                watertight: true,
                faces: tag,
                deflection: Deflection::new(0.02, 0.1).unwrap(),
            })
            .unwrap(),
        ))
    }

    fn faces_of(cached: &Cached) -> usize {
        match cached {
            Cached::Mesh(mesh) => mesh.faces,
            Cached::Refused(reason) => panic!("a refusal: {reason}"),
        }
    }

    fn synthetic_key(tag: u64) -> TessellationKey {
        let hash = HashedValue::new(ValueData::Integer(i64::try_from(tag).unwrap()))
            .unwrap()
            .hash();
        TessellationKey::new(hash, Deflection::new(0.02, 0.1).unwrap())
    }

    #[test]
    fn recency_order_holds_at_scale() {
        // The recency index replaced a linear scan; this is its contract at
        // a size where the scan would have mattered (the wall's part count
        // and then some): 2,000 entries in, every even one re-touched, then
        // enough new entries to evict half — exactly the untouched (odd)
        // ones go, in insertion order, and every even one still hits.
        const N: u64 = 2_000;
        let cache = SolidCache::new(usize::try_from(N).unwrap() * 144);
        for tag in 0..N {
            cache.insert(synthetic_key(tag), synthetic(usize::try_from(tag).unwrap()));
        }
        assert_eq!(cache.stats().entries, usize::try_from(N).unwrap());
        assert_eq!(cache.stats().bytes, usize::try_from(N).unwrap() * 144);
        for tag in (0..N).step_by(2) {
            let found = cache.lookup(synthetic_key(tag)).expect("present");
            assert_eq!(faces_of(&found), usize::try_from(tag).unwrap());
        }
        // N/2 new entries: the budget is full, so N/2 evictions, the least
        // recently used first — the odd tags, never an even one.
        for tag in N..N + N / 2 {
            cache.insert(synthetic_key(tag), synthetic(usize::try_from(tag).unwrap()));
        }
        let stats = cache.stats();
        assert_eq!(stats.entries, usize::try_from(N).unwrap());
        assert_eq!(stats.bytes, stats.budget, "exactly full, never over");
        assert_eq!(stats.evictions, N / 2);
        for tag in 0..N {
            let present = cache.lookup(synthetic_key(tag)).is_some();
            assert_eq!(
                present,
                tag % 2 == 0,
                "tag {tag}: touched entries survive, untouched ones were evicted"
            );
        }
        for tag in N..N + N / 2 {
            assert!(cache.lookup(synthetic_key(tag)).is_some());
        }
        // The next eviction round takes the oldest SURVIVORS in touch
        // order: the even tags were touched in ascending order, so tag 0
        // goes first.
        cache.insert(synthetic_key(N * 3), synthetic(3));
        assert!(
            cache.lookup(synthetic_key(0)).is_none(),
            "tag 0 was the LRU"
        );
        assert!(cache.lookup(synthetic_key(2)).is_some());
    }

    /// A second synthetic size: an octahedron (6 vertices × 24 B + 8
    /// triangles × 12 B = 240 B).
    fn synthetic_octahedron() -> Cached {
        let mesh = Mesh::new(
            vec![
                1.0, 0.0, 0.0, //
                -1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, -1.0, 0.0, //
                0.0, 0.0, 1.0, //
                0.0, 0.0, -1.0,
            ],
            vec![
                0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, 4, //
                2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, 5,
            ],
        )
        .unwrap();
        assert!(mesh.is_watertight());
        assert_eq!(mesh_bytes(&mesh), 240);
        Cached::Mesh(Arc::new(
            DisplayMesh::new(solids::DisplayTessellation {
                mesh,
                watertight: true,
                faces: 8,
                deflection: Deflection::new(0.02, 0.1).unwrap(),
            })
            .unwrap(),
        ))
    }

    #[test]
    fn an_entry_larger_than_the_budget_is_served_but_never_kept() {
        // Budget 200 B: the 144 B tetrahedron fits; the 240 B octahedron
        // never can. Keeping it anyway would have evicted the tetrahedron
        // for an entry that still left the cache over budget — so it is
        // counted (`oversized`), not kept, and the tetrahedron survives.
        // `bytes` never exceeds `budget`.
        let cache = SolidCache::new(200);
        cache.insert(synthetic_key(1), synthetic(1));
        assert_eq!(cache.stats().entries, 1);
        cache.insert(synthetic_key(2), synthetic_octahedron());
        let stats = cache.stats();
        assert_eq!(stats.entries, 1, "the oversized entry was not kept");
        assert_eq!(stats.bytes, 144);
        assert_eq!(stats.oversized, 1);
        assert_eq!(stats.evictions, 0, "nothing was thrown out to make room");
        assert!(stats.bytes <= stats.budget);
        assert!(cache.lookup(synthetic_key(1)).is_some());
        assert!(cache.lookup(synthetic_key(2)).is_none());
        // Exactly the budget fits.
        let exact = SolidCache::new(240);
        exact.insert(synthetic_key(2), synthetic_octahedron());
        assert_eq!(exact.stats().entries, 1);
        assert_eq!(exact.stats().oversized, 0);
        // A refusal evicts and is evicted like a mesh: 144 + 10 > 150.
        let mixed = SolidCache::new(150);
        mixed.insert(synthetic_key(1), synthetic(1));
        mixed.insert(synthetic_key(2), Cached::Refused(Arc::from("0123456789")));
        let stats = mixed.stats();
        assert_eq!((stats.entries, stats.refusals, stats.evictions), (1, 1, 1));
        assert_eq!(stats.bytes, 10);
        mixed.insert(synthetic_key(3), synthetic(3));
        let stats = mixed.stats();
        assert_eq!((stats.entries, stats.refusals, stats.evictions), (1, 0, 2));
    }

    /// The cache resizes live (`set_display_cache`, docs/13): shrinking
    /// evicts least-recently-used entries AT ONCE until the held bytes fit,
    /// growing evicts nothing — and evictions of WATCHED entries (what the
    /// previous complete generation displayed) are counted apart, which is
    /// the `thrash` flag (docs/12 §Display). Watching TOUCHES: the picture
    /// on screen is the newest thing in the cache, so a shrink (or a
    /// pass's insertions) evicts unwatched entries first — the thrash
    /// count moves only when the picture itself has to go.
    #[test]
    fn the_cache_resizes_live_and_counts_watched_evictions() {
        let cache = SolidCache::new(700);
        for tag in 1_usize..=3 {
            cache.insert(synthetic_key(tag as u64), synthetic(tag));
        }
        assert_eq!((cache.stats().entries, cache.stats().bytes), (3, 432));
        assert_eq!(cache.budget(), 700);
        // Watch 1 and 2 — "the previous picture": touched, so tag 3 — the
        // newest insert, but unwatched — is now the least recently used.
        cache.watch([synthetic_key(1), synthetic_key(2)]);
        assert_eq!(cache.watched_evictions(), 0);
        // Shrink to 300: 432 > 300 → the unwatched tag 3 goes; 288 fits.
        // One eviction, none of the picture.
        assert_eq!(cache.set_budget(300), 1);
        let stats = cache.stats();
        assert_eq!((stats.entries, stats.bytes, stats.budget), (2, 288, 300));
        assert_eq!(stats.evictions, 1);
        assert_eq!(cache.watched_evictions(), 0);
        assert!(!cache.contains(synthetic_key(3)));
        assert!(cache.contains(synthetic_key(1)));
        assert!(cache.contains(synthetic_key(2)));
        // Shrink to 100: both remaining go — both watched: the picture
        // itself had to leave.
        assert_eq!(cache.set_budget(100), 2);
        assert_eq!(cache.stats().bytes, 0);
        assert_eq!(cache.watched_evictions(), 2);
        // A fresh watch resets the count; growing evicts nothing and the
        // stats say so; an entry larger than the new budget is oversized.
        cache.watch([]);
        assert_eq!(cache.watched_evictions(), 0);
        assert_eq!(cache.set_budget(1000), 0);
        assert_eq!(cache.stats().budget, 1000);
        cache.insert(synthetic_key(4), synthetic(4));
        assert!(cache.contains(synthetic_key(4)));
        assert_eq!(cache.set_budget(100), 1);
        cache.insert(synthetic_key(5), synthetic(5));
        assert_eq!(cache.stats().oversized, 1, "144 B never fits 100 B");
        // An insert's own eviction takes the unwatched entry first (tag 2,
        // inserted after tag 1 but not on screen), and only when nothing
        // unwatched is left does the picture go — a watched eviction, the
        // honest thrash: the previous picture and this one do not fit
        // together. (Under plain recency tag 1, never looked up once
        // displayed, left first and a fitting picture read as thrash.)
        let lru = SolidCache::new(300);
        lru.insert(synthetic_key(1), synthetic(1));
        lru.insert(synthetic_key(2), synthetic(2));
        lru.watch([synthetic_key(1)]);
        lru.insert(synthetic_key(3), synthetic(3));
        assert_eq!((lru.stats().evictions, lru.watched_evictions()), (1, 0));
        assert!(
            lru.contains(synthetic_key(1)),
            "the picture outlives the garbage"
        );
        assert!(!lru.contains(synthetic_key(2)));
        lru.insert(synthetic_key(4), synthetic(4));
        assert_eq!((lru.stats().evictions, lru.watched_evictions()), (2, 1));
        assert!(
            !lru.contains(synthetic_key(1)),
            "now the picture itself had to go"
        );
        // Watching a key the cache does not hold touches nothing and counts
        // nothing; re-watching resets the count.
        lru.watch([synthetic_key(99), synthetic_key(4)]);
        assert_eq!(lru.watched_evictions(), 0);
        assert_eq!(lru.stats().entries, 2);
    }

    /// The triangle budget's verdict (docs/12 §Display; the D1 contract) is
    /// a pure function of the value set, the requested tier and the limit:
    /// fine when the distinct solids' fine triangles fit, preview when they
    /// do not, preview + `over_budget` when even the preview triangles do
    /// not; the same for either order of the solids; a preview request
    /// never meshes anything fine; no solids → as asked.
    #[test]
    #[allow(clippy::too_many_lines)] // one verdict table: fits, dropped, over, asked, reversed, none, fetched
    fn the_triangle_budget_chooses_the_tier_from_the_value_set() {
        let test = TestContext::new();
        let config = &test.config;
        let solids = distinct_solids(&[cylinder(), probe_box()]);
        assert_eq!(solids.len(), 2);
        let map = serial_map;
        let count = |tier: DisplayTier| -> u64 {
            solids
                .iter()
                .map(|(hash, solid)| {
                    test.solids
                        .tessellation(*hash, solid, tier.deflection(config))
                        .unwrap()
                        .mesh()
                        .triangle_count() as u64
                })
                .sum()
        };
        let fine = count(DisplayTier::Fine);
        let preview = count(DisplayTier::Preview);
        assert!(preview < fine, "preview {preview} vs fine {fine}");
        assert!(
            preview > 12,
            "the cylinder's preview mesh is more than the cube"
        );
        let choose = |requested, limit| {
            choose_tier(&solids, requested, limit, config, &test.solids, &map).stats
        };
        // Fits: fine, exact — and the fine meshes come back for the pin.
        let fits = choose_tier(&solids, DisplayTier::Fine, fine, config, &test.solids, &map);
        assert_eq!(
            fits.stats,
            BudgetStats {
                limit: fine,
                requested: DisplayTier::Fine,
                drawn: DisplayTier::Fine,
                triangles: fine,
                over_budget: false,
            }
        );
        assert_eq!(fits.meshes.len(), 2);
        // The decision reports its lookups on a WARM cache too (the profiler's
        // per-output counters): `count` meshed both tiers above, so the fine
        // tally is two hits and no kernel call — a decision that dropped its
        // hit tally would show `0 / 0` for every value sharing a cached body
        // (review finding L2-P1-2: nothing asserted a warm decision's hits).
        assert_eq!((fits.hits, fits.misses), (2, 0));
        let fine_deflection = DisplayTier::Fine.deflection(config);
        for (hash, mesh) in &fits.meshes {
            let body = solids.iter().find(|(h, _)| h == hash).unwrap();
            let cached = test
                .solids
                .tessellation(*hash, &body.1, fine_deflection)
                .unwrap();
            assert!(Arc::ptr_eq(mesh, &cached), "the pinned mesh IS the cache's");
        }
        // One short of the fine total: preview, exact, within budget — the
        // PREVIEW meshes come back, never the fine ones the tally paid for.
        let dropped = choose_tier(
            &solids,
            DisplayTier::Fine,
            fine - 1,
            config,
            &test.solids,
            &map,
        );
        assert_eq!(
            dropped.stats,
            BudgetStats {
                limit: fine - 1,
                requested: DisplayTier::Fine,
                drawn: DisplayTier::Preview,
                triangles: preview,
                over_budget: false,
            }
        );
        assert_eq!(dropped.meshes.len(), 2);
        assert_eq!(
            dropped
                .meshes
                .iter()
                .map(|(_, m)| m.mesh().triangle_count() as u64)
                .sum::<u64>(),
            preview
        );
        // Both tallies' lookups are counted: the fine one (two hits, past the
        // limit only at its second solid) and the preview one (two hits).
        assert_eq!((dropped.hits, dropped.misses), (4, 0));
        // Below even the preview total: preview anyway, and said so.
        let over = choose(DisplayTier::Fine, preview - 1);
        assert_eq!(over.drawn, DisplayTier::Preview);
        assert_eq!(over.triangles, preview);
        assert!(over.over_budget);
        // A preview request is drawn at preview whatever the limit.
        let asked = choose(DisplayTier::Preview, fine);
        assert_eq!(
            (asked.drawn, asked.triangles),
            (DisplayTier::Preview, preview)
        );
        assert!(!asked.over_budget);
        // Order-independent: the reversed set gets the same verdicts.
        let mut reversed = solids.clone();
        reversed.reverse();
        for (requested, limit) in [
            (DisplayTier::Fine, fine),
            (DisplayTier::Fine, fine - 1),
            (DisplayTier::Fine, preview - 1),
            (DisplayTier::Preview, fine),
        ] {
            assert_eq!(
                choose_tier(&reversed, requested, limit, config, &test.solids, &map).stats,
                choose(requested, limit),
                "{requested:?} at {limit}"
            );
        }
        // No solids: as asked, nothing measured, nothing to pin.
        let none = choose_tier(&[], DisplayTier::Fine, 1, config, &test.solids, &map);
        assert_eq!(
            (
                none.stats.drawn,
                none.stats.triangles,
                none.stats.over_budget
            ),
            (DisplayTier::Fine, 0, false)
        );
        assert!(none.meshes.is_empty());
        // A preview request on a fresh cache meshes the two solids at
        // preview and nothing at fine: two kernel calls, not four.
        let fresh = TestContext::new();
        let verdict = choose_tier(
            &solids,
            DisplayTier::Preview,
            u64::MAX,
            &fresh.config,
            &fresh.solids,
            &map,
        );
        assert_eq!(verdict.stats.drawn, DisplayTier::Preview);
        assert_eq!(fresh.solids.stats().misses, 2);
        // The decision reports its own lookups (the profiler's per-output
        // counters): two kernel calls, no hit.
        assert_eq!((verdict.hits, verdict.misses), (0, 2));
        // `fetch_meshes` at that tier is two hits — what a pass does for an
        // output whose verdict the memo knew; after an eviction it is the
        // kernel again, on the pool, never under the session lock.
        let again = fetch_meshes(
            &solids,
            DisplayTier::Preview,
            &fresh.config,
            &fresh.solids,
            &map,
        );
        assert_eq!(again.meshes.len(), 2);
        assert_eq!((again.hits, again.misses), (2, 0));
        assert_eq!(
            (fresh.solids.stats().hits, fresh.solids.stats().misses),
            (2, 2)
        );
        fresh.solids.set_budget(1);
        assert_eq!(fresh.solids.stats().entries, 0);
        let refetched = fetch_meshes(
            &solids,
            DisplayTier::Preview,
            &fresh.config,
            &fresh.solids,
            &map,
        );
        assert_eq!(refetched.meshes.len(), 2, "served although too big to keep");
        assert_eq!((refetched.hits, refetched.misses), (0, 2));
        assert_eq!(fresh.solids.stats().misses, 4);
    }

    /// The encode draws a pinned mesh without asking the cache: a pass that
    /// hands `frames_for_value` the meshes its warm-up fetched tessellates
    /// nothing under the session lock — even on a cache that has since
    /// dropped every entry (the cascade of a working set larger than the
    /// budget; review finding 2026-08-25). Without the pin the same call
    /// misses and calls the kernel.
    #[test]
    fn a_pinned_mesh_is_drawn_without_the_cache() {
        let test = TestContext::new();
        let solid = probe_box();
        let body = solid_of(&solid);
        let tier = DisplayTier::Fine;
        let deflection = tier.deflection(&test.config);
        let mesh = test
            .solids
            .tessellation(solid.hash(), body, deflection)
            .unwrap();
        assert_eq!(test.solids.stats().misses, 1);
        // Everything leaves the cache; the pass still holds the mesh.
        test.solids.set_budget(1);
        assert_eq!(test.solids.stats().entries, 0);
        let mut pinned = PinnedMeshes::new();
        pinned.insert(
            TessellationKey::new(solid.hash(), deflection),
            Arc::clone(&mesh),
        );
        let context = DisplayContext {
            pinned: Some(&pinned),
            ..test.at(tier)
        };
        let mut picks = PickTable::default();
        let out = frames_for_value(
            &solid,
            1,
            0,
            0,
            &mut |e: &[u32]| picks.ids_for(0, 0, e),
            &context,
        );
        assert_eq!(out.stats.solids, 1);
        assert_eq!(out.stats.triangles, mesh.mesh().triangle_count());
        assert_eq!(
            (test.solids.stats().hits, test.solids.stats().misses),
            (0, 1),
            "the cache was not asked"
        );
        // The same call without the pin goes to the kernel (a miss: the
        // entry is gone), so the pin is what keeps the encode off it.
        let plain = frames_for_value(
            &solid,
            1,
            0,
            0,
            &mut |e: &[u32]| picks.ids_for(0, 0, e),
            &test.at(tier),
        );
        assert_eq!(plain.stats.solids, 1);
        assert_eq!(test.solids.stats().misses, 2);
        // A pin at another tier is not this tier's: the cache is asked.
        let other = DisplayContext {
            pinned: Some(&pinned),
            ..test.at(DisplayTier::Preview)
        };
        let _ = frames_for_value(
            &solid,
            1,
            0,
            0,
            &mut |e: &[u32]| picks.ids_for(0, 0, e),
            &other,
        );
        assert_eq!(test.solids.stats().misses, 3);
    }

    /// The fine tally stops at the budget: once the running total is past
    /// the limit the remaining solids are not meshed fine (the verdict is
    /// certain), so the work wasted before a "preview" verdict is bounded —
    /// the preview pass then meshes them all. The serial map makes the
    /// order the input's: the cylinder's fine mesh alone is past a limit of
    /// 1, so the cube is never meshed fine.
    #[test]
    fn the_fine_tally_stops_at_the_budget() {
        let test = TestContext::new();
        let cylinder = cylinder();
        let cube = probe_box();
        let solids = vec![
            (cylinder.hash(), solid_of(&cylinder).clone()),
            (cube.hash(), solid_of(&cube).clone()),
        ];
        let map = serial_map;
        let verdict = choose_tier(
            &solids,
            DisplayTier::Fine,
            1,
            &test.config,
            &test.solids,
            &map,
        );
        let verdict = verdict.stats;
        assert_eq!(verdict.drawn, DisplayTier::Preview);
        assert!(
            verdict.over_budget,
            "a limit of 1 triangle is below any preview"
        );
        // Fine: the cylinder only (the cube was skipped); preview: both.
        let fine = DisplayTier::Fine.deflection(&test.config);
        let preview = DisplayTier::Preview.deflection(&test.config);
        assert!(
            test.solids
                .contains(TessellationKey::new(cylinder.hash(), fine))
        );
        assert!(
            !test
                .solids
                .contains(TessellationKey::new(cube.hash(), fine)),
            "the tally stopped before the cube"
        );
        assert!(
            test.solids
                .contains(TessellationKey::new(cylinder.hash(), preview))
        );
        assert!(
            test.solids
                .contains(TessellationKey::new(cube.hash(), preview))
        );
        assert_eq!(test.solids.stats().misses, 3, "one fine, two preview");
    }

    /// A live emission records the budget's verdict and the output's share
    /// of the cache's working set — the distinct solids' display-mesh bytes
    /// at the drawn tier; a restream (no verdict) records neither.
    #[test]
    fn frames_carry_the_budget_verdict_and_the_working_set() {
        let test = TestContext::new();
        let solid = probe_box();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(solid.clone()), Some(solid.clone()), Some(cylinder())],
        }))
        .unwrap();
        let verdict = BudgetStats {
            limit: 1000,
            requested: DisplayTier::Fine,
            drawn: DisplayTier::Preview,
            triangles: 40,
            over_budget: false,
        };
        let mut picks = PickTable::default();
        let context = DisplayContext {
            budget: Some(verdict),
            ..test.at(DisplayTier::Preview)
        };
        let frames = frames_for_value(
            &list,
            3,
            1,
            0,
            &mut |e: &[u32]| picks.ids_for(1, 0, e),
            &context,
        );
        assert_eq!(frames.stats.budget, Some(verdict));
        assert_eq!(frames.stats.tier, Some(DisplayTier::Preview));
        // Two DISTINCT solids (the cube twice is one), each sized as the cache counts it.
        assert_eq!(frames.solids.len(), 2);
        let preview = DisplayTier::Preview.deflection(&test.config);
        let round = cylinder();
        for (hash, bytes) in &frames.solids {
            let body = if *hash == solid.hash() {
                solid_of(&solid)
            } else {
                solid_of(&round)
            };
            let mesh = test.solids.tessellation(*hash, body, preview).unwrap();
            assert_eq!(*bytes, mesh_bytes(mesh.mesh()));
        }
        let json = serde_json::to_value(&frames.stats).unwrap();
        assert_eq!(
            json["budget"],
            serde_json::json!({"limit": 1000, "requested": "fine", "drawn": "preview", "triangles": 40})
        );
        // Without a verdict (a restream), nothing is claimed.
        let plain = frames_for_value(
            &list,
            3,
            1,
            0,
            &mut |e: &[u32]| picks.ids_for(1, 0, e),
            &test.at(DisplayTier::Preview),
        );
        assert!(plain.stats.budget.is_none());
        assert!(
            serde_json::to_value(&plain.stats)
                .unwrap()
                .get("budget")
                .is_none()
        );
        // A mesh value holds no solids and no verdict.
        let mesh = HashedValue::new(ValueData::Mesh(tetra(0.0))).unwrap();
        let meshed = frames_for_value(
            &mesh,
            3,
            2,
            0,
            &mut |e: &[u32]| picks.ids_for(2, 0, e),
            &context,
        );
        assert!(meshed.solids.is_empty());
        assert!(meshed.stats.budget.is_none());
    }
}
