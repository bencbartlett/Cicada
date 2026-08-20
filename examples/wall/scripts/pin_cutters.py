# Wall corpus: pin-hole cutter solids + board drill points (stage 6).
#
# Ported from the wall repo's pin_holes.py: the pure layout math and the
# 2D bore profiles (hole_layout, coil_hole_layout, rib_profile_pts,
# capsule_pts, slot_profile_pts, rot_pts, all the section-4 constants),
# plus a pure-Python mesh construction replacing the Rhino brep builders
# (make_bore_cutter / make_chamfer_cutter / make_bore_cone and their slot
# variants, pin_holes.py:765-868): every cutter is its own watertight
# triangle mesh with outward winding -- prisms capped by ear clipping
# (the rib/slot profiles are non-convex), ring lofts for the chamfer
# frusta and the 60-degree cone ceilings.
#
# Per part, two bores (pin_holes.py:17-33): the CENTROID bore is the round
# locator with RibCount crush ribs; the LEAN bore, PinSpacing along the
# field direction (clamped to 0.35 x the cell equivalent diameter), is a
# SLOT elongated +-SlotHalf with two crush vanes. Each bore gets
# [bore prism, 45-degree mouth chamfer, 60-degree cone ceiling] = 6 cutters
# per part, in production order; the cutters start UNDER (1 mm) below the
# base face so the boolean cuts cleanly through z = 0.
#
# Geometry conventions: model mm, z up, parts stand on z = 0 (the centroid
# z) with the cell as the base.

import array
import math

import cicada

CONE_TIP_R = 0.6  # pin_holes.py:413 (mm)
UNDER = 1.0       # pin_holes.py:421 cutter extension past the face (mm)
CIRCLE_SEGMENTS = 48  # ring tessellation for the round chamfer / cone lofts


# ============================================================
# Pure layout math (pin_holes.py:460-497)
# ============================================================

def equiv_dia(area):
    """ported verbatim from pin_holes.py:460-463"""
    if area is None or area <= 0.0:
        return 0.0
    return 2.0 * math.sqrt(area / math.pi)


def unit_xy(dx, dy):
    """ported verbatim from pin_holes.py:466-470 (degenerate -> +X, flag)"""
    L = math.hypot(dx, dy)
    if L < 1e-9:
        return 1.0, 0.0, True
    return dx / L, dy / L, False


def hole_layout(cx, cy, d_equiv, dx, dy, pin_spacing):
    """ported verbatim from pin_holes.py:473-484 (adapted: PIN_SPACING is
    an argument) -- pin 1 at the centroid, pin 2 at PinSpacing along the
    unit lean direction (clamped to 0.35 x cell equivalent diameter).
    Returns ([(x, y, tag), ...], spacing_used)."""
    off = pin_spacing
    if d_equiv > 0.0:
        off = min(off, 0.35 * d_equiv)
    holes = [
        (cx, cy, "center"),
        (cx + dx * off, cy + dy * off, "lean"),
    ]
    return holes, off


def coil_hole_layout(cx, cy, r, count, frac, phase, ci):
    """ported verbatim from pin_holes.py:487-496 -- CoilPinCount pins
    evenly spaced on a frac x r circle about the cylinder axis."""
    ring = frac * r
    holes = []
    for k in range(count):
        a = phase + k * 2.0 * math.pi / count
        holes.append((cx + ring * math.cos(a), cy + ring * math.sin(a),
                      "coil%d-%d" % (ci, k + 1)))
    return holes, ring


def rib_profile_pts(r_bore, r_rib, rib_w, rib_n, seg_ang_deg=15.0):
    """ported verbatim from pin_holes.py:594-624 -- closed CCW 2D profile
    of the bore void with rib indents, centered at origin. Ribs are
    drafted trapezoids."""
    if r_rib >= r_bore:
        r_rib = r_bore - 1e-6
    hw1 = (0.5 * rib_w) / r_rib                     # rib half-width at rib radius
    draft = (r_bore - r_rib) / r_bore               # ~45 deg side draft, angular
    hw2 = hw1 + draft
    sector = 2.0 * math.pi / rib_n
    if 2.0 * hw2 >= sector * 0.8:
        hw2 = sector * 0.4
        hw1 = max(hw2 - draft, hw2 * 0.5)

    seg = math.radians(seg_ang_deg)
    pts = []
    for k in range(rib_n):
        a_mid = -0.5 * math.pi + (k + 0.5) * sector
        arc_start = a_mid - sector + hw2
        arc_end = a_mid - hw2
        span = arc_end - arc_start
        n_seg = max(2, int(math.ceil(span / seg)))
        for i in range(n_seg + 1):
            a = arc_start + span * i / float(n_seg)
            pts.append((r_bore * math.cos(a), r_bore * math.sin(a)))
        pts.append((r_rib * math.cos(a_mid - hw1), r_rib * math.sin(a_mid - hw1)))
        n_rib = 3
        for i in range(1, n_rib):
            a = (a_mid - hw1) + (2.0 * hw1) * i / float(n_rib)
            pts.append((r_rib * math.cos(a), r_rib * math.sin(a)))
        pts.append((r_rib * math.cos(a_mid + hw1), r_rib * math.sin(a_mid + hw1)))
    return pts


def capsule_pts(r, s, seg_ang_deg=15.0):
    """ported verbatim from pin_holes.py:627-641 -- closed CCW stadium: two
    radius-r arcs centered (+-s, 0) joined by straight walls. LOCAL frame,
    +X = slot axis; the vertex count depends only on seg_ang_deg."""
    seg = math.radians(seg_ang_deg)
    n_seg = max(2, int(math.ceil(math.pi / seg)))
    pts = []
    for i in range(n_seg + 1):          # right cap: -90 -> +90 deg
        a = -0.5 * math.pi + math.pi * i / float(n_seg)
        pts.append((s + r * math.cos(a), r * math.sin(a)))
    for i in range(n_seg + 1):          # left cap: +90 -> +270 deg
        a = 0.5 * math.pi + math.pi * i / float(n_seg)
        pts.append((-s + r * math.cos(a), r * math.sin(a)))
    return pts


def slot_profile_pts(r_bore, r_rib, rib_w, s, seg_ang_deg=15.0):
    """ported verbatim from pin_holes.py:644-685 -- closed CCW profile of
    the SLOT bore void (LOCAL frame, +X = the pair axis): a bore stadium
    elongated +-s with exactly TWO crush vanes (flat rails at y = +-r_rib)
    tied to the cap arcs by 45-degree drafts."""
    if r_rib >= r_bore:
        r_rib = r_bore - 1e-6
    hw = 0.5 * rib_w                    # rail beyond the travel, per side
    q = (r_rib - hw) / (math.sqrt(2.0) * r_bore)
    if q > 0.999:
        q = 0.999
    if q < -0.999:
        q = -0.999
    a_end = 0.25 * math.pi + math.asin(q)
    if a_end < math.radians(10.0):
        a_end = math.radians(10.0)
    if a_end > math.radians(85.0):
        a_end = math.radians(85.0)
    seg = math.radians(seg_ang_deg)
    n1 = max(2, int(math.ceil(2.0 * a_end / seg)))
    pts = []
    for i in range(n1 + 1):             # right cap arc: -a_end -> +a_end
        a = -a_end + 2.0 * a_end * i / float(n1)
        pts.append((s + r_bore * math.cos(a), r_bore * math.sin(a)))
    n_rail = 3
    pts.append((s + hw, r_rib))         # top rail, right -> left
    for i in range(1, n_rail):
        pts.append(((s + hw) - 2.0 * (s + hw) * i / float(n_rail), r_rib))
    pts.append((-(s + hw), r_rib))
    for i in range(n1 + 1):             # left cap arc: 180-a_end -> 180+a_end
        a = (math.pi - a_end) + 2.0 * a_end * i / float(n1)
        pts.append((-s + r_bore * math.cos(a), r_bore * math.sin(a)))
    pts.append((-(s + hw), -r_rib))     # bottom rail, left -> right
    for i in range(1, n_rail):
        pts.append((-(s + hw) + 2.0 * (s + hw) * i / float(n_rail), -r_rib))
    pts.append((s + hw, -r_rib))
    return pts


def rot_pts(pts, cs, sn):
    """ported verbatim from pin_holes.py:688-691 -- rotate LOCAL profile
    points so +X maps onto the (cs, sn) unit pair axis."""
    return [(p[0] * cs - p[1] * sn, p[0] * sn + p[1] * cs) for p in pts]


def circle_pts(r, n=CIRCLE_SEGMENTS):
    """CCW n-gon approximating a radius-r circle (seam on +X)."""
    return [(r * math.cos(2.0 * math.pi * k / n), r * math.sin(2.0 * math.pi * k / n))
            for k in range(n)]


# ============================================================
# Pure-Python mesh construction (replaces the Rhino brep builders)
# ============================================================

def signed_area2(pts):
    a = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        a += x1 * y2 - x2 * y1
    return a


def _cross(o, a, b):
    return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])


def _point_in_tri(p, a, b, c, eps):
    """p strictly inside or on the boundary of CCW triangle (a, b, c)."""
    d1 = _cross(a, b, p)
    d2 = _cross(b, c, p)
    d3 = _cross(c, a, p)
    return d1 >= -eps and d2 >= -eps and d3 >= -eps


def triangulate_polygon(pts):
    """Ear-clipping triangulation of a simple polygon given as [(x, y)]
    (either winding). Returns triangles as index triples that are CCW in
    the polygon's own orientation (i.e. indices into `pts`, wound CCW when
    the polygon is CCW). Collinear vertices are handled: a collinear
    vertex is never clipped as an ear itself, only as part of a strictly
    convex neighbor's ear, so every boundary vertex stays on the cap and
    no zero-area triangle is emitted. Raises on degenerate input."""
    n = len(pts)
    if n < 3:
        raise ValueError("triangulate_polygon: %d vertices" % n)
    area2 = signed_area2(pts)
    if abs(area2) < 1e-12:
        raise ValueError("triangulate_polygon: zero-area polygon")
    order = list(range(n))
    if area2 < 0.0:
        order.reverse()
    # scale-aware epsilon
    span = max(max(p[0] for p in pts) - min(p[0] for p in pts),
               max(p[1] for p in pts) - min(p[1] for p in pts))
    eps = 1e-12 * max(span * span, 1e-30)
    tris = []
    idx = list(order)
    guard = 0
    while len(idx) > 3:
        m = len(idx)
        clipped = False
        for k in range(m):
            ia, ib, ic = idx[k - 1], idx[k], idx[(k + 1) % m]
            a, b, c = pts[ia], pts[ib], pts[ic]
            if _cross(a, b, c) <= eps:
                continue  # reflex or collinear: not an ear
            ok = True
            for j in range(m):
                io = idx[j]
                if io in (ia, ib, ic):
                    continue
                if _point_in_tri(pts[io], a, b, c, eps):
                    ok = False
                    break
            if not ok:
                continue
            tris.append((ia, ib, ic))
            del idx[k]
            clipped = True
            break
        if not clipped:
            raise ValueError("triangulate_polygon: no ear found (self-intersecting or degenerate polygon)")
        guard += 1
        if guard > 10 * n:
            raise ValueError("triangulate_polygon: did not converge")
    a, b, c = idx
    if _cross(pts[a], pts[b], pts[c]) <= eps:
        raise ValueError("triangulate_polygon: final triangle is degenerate")
    tris.append((a, b, c))
    return tris


def loft_rings_mesh(rings):
    """A capped solid from a stack of closed CCW rings [(z, [(x, y)..])]
    with identical vertex counts (z increasing). Side quads between
    consecutive rings, bottom cap wound downward, top cap upward -- every
    triangle outward. Returns (vertices, triangles) -- a cicada.Mesh is
    built by the caller."""
    if len(rings) < 2:
        raise ValueError("loft_rings_mesh: need at least 2 rings")
    m = len(rings[0][1])
    for (z, ring) in rings:
        if len(ring) != m:
            raise ValueError("loft_rings_mesh: ring vertex counts differ (%d vs %d)" % (m, len(ring)))
        if signed_area2(ring) <= 0.0:
            raise ValueError("loft_rings_mesh: rings must be CCW with positive area")
    for k in range(1, len(rings)):
        if rings[k][0] <= rings[k - 1][0]:
            raise ValueError("loft_rings_mesh: ring z must increase")
    verts = []
    for (z, ring) in rings:
        for (x, y) in ring:
            verts.append((x, y, z))
    tris = []
    for k in range(len(rings) - 1):
        b0 = k * m
        t0 = (k + 1) * m
        for j in range(m):
            j1 = (j + 1) % m
            tris.append((b0 + j, b0 + j1, t0 + j1))
            tris.append((b0 + j, t0 + j1, t0 + j))
    cap = triangulate_polygon(rings[0][1])
    for (a, b, c) in cap:
        tris.append((a, c, b))  # bottom: wound downward (-z)
    top0 = (len(rings) - 1) * m
    cap = triangulate_polygon(rings[-1][1])
    for (a, b, c) in cap:
        tris.append((top0 + a, top0 + b, top0 + c))
    return verts, tris


def prism_mesh(profile, z0, z1):
    """Straight extrusion of a closed profile [(x, y)] (either winding)
    from z0 to z1 as a capped, outward-wound triangle mesh."""
    if z1 <= z0:
        raise ValueError("prism_mesh: z1 must exceed z0")
    pts = list(profile)
    if signed_area2(pts) < 0.0:
        pts.reverse()
    return loft_rings_mesh([(z0, pts), (z1, pts)])


def make_mesh(verts, tris):
    pos = array.array("d")
    for (x, y, z) in verts:
        pos.append(x)
        pos.append(y)
        pos.append(z)
    idx = array.array("I")
    for (a, b, c) in tris:
        idx.append(a)
        idx.append(b)
        idx.append(c)
    return cicada.Mesh(pos, idx)


def translate(pts, hx, hy):
    return [(p[0] + hx, p[1] + hy) for p in pts]


def mesh_signed_volume(verts, tris):
    """Signed volume by the divergence theorem (positive = outward)."""
    v6 = 0.0
    for (a, b, c) in tris:
        ax, ay, az = verts[a]
        bx, by, bz = verts[b]
        cx, cy, cz = verts[c]
        v6 += ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx) + az * (bx * cy - by * cx)
    return v6 / 6.0


def mesh_is_watertight(verts, tris):
    """Every directed edge used exactly once and its reverse exactly once
    (closed, consistently oriented, no degenerate triangles)."""
    seen = {}
    for (a, b, c) in tris:
        if a == b or b == c or c == a:
            return False
        for (u, v) in ((a, b), (b, c), (c, a)):
            if (u, v) in seen:
                return False
            seen[(u, v)] = True
    for (u, v) in seen:
        if (v, u) not in seen:
            return False
    return True


def check_solid(verts, tris, what):
    if not mesh_is_watertight(verts, tris):
        raise ValueError("pin_cutters: %s is not watertight" % what)
    vol = mesh_signed_volume(verts, tris)
    if vol <= 0.0:
        raise ValueError("pin_cutters: %s has non-positive volume %.6g" % (what, vol))
    return vol


# ============================================================
# Cutter builders (pin_holes.py:765-868 re-expressed as meshes)
# ============================================================

class PinParams(object):
    """adapted: pin_holes.py module parameters (pin_holes.py:365-421) in mm."""

    def __init__(self, bore=3.4, rib_eff_dia=3.12, rib_width=1.0, rib_count=3,
                 chamfer=1.0, relief=1.6, pin_len=15.875, board_depth=8.5,
                 pin_spacing=12.0, slot_half=0.15, hole_cone=True):
        self.BORE = float(bore)
        self.RIB_EFF = float(rib_eff_dia)
        self.RIB_W = float(rib_width)
        self.RIB_N = min(max(int(rib_count), 2), 6)
        self.CHAMFER = float(chamfer)
        self.RELIEF = float(relief)
        self.PIN_LEN = float(pin_len)
        self.BOARD_DEPTH = float(board_depth)
        self.PIN_SPACING = float(pin_spacing)
        self.SLOT_HALF = max(float(slot_half), 0.0)
        self.SLOT_ON = self.SLOT_HALF >= 0.01  # below this, capsule sections degenerate
        self.HOLE_CONE = bool(hole_cone)
        self.R_BORE = 0.5 * self.BORE
        self.PROUD = self.PIN_LEN - self.BOARD_DEPTH      # pin standing proud of the board
        self.HOLE_DEPTH = self.PROUD + self.RELIEF        # part-side hole depth
        if self.RIB_EFF >= self.BORE:
            raise ValueError("pin_cutters: rib_eff_dia %.3f >= bore %.3f (ribs would vanish)"
                             % (self.RIB_EFF, self.BORE))
        if self.PROUD <= 0.0:
            raise ValueError("pin_cutters: board_depth >= pin_len (no proud length)")
        if CONE_TIP_R >= self.R_BORE:
            raise ValueError("pin_cutters: cone tip radius %.2f >= bore radius %.3f" % (CONE_TIP_R, self.R_BORE))


def bore_cutter(hx, hy, z0, profile_local, P):
    """pin_holes.py:765-776 make_bore_cutter: the bore void prism from
    z0 - UNDER up to z0 + HOLE_DEPTH."""
    return prism_mesh(translate(profile_local, hx, hy), z0 - UNDER, z0 + P.HOLE_DEPTH)


def chamfer_cutter(hx, hy, z_face, P):
    """pin_holes.py:779-795 make_chamfer_cutter (sign +1): 45-degree mouth
    chamfer frustum lofted through three circles."""
    r_ch = P.R_BORE + P.CHAMFER
    return loft_rings_mesh([
        (z_face - UNDER, translate(circle_pts(r_ch), hx, hy)),
        (z_face, translate(circle_pts(r_ch), hx, hy)),
        (z_face + P.CHAMFER, translate(circle_pts(P.R_BORE), hx, hy)),
    ])


def bore_cone(hx, hy, z0, P):
    """pin_holes.py:798-822 make_bore_cone: self-supporting 60-degree cone
    ceiling truncated at CONE_TIP_R, overlapping 0.5 mm into the bore."""
    cone_h = (P.R_BORE - CONE_TIP_R) * math.tan(math.radians(60.0))
    z_base = z0 + P.HOLE_DEPTH
    return loft_rings_mesh([
        (z_base - 0.5, translate(circle_pts(P.R_BORE), hx, hy)),
        (z_base, translate(circle_pts(P.R_BORE), hx, hy)),
        (z_base + cone_h, translate(circle_pts(CONE_TIP_R), hx, hy)),
    ])


def slot_chamfer(hx, hy, z_face, cs, sn, P):
    """pin_holes.py:843-851 make_slot_chamfer: capsule frustum."""
    cap_ch = capsule_pts(P.R_BORE + P.CHAMFER, P.SLOT_HALF)
    cap_bore = capsule_pts(P.R_BORE, P.SLOT_HALF)
    return loft_rings_mesh([
        (z_face - UNDER, translate(rot_pts(cap_ch, cs, sn), hx, hy)),
        (z_face, translate(rot_pts(cap_ch, cs, sn), hx, hy)),
        (z_face + P.CHAMFER, translate(rot_pts(cap_bore, cs, sn), hx, hy)),
    ])


def slot_cone(hx, hy, z0, cs, sn, P):
    """pin_holes.py:854-868 make_slot_cone: capsule shrinking to a capsule tip."""
    cone_h = (P.R_BORE - CONE_TIP_R) * math.tan(math.radians(60.0))
    z_base = z0 + P.HOLE_DEPTH
    cap_bore = capsule_pts(P.R_BORE, P.SLOT_HALF)
    return loft_rings_mesh([
        (z_base - 0.5, translate(rot_pts(cap_bore, cs, sn), hx, hy)),
        (z_base, translate(rot_pts(cap_bore, cs, sn), hx, hy)),
        (z_base + cone_h, translate(rot_pts(capsule_pts(CONE_TIP_R, P.SLOT_HALF), cs, sn), hx, hy)),
    ])


def part_cutters(cx, cy, cz, dx, dy, d_equiv, P, profile, slot_profile, templates=None):
    """All cutter meshes of one part in production order (pin_holes.py:
    1062-1084): for each hole [bore, chamfer, cone]; the lean hole is the
    slot version when SLOT_ON. Returns (list of (verts, tris), holes,
    spacing_used). With `templates` (see cutter_templates) the meshes are
    rigid copies of checked local templates -- same geometry, built once."""
    holes, off = hole_layout(cx, cy, d_equiv, dx, dy, P.PIN_SPACING)
    out = []
    for (hx, hy, tag) in holes:
        slot = P.SLOT_ON and tag == "lean"
        if templates is not None:
            for (verts, tris) in templates["slot" if slot else "round"]:
                if slot:
                    # local +X = the pair axis: rotate onto (dx, dy), then place
                    moved = [(hx + x * dx - y * dy, hy + x * dy + y * dx, cz + z) for (x, y, z) in verts]
                else:
                    moved = [(hx + x, hy + y, cz + z) for (x, y, z) in verts]
                out.append((moved, tris))
            continue
        if slot:
            bore = bore_cutter(hx, hy, cz, rot_pts(slot_profile, dx, dy), P)
            cham = slot_chamfer(hx, hy, cz, dx, dy, P)
            cone = slot_cone(hx, hy, cz, dx, dy, P) if P.HOLE_CONE else None
        else:
            bore = bore_cutter(hx, hy, cz, profile, P)
            cham = chamfer_cutter(hx, hy, cz, P)
            cone = bore_cone(hx, hy, cz, P) if P.HOLE_CONE else None
        for mesh in (bore, cham, cone):
            if mesh is not None:
                out.append(mesh)
    return out, holes, off


def cutter_templates(P, profile, slot_profile):
    """The six cutter meshes in LOCAL coordinates (hole at the origin,
    face at z = 0, slot axis = +X), built with the production builders and
    checked watertight + positive volume once. Every part's cutters are
    rigid motions of these (rotation about z and translation), which
    preserve watertightness and volume."""
    out = {"round": [], "slot": []}
    bore = bore_cutter(0.0, 0.0, 0.0, profile, P)
    cham = chamfer_cutter(0.0, 0.0, 0.0, P)
    cone = bore_cone(0.0, 0.0, 0.0, P) if P.HOLE_CONE else None
    out["round"] = [m for m in (bore, cham, cone) if m is not None]
    if P.SLOT_ON:
        sbore = bore_cutter(0.0, 0.0, 0.0, slot_profile, P)  # +X = pair axis
        scham = slot_chamfer(0.0, 0.0, 0.0, 1.0, 0.0, P)
        scone = slot_cone(0.0, 0.0, 0.0, 1.0, 0.0, P) if P.HOLE_CONE else None
        out["slot"] = [m for m in (sbore, scham, scone) if m is not None]
    else:
        out["slot"] = out["round"]
    for kind in ("round", "slot"):
        for k, (verts, tris) in enumerate(out[kind]):
            check_solid(verts, tris, "%s cutter template %d" % (kind, k))
    return out


def poly_area(pts):
    return 0.5 * abs(signed_area2(pts))




def _cell_points(cell):
    """A cell arrives either as a closed curve from the engine (a
    cicada.Polyline-like object with `.points`, the Voronoi cell after the
    per-cell shrink) or as a plain list of points (offline tests, the
    production-layout lists). Both become a list of 3-tuples."""
    pts = getattr(cell, "points", None)
    if pts is None:
        pts = cell
    return [(float(p[0]), float(p[1]), float(p[2]) if len(p) > 2 else 0.0) for p in pts]


@cicada.node(
    title="Pin Cutters",
    description="per-part crush-rib pin-hole cutter solids (bore prism, mouth chamfer, cone ceiling x 2 pins) and the board drill points (ported pin_holes.py).",
)
def pin_cutters(
    centroids: "[Point]",
    directions: "[Vector]",
    cells: "[Closed<Curve>]",
    bore: "Number" = 3.4,
    rib_eff_dia: "Number" = 3.12,
    rib_width: "Number" = 1.0,
    rib_count: "Integer" = 3,
    chamfer: "Number" = 1.0,
    relief: "Number" = 1.6,
    pin_len: "Number" = 15.875,
    board_depth: "Number" = 8.5,
    pin_spacing: "Number" = 12.0,
    slot_half: "Number" = 0.15,
    hole_cone: "Boolean" = True,
) -> {
    "cutters": "[[Watertight<Mesh>]]",
    "board_points": "[Point]",
    "spacing": "[Number]",
    "notes": "[Text]",
}:
    n = len(centroids)
    if not (len(directions) == len(cells) == n):
        raise ValueError("pin_cutters: list lengths differ (centroids %d, directions %d, cells %d)"
                         % (n, len(directions), len(cells)))
    P = PinParams(bore, rib_eff_dia, rib_width, rib_count, chamfer, relief, pin_len,
                  board_depth, pin_spacing, slot_half, hole_cone)
    profile = rib_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.RIB_N)
    slot_profile = slot_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.SLOT_HALF)

    templates = cutter_templates(P, profile, slot_profile)
    cutters = []
    board_points = []
    spacing = []
    degenerate_dirs = 0
    reduced_cells = 0
    tight_cells = 0
    for i in range(n):
        cx, cy, cz = centroids[i]
        dx, dy, bad = unit_xy(directions[i][0], directions[i][1])
        if bad:
            degenerate_dirs += 1
        d = equiv_dia(poly_area([(p[0], p[1]) for p in _cell_points(cells[i])]))
        meshes, holes, off = part_cutters(cx, cy, cz, dx, dy, d, P, profile, slot_profile, templates)
        if off < P.PIN_SPACING - 1e-9:
            reduced_cells += 1
        if off < P.BORE + 1.0:
            tight_cells += 1
        part = []
        for (verts, tris) in meshes:
            part.append(make_mesh(verts, tris))
        cutters.append(part)
        for (hx, hy, _tag) in holes:
            board_points.append((hx, hy, cz))
        spacing.append(off)
    notes = [
        "Parts: %d | Holes: %d | PinSpacing: %.1f mm along lean | %s | HoleDepth: %.2f mm | "
        "PinProud: %.2f mm | RibEff: %.2f mm | Bore: %.2f mm" % (
            n, 2 * n, P.PIN_SPACING,
            ("lean slot +-%.2f mm, 2 vanes" % P.SLOT_HALF) if P.SLOT_ON else "lean bore ROUND (slot off)",
            P.HOLE_DEPTH, P.PROUD, P.RIB_EFF, P.BORE),
    ]
    if degenerate_dirs:
        notes.append("%d parts with zero-length direction, used +X" % degenerate_dirs)
    if reduced_cells:
        notes.append("%d cells with pin spacing reduced below %.1f mm" % (reduced_cells, P.PIN_SPACING))
    if tight_cells:
        notes.append("%d cells too small for two separated pins (check layout)" % tight_cells)
    return {"cutters": cutters, "board_points": board_points, "spacing": spacing, "notes": notes}
