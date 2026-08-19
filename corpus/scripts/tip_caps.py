# Wall corpus: triangular tip caps (stage 6, docs/15).
#
# Ported from the wall repo's tip_caps.py: per part, a closed polygon at
# the apex that IS the field-aligned tip triangle but has the SAME vertex
# count and seam as the base cell, so `loft` pairs vertex i <-> vertex i
# with no twisting (tip_caps.py:1-44). CORNER mode (production default,
# CornerSnap=True): contiguous runs of base corners fuse onto the three
# triangle corners (tip_caps.py:329-394). Pure core ported verbatim; the
# Rhino/GH plumbing is not.
#
# The apex of part i is centroid_i + direction_i * lean_length_i at
# z = height_i (the production apex = centroid displaced along the field
# unit vector by the lean length, at the part's vertical height).
#
# PRODUCTION FINDING (measured on the shipped 1.4.1 H2 meshes): the parts
# that were printed do NOT carry tip_caps' triangles -- their tip is the
# base cell SCALED BY EXACTLY 0.07 about the apex ("the scaled mini-cell
# cap" the tip_caps docstring says it replaces; 0.07000 +- 0.00000 on
# both axes over 79 parts). `cell_scale` > 0 selects that cap (same vertex
# count and seam as the cell, so `loft` pairs vertex i <-> i); 0 (default)
# gives the ported field-aligned triangle. To reproduce production
# geometry wire cell_scale=0.07.

import math

import cicada

TWO_PI = 2.0 * math.pi


def signed_area(pts):
    """ported verbatim from tip_caps.py:234-241"""
    a = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        a += x1 * y2 - x2 * y1
    return 0.5 * a


def cyc_dist(a, b):
    """ported verbatim from tip_caps.py:244-251 -- absolute shortest
    angular distance."""
    d = math.fmod(a - b, TWO_PI)
    if d < 0.0:
        d += TWO_PI
    if d > math.pi:
        d = TWO_PI - d
    return d


def tri_corners(ax, ay, ux, uy, radius, elong, extra_rad, ccw):
    """ported verbatim from tip_caps.py:254-267 -- triangle corner XYs:
    nose along (ux, uy), then the other two corners at +-120 deg
    following the base winding sense."""
    rot = math.atan2(uy, ux) + extra_rad
    cr, sr = math.cos(rot), math.sin(rot)
    sense = 1.0 if ccw else -1.0
    out = []
    for k in range(3):
        phi = sense * k * TWO_PI / 3.0
        lx = radius * math.cos(phi) * elong
        ly = radius * math.sin(phi)
        out.append((ax + cr * lx - sr * ly, ay + sr * lx + cr * ly))
    return out


def cap_points(base_pts, ax, ay, ux, uy, radius, elong=1.0, extra_rad=0.0):
    """ported verbatim from tip_caps.py:270-326 -- EDGE mode (three base
    corners snap to the triangle corners, the rest spread along the
    edges). Returns (pts_xy, note); pts_xy is None on degenerate input."""
    n = len(base_pts)
    if n < 3:
        return None, "too few corners"
    ccw = signed_area(base_pts) > 0.0
    corners = tri_corners(ax, ay, ux, uy, radius, elong, extra_rad, ccw)
    thetas = [math.atan2(c[1] - ay, c[0] - ax) for c in corners]
    alphas = []
    for (px, py) in base_pts:
        dx, dy = px - ax, py - ay
        if dx * dx + dy * dy < 1e-18:
            alphas.append(0.0)
        else:
            alphas.append(math.atan2(dy, dx))

    best = None
    for i0 in range(n):
        d0 = cyc_dist(alphas[i0], thetas[0])
        for i1 in range(n):
            if i1 == i0:
                continue
            g1 = (i1 - i0) % n
            d01 = d0 + cyc_dist(alphas[i1], thetas[1])
            for i2 in range(n):
                if i2 == i0 or i2 == i1:
                    continue
                g2 = (i2 - i0) % n
                if not (0 < g1 < g2):
                    continue
                score = d01 + cyc_dist(alphas[i2], thetas[2])
                if best is None or score < best[0]:
                    best = (score, i0, i1, i2)
    if best is None:
        return None, "no snap triple"
    _s, i0, i1, i2 = best

    out = [None] * n
    snap = (i0, i1, i2)
    for k in range(3):
        ia = snap[k]
        ib = snap[(k + 1) % 3]
        ca = corners[k]
        cb = corners[(k + 1) % 3]
        out[ia] = ca
        gap = (ib - ia) % n
        for step in range(1, gap):
            f = step / float(gap)
            j = (ia + step) % n
            out[j] = (ca[0] + f * (cb[0] - ca[0]),
                      ca[1] + f * (cb[1] - ca[1]))
    return out, None


def cap_points_corners(base_pts, ax, ay, ux, uy, radius, elong=1.0,
                       extra_rad=0.0, fuse=0.03):
    """ported verbatim from tip_caps.py:329-394 -- CORNER mode: contiguous
    runs of base corners collapse onto the three triangle corners, run
    sizes balanced with remainders to the nose first (6 -> 2/2/2,
    5 -> 2/2/1, 7 -> 3/2/2), rotation offset brute-forced; within a run
    vertices fuse to within `fuse` of the corner along the adjacent
    edges so every segment stays nonzero."""
    n = len(base_pts)
    if n < 3:
        return None, "too few corners"
    ccw = signed_area(base_pts) > 0.0
    corners = tri_corners(ax, ay, ux, uy, radius, elong, extra_rad, ccw)
    thetas = [math.atan2(c[1] - ay, c[0] - ax) for c in corners]
    alphas = [math.atan2(py - ay, px - ax) for (px, py) in base_pts]
    sizes = [n // 3 + (1 if k < n % 3 else 0) for k in range(3)]

    best = None
    for r in range(n):
        score = 0.0
        idx = r
        for k in range(3):
            for _ in range(sizes[k]):
                score += cyc_dist(alphas[idx % n], thetas[k])
                idx += 1
        if best is None or score < best[0]:
            best = (score, r)
    r = best[1]

    elens = []
    for k in range(3):
        c1, c2 = corners[k], corners[(k + 1) % 3]
        elens.append(math.hypot(c2[0] - c1[0], c2[1] - c1[1]))
    if min(elens) < 1e-9:
        return None, "degenerate triangle"

    out = [None] * n
    idx = r
    for k in range(3):
        m = sizes[k]
        ck = corners[k]
        cin = corners[(k + 2) % 3]
        cout = corners[(k + 1) % 3]
        e_in = elens[(k + 2) % 3]
        e_out = elens[k]
        pre = (m - 1) // 2
        post = m - 1 - pre
        f_in = min(fuse, 0.1 * e_in / max(pre, 1))
        f_out = min(fuse, 0.1 * e_out / max(post, 1))
        for j in range(m):
            if j < pre:
                d = f_in * (pre - j) / e_in
                px = ck[0] + (cin[0] - ck[0]) * d
                py = ck[1] + (cin[1] - ck[1]) * d
            elif j == pre:
                px, py = ck
            else:
                d = f_out * (j - pre) / e_out
                px = ck[0] + (cout[0] - ck[0]) * d
                py = ck[1] + (cout[1] - ck[1]) * d
            out[idx % n] = (px, py)
            idx += 1
    return out, None




def _cell_points(cell):
    """A cell arrives either as a closed curve from the engine (a
    cicada.Polyline-like object with `.points`, the Voronoi cell after the
    per-cell shrink) or as a plain list of points (offline tests, the
    production-layout lists). Both become a list of 3-tuples."""
    pts = getattr(cell, "points", None)
    if pts is None:
        pts = cell
    return [(float(p[0]), float(p[1]), float(p[2]) if len(p) > 2 else 0.0) for p in pts]


def outline_xy(cell):
    """Cell polygon -> [(x, y)] with a duplicated closing vertex dropped
    (tip_caps.py:224-227 outline_pts tail)."""
    pts = [(p[0], p[1]) for p in _cell_points(cell)]
    if len(pts) >= 2 and math.hypot(pts[-1][0] - pts[0][0], pts[-1][1] - pts[0][1]) < 1e-9:
        pts = pts[:-1]
    return pts


@cicada.node(
    title="Tip Caps",
    description="field-aligned triangular tip-cap polygons with the base cell's vertex count and seam (ported tip_caps.py, corner-snap mode).",
)
def tip_caps(
    cells: "[Closed<Curve>]",
    centroids: "[Point]",
    directions: "[Vector]",
    lean_lengths: "[Number]",
    heights: "[Number]",
    tip_radius: "Number" = 1.8,
    elongate: "Number" = 1.0,
    rotate_deg: "Number" = 0.0,
    corner_snap: "Boolean" = True,
    fuse: "Number" = 0.03,
    cell_scale: "Number" = 0.0,
) -> "[[Point]]":
    n = len(cells)
    if not (len(centroids) == len(directions) == len(lean_lengths) == len(heights) == n):
        raise ValueError(
            "tip_caps: list lengths differ (cells %d, centroids %d, directions %d, "
            "lean_lengths %d, heights %d)" % (n, len(centroids), len(directions),
                                                len(lean_lengths), len(heights)))
    if tip_radius <= 0.0:
        raise ValueError("tip_caps: tip_radius must be positive, got %r" % tip_radius)
    if elongate <= 0.0:
        raise ValueError("tip_caps: elongate must be positive, got %r" % elongate)
    rot_rad = math.radians(rotate_deg)
    caps = []
    for i in range(n):
        cx, cy, cz = centroids[i]
        ux, uy = directions[i][0], directions[i][1]
        if math.hypot(ux, uy) < 1e-9:
            ux, uy = 0.0, 1.0  # tip_caps.py:437-439: degenerate direction -> +Y
        ax = cx + ux * lean_lengths[i]
        ay = cy + uy * lean_lengths[i]
        az = cz + heights[i]
        pts = outline_xy(cells[i])
        if len(pts) < 3:
            raise ValueError("tip_caps: cell %d has %d distinct vertices (< 3)" % (i, len(pts)))
        if cell_scale > 0.0:
            # production 1.4.1 cap: the cell scaled about its centroid by
            # cell_scale, translated to the apex (centroid preserved)
            caps.append([(ax + cell_scale * (px - cx), ay + cell_scale * (py - cy), az) for (px, py) in pts])
            continue
        if corner_snap:
            cap, note = cap_points_corners(pts, ax, ay, ux, uy, tip_radius, elongate, rot_rad, fuse)
        else:
            cap, note = cap_points(pts, ax, ay, ux, uy, tip_radius, elongate, rot_rad)
        if cap is None:
            raise ValueError("tip_caps: part %d: %s" % (i, note))
        caps.append([(p[0], p[1], az) for p in cap])
    return caps
