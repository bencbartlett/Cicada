//! Glyph geometry from a TrueType face — the ttf-parser seam (DECISIONS.md
//! rented-kernels row: "ttf-parser/rustybuzz (text outlines)"; the spike
//! rents only the parser and lays text out itself).
//!
//! What lives here, in pipeline order:
//!
//! 1. [`Font`]: a parsed face plus its cap height — the size reference for
//!    fabrication text (`size` = capital-letter height, how labels are
//!    specified on drawings): OS/2 `sCapHeight` when the font carries it,
//!    else the bounding-box height of `H`.
//! 2. [`Font::glyph`]: a character's outline as flattened closed contours
//!    in font units — every bézier span becomes `segments` chords (uniform
//!    parameter steps; pure arithmetic, so goldens stay platform-stable).
//! 3. [`layout`]: left-to-right advances (kerning ignored), the baseline on
//!    the x axis, `\n` stacking lines downward by `line_gap × size`, lines
//!    left-aligned; contours scaled into document units and classified
//!    outer/hole by even-odd depth (point-in-polygon of each contour's
//!    first vertex against the others — robust to the TrueType/CFF winding
//!    conventions).
//! 4. [`glyph_solid`]: one watertight prism per glyph, the glyph region
//!    triangulated WITH its holes (holes bridged into their outer ring
//!    earcut-style, then the house ear clipper) and extruded along the
//!    frame normal.
//!
//! Everything refuses loudly (docs/08 rule 7): a missing glyph names the
//! character, a degenerate size or segment count names the parameter, a
//! contour that cannot triangulate surfaces the ear clipper's reason.

use glam::DVec2;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::frame::Frame;
use crate::triangulate::signed_area_doubled;
use crate::{GeomError, tol};

/// A parsed face with its cap height resolved. Borrows the font bytes
/// (the stdlib bundles them `'static`).
#[derive(Debug)]
pub struct Font<'a> {
    face: Face<'a>,
    /// Capital-letter height in font units (see [`Font::cap_height`]).
    cap_height: f64,
}

/// One glyph's outline in FONT units: flattened closed contours (the
/// closing vertex implicit, consecutive exact duplicates dropped) and the
/// horizontal advance. Contours keep the font's native vertex order and
/// winding.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphOutline {
    /// Closed contours, font order.
    pub contours: Vec<Vec<DVec2>>,
    /// Horizontal advance in font units.
    pub advance: f64,
}

/// A classified contour of a laid-out glyph, in document units relative to
/// the text origin (x right, y up, first baseline at y = 0).
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    /// The vertices (closing edge implicit), font order and winding.
    pub points: Vec<DVec2>,
    /// Even-odd nesting depth: how many other contours of the same glyph
    /// contain this one. Even = outer boundary, odd = hole.
    pub depth: usize,
    /// The innermost contour containing this one (index into the glyph's
    /// contour list) — a hole's outer ring; `None` at depth 0.
    pub parent: Option<usize>,
}

impl Contour {
    /// Odd depth = a hole (a counter).
    #[must_use]
    pub fn is_hole(&self) -> bool {
        self.depth % 2 == 1
    }
}

/// A glyph placed by [`layout`]: its contours in document units, already
/// translated to the pen position. Whitespace and other empty-outline
/// glyphs advance the pen but never appear here.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedGlyph {
    /// The character this glyph renders.
    pub character: char,
    /// Classified contours, font order.
    pub contours: Vec<Contour>,
}

impl<'a> Font<'a> {
    /// Parse a face from TrueType/OpenType bytes and resolve its cap height.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] when ttf-parser rejects the bytes, or when the
    /// face has neither an OS/2 cap height nor an `H` to measure.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, GeomError> {
        let face = Face::parse(data, 0).map_err(|error| GeomError::Kernel {
            reason: format!("ttf-parser refused the font bytes: {error}"),
        })?;
        let cap_height = match face.capital_height() {
            Some(height) if height > 0 => f64::from(height),
            _ => {
                let h = face.glyph_index('H').ok_or_else(|| GeomError::Kernel {
                    reason: "font has no OS/2 cap height and no `H` glyph to measure".to_owned(),
                })?;
                let bbox = face
                    .glyph_bounding_box(h)
                    .ok_or_else(|| GeomError::Kernel {
                        reason: "font has no OS/2 cap height and its `H` has no outline".to_owned(),
                    })?;
                let height = i32::from(bbox.y_max) - i32::from(bbox.y_min);
                if height <= 0 {
                    return Err(GeomError::Kernel {
                        reason: format!(
                            "font has no OS/2 cap height and its `H` bounding box has height \
                             {height}"
                        ),
                    });
                }
                f64::from(height)
            }
        };
        Ok(Self { face, cap_height })
    }

    /// Capital-letter height in font units: OS/2 `sCapHeight`, else the
    /// bounding-box height of `H`. `size` in the text nodes maps to this.
    #[must_use]
    pub fn cap_height(&self) -> f64 {
        self.cap_height
    }

    /// Font units per em.
    #[must_use]
    pub fn units_per_em(&self) -> f64 {
        f64::from(self.face.units_per_em())
    }

    /// Whether the face maps `character` to a glyph (outline or not).
    #[must_use]
    pub fn has_glyph(&self, character: char) -> bool {
        self.face.glyph_index(character).is_some()
    }

    /// A character's outline in font units, béziers flattened to
    /// `segments` chords per span. A glyph without an outline (space) has
    /// no contours and a positive advance.
    ///
    /// # Errors
    ///
    /// [`GeomError::MissingGlyph`] when the face has no glyph for the
    /// character; [`GeomError::BadParameter`] when `segments < 1`.
    pub fn glyph(&self, character: char, segments: i64) -> Result<GlyphOutline, GeomError> {
        if segments < 1 {
            return Err(GeomError::BadParameter {
                name: "segments",
                value: segments.to_string(),
                requirement: "must be >= 1 (chords per bézier span)",
            });
        }
        let id = self
            .face
            .glyph_index(character)
            .ok_or(GeomError::MissingGlyph { character })?;
        let advance = self.face.glyph_hor_advance(id).map_or(0.0, f64::from);
        let mut builder = Flattener {
            segments,
            contours: Vec::new(),
            current: Vec::new(),
        };
        // `None` = no outline (whitespace): zero contours, advance only.
        let _bbox = self.face.outline_glyph(id, &mut builder);
        builder.finish_contour();
        Ok(GlyphOutline {
            contours: builder.contours,
            advance,
        })
    }

    /// The face's glyph id for a character, when mapped.
    #[must_use]
    pub fn glyph_id(&self, character: char) -> Option<u16> {
        self.face.glyph_index(character).map(|GlyphId(id)| id)
    }
}

/// Flattens ttf-parser's outline callbacks into closed contours. Chords
/// are uniform in parameter: `t = k / segments` — arithmetic only.
struct Flattener {
    segments: i64,
    contours: Vec<Vec<DVec2>>,
    current: Vec<DVec2>,
}

impl Flattener {
    fn push(&mut self, point: DVec2) {
        // Exact consecutive duplicates carry no geometry (degenerate spans,
        // the explicit closing vertex some fonts emit).
        if self.current.last() != Some(&point) {
            self.current.push(point);
        }
    }

    fn last(&self) -> DVec2 {
        self.current.last().copied().unwrap_or(DVec2::ZERO)
    }

    fn finish_contour(&mut self) {
        if self.current.len() > 1 && self.current.first() == self.current.last() {
            self.current.pop();
        }
        // Fewer than 3 distinct vertices enclose nothing: no region, no
        // geometry (anchor-point contours some fonts carry).
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

/// Chord parameters `k / segments` for `k = 1..=segments`.
#[allow(clippy::cast_precision_loss)] // segment counts are tiny
fn steps(segments: i64) -> impl Iterator<Item = f64> {
    (1..=segments).map(move |k| k as f64 / segments as f64)
}

impl OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.push(DVec2::new(f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(DVec2::new(f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last();
        let p1 = DVec2::new(f64::from(x1), f64::from(y1));
        let p2 = DVec2::new(f64::from(x), f64::from(y));
        for t in steps(self.segments) {
            let u = 1.0 - t;
            self.push(p0 * (u * u) + p1 * (2.0 * u * t) + p2 * (t * t));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last();
        let p1 = DVec2::new(f64::from(x1), f64::from(y1));
        let p2 = DVec2::new(f64::from(x2), f64::from(y2));
        let p3 = DVec2::new(f64::from(x), f64::from(y));
        for t in steps(self.segments) {
            let u = 1.0 - t;
            self.push(
                p0 * (u * u * u)
                    + p1 * (3.0 * u * u * t)
                    + p2 * (3.0 * u * t * t)
                    + p3 * (t * t * t),
            );
        }
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

/// Lay text out: glyphs left to right from the origin along +x with the
/// baseline on the x axis, advances from the font (kerning ignored), `\n`
/// starting the next line `line_gap × size` lower (lines left-aligned).
/// `size` is the cap height in document units. Contours come back in
/// document units, classified outer/hole; empty-outline glyphs (space)
/// advance the pen and yield no entry.
///
/// # Errors
///
/// [`GeomError::BadParameter`] when `size` is not above `tolerance` or
/// `segments < 1`; [`GeomError::MissingGlyph`] for a character the font
/// lacks (`\n` is layout, never looked up); [`GeomError::DegenerateCurve`]
/// when a scaled contour has consecutive vertices within tolerance (the
/// size is too small for the tolerance to resolve the glyph);
/// [`GeomError::NotSimple`] when two contours of one glyph touch at the
/// classification vertex.
pub fn layout(
    font: &Font<'_>,
    text: &str,
    size: f64,
    segments: i64,
    line_gap: f64,
    tolerance: f64,
) -> Result<Vec<PlacedGlyph>, GeomError> {
    if size <= tolerance {
        return Err(GeomError::BadParameter {
            name: "size",
            value: size.to_string(),
            requirement: "cap height must exceed tolerance",
        });
    }
    if segments < 1 {
        return Err(GeomError::BadParameter {
            name: "segments",
            value: segments.to_string(),
            requirement: "must be >= 1 (chords per bézier span)",
        });
    }
    let scale = size / font.cap_height();
    let mut placed = Vec::new();
    let mut pen = DVec2::ZERO;
    let mut line = 0u32;
    for character in text.chars() {
        if character == '\n' {
            line += 1;
            pen = DVec2::new(0.0, -f64::from(line) * line_gap * size);
            continue;
        }
        let outline = font.glyph(character, segments)?;
        if !outline.contours.is_empty() {
            let mut points: Vec<Vec<DVec2>> = Vec::with_capacity(outline.contours.len());
            for contour in &outline.contours {
                let scaled: Vec<DVec2> = contour.iter().map(|p| *p * scale + pen).collect();
                for (i, a) in scaled.iter().enumerate() {
                    let b = scaled[(i + 1) % scaled.len()];
                    if tol::near_zero(a.distance(b), tolerance) {
                        return Err(GeomError::DegenerateCurve {
                            reason: format!(
                                "glyph {character:?} at size {size}: contour vertices {i} and \
                                 {} coincide within tolerance",
                                (i + 1) % scaled.len()
                            ),
                        });
                    }
                }
                points.push(scaled);
            }
            let contours = classify(points, character, tolerance)?;
            placed.push(PlacedGlyph {
                character,
                contours,
            });
        }
        pen.x += outline.advance * scale;
    }
    Ok(placed)
}

/// Even-odd depth classification. Each contour's first vertex is tested
/// against every other contour: the count of containing contours is its
/// depth, the deepest container its parent.
fn classify(
    points: Vec<Vec<DVec2>>,
    character: char,
    tolerance: f64,
) -> Result<Vec<Contour>, GeomError> {
    let mut depths = vec![0usize; points.len()];
    let mut containers: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (i, contour) in points.iter().enumerate() {
        let probe = contour[0];
        for (j, other) in points.iter().enumerate() {
            if i == j {
                continue;
            }
            if distance_to_ring(other, probe) <= tolerance {
                return Err(GeomError::NotSimple {
                    reason: format!(
                        "glyph {character:?}: contour {i} touches contour {j} (vertex 0 lies on \
                         it within tolerance)"
                    ),
                });
            }
            if contains(other, probe) {
                depths[i] += 1;
                containers[i].push(j);
            }
        }
    }
    Ok(points
        .into_iter()
        .enumerate()
        .map(|(i, pts)| {
            let parent = containers[i].iter().copied().max_by_key(|&j| depths[j]);
            Contour {
                points: pts,
                depth: depths[i],
                parent,
            }
        })
        .collect())
}

/// Even-odd point-in-polygon (crossing number, half-open edge rule). A
/// combinatorial parity test: raw comparisons are sanctioned here because
/// the caller has already refused probes within tolerance of the ring,
/// so no decision falls in the ambiguous band (tol discipline, doc 14).
fn contains(ring: &[DVec2], probe: DVec2) -> bool {
    let mut inside = false;
    let count = ring.len();
    let mut prev = count - 1;
    for here in 0..count {
        let (a, b) = (ring[here], ring[prev]);
        if (a.y > probe.y) != (b.y > probe.y) {
            let x_cross = a.x + (probe.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if probe.x < x_cross {
                inside = !inside;
            }
        }
        prev = here;
    }
    inside
}

/// Distance from a point to the nearest edge of a closed ring.
fn distance_to_ring(ring: &[DVec2], p: DVec2) -> f64 {
    let mut best = f64::INFINITY;
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        let ab = b - a;
        let len2 = ab.length_squared();
        let t = if len2 > 0.0 {
            ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min(p.distance(a + ab * t));
    }
    best
}

/// Bridge `holes` (each a CW ring of indices into `points`) into `outer`
/// (a CCW ring of indices), earcut-style: each hole's rightmost vertex
/// shoots a ray to +x; the first ring edge hit yields a mutually visible
/// ring vertex (the edge endpoint with the larger x, or the reflex vertex
/// inside the (hole vertex, hit, endpoint) triangle with the smallest
/// angle — Eberly's construction); the hole is spliced in through a
/// zero-width bridge (both endpoints duplicated). Holes go in by
/// decreasing rightmost x so every later ray sees the already-merged
/// rings. The result is a weakly simple polygon for [`ear_clip_weak`].
fn bridge_holes(
    points: &[DVec2],
    outer: Vec<u32>,
    holes: &[Vec<u32>],
    tolerance: f64,
) -> Result<Vec<u32>, GeomError> {
    let rightmost = |ring: &[u32]| -> usize {
        let mut best = 0;
        for (k, &id) in ring.iter().enumerate() {
            let (p, b) = (points[id as usize], points[ring[best] as usize]);
            // Ties toward the lower vertex: a total order, deterministic.
            if p.x.total_cmp(&b.x).then(b.y.total_cmp(&p.y)) == std::cmp::Ordering::Greater {
                best = k;
            }
        }
        best
    };
    let mut order: Vec<(usize, usize)> = holes
        .iter()
        .enumerate()
        .map(|(h, ring)| (h, rightmost(ring)))
        .collect();
    order.sort_by(|&(ha, ma), &(hb, mb)| {
        let a = points[holes[ha][ma] as usize];
        let b = points[holes[hb][mb] as usize];
        b.x.total_cmp(&a.x)
            .then(a.y.total_cmp(&b.y))
            .then(ha.cmp(&hb))
    });

    let mut ring = outer;
    for (h, m) in order {
        let hole = &holes[h];
        let m_point = points[hole[m] as usize];
        let target = find_bridge_target(points, &ring, m_point, tolerance)?;
        let mut merged = Vec::with_capacity(ring.len() + hole.len() + 2);
        merged.extend_from_slice(&ring[..=target]);
        merged.extend(hole[m..].iter().copied());
        merged.extend(hole[..m].iter().copied());
        merged.push(hole[m]);
        merged.push(ring[target]);
        merged.extend_from_slice(&ring[target + 1..]);
        ring = merged;
    }
    Ok(ring)
}

/// The ring position a hole's rightmost vertex `m` bridges to (see
/// [`bridge_holes`]).
///
/// # Errors
///
/// [`GeomError::NotSimple`] when the rightward ray hits no ring edge (the
/// hole is not inside the ring) or a ring vertex coincides with `m` (the
/// rings touch).
fn find_bridge_target(
    points: &[DVec2],
    ring: &[u32],
    m: DVec2,
    tolerance: f64,
) -> Result<usize, GeomError> {
    let count = ring.len();
    let at = |k: usize| points[ring[k % count] as usize];
    // 1. First edge the rightward ray crosses (half-open in y so a vertex
    //    exactly on the ray registers once).
    let mut hit: Option<(usize, f64)> = None;
    for i in 0..count {
        let (a, b) = (at(i), at(i + 1));
        if (a.y <= m.y) != (b.y <= m.y) {
            let x_cross = a.x + (m.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if x_cross >= m.x && hit.is_none_or(|(_, best)| x_cross < best) {
                hit = Some((i, x_cross));
            }
        }
    }
    let Some((edge, x_hit)) = hit else {
        return Err(GeomError::NotSimple {
            reason: format!(
                "hole contour (rightmost vertex {m:?}) lies outside its outer ring — no edge \
                 to bridge to"
            ),
        });
    };
    let hit_point = DVec2::new(x_hit, m.y);
    let (ia, ib) = (edge, (edge + 1) % count);
    let (a, b) = (at(ia), at(ib));
    let mut target = if tol::near_zero(a.distance(hit_point), tolerance) {
        ia
    } else if tol::near_zero(b.distance(hit_point), tolerance) {
        ib
    } else if a.x > b.x {
        ia
    } else {
        ib
    };
    // 2. Reflex ring vertices inside the (m, hit, target) triangle block
    //    the view; the one with the smallest angle to the ray (ties:
    //    nearest) is visible instead.
    let area_tol = tolerance * tolerance;
    let cross = |o: DVec2, p: DVec2, q: DVec2| (p - o).perp_dot(q - o);
    let apex = at(target);
    let mut best: Option<(usize, f64, f64)> = None;
    for k in 0..count {
        if k == target {
            continue;
        }
        let candidate = at(k);
        if candidate.x < m.x {
            continue;
        }
        if tol::near_zero(candidate.distance(m), tolerance) {
            return Err(GeomError::NotSimple {
                reason: format!(
                    "hole contour touches its outer ring at {m:?} (vertices coincide within \
                     tolerance)"
                ),
            });
        }
        let reflex = cross(at(k + count - 1), candidate, at(k + 1)) < -area_tol;
        if !reflex || !in_triangle(m, hit_point, apex, candidate, area_tol) {
            continue;
        }
        let dx = candidate.x - m.x;
        let dy = (candidate.y - m.y).abs();
        // Angle to the ray as a tangent (dx >= 0 here; dx == 0 → ∞).
        let tan = if dx > 0.0 { dy / dx } else { f64::INFINITY };
        let dist = dx * dx + dy * dy;
        let better = best.is_none_or(|(_, bt, bd)| {
            tan.total_cmp(&bt).then(dist.total_cmp(&bd)) == std::cmp::Ordering::Less
        });
        if better {
            best = Some((k, tan, dist));
        }
    }
    if let Some((k, _, _)) = best {
        target = k;
    }
    // 3. Earlier bridges duplicate their endpoints: several ring positions
    //    may sit on the target point, each owning one slice of its interior
    //    angle. Bridge into the occurrence whose sector faces `m` — the
    //    bridge edge must leave the ring vertex INTO the region.
    if !sector_faces(points, ring, target, m) {
        let point = at(target);
        // Identity of the point (same index → same value), not a geometric
        // comparison: duplicates are exact copies by construction.
        if let Some(k) =
            (0..count).find(|&k| k != target && at(k) == point && sector_faces(points, ring, k, m))
        {
            target = k;
        }
    }
    Ok(target)
}

/// Whether the direction from ring vertex `k` toward `toward` lies inside
/// the ring's interior angle at `k` (CCW ring: interior on the left).
/// earcut's `locallyInside`; boundary directions count as inside.
fn sector_faces(points: &[DVec2], ring: &[u32], at_index: usize, toward: DVec2) -> bool {
    let count = ring.len();
    let at = |i: usize| points[ring[i % count] as usize];
    let corner = at(at_index);
    let to_next = at(at_index + 1) - corner;
    let to_prev = at(at_index + count - 1) - corner;
    let dir = toward - corner;
    // Raw sign tests sanctioned: a pure orientation predicate on the ring's
    // own (exact, tolerance-deduped) vertices.
    if to_next.perp_dot(to_prev) >= 0.0 {
        // Convex (or flat) corner: the interior sweeps from the next-edge
        // direction counter-clockwise to the prev-edge direction, under 180°.
        to_next.perp_dot(dir) >= 0.0 && dir.perp_dot(to_prev) >= 0.0
    } else {
        // Reflex corner: everything but the exterior sweep from prev to next.
        !(to_prev.perp_dot(dir) > 0.0 && dir.perp_dot(to_next) > 0.0)
    }
}

/// Closed (boundary-inclusive, tolerance-padded) point-in-triangle for
/// either orientation.
fn in_triangle(a: DVec2, b: DVec2, c: DVec2, p: DVec2, area_tol: f64) -> bool {
    let d1 = (b - a).perp_dot(p - a);
    let d2 = (c - b).perp_dot(p - b);
    let d3 = (a - c).perp_dot(p - c);
    let has_neg = d1 < -area_tol || d2 < -area_tol || d3 < -area_tol;
    let has_pos = d1 > area_tol || d2 > area_tol || d3 > area_tol;
    !(has_neg && has_pos)
}

/// Ear clipping for WEAKLY simple polygons — the bridged rings
/// [`bridge_holes`] produces, where a vertex position may occur twice
/// (bridge endpoints) and the boundary touches itself along zero-width
/// bridges. Same contract as [`crate::triangulate::ear_clip`] (every vertex used, CCW output,
/// deterministic first-ear-from-the-head order, post-validated), with the
/// blocking rule an ear needs for duplicates:
///
/// - only REFLEX (or flat) vertices block an ear — the classical
///   optimization (a convex vertex inside an ear triangle implies a reflex
///   one inside it too), which is also what lets the convex duplicate of a
///   bridge endpoint stand on an ear's corner without blocking it;
/// - a vertex coincident with one of the ear's corners never blocks: its
///   own edges leave that point inside the COMPLEMENTARY angular sector
///   (the bridge splits the original interior angle in two), so they cannot
///   enter the ear triangle;
/// - otherwise the test is the closed, tolerance-padded triangle (a reflex
///   vertex on the closing diagonal blocks, as in `ear_clip`).
///
/// Zero-area ears (collinear, or a corner duplicated) clip immediately:
/// nothing can lie strictly inside them, and for a duplicated corner the
/// degenerate triangle's three edges map to a canceling pair plus a loop —
/// [`triangulate_with_holes`] drops it without changing the boundary.
///
/// # Errors
///
/// [`GeomError::NotSimple`] exactly as `ear_clip`: under 3 vertices, zero
/// area, no clippable ear, a clockwise triangle or an area mismatch in
/// post-validation.
pub fn ear_clip_weak(polygon: &[DVec2], tol: f64) -> Result<Vec<[u32; 3]>, GeomError> {
    if polygon.len() < 3 {
        return Err(GeomError::NotSimple {
            reason: format!("{} vertices (need 3)", polygon.len()),
        });
    }
    let area2 = signed_area_doubled(polygon);
    let area_tol = tol * tol;
    if area2.abs() <= area_tol {
        return Err(GeomError::NotSimple {
            reason: format!("effectively zero area ({})", area2 / 2.0),
        });
    }
    let mut order: Vec<u32> =
        (0..u32::try_from(polygon.len()).map_err(|_| GeomError::NotSimple {
            reason: "more than u32::MAX vertices".to_owned(),
        })?)
            .collect();
    if area2 < 0.0 {
        order.reverse();
    }

    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut triangles = Vec::with_capacity(polygon.len() - 2);
    while order.len() > 3 {
        let n = order.len();
        let mut clipped = false;
        for i in 0..n {
            let prev = polygon[order[(i + n - 1) % n] as usize];
            let here = polygon[order[i] as usize];
            let next = polygon[order[(i + 1) % n] as usize];
            let turn = cross(prev, here, next);
            if turn < -area_tol {
                continue; // reflex — never an ear
            }
            let is_ear = turn.abs() <= area_tol
                || (0..n)
                    .filter(|&j| j != i && j != (i + n - 1) % n && j != (i + 1) % n)
                    .all(|j| {
                        let p = polygon[order[j] as usize];
                        if tol::near_zero(p.distance(prev), tol)
                            || tol::near_zero(p.distance(here), tol)
                            || tol::near_zero(p.distance(next), tol)
                        {
                            return true; // a duplicate of a corner: never blocks
                        }
                        let p_prev = polygon[order[(j + n - 1) % n] as usize];
                        let p_next = polygon[order[(j + 1) % n] as usize];
                        let reflex_or_flat = cross(p_prev, p, p_next) <= area_tol;
                        if !reflex_or_flat {
                            return true; // convex vertices never block
                        }
                        !(cross(prev, here, p) >= -area_tol
                            && cross(here, next, p) >= -area_tol
                            && cross(next, prev, p) >= -area_tol)
                    });
            if is_ear {
                triangles.push([order[(i + n - 1) % n], order[i], order[(i + 1) % n]]);
                order.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            return Err(GeomError::NotSimple {
                reason: "no clippable ear (self-intersecting or touching contours?)".to_owned(),
            });
        }
    }
    triangles.push([order[0], order[1], order[2]]);

    // Post-validation, as ear_clip: no strictly clockwise triangle, and the
    // signed areas sum to the polygon's.
    #[allow(clippy::cast_precision_loss)] // vertex counts are far below 2^53
    let sum_tol = area_tol * polygon.len() as f64 + 1e-12 * area2.abs();
    let mut signed_sum = 0.0;
    for &[a, b, c] in &triangles {
        let doubled = cross(
            polygon[a as usize],
            polygon[b as usize],
            polygon[c as usize],
        );
        if doubled < -area_tol {
            return Err(GeomError::NotSimple {
                reason: format!(
                    "triangulation emitted a clockwise triangle (signed area {}); \
                     self-intersecting or touching contours",
                    doubled / 2.0
                ),
            });
        }
        signed_sum += doubled;
    }
    if (signed_sum - area2.abs()).abs() > sum_tol {
        return Err(GeomError::NotSimple {
            reason: format!(
                "triangulation area {} does not match polygon area {}; \
                 self-intersecting or touching contours",
                signed_sum / 2.0,
                area2.abs() / 2.0
            ),
        });
    }
    Ok(triangles)
}

/// Triangulate a region with holes: `outer` and each hole are rings of
/// indices into `points` (any winding — normalized here). Triangles index
/// `points` and wind counter-clockwise; every ring vertex is used, so a
/// prism's walls stitch against the cap exactly (the `ear_clip`
/// boundary contract extended to holes). Zero-area bridge triangles are
/// dropped — they contribute canceling edge pairs only.
///
/// # Errors
///
/// [`GeomError::NotSimple`] when a hole finds no visible outer vertex, or
/// the bridged polygon fails the ear clipper (self-intersecting or
/// touching contours).
pub fn triangulate_with_holes(
    points: &[DVec2],
    outer: &[u32],
    holes: &[Vec<u32>],
    tolerance: f64,
) -> Result<Vec<[u32; 3]>, GeomError> {
    let gather =
        |ring: &[u32]| -> Vec<DVec2> { ring.iter().map(|&i| points[i as usize]).collect() };
    let oriented = |ring: &[u32], ccw: bool| -> Vec<u32> {
        let area2 = signed_area_doubled(&gather(ring));
        // Raw sign test sanctioned: a zero-area ring is refused by the ear
        // clipper on the same polygon (the ambiguous band never reaches here).
        if (area2 > 0.0) == ccw {
            ring.to_vec()
        } else {
            ring.iter().rev().copied().collect()
        }
    };
    let outer_ccw = oriented(outer, true);
    let holes_cw: Vec<Vec<u32>> = holes.iter().map(|h| oriented(h, false)).collect();
    let polygon = bridge_holes(points, outer_ccw, &holes_cw, tolerance)?;
    let local = ear_clip_weak(&gather(&polygon), tolerance)?;
    Ok(local
        .into_iter()
        .map(|[a, b, c]| {
            [
                polygon[a as usize],
                polygon[b as usize],
                polygon[c as usize],
            ]
        })
        .filter(|[a, b, c]| a != b && b != c && a != c)
        .collect())
}

/// One watertight prism for a laid-out glyph: the glyph region (outer
/// contours with their holes) triangulated and extruded by `depth` along
/// the frame's z (negative = the other way). Glyphs with several outer
/// contours (`i`, `%`, `"`) yield one mesh with several shells.
///
/// # Errors
///
/// [`GeomError::BadParameter`] when `|depth|` is within tolerance of zero;
/// [`GeomError::NotSimple`] when the region cannot be triangulated;
/// [`GeomError::Mesh`] on a builder bug (refused, never silently wrong).
pub fn glyph_solid(
    glyph: &PlacedGlyph,
    frame: &Frame,
    depth: f64,
    tolerance: f64,
) -> Result<cicada_core::geometry::Mesh, GeomError> {
    if tol::near_zero(depth, tolerance) {
        return Err(GeomError::BadParameter {
            name: "depth",
            value: depth.to_string(),
            requirement: "must exceed tolerance in magnitude",
        });
    }
    // Global 2D vertex list: contour after contour.
    let mut points: Vec<DVec2> = Vec::new();
    let mut rings: Vec<Vec<u32>> = Vec::with_capacity(glyph.contours.len());
    for contour in &glyph.contours {
        let start = u32::try_from(points.len()).map_err(|_| GeomError::NotSimple {
            reason: "more than u32::MAX glyph vertices".to_owned(),
        })?;
        let end = start
            + u32::try_from(contour.points.len()).map_err(|_| GeomError::NotSimple {
                reason: "more than u32::MAX glyph vertices".to_owned(),
            })?;
        rings.push((start..end).collect());
        points.extend_from_slice(&contour.points);
    }
    let n = points.len();
    let top_offset = u32::try_from(n).map_err(|_| GeomError::NotSimple {
        reason: "more than u32::MAX glyph vertices".to_owned(),
    })?;

    // Caps: every outer ring with the holes it parents.
    let mut cap: Vec<[u32; 3]> = Vec::new();
    for (i, contour) in glyph.contours.iter().enumerate() {
        if contour.is_hole() {
            continue;
        }
        let holes: Vec<Vec<u32>> = glyph
            .contours
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_hole() && c.parent == Some(i))
            .map(|(j, _)| rings[j].clone())
            .collect();
        cap.extend(triangulate_with_holes(
            &points, &rings[i], &holes, tolerance,
        )?);
    }

    // Boundary walk with the region on the left: outer CCW, holes CW.
    let mut boundary: Vec<Vec<u32>> = Vec::with_capacity(rings.len());
    for (contour, ring) in glyph.contours.iter().zip(&rings) {
        let area2 = signed_area_doubled(&contour.points);
        // Raw sign test sanctioned: zero-area rings were refused by the
        // triangulation above.
        let keep = (area2 > 0.0) != contour.is_hole();
        boundary.push(if keep {
            ring.clone()
        } else {
            ring.iter().rev().copied().collect()
        });
    }

    let direction = frame.z * depth;
    let mut positions = Vec::with_capacity(n * 2 * 3);
    for p in &points {
        let bottom = frame.point_at(p.x, p.y).0;
        positions.extend_from_slice(&[bottom.x, bottom.y, bottom.z]);
    }
    for p in &points {
        let top = frame.point_at(p.x, p.y).0 + direction;
        positions.extend_from_slice(&[top.x, top.y, top.z]);
    }
    // Same convention as meshbuild::extrude: CCW cap triangles face +z, so
    // an upward prism flips the bottom cap; a downward one mirrors.
    // Raw sign test sanctioned: |depth| <= tolerance was refused above.
    let up = depth > 0.0;
    let mut indices = Vec::with_capacity((cap.len() * 2 + n * 2) * 3);
    for &[a, b, c] in &cap {
        if up {
            indices.extend_from_slice(&[a, c, b]);
            indices.extend_from_slice(&[a + top_offset, b + top_offset, c + top_offset]);
        } else {
            indices.extend_from_slice(&[a, b, c]);
            indices.extend_from_slice(&[a + top_offset, c + top_offset, b + top_offset]);
        }
    }
    for ring in &boundary {
        for (k, &i) in ring.iter().enumerate() {
            let j = ring[(k + 1) % ring.len()];
            let (bi, bj, ti, tj) = (i, j, i + top_offset, j + top_offset);
            if up {
                indices.extend_from_slice(&[bi, bj, tj]);
                indices.extend_from_slice(&[bi, tj, ti]);
            } else {
                indices.extend_from_slice(&[bi, tj, bj]);
                indices.extend_from_slice(&[bi, ti, tj]);
            }
        }
    }
    let mesh = cicada_core::geometry::Mesh::new(positions, indices)?;
    if !mesh.is_watertight() {
        return Err(GeomError::NotSimple {
            reason: format!(
                "glyph {:?}: the triangulated region does not close into a watertight prism \
                 (touching or overlapping contours?)",
                glyph.character
            ),
        });
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-6;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<DVec2> {
        vec![
            DVec2::new(x0, y0),
            DVec2::new(x1, y0),
            DVec2::new(x1, y1),
            DVec2::new(x0, y1),
        ]
    }

    fn area(points: &[DVec2], triangles: &[[u32; 3]]) -> f64 {
        triangles
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (points[a as usize], points[b as usize], points[c as usize]);
                (b - a).perp_dot(c - a) / 2.0
            })
            .sum()
    }

    /// Directed edges of a CCW triangulation: boundary edges once, interior
    /// edges as canceling pairs.
    fn edge_balance(triangles: &[[u32; 3]]) -> std::collections::HashMap<(u32, u32), i32> {
        let mut net = std::collections::HashMap::new();
        for &[a, b, c] in triangles {
            for (from, to) in [(a, b), (b, c), (c, a)] {
                *net.entry((from.min(to), from.max(to))).or_insert(0) +=
                    if from < to { 1 } else { -1 };
            }
        }
        net.retain(|_, v| *v != 0);
        net
    }

    #[test]
    fn square_with_square_hole_triangulates_to_the_ring_area() {
        let mut points = square(0.0, 0.0, 10.0, 10.0);
        points.extend(square(4.0, 4.0, 6.0, 6.0));
        let outer: Vec<u32> = (0..4).collect();
        let hole: Vec<u32> = (4..8).collect();
        let triangles = triangulate_with_holes(&points, &outer, std::slice::from_ref(&hole), TOL)
            .expect("triangulates");
        assert!((area(&points, &triangles) - 96.0).abs() < 1e-9);
        // Every triangle CCW.
        for &[a, b, c] in &triangles {
            let (a, b, c) = (points[a as usize], points[b as usize], points[c as usize]);
            assert!((b - a).perp_dot(c - a) > 0.0);
        }
        // Net boundary = outer CCW + hole CW, each edge exactly once.
        let net = edge_balance(&triangles);
        assert_eq!(
            net.len(),
            8,
            "8 boundary edges, interior edges cancel: {net:?}"
        );
    }

    #[test]
    fn hole_winding_and_outer_winding_are_normalized() {
        // Outer given CW, hole given CCW: same result as the canonical case.
        let mut points = square(0.0, 0.0, 10.0, 10.0);
        points.extend(square(4.0, 4.0, 6.0, 6.0));
        let outer_cw: Vec<u32> = vec![0, 3, 2, 1];
        let hole_ccw: Vec<u32> = vec![4, 5, 6, 7];
        let triangles =
            triangulate_with_holes(&points, &outer_cw, &[hole_ccw], TOL).expect("triangulates");
        assert!((area(&points, &triangles) - 96.0).abs() < 1e-9);
        assert_eq!(edge_balance(&triangles).len(), 8);
    }

    #[test]
    fn two_holes_bridge_in_rightmost_first_order() {
        let mut points = square(0.0, 0.0, 20.0, 10.0);
        points.extend(square(2.0, 4.0, 4.0, 6.0)); // left hole
        points.extend(square(12.0, 3.0, 16.0, 7.0)); // right hole
        let outer: Vec<u32> = (0..4).collect();
        let holes = vec![(4..8).collect::<Vec<u32>>(), (8..12).collect()];
        let triangles = triangulate_with_holes(&points, &outer, &holes, TOL).expect("triangulates");
        assert!((area(&points, &triangles) - (200.0 - 4.0 - 16.0)).abs() < 1e-9);
        assert_eq!(edge_balance(&triangles).len(), 12);
    }

    #[test]
    fn hole_outside_outer_refuses() {
        let mut points = square(0.0, 0.0, 10.0, 10.0);
        points.extend(square(14.0, 4.0, 16.0, 6.0));
        let outer: Vec<u32> = (0..4).collect();
        let hole: Vec<u32> = (4..8).collect();
        assert!(matches!(
            triangulate_with_holes(&points, &outer, &[hole], TOL),
            Err(GeomError::NotSimple { .. })
        ));
    }

    #[test]
    fn contains_is_even_odd() {
        let ring = square(0.0, 0.0, 10.0, 10.0);
        assert!(contains(&ring, DVec2::new(5.0, 5.0)));
        assert!(!contains(&ring, DVec2::new(15.0, 5.0)));
        assert!(!contains(&ring, DVec2::new(-1.0, 5.0)));
    }

    #[test]
    fn classify_nests_outer_hole_island() {
        let contours = vec![
            square(0.0, 0.0, 10.0, 10.0),
            square(2.0, 2.0, 8.0, 8.0),
            square(4.0, 4.0, 6.0, 6.0),
        ];
        let classified = classify(contours, 'x', TOL).expect("classifies");
        assert_eq!(
            classified.iter().map(|c| c.depth).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            classified.iter().map(|c| c.parent).collect::<Vec<_>>(),
            vec![None, Some(0), Some(1)]
        );
        assert!(!classified[0].is_hole() && classified[1].is_hole() && !classified[2].is_hole());
    }

    #[test]
    fn classify_refuses_touching_contours() {
        // Contour 1's first vertex lies on contour 0's edge.
        let contours = vec![square(0.0, 0.0, 10.0, 10.0), square(10.0, 4.0, 12.0, 6.0)];
        assert!(matches!(
            classify(contours, 'x', TOL),
            Err(GeomError::NotSimple { .. })
        ));
    }

    #[test]
    fn unparseable_font_bytes_refuse() {
        assert!(matches!(
            Font::from_bytes(b"not a font"),
            Err(GeomError::Kernel { .. })
        ));
    }

    proptest::proptest! {
        // A square ring with a square hole anywhere inside, any size:
        // triangulates to the ring area, closed boundary, CCW throughout.
        #[test]
        fn property_square_ring_with_hole(
            cx in 2.0f64..8.0, cy in 2.0f64..8.0,
            half in 0.1f64..1.9,
        ) {
            let mut points = square(0.0, 0.0, 10.0, 10.0);
            points.extend(square(cx - half, cy - half, cx + half, cy + half));
            let outer: Vec<u32> = (0..4).collect();
            let hole: Vec<u32> = (4..8).collect();
            let triangles = triangulate_with_holes(&points, &outer, &[hole], TOL)
                .expect("triangulates");
            let want = 100.0 - (2.0 * half) * (2.0 * half);
            proptest::prop_assert!((area(&points, &triangles) - want).abs() < 1e-9);
            proptest::prop_assert_eq!(edge_balance(&triangles).len(), 8);
        }
    }
}
