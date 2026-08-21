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

use std::collections::{BTreeMap, HashMap};
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
/// buffers, as uploaded). 256 MiB holds the wall's part count many times
/// over at display deflection; a budget, not a correctness boundary —
/// eviction only costs a re-tessellation.
pub const SOLID_CACHE_BUDGET: usize = 256 * 1024 * 1024;

/// Which deflection a display pass tessellates solids at (docs/03 §Display
/// tessellation): `Preview` for the generations of a slider drag — coarse,
/// what a drag can afford — and `Fine` for structural generations, the
/// release, a joining client and the inspector. Ordered: a fine drawing
/// satisfies a preview request, never the reverse, so an output drawn at
/// `Preview` is redrawn by the next `Fine` generation of the same value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
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
/// display deflection), the solid tessellation cache, and the tier this
/// pass draws at.
#[derive(Clone, Copy)]
pub struct DisplayContext<'a> {
    /// The project's configuration.
    pub config: &'a ProjectConfig,
    /// The session's tessellation cache.
    pub solids: &'a SolidCache,
    /// The tier of this pass.
    pub tier: DisplayTier,
}

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
/// function of the solid, so it needs no place in the key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TessellationKey {
    hash: ValueHash,
    linear_bits: u64,
    angular_bits: u64,
}

impl TessellationKey {
    fn new(hash: ValueHash, deflection: Deflection) -> Self {
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
}

impl CacheState {
    fn stamp(&mut self) -> u64 {
        self.clock += 1;
        self.clock
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
/// budget cannot hold anyway.
pub struct SolidCache {
    state: std::sync::Mutex<CacheState>,
    budget: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    oversized: AtomicU64,
}

/// The cache's counters, as `/debug/state` → `display_cache` reports them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
            }),
            budget,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            oversized: AtomicU64::new(0),
        }
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
        let key = TessellationKey::new(hash, deflection);
        if let Some(found) = self.lookup(key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return match found {
                Cached::Mesh(mesh) => Ok(mesh),
                Cached::Refused(reason) => Err(reason.to_string()),
            };
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let result = solids::tessellate_display(solid, deflection)
            .map_err(|error| error.to_string())
            .and_then(DisplayMesh::new);
        match result {
            Ok(mesh) => {
                let mesh = Arc::new(mesh);
                self.insert(key, Cached::Mesh(Arc::clone(&mesh)));
                Ok(mesh)
            }
            Err(reason) => {
                self.insert(key, Cached::Refused(Arc::from(reason.as_str())));
                Err(reason)
            }
        }
    }

    /// The display mesh a SUMMARY should read: whatever is cached for this
    /// solid at either tier (the fine one preferred), or the fine one
    /// computed — so an inspector read during a drag costs a lookup, not a
    /// fine tessellation under the session lock.
    ///
    /// # Errors
    ///
    /// As [`SolidCache::tessellation`].
    pub fn tessellation_for_summary(
        &self,
        hash: ValueHash,
        solid: &Solid,
        config: &ProjectConfig,
    ) -> Result<Arc<DisplayMesh>, String> {
        for tier in [DisplayTier::Fine, DisplayTier::Preview] {
            let key = TessellationKey::new(hash, tier.deflection(config));
            if let Some(found) = self.lookup(key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return match found {
                    Cached::Mesh(mesh) => Ok(mesh),
                    Cached::Refused(reason) => Err(reason.to_string()),
                };
            }
        }
        self.tessellation(hash, solid, DisplayTier::Fine.deflection(config))
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
        if size > self.budget {
            // Nothing in the cache could make room for this; evicting
            // everything for an entry that still does not fit would only
            // cost the other solids their hits.
            self.oversized.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A concurrent miss on the same key may have inserted first: keep
        // the one that is there (identical content), count nothing twice.
        if state.entries.contains_key(&key) {
            return;
        }
        while state.bytes + size > self.budget {
            let Some((_, oldest)) = state.recency.pop_first() else {
                break;
            };
            if let Some(evicted) = state.entries.remove(&oldest) {
                state.bytes -= evicted.size;
                if matches!(evicted.cached, Cached::Refused(_)) {
                    state.refusals -= 1;
                }
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
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
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SolidCacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            budget: self.budget,
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
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_zero(n: &usize) -> bool {
    *n == 0
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

    // Solids: tessellated through the cache at this pass's tier (a hit
    // when the session warmed the cache on the solve loop's workers), then
    // drawn as meshes under the DISPLAY MESH's value hash — identical
    // solids at one deflection instance like identical meshes, and the same
    // solid at another deflection is another blob. A mesh that did not
    // close draws all the same, with a warning on record; a solid that
    // cannot be tessellated is reported, not drawn.
    let mut tessellated: Vec<(u32, Arc<DisplayMesh>)> = Vec::new();
    if !solids.is_empty() {
        let deflection = context.deflection();
        for (element, value) in &solids {
            let ValueData::Solid(solid) = value.data() else {
                continue;
            };
            match context.solids.tessellation(value.hash(), solid, deflection) {
                Ok(mesh) => {
                    if !mesh.watertight {
                        out.stats.warnings.push(format!(
                            "element {element} (Solid): the kernel's mesh does not close at \
                             this deflection; drawn as is"
                        ));
                    }
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
        }
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
    let mut faces = 0;
    let mut unclosed = 0;
    let mut errors = Vec::new();
    for (element, value) in &solids {
        let ValueData::Solid(solid) = value.data() else {
            continue;
        };
        match context
            .solids
            .tessellation_for_summary(value.hash(), solid, context.config)
        {
            Ok(display) => {
                stats.grow(display.mesh().positions());
                triangles += display.mesh().triangle_count();
                faces += display.faces;
                if !display.watertight {
                    unclosed += 1;
                }
            }
            Err(reason) => errors.push(format!("element {element} (Solid): {reason}")),
        }
    }
    if triangles > 0 {
        summary
            .facts
            .insert("triangles".to_owned(), serde_json::json!(triangles));
    }
    if !solids.is_empty() {
        summary
            .facts
            .insert("solids".to_owned(), serde_json::json!(solids.len()));
        summary
            .facts
            .insert("faces".to_owned(), serde_json::json!(faces));
        if unclosed > 0 {
            summary
                .facts
                .insert("unclosed".to_owned(), serde_json::json!(unclosed));
        }
    }
    if !errors.is_empty() {
        summary
            .facts
            .insert("error".to_owned(), serde_json::json!(errors.join("; ")));
    }
    summary.bounds = stats.bounds;
}

/// The solid arm of [`summarize`] — "Solid, N faces, bbox": the facts come
/// from the display tessellation (a cache hit when the value is on screen,
/// at whichever tier drew it); a solid the kernel cannot tessellate says
/// why instead, and one whose mesh did not close says `watertight: false`.
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
        .tessellation_for_summary(value.hash(), solid, context.config)
    {
        Ok(display) => {
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
        Err(reason) => {
            summary
                .facts
                .insert("error".to_owned(), serde_json::json!(reason));
        }
    }
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
}
