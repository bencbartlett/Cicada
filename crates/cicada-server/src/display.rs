//! Values → display: binary frames for the viewport (hash-driven
//! instancing, pick ids) and compact [`ValueSummary`]s for the inspector,
//! wire hover, and the closest zoom tier's port previews (docs/16). Both
//! read cached values only — display never re-solves anything.
//!
//! A `Solid` (v0.1 item 3 WP-B) draws through tessellation — the kernel
//! meshes its canonical bytes at the project's display deflection
//! (`cicada_geom::solid::Deflection::display`, docs/03) — cached by the
//! solid's VALUE hash in the session's [`SolidCache`] (docs/12 §Display
//! cache): a hit is a map lookup, a miss is one kernel call, and the
//! frames are the ordinary mesh frames (`frames.rs` is unchanged; the
//! instancing key is the solid's hash, so identical solids travel once).
//! A solid that cannot be tessellated — a build without the kernel, bytes
//! the kernel refuses — draws nothing and says why in the output's
//! [`DisplayStats::errors`] and in its summary; never a silent skip.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Mesh, Solid};
use cicada_core::hash::ValueHash;
use cicada_core::value::{HashedValue, ValueData};
use cicada_geom::curve::tessellate_closed;
use cicada_geom::solid::{self as solids, Deflection, Tessellation};

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

    /// Resolve a pick id back to `(node ref, output, element)`.
    #[must_use]
    pub fn resolve(&self, id: u32) -> Option<(u32, u32, u32)> {
        self.back.get(&id).copied()
    }
}

/// Default byte budget of a [`SolidCache`]: the welded display meshes it
/// may hold before evicting least-recently-used entries (positions + index
/// buffers, as uploaded). 256 MiB holds the wall's part count many times
/// over at display deflection; a budget, not a correctness boundary —
/// eviction only costs a re-tessellation.
pub const SOLID_CACHE_BUDGET: usize = 256 * 1024 * 1024;

/// What the session passes the display path: the project configuration
/// (tolerance for curve tessellation, tolerance + unit for the solid
/// display deflection) and the solid tessellation cache.
#[derive(Clone, Copy)]
pub struct DisplayContext<'a> {
    /// The project's configuration.
    pub config: &'a ProjectConfig,
    /// The session's tessellation cache.
    pub solids: &'a SolidCache,
}

impl DisplayContext<'_> {
    /// The display deflection for this project (docs/03 formula).
    #[must_use]
    pub fn deflection(&self) -> Deflection {
        Deflection::display(self.config)
    }
}

/// The key of a cached tessellation: the solid's value hash plus the
/// deflection it was meshed at (bit patterns — the deflection is a pure
/// function of the project configuration, and a configuration change is
/// exactly what must miss).
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

struct CacheState {
    entries: HashMap<TessellationKey, Arc<Tessellation>>,
    /// Least recently used first. Re-touching an entry moves it to the
    /// back; eviction pops the front. Linear in the number of entries per
    /// touch — bounded by the budget, and a display pass touches each
    /// solid once.
    order: VecDeque<TessellationKey>,
    bytes: usize,
}

/// The hash-keyed solid tessellation cache (docs/12 §Display cache; DECISIONS.md
/// row 42: "display tessellates Solids through a hash-keyed cache").
/// Internally synchronized so the frame path and the summary path share
/// one instance behind `&`; bounded by bytes, evicted least-recently-used;
/// hit/miss/eviction counts are observable in `/debug/state` (additive).
/// Errors are not cached: a solid the kernel refuses is refused again on
/// the next display pass (the refusal itself is cheap — it fails at the
/// read), so a fixed build or a corrected value recovers by itself.
pub struct SolidCache {
    state: std::sync::Mutex<CacheState>,
    budget: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

/// The cache's counters, as `/debug/state` → `display_cache` reports them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SolidCacheStats {
    /// Tessellations held.
    pub entries: usize,
    /// Bytes held (mesh buffers as uploaded).
    pub bytes: usize,
    /// The byte budget.
    pub budget: usize,
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that tessellated.
    pub misses: u64,
    /// Entries evicted to stay within budget.
    pub evictions: u64,
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
                order: VecDeque::new(),
                bytes: 0,
            }),
            budget,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// The tessellation of `solid` (whose sealed value hash is `hash`) at
    /// `deflection`: the cached one, or the kernel's, which is then cached.
    /// The error is the kernel's reason, rendered for the stats and the
    /// summary — the caller attaches the output and element.
    ///
    /// # Errors
    ///
    /// The `GeomError` of `cicada_geom::solid::tessellate`, rendered:
    /// `KernelUnavailable` in a build without `occt`, `Serialization` for
    /// bytes the kernel cannot read, the mesher's and the weld's refusals.
    pub fn tessellation(
        &self,
        hash: ValueHash,
        solid: &Solid,
        deflection: Deflection,
    ) -> Result<Arc<Tessellation>, String> {
        let key = TessellationKey::new(hash, deflection);
        if let Some(found) = self.lookup(key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(found);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let tessellation =
            Arc::new(solids::tessellate(solid, deflection).map_err(|e| e.to_string())?);
        self.insert(key, Arc::clone(&tessellation));
        Ok(tessellation)
    }

    fn lookup(&self, key: TessellationKey) -> Option<Arc<Tessellation>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let found = Arc::clone(state.entries.get(&key)?);
        if let Some(position) = state.order.iter().position(|k| *k == key) {
            state.order.remove(position);
        }
        state.order.push_back(key);
        Some(found)
    }

    fn insert(&self, key: TessellationKey, tessellation: Arc<Tessellation>) {
        let size = mesh_bytes(&tessellation.mesh.0);
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
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if let Some(evicted) = state.entries.remove(&oldest) {
                state.bytes -= mesh_bytes(&evicted.mesh.0);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        state.entries.insert(key, tessellation);
        state.order.push_back(key);
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
        }
    }
}

/// A mesh's footprint as the cache and the frames see it: f64 positions
/// and u32 indices.
fn mesh_bytes(mesh: &Mesh) -> usize {
    std::mem::size_of_val(mesh.positions()) + std::mem::size_of_val(mesh.indices())
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
    /// Elements that could not be drawn, with the reason (a solid in a
    /// build without the kernel, bytes the kernel refused). Additive;
    /// empty means everything drawable was drawn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
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
/// configuration and cache draw solids.
#[must_use]
#[allow(clippy::too_many_lines)] // one pass over points, curves, meshes, solids — splitting hides the frame order
pub fn frames_for_value(
    value: &HashedValue,
    generation: u64,
    node: u32,
    output: u32,
    picks: &mut PickTable,
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
            let pick = picks.id_for(node, output, *element);
            batch.push_element(*element, pick, xyz, &[]);
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
            let pick = picks.id_for(node, output, *element);
            out.stats.grow(&positions);
            out.stats.segments += indices.len() / 2;
            out.stats.vertices += positions.len() / 3;
            out.stats.elements += 1;
            batch.push_element(*element, pick, &positions, &indices);
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

    // Solids: tessellated through the cache, then drawn as meshes under
    // their own value hash (identical solids instance like identical
    // meshes). A solid that cannot be tessellated is reported, not drawn.
    let mut tessellated: Vec<(u32, ValueHash, Arc<Tessellation>)> = Vec::new();
    if !solids.is_empty() {
        let deflection = context.deflection();
        for (element, value) in &solids {
            let ValueData::Solid(solid) = value.data() else {
                continue;
            };
            match context.solids.tessellation(value.hash(), solid, deflection) {
                Ok(tessellation) => tessellated.push((*element, value.hash(), tessellation)),
                Err(reason) => out
                    .stats
                    .errors
                    .push(format!("element {element} (Solid): {reason}")),
            }
        }
        out.stats.solids = tessellated.len();
    }

    // Meshes: hash-driven instancing — a hash seen once goes inline; a
    // hash shared by several elements travels once as a blob plus an
    // instances frame (identity transforms in the spike).
    if !meshes.is_empty() || !tessellated.is_empty() {
        // The element's VALUE hash is the interning key (docs/12) — no
        // re-hashing: list slots are already-hashed values.
        let mut by_hash: BTreeMap<ValueHash, Vec<(u32, &Mesh)>> = BTreeMap::new();
        for (element, value) in &meshes {
            if let ValueData::Mesh(mesh) = value.data() {
                by_hash
                    .entry(value.hash())
                    .or_default()
                    .push((*element, mesh));
            }
        }
        for (element, hash, tessellation) in &tessellated {
            by_hash
                .entry(*hash)
                .or_default()
                .push((*element, &tessellation.mesh.0));
        }
        let mut batch = Batch::new();
        for (hash, group) in &by_hash {
            let mesh = group[0].1;
            if group.len() == 1 {
                let (element, mesh) = group[0];
                let pick = picks.id_for(node, output, element);
                batch.push_element(element, pick, mesh.positions(), mesh.indices());
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
                    pick_id: picks.id_for(node, output, element),
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
    let mut errors = Vec::new();
    for (element, value) in &solids {
        let ValueData::Solid(solid) = value.data() else {
            continue;
        };
        match context
            .solids
            .tessellation(value.hash(), solid, context.deflection())
        {
            Ok(tessellation) => {
                stats.grow(tessellation.mesh.0.positions());
                triangles += tessellation.mesh.0.triangle_count();
                faces += tessellation.faces;
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
    }
    if !errors.is_empty() {
        summary
            .facts
            .insert("error".to_owned(), serde_json::json!(errors.join("; ")));
    }
    summary.bounds = stats.bounds;
}

/// The solid arm of [`summarize`] — "Solid, N faces, bbox": the facts come
/// from the display tessellation (a cache hit when the value is on screen);
/// a solid the kernel cannot tessellate says why instead.
fn summarize_solid(
    value: &HashedValue,
    solid: &Solid,
    summary: &mut ValueSummary,
    context: &DisplayContext<'_>,
) {
    // "Solid, N faces, bbox": the facts come from the display
    // tessellation (a cache hit when the value is on screen); a
    // solid the kernel cannot tessellate says why instead.
    summary.samples = vec![render(value)];
    summary
        .facts
        .insert("bytes".to_owned(), serde_json::json!(solid.bytes().len()));
    match context
        .solids
        .tessellation(value.hash(), solid, context.deflection())
    {
        Ok(tessellation) => {
            summary
                .facts
                .insert("faces".to_owned(), serde_json::json!(tessellation.faces));
            summary.facts.insert(
                "triangles".to_owned(),
                serde_json::json!(tessellation.mesh.0.triangle_count()),
            );
            let mut stats = DisplayStats::default();
            stats.grow(tessellation.mesh.0.positions());
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
    use cicada_core::spatial::Point;
    use cicada_core::value::List;
    use std::sync::Arc;

    fn point(x: f64) -> Arc<HashedValue> {
        HashedValue::new(ValueData::Point(Point::new(x, 0.0, 0.0))).unwrap()
    }

    /// A default project and a fresh cache — what the session hands in.
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
            DisplayContext {
                config: &self.config,
                solids: &self.solids,
            }
        }
    }

    /// The probe's 10 × 20 × 30 box at the origin: real canonical bytes
    /// (WP-A's golden `e220198a…`), committed so these tests draw a real
    /// solid when the kernel is linked and assert the typed refusal when
    /// it is not.
    fn probe_box() -> Arc<HashedValue> {
        let bytes = include_bytes!("../tests/fixtures/box-10x20x30.brep.bin");
        assert_eq!(bytes.len(), 4494);
        HashedValue::new(ValueData::Solid(
            Solid::from_canonical_bytes(bytes.to_vec()).unwrap(),
        ))
        .unwrap()
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
    fn a_point_list_becomes_one_point_batch_with_stable_picks() {
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(point(0.0)), None, Some(point(2.0))],
        }))
        .unwrap();
        let test = TestContext::new();
        let mut picks = PickTable::default();
        let first = frames_for_value(&list, 1, 5, 0, &mut picks, &test.context());
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
        let second = frames_for_value(&list, 2, 5, 0, &mut picks, &test.context());
        let Frame::Batch { batch, .. } = decode(&second.frames[0]).unwrap() else {
            panic!("point batch")
        };
        assert_eq!(batch.elements[1].pick_id, pick_of_third);
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
        let out = frames_for_value(&list, 3, 1, 0, &mut picks, &test.context());
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
        let out = frames_for_value(&number, 1, 1, 0, &mut PickTable::default(), &test.context());
        assert_eq!(out.stats.kinds, vec!["clear"]);
        let circle = HashedValue::new(ValueData::Curve(Curve::Circle(
            cicada_core::geometry::Circle {
                plane: cicada_core::spatial::Plane::world_xy(),
                radius: 2.0,
            },
        )))
        .unwrap();
        let out = frames_for_value(&circle, 1, 1, 0, &mut PickTable::default(), &test.context());
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

    /// One element of a drawn list, or one bare value: the box as the
    /// display path reports it. Both worlds are asserted — with the kernel
    /// a real cube; without, the typed refusal in the stats — so the test
    /// never passes vacuously.
    #[test]
    #[allow(clippy::float_cmp)] // exact bounds from exact planar geometry
    fn a_solid_draws_as_mesh_frames_through_the_cache_or_says_why() {
        let test = TestContext::new();
        let solid = probe_box();
        assert!(is_drawable(&solid));
        let mut picks = PickTable::default();
        let out = frames_for_value(&solid, 1, 7, 0, &mut picks, &test.context());
        let summary = summarize(&solid, &test.context());
        assert_eq!(summary.kind, "Solid");
        assert_eq!(summary.facts["bytes"], 4494);
        assert_eq!(summary.samples, vec!["Solid(4494 bytes)"]);
        if solids::kernel_available() {
            assert_eq!(out.stats.kinds, vec!["mesh"]);
            assert_eq!(out.stats.solids, 1);
            assert_eq!(out.stats.elements, 1);
            assert_eq!(out.stats.triangles, 12);
            assert_eq!(out.stats.vertices, 8);
            assert!(out.stats.errors.is_empty(), "{:?}", out.stats.errors);
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
            // The summary: "Solid, N faces, bbox".
            assert_eq!(summary.facts["faces"], 6);
            assert_eq!(summary.facts["triangles"], 12);
            assert_eq!(summary.bounds, Some([[0.0, 0.0, 0.0], [10.0, 20.0, 30.0]]));
            assert!(!summary.facts.contains_key("error"));
            // One miss for the frames, one hit for the summary.
            let stats = test.solids.stats();
            assert_eq!((stats.misses, stats.hits, stats.entries), (1, 1, 1));
            assert_eq!(stats.bytes, 8 * 3 * 8 + 12 * 3 * 4);
            // Drawing it again is a hit, not a kernel call.
            let again = frames_for_value(&solid, 2, 7, 0, &mut picks, &test.context());
            assert_eq!(again.stats.triangles, 12);
            assert_eq!(test.solids.stats().hits, 2);
            assert_eq!(test.solids.stats().misses, 1);
        } else {
            // No kernel: nothing drawn, the reason on record, a clear frame.
            assert_eq!(out.stats.kinds, vec!["clear"]);
            assert_eq!(out.stats.solids, 0);
            assert_eq!(out.stats.elements, 0);
            assert_eq!(out.stats.errors.len(), 1);
            assert!(
                out.stats.errors[0].contains("element 0 (Solid)")
                    && out.stats.errors[0].contains("OCCT")
                    && out.stats.errors[0].contains("feature `occt`"),
                "{}",
                out.stats.errors[0]
            );
            assert!(
                summary.facts["error"]
                    .as_str()
                    .unwrap()
                    .contains("feature `occt`"),
                "{:?}",
                summary.facts
            );
            assert!(summary.bounds.is_none());
            // Errors are not cached: two misses, no entries.
            let stats = test.solids.stats();
            assert_eq!((stats.misses, stats.hits, stats.entries), (2, 0, 0));
        }
    }

    #[test]
    fn identical_solids_in_a_list_instance_under_their_value_hash() {
        let test = TestContext::new();
        let solid = probe_box();
        let list = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(solid.clone()), None, Some(solid.clone())],
        }))
        .unwrap();
        assert!(is_drawable(&list));
        let mut picks = PickTable::default();
        let out = frames_for_value(&list, 1, 2, 0, &mut picks, &test.context());
        let summary = summarize(&list, &test.context());
        assert_eq!(summary.count, Some(3));
        assert_eq!(summary.facts["element_kind"], "Solid");
        assert_eq!(summary.facts["solids"], 2);
        if solids::kernel_available() {
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
            assert_eq!(
                hash,
                solid.hash(),
                "the blob is keyed by the SOLID's value hash"
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
            assert_eq!(summary.facts["faces"], 12);
            assert_eq!(summary.facts["triangles"], 24);
            // The two elements share one value: one miss, then hits.
            let stats = test.solids.stats();
            assert_eq!(stats.misses, 1);
            assert_eq!(stats.entries, 1);
        } else {
            assert_eq!(out.stats.errors.len(), 2);
            assert_eq!(out.stats.kinds, vec!["clear"]);
            assert!(summary.facts["error"].as_str().unwrap().contains("occt"));
        }
    }

    #[test]
    fn the_cache_evicts_least_recently_used_within_its_budget() {
        // Only meaningful with the kernel (nothing is cached otherwise);
        // the default build asserts that instead.
        let solid = probe_box();
        let ValueData::Solid(bytes) = solid.data() else {
            panic!("solid")
        };
        // A cube's display mesh is 8 vertices + 12 triangles = 336 bytes.
        let test = TestContext::with_budget(700);
        let context = test.context();
        let deflection = context.deflection();
        let coarser = Deflection::new(deflection.linear() * 2.0, deflection.angular()).unwrap();
        let finer = Deflection::new(deflection.linear() / 2.0, deflection.angular()).unwrap();
        if !solids::kernel_available() {
            assert!(
                test.solids
                    .tessellation(solid.hash(), bytes, deflection)
                    .is_err()
            );
            assert_eq!(test.solids.stats().entries, 0);
            return;
        }
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

    #[test]
    fn bad_solid_bytes_are_an_error_on_record_not_a_crash() {
        // Core accepts the header alone; the kernel does not. Either way
        // the display path reports and moves on.
        let test = TestContext::new();
        let pseudo = HashedValue::new(ValueData::Solid(
            Solid::from_canonical_bytes(cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec())
                .unwrap(),
        ))
        .unwrap();
        let out = frames_for_value(&pseudo, 1, 1, 0, &mut PickTable::default(), &test.context());
        assert_eq!(out.stats.kinds, vec!["clear"]);
        assert_eq!(out.stats.errors.len(), 1);
        let summary = summarize(&pseudo, &test.context());
        assert!(summary.facts.contains_key("error"), "{:?}", summary.facts);
        assert_eq!(test.solids.stats().entries, 0, "errors are never cached");
    }
}
