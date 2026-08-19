//! Values → display: binary frames for the viewport (hash-driven
//! instancing, pick ids) and compact [`ValueSummary`]s for the inspector,
//! wire hover, and the closest zoom tier's port previews (docs/16). Both
//! read cached values only — display never re-solves anything.

use std::collections::{BTreeMap, HashMap};

use cicada_core::geometry::{Curve, Mesh};
use cicada_core::hash::ValueHash;
use cicada_core::value::{HashedValue, ValueData};
use cicada_geom::curve::tessellate_closed;

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
}

impl DisplayStats {
    fn grow(&mut self, positions: &[f64]) {
        for xyz in positions.chunks_exact(3) {
            let point = [xyz[0], xyz[1], xyz[2]];
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
        ValueData::Point(_) | ValueData::Curve(_) | ValueData::Mesh(_) => true,
        ValueData::List(list) => list
            .slots
            .iter()
            .flatten()
            .any(|element| is_drawable(element)),
        _ => false,
    }
}

/// Encode the display frames of `value` for `(node, output)` at
/// `generation`. `tolerance` feeds curve tessellation.
#[must_use]
#[allow(clippy::too_many_lines)] // one pass over points, curves, meshes — splitting hides the frame order
pub fn frames_for_value(
    value: &HashedValue,
    generation: u64,
    node: u32,
    output: u32,
    picks: &mut PickTable,
    tolerance: f64,
) -> DisplayFrames {
    let mut points: Vec<(u32, [f64; 3])> = Vec::new();
    let mut curves: Vec<(u32, &Curve)> = Vec::new();
    let mut meshes: Vec<(u32, &HashedValue)> = Vec::new();
    collect(value, None, &mut points, &mut curves, &mut meshes);
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

    // Meshes: hash-driven instancing — a hash seen once goes inline; a
    // hash shared by several elements travels once as a blob plus an
    // instances frame (identity transforms in the spike).
    if !meshes.is_empty() {
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
) {
    let index = element.unwrap_or(0);
    match value.data() {
        ValueData::Point(p) => points.push((index, [p.0.x, p.0.y, p.0.z])),
        ValueData::Curve(curve) => curves.push((index, curve)),
        ValueData::Mesh(_) => meshes.push((index, value)),
        ValueData::List(list) => {
            for (slot, item) in list.slots.iter().enumerate() {
                if let Some(item) = item {
                    let own = element.unwrap_or(u32::try_from(slot).unwrap_or(u32::MAX));
                    collect(item, Some(own), points, curves, meshes);
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

/// A compact summary of a value (inspector / hover / port preview).
#[must_use]
pub fn summarize(value: &HashedValue) -> ValueSummary {
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
        ValueData::List(list) => {
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
            collect(value, None, &mut points, &mut curves, &mut meshes);
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
            if triangles > 0 {
                summary
                    .facts
                    .insert("triangles".to_owned(), serde_json::json!(triangles));
            }
            summary.bounds = stats.bounds;
        }
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
        let mut picks = PickTable::default();
        let first = frames_for_value(&list, 1, 5, 0, &mut picks, 1e-6);
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
        let second = frames_for_value(&list, 2, 5, 0, &mut picks, 1e-6);
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
        let mut picks = PickTable::default();
        let out = frames_for_value(&list, 3, 1, 0, &mut picks, 1e-6);
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
        let number = HashedValue::new(ValueData::Number(1.0)).unwrap();
        assert!(!is_drawable(&number));
        let out = frames_for_value(&number, 1, 1, 0, &mut PickTable::default(), 1e-6);
        assert_eq!(out.stats.kinds, vec!["clear"]);
        let circle = HashedValue::new(ValueData::Curve(Curve::Circle(
            cicada_core::geometry::Circle {
                plane: cicada_core::spatial::Plane::world_xy(),
                radius: 2.0,
            },
        )))
        .unwrap();
        let out = frames_for_value(&circle, 1, 1, 0, &mut PickTable::default(), 1e-6);
        assert_eq!(out.stats.kinds, vec!["curve"]);
        assert_eq!(
            out.stats.segments,
            usize::try_from(CIRCLE_SEGMENTS).unwrap(),
            "closed loop"
        );
        let bounds = out.stats.bounds.unwrap();
        assert!((bounds[1][0] - 2.0).abs() < 1e-9);
        let summary = summarize(&circle);
        assert_eq!(summary.kind, "Curve");
        assert_eq!(summary.facts["closed"], true);
    }
}
