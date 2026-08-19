# Wall corpus: part IDs, deboss placement, ghost outlines and the board
# engraving text (stage 6, docs/15).
#
# Ported from two wall-repo scripts:
#   labels.py          -- zones + banded-reading-order IDs (zone_of,
#                         assign_ordinals), the deboss placement ladder
#                         (try_place / force_place and helpers), the
#                         single-stroke GLYPHS font whose metrics size the
#                         deboss block, coords_mm, equiv_dia, unit_xy, the
#                         75%-scaled ghost outlines.
#   board_final_dxf.py -- the production board TEXT layer: single-line
#                         part IDs in the connected stroke FONT, fitted
#                         inside each cell with place_id (poly_centroid,
#                         signed_inside, seg_point_dist, id_strokes).
#                         (The shipped board_postprocessed.dxf was made with
#                         TEXT regeneration ON, so labels.py's own board
#                         strokes never reached the CNC file.)
# Pure cores ported verbatim (module constants threaded through a params
# object -- marked 'adapted'); the Rhino/GH plumbing is not ported.
#
# Geometry conventions (stage-6 contract): model mm, z up, parts stand on
# z = 0 with the cell as the base. The deboss is cut by Rust `text_solids`
# from the plane returned here: origin on the base face at z = +1.0, x =
# the text baseline, y flipped so the plane normal (x cross y) points -z;
# with depth 2.0 the cutter spans z in [-1, +1] -- a 1 mm deep deboss
# that reads correctly from BELOW (production MirrorPartText=True).
#
# DEVIATIONS from production (declared):
#   * deboss font: production extruded Arial Black outlines; Cicada bundles
#     exactly one font, DejaVu Sans Bold (contract section 2). The block is
#     placed by the SAME ported ladder; `deboss_size` shrinks the cap
#     height so the DejaVu text (advance widths embedded below) stays
#     inside the ladder's stroke-font block; lines are left-aligned from
#     one origin (text_solids lays out left-aligned) instead of centered
#     per line.
#   * ghosts / board strokes are returned in MODEL coordinates (z = 0);
#     export_dxf applies the physical-board datum shift.

import math

import cicada

# ============================================================
# labels.py: single-stroke font (board engraving preview; here it only
# sizes the deboss block). Glyphs live in a 0..0.6 x 0..1 box, cap height
# 1.0; advance 0.85. Each glyph = list of polylines.
# ported verbatim from labels.py:480-529
# ============================================================

GLYPHS = {
    "0": [[(0.15, 0), (0.45, 0), (0.6, 0.2), (0.6, 0.8), (0.45, 1), (0.15, 1), (0, 0.8), (0, 0.2), (0.15, 0)]],
    "1": [[(0.15, 0.75), (0.3, 1), (0.3, 0)], [(0.1, 0), (0.5, 0)]],
    "2": [[(0, 0.75), (0.1, 0.95), (0.3, 1), (0.5, 0.95), (0.6, 0.75), (0.6, 0.6), (0, 0.15), (0, 0), (0.6, 0)]],
    "3": [[(0, 0.85), (0.15, 1), (0.45, 1), (0.6, 0.85), (0.6, 0.65), (0.45, 0.5), (0.2, 0.5)],
          [(0.45, 0.5), (0.6, 0.35), (0.6, 0.15), (0.45, 0), (0.15, 0), (0, 0.15)]],
    "4": [[(0.6, 0.3), (0, 0.3), (0.45, 1), (0.45, 0)]],
    "5": [[(0.6, 1), (0, 1), (0, 0.55), (0.4, 0.55), (0.6, 0.4), (0.6, 0.15), (0.4, 0), (0.1, 0), (0, 0.1)]],
    "6": [[(0.5, 1), (0.15, 0.55), (0, 0.3), (0, 0.15), (0.15, 0), (0.45, 0), (0.6, 0.15), (0.6, 0.3),
           (0.45, 0.45), (0.15, 0.45), (0, 0.3)]],
    "7": [[(0, 1), (0.6, 1), (0.2, 0)]],
    "8": [[(0.3, 0.5), (0.1, 0.6), (0.05, 0.8), (0.15, 1), (0.45, 1), (0.55, 0.8), (0.5, 0.6), (0.3, 0.5),
           (0.1, 0.4), (0.05, 0.2), (0.15, 0), (0.45, 0), (0.55, 0.2), (0.5, 0.4), (0.3, 0.5)]],
    "9": [[(0.1, 0), (0.45, 0.55), (0.6, 0.7), (0.6, 0.85), (0.45, 1), (0.15, 1), (0, 0.85), (0, 0.7),
           (0.15, 0.55), (0.45, 0.55), (0.6, 0.7)]],
    "A": [[(0, 0), (0.3, 1), (0.6, 0)], [(0.12, 0.4), (0.48, 0.4)]],
    "B": [[(0, 0), (0, 1), (0.45, 1), (0.6, 0.85), (0.6, 0.65), (0.45, 0.5), (0, 0.5)],
          [(0.45, 0.5), (0.6, 0.35), (0.6, 0.15), (0.45, 0), (0, 0)]],
    "C": [[(0.6, 0.85), (0.45, 1), (0.15, 1), (0, 0.85), (0, 0.15), (0.15, 0), (0.45, 0), (0.6, 0.15)]],
    "D": [[(0, 0), (0, 1), (0.4, 1), (0.6, 0.8), (0.6, 0.2), (0.4, 0), (0, 0)]],
    "E": [[(0.6, 1), (0, 1), (0, 0), (0.6, 0)], [(0, 0.5), (0.45, 0.5)]],
    "F": [[(0.6, 1), (0, 1), (0, 0)], [(0, 0.5), (0.45, 0.5)]],
    "G": [[(0.6, 0.85), (0.45, 1), (0.15, 1), (0, 0.85), (0, 0.15), (0.15, 0), (0.45, 0), (0.6, 0.15),
           (0.6, 0.4), (0.35, 0.4)]],
    "H": [[(0, 0), (0, 1)], [(0.6, 0), (0.6, 1)], [(0, 0.5), (0.6, 0.5)]],
    "I": [[(0.3, 0), (0.3, 1)], [(0.1, 1), (0.5, 1)], [(0.1, 0), (0.5, 0)]],
    "J": [[(0.6, 1), (0.6, 0.15), (0.45, 0), (0.15, 0), (0, 0.15)]],
    "K": [[(0, 0), (0, 1)], [(0.6, 1), (0, 0.4)], [(0.25, 0.55), (0.6, 0)]],
    "L": [[(0, 1), (0, 0), (0.6, 0)]],
    "M": [[(0, 0), (0, 1), (0.3, 0.5), (0.6, 1), (0.6, 0)]],
    "N": [[(0, 0), (0, 1), (0.6, 0), (0.6, 1)]],
    "O": [[(0.15, 0), (0.45, 0), (0.6, 0.2), (0.6, 0.8), (0.45, 1), (0.15, 1), (0, 0.8), (0, 0.2), (0.15, 0)]],
    "P": [[(0, 0), (0, 1), (0.45, 1), (0.6, 0.85), (0.6, 0.6), (0.45, 0.45), (0, 0.45)]],
    "Q": [[(0.15, 0), (0.45, 0), (0.6, 0.2), (0.6, 0.8), (0.45, 1), (0.15, 1), (0, 0.8), (0, 0.2), (0.15, 0)],
          [(0.38, 0.22), (0.6, 0)]],
    "R": [[(0, 0), (0, 1), (0.45, 1), (0.6, 0.85), (0.6, 0.6), (0.45, 0.45), (0, 0.45)],
          [(0.3, 0.45), (0.6, 0)]],
    "S": [[(0.6, 0.85), (0.45, 1), (0.15, 1), (0, 0.85), (0, 0.65), (0.15, 0.5), (0.45, 0.5), (0.6, 0.35),
           (0.6, 0.15), (0.45, 0), (0.15, 0), (0, 0.15)]],
    "T": [[(0, 1), (0.6, 1)], [(0.3, 1), (0.3, 0)]],
    "U": [[(0, 1), (0, 0.15), (0.15, 0), (0.45, 0), (0.6, 0.15), (0.6, 1)]],
    "V": [[(0, 1), (0.3, 0), (0.6, 1)]],
    "W": [[(0, 1), (0.15, 0), (0.3, 0.5), (0.45, 0), (0.6, 1)]],
    "X": [[(0, 0), (0.6, 1)], [(0, 1), (0.6, 0)]],
    "Y": [[(0, 1), (0.3, 0.5), (0.6, 1)], [(0.3, 0.5), (0.3, 0)]],
    "Z": [[(0, 1), (0.6, 1), (0, 0), (0.6, 0)]],
    "-": [[(0.1, 0.5), (0.5, 0.5)]],
    " ": [],
}
ADVANCE = 0.85
LINE_GAP = 1.35  # stacked-line spacing factor x text height (labels.py:441)


def line_width(text, height):
    """ported verbatim from labels.py:532-535"""
    if not text:
        return 0.0
    return (len(text) * ADVANCE - (ADVANCE - 0.6)) * height


def layout_lines(lines, height):
    """ported verbatim from labels.py:538-558 -- stroke polylines for
    stacked `lines` (first = top), each line centered horizontally,
    block centered on origin. Returns (strokes, block_w, block_h)."""
    n_l = len(lines)
    block_h = ((n_l - 1) * LINE_GAP + 1.0) * height
    block_w = max(line_width(s, height) for s in lines) if n_l else 0.0
    strokes = []
    for k, s in enumerate(lines):
        w = line_width(s, height)
        ox = -0.5 * w
        oy = 0.5 * block_h - (k * LINE_GAP + 1.0) * height
        x = 0.0
        for ch in s:
            glyph = GLYPHS.get(ch.upper())
            if glyph is None:
                glyph = GLYPHS.get("-")
            for pl in glyph:
                strokes.append([(ox + x + px * height, oy + py * height) for (px, py) in pl])
            x += ADVANCE * height
    return strokes, block_w, block_h


# ============================================================
# labels.py: IDs, zones, placement (pure)
# ============================================================

class LabelParams(object):
    """adapted: labels.py module-level tuning constants (labels.py:406-442)
    as one object threaded through the ported functions. mm units (the
    corpus model is mm, UnitScale 1)."""

    def __init__(self, text_height=5.0, min_text_height=2.5, edge_margin=2.0,
                 edge_margin_min=1.0, pin_spacing=12.0, pin_clearance=3.7,
                 zone_cols=3, zone_rows=3, zone_letters="ABCDEFGHIJKLMNOPQRSTUVWXYZ"):
        self.MM = 1.0
        self.TEXT_H = float(text_height)
        self.MIN_TEXT_H = float(min_text_height)
        self.ABS_MIN_H = 1.8
        self.EDGE_MARGIN = float(edge_margin)
        self.EDGE_FLOOR = float(edge_margin_min)
        if self.EDGE_FLOOR > self.EDGE_MARGIN:
            self.EDGE_FLOOR = self.EDGE_MARGIN
        self.PIN_SPACING = float(pin_spacing)
        self.PIN_CLEAR = float(pin_clearance)
        self.ZONE_COLS = min(max(int(zone_cols), 1), 9)
        self.ZONE_ROWS = min(max(int(zone_rows), 1), 9)
        self.ZONE_LETTERS = str(zone_letters)


def coords_mm(x_model, y_model, x0, y0, MM=1.0):
    """ported verbatim from labels.py:565-572"""
    xm = int(round((x_model - x0) / MM))
    ym = int(round((y_model - y0) / MM))
    if xm < 0:
        xm = 0
    if ym < 0:
        ym = 0
    return xm, ym


def col_of(x, seams, P):
    """ported verbatim from labels.py:575-582 (adapted: ZONE_COLS from P)"""
    c = 0
    for s in seams:
        if x > s:
            c += 1
    if c > P.ZONE_COLS - 1:
        c = P.ZONE_COLS - 1
    return c


def row_top_of(y, row_bounds, P):
    """ported verbatim from labels.py:585-592 (adapted: ZONE_ROWS from P)"""
    rb = 0
    for b in row_bounds:
        if y > b:
            rb += 1
    if rb > P.ZONE_ROWS - 1:
        rb = P.ZONE_ROWS - 1
    return P.ZONE_ROWS - 1 - rb


def zone_of(x, y, seams, row_bounds, P):
    """ported verbatim from labels.py:595-599 (adapted: P)"""
    idx = col_of(x, seams, P) * P.ZONE_ROWS + row_top_of(y, row_bounds, P)
    if idx >= len(P.ZONE_LETTERS):
        return "Z"
    return P.ZONE_LETTERS[idx]


def assign_ordinals(cents, zones):
    """ported verbatim from labels.py:602-643 -- 1-based per-zone part
    numbers in BANDED READING ORDER (R = round(sqrt(N * zh / zw)) bands
    top-down, left-to-right within a band; ties x, then higher y, then
    pipeline index)."""
    by_zone = {}
    for i, z in enumerate(zones):
        by_zone.setdefault(z, []).append(i)
    out = [0] * len(zones)
    for z in by_zone:
        idxs = by_zone[z]
        nz = len(idxs)
        zy1 = max(cents[i][1] for i in idxs)
        zy0 = min(cents[i][1] for i in idxs)
        zx1 = max(cents[i][0] for i in idxs)
        zx0 = min(cents[i][0] for i in idxs)
        zh = max(zy1 - zy0, 1e-9)
        zw = max(zx1 - zx0, 1e-9)
        rows = int(round(math.sqrt(nz * zh / zw)))
        if rows < 1:
            rows = 1
        if rows > nz:
            rows = nz
        band = zh / rows

        def band_key(i, _zy1=zy1, _band=band, _rows=rows):
            r = int((_zy1 - cents[i][1]) / _band)
            if r > _rows - 1:
                r = _rows - 1
            return (r, cents[i][0], -cents[i][1], i)

        for k, i in enumerate(sorted(idxs, key=band_key)):
            out[i] = k + 1
    return out


def equiv_dia(area):
    """ported verbatim from labels.py:646-649"""
    if area is None or area <= 0.0:
        return 0.0
    return 2.0 * math.sqrt(area / math.pi)


def unit_xy(dx, dy):
    """ported verbatim from labels.py:652-656 (degenerate -> +X)"""
    L = math.hypot(dx, dy)
    if L < 1e-9:
        return 1.0, 0.0
    return dx / L, dy / L


def rect_clears_circle(cx, cy, w, h, px, py, r):
    """ported verbatim from labels.py:659-662"""
    qx = max(abs(px - cx) - 0.5 * w, 0.0)
    qy = max(abs(py - cy) - 0.5 * h, 0.0)
    return math.hypot(qx, qy) >= r


def block_dims(lines, h):
    """ported verbatim from labels.py:665-668"""
    bw = max(line_width(s, h) for s in lines)
    bh = ((len(lines) - 1) * LINE_GAP + 1.0) * h
    return bw, bh


def fit_scale(w, h, target_w, target_h):
    """ported verbatim from labels.py:671-679"""
    if h <= 1e-12 or w <= 1e-12:
        return 1.0
    s = target_h / h
    if w * s > target_w:
        s = target_w / w
    return s


def clean_poly(pts):
    """ported verbatim from labels.py:716-724"""
    if pts and len(pts) >= 2:
        (x1, y1), (x2, y2) = pts[0], pts[-1]
        if math.hypot(x2 - x1, y2 - y1) < 1e-9:
            pts = pts[:-1]
    if pts is None or len(pts) < 3:
        return None
    return pts


def point_in_convex(px, py, poly):
    """ported verbatim from labels.py:727-741"""
    sign = 0
    m = len(poly)
    for i in range(m):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % m]
        cr = (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1)
        if abs(cr) < 1e-12:
            continue
        s = 1 if cr > 0 else -1
        if sign == 0:
            sign = s
        elif s != sign:
            return False
    return True


def rect_in_cell(tx, ty, w, h, poly, clearance, cx, cy, d_equiv):
    """ported verbatim from labels.py:744-759"""
    hw = 0.5 * w + clearance
    hh = 0.5 * h + clearance
    if poly is not None:
        for (qx, qy) in ((tx - hw, ty - hh), (tx + hw, ty - hh),
                         (tx + hw, ty + hh), (tx - hw, ty + hh)):
            if not point_in_convex(qx, qy, poly):
                return False
        return True
    if d_equiv <= 0.0:
        return True
    corner = math.hypot(hw, hh)
    return math.hypot(tx - cx, ty - cy) + corner <= 0.5 * d_equiv


def rot_pt(x, y, orient):
    """ported verbatim from labels.py:762-766"""
    if orient == 0:
        return (x, y)
    return (-y, x)


def eff_dims(bw, bh, orient):
    """ported verbatim from labels.py:769-772"""
    if orient == 0:
        return bw, bh
    return bh, bw


def inside_depth(px, py, poly):
    """ported verbatim from labels.py:775-796"""
    m = len(poly)
    area2 = 0.0
    for i in range(m):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % m]
        area2 += x1 * y2 - x2 * y1
    sgn = 1.0 if area2 > 0 else -1.0
    best = None
    for i in range(m):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % m]
        ex, ey = x2 - x1, y2 - y1
        L = math.hypot(ex, ey)
        if L < 1e-12:
            continue
        nx, ny = -ey * sgn / L, ex * sgn / L
        dist = (px - x1) * nx + (py - y1) * ny
        best = dist if best is None else min(best, dist)
    return best if best is not None else 0.0


def rect_score(tx, ty, w, h, poly, clearance, cx, cy, d_equiv):
    """ported verbatim from labels.py:799-810"""
    hw, hh = 0.5 * w, 0.5 * h
    corners = ((tx - hw, ty - hh), (tx + hw, ty - hh),
               (tx + hw, ty + hh), (tx - hw, ty + hh))
    if poly is not None:
        return min(inside_depth(qx, qy, poly) for (qx, qy) in corners) - clearance
    if d_equiv <= 0.0:
        return 0.0
    corner = math.hypot(hw, hh)
    return 0.5 * d_equiv - (math.hypot(tx - cx, ty - cy) + corner) - clearance


def _pin_holes(cx, cy, d_equiv, dx, dy, P):
    """ported verbatim from labels.py:813-817 (adapted: P)"""
    off_pin = P.PIN_SPACING
    if d_equiv > 0.0:
        off_pin = min(off_pin, 0.35 * d_equiv)
    return [(cx, cy), (cx + dx * off_pin, cy + dy * off_pin)], off_pin


def _candidates(cx, cy, dx, dy, off_pin, we, he, pin_clear, MM=1.0):
    """ported verbatim from labels.py:820-833"""
    px_, py_ = -dy, dx
    bases = []
    for half in (0.5 * he, 0.5 * we):
        b = half + pin_clear
        if b not in bases:
            bases.append(b)
    for base in bases:
        for extra in (0.0, 1.5 * MM, 3.0 * MM):
            m = base + extra
            yield cx + px_ * m, cy + py_ * m
            yield cx - px_ * m, cy - py_ * m
            yield cx - dx * m, cy - dy * m
            yield cx + dx * (off_pin + m), cy + dy * (off_pin + m)


def try_place(cx, cy, d_equiv, dx, dy, lines, poly, margin, pin_clear, orients, P,
              min_h=None):
    """ported verbatim from labels.py:836-865 (adapted: P) -- one rung of
    the ladder: given margins and allowed orientations, shrink text
    height until the block fits. Returns (tx, ty, h, orient) or None."""
    if min_h is None:
        min_h = P.MIN_TEXT_H
    holes, off_pin = _pin_holes(cx, cy, d_equiv, dx, dy, P)
    h = P.TEXT_H
    if d_equiv > 0.0:
        h = min(h, 0.22 * d_equiv)
    if h < min_h:
        h = min_h
    heights = []
    hh = h
    while hh > min_h + 1e-9:
        heights.append(hh)
        hh *= 0.85
    heights.append(min_h)  # the floor is always tried exactly
    for hh in heights:
        bw, bh = block_dims(lines, hh)
        for orient in orients:
            we, he = eff_dims(bw, bh, orient)
            for (tx, ty) in _candidates(cx, cy, dx, dy, off_pin, we, he, pin_clear, P.MM):
                if not rect_in_cell(tx, ty, we, he, poly, margin, cx, cy, d_equiv):
                    continue
                if all(rect_clears_circle(tx, ty, we, he, hx, hy, pin_clear)
                       for (hx, hy) in holes):
                    return tx, ty, hh, orient
    return None


def force_place(cx, cy, d_equiv, dx, dy, lines, poly, pin_clear, P, h=None):
    """ported verbatim from labels.py:868-896 (adapted: P) -- last resort:
    the position that comes closest to honoring the EdgeMarginMin wall
    clearance while still clearing the pin holes. Returns (tx, ty, h,
    orient, violation_mm)."""
    if h is None:
        h = P.ABS_MIN_H
    holes, off_pin = _pin_holes(cx, cy, d_equiv, dx, dy, P)
    bw, bh = block_dims(lines, h)
    best = None
    for orient in (0, 90):
        we, he = eff_dims(bw, bh, orient)
        for (tx, ty) in _candidates(cx, cy, dx, dy, off_pin, we, he, pin_clear, P.MM):
            if not all(rect_clears_circle(tx, ty, we, he, hx, hy, pin_clear)
                       for (hx, hy) in holes):
                continue
            s = rect_score(tx, ty, we, he, poly, P.EDGE_FLOOR, cx, cy, d_equiv)
            if best is None or s > best[4]:
                best = (tx, ty, h, orient, s)
    if best is None:
        # cannot even clear the pins: park behind the center pin
        bwe, bhe = eff_dims(bw, bh, 0)
        mag = 0.5 * bhe + pin_clear
        best = (cx - dx * mag, cy - dy * mag, h, 0,
                rect_score(cx - dx * mag, cy - dy * mag, bwe, bhe, poly,
                           P.EDGE_FLOOR, cx, cy, d_equiv))
    tx, ty, th, orient, s = best
    violation = -s if s < 0 else 0.0
    return tx, ty, th, orient, violation


def place_label(cx, cy, d, dx, dy, lines3, lines2, lines1, poly, P):
    """ported verbatim from labels.py:1292-1366 (the six-rung ladder for
    one part; adapted: P, and returns instead of appending). Returns
    (tx, ty, th, mode, orient, lines, violation)."""
    relax_stages = []
    MM = P.MM
    for (m_mu, pc_mm) in ((0.5 * (P.EDGE_MARGIN + P.EDGE_FLOOR), P.PIN_CLEAR / MM),
                          (P.EDGE_FLOOR, P.PIN_CLEAR / MM),
                          (P.EDGE_FLOOR, 3.0),
                          (P.EDGE_FLOOR, 2.4)):
        pc_mu = pc_mm * MM
        if m_mu < P.EDGE_MARGIN - 1e-9 or pc_mu < P.PIN_CLEAR - 1e-9:
            relax_stages.append((m_mu, pc_mu))

    mode = None
    lines = lines3
    orient = 0
    tx = ty = th = 0.0
    viol = 0.0
    # rungs 1-3: full target margin, decreasing line count
    for (cand_lines, tag, ors) in ((lines3, "ok3", (0, 90)),
                                   (lines2, "ok2", (0, 90)),
                                   (lines1, "ok1", (0, 90))):
        res = try_place(cx, cy, d, dx, dy, cand_lines, poly,
                        P.EDGE_MARGIN, P.PIN_CLEAR, ors, P)
        if res is not None:
            tx, ty, th, orient = res
            mode = tag
            lines = cand_lines
            break
    # rung 4: relaxed margins/keep-out, 2-line then 1-line
    if mode is None:
        for (m_mu, pc_mu) in relax_stages:
            for cand_lines in (lines2, lines1):
                res = try_place(cx, cy, d, dx, dy, cand_lines, poly,
                                m_mu, pc_mu, (0, 90), P)
                if res is not None:
                    tx, ty, th, orient = res
                    mode = "relaxed"
                    lines = cand_lines
                    break
            if mode is not None:
                break
    # rung 5: sub-minimum text height at the hard floor margins
    if mode is None:
        for cand_lines in (lines2, lines1):
            res = try_place(cx, cy, d, dx, dy, cand_lines, poly,
                            P.EDGE_FLOOR, 2.4 * MM, (0, 90), P, min_h=P.ABS_MIN_H)
            if res is not None:
                tx, ty, th, orient = res
                mode = "tiny"
                lines = cand_lines
                break
    # rung 6: forced least-violation placement (should never happen)
    if mode is None:
        tx, ty, th, orient, viol = force_place(cx, cy, d, dx, dy,
                                               lines1, poly, 2.4 * MM, P)
        mode = "forced"
        lines = lines1
    return tx, ty, th, mode, orient, lines, viol


# ============================================================
# board_final_dxf.py: connected single-stroke font (plunge-minimal)
# Each glyph is 1-2 polylines in a 1.0-tall em box (y=0 baseline, y=1
# cap). Entries: (advance_width, [(closed, pts), ...]).
# ported verbatim from board_final_dxf.py:433-483
# ============================================================

FONT = {
    "0": (0.55, [(True, [(0.12, 0.0), (0.43, 0.0), (0.55, 0.15), (0.55, 0.85),
                         (0.43, 1.0), (0.12, 1.0), (0.0, 0.85), (0.0, 0.15)])]),
    "1": (0.35, [(False, [(0.02, 0.75), (0.2, 1.0), (0.2, 0.0)])]),
    "2": (0.55, [(False, [(0.0, 0.82), (0.13, 0.98), (0.42, 0.98), (0.55, 0.82),
                          (0.55, 0.6), (0.0, 0.12), (0.0, 0.0), (0.55, 0.0)])]),
    "3": (0.55, [(False, [(0.0, 0.86), (0.14, 1.0), (0.41, 1.0), (0.55, 0.86),
                          (0.55, 0.63), (0.41, 0.5), (0.22, 0.5), (0.41, 0.5),
                          (0.55, 0.37), (0.55, 0.14), (0.41, 0.0), (0.14, 0.0),
                          (0.0, 0.14)])]),
    "4": (0.55, [(False, [(0.42, 0.0), (0.42, 1.0), (0.0, 0.32), (0.55, 0.32)])]),
    "5": (0.55, [(False, [(0.55, 1.0), (0.0, 1.0), (0.0, 0.55), (0.34, 0.58),
                          (0.51, 0.46), (0.55, 0.28), (0.46, 0.06), (0.16, 0.0),
                          (0.0, 0.12)])]),
    "6": (0.55, [(False, [(0.45, 1.0), (0.13, 0.55), (0.0, 0.28), (0.08, 0.06),
                          (0.3, 0.0), (0.49, 0.08), (0.55, 0.3), (0.45, 0.5),
                          (0.2, 0.53), (0.03, 0.38)])]),
    "7": (0.55, [(False, [(0.0, 1.0), (0.55, 1.0), (0.18, 0.0)])]),
    "8": (0.55, [(True, [(0.275, 0.5), (0.08, 0.65), (0.08, 0.88), (0.275, 1.0),
                         (0.47, 0.88), (0.47, 0.65), (0.275, 0.5), (0.08, 0.35),
                         (0.08, 0.12), (0.275, 0.0), (0.47, 0.12), (0.47, 0.35)])]),
    "9": (0.55, [(False, [(0.1, 0.0), (0.42, 0.45), (0.55, 0.72), (0.47, 0.94),
                          (0.25, 1.0), (0.06, 0.92), (0.0, 0.7), (0.1, 0.5),
                          (0.35, 0.47), (0.52, 0.62)])]),
    "A": (0.55, [(False, [(0.0, 0.0), (0.275, 1.0), (0.55, 0.0), (0.44, 0.4),
                          (0.11, 0.4)])]),
    "B": (0.55, [(False, [(0.0, 0.0), (0.0, 1.0), (0.4, 1.0), (0.55, 0.85),
                          (0.55, 0.65), (0.4, 0.5), (0.0, 0.5), (0.4, 0.5),
                          (0.55, 0.35), (0.55, 0.15), (0.4, 0.0), (0.0, 0.0)])]),
    "C": (0.55, [(False, [(0.55, 0.85), (0.4, 1.0), (0.15, 1.0), (0.0, 0.85),
                          (0.0, 0.15), (0.15, 0.0), (0.4, 0.0), (0.55, 0.15)])]),
    "D": (0.55, [(True, [(0.0, 0.0), (0.0, 1.0), (0.35, 1.0), (0.55, 0.8),
                         (0.55, 0.2), (0.35, 0.0)])]),
    "E": (0.5, [(False, [(0.5, 1.0), (0.0, 1.0), (0.0, 0.5), (0.36, 0.5),
                         (0.0, 0.5), (0.0, 0.0), (0.5, 0.0)])]),
    "F": (0.5, [(False, [(0.5, 1.0), (0.0, 1.0), (0.0, 0.5), (0.36, 0.5),
                         (0.0, 0.5), (0.0, 0.0)])]),
    "G": (0.55, [(False, [(0.55, 0.85), (0.4, 1.0), (0.15, 1.0), (0.0, 0.85),
                          (0.0, 0.15), (0.15, 0.0), (0.4, 0.0), (0.55, 0.15),
                          (0.55, 0.45), (0.3, 0.45)])]),
    "H": (0.55, [(False, [(0.0, 1.0), (0.0, 0.0), (0.0, 0.5), (0.55, 0.5),
                          (0.55, 1.0), (0.55, 0.0)])]),
    "I": (0.5, [(False, [(0.05, 1.0), (0.45, 1.0), (0.25, 1.0), (0.25, 0.0),
                         (0.05, 0.0), (0.45, 0.0)])]),
    "J": (0.55, [(False, [(0.55, 1.0), (0.55, 0.15), (0.4, 0.0), (0.15, 0.0),
                          (0.0, 0.15)])]),
    "K": (0.55, [(False, [(0.0, 1.0), (0.0, 0.0), (0.0, 0.45), (0.5, 1.0),
                          (0.0, 0.45), (0.5, 0.0)])]),
    "L": (0.5, [(False, [(0.0, 1.0), (0.0, 0.0), (0.5, 0.0)])]),
}
FONT_SPACING = 0.22  # advance gap between glyphs, em


def id_strokes(text, cx, cy, h, unknown):
    """ported verbatim from board_final_dxf.py:486-505 -- stroke polylines
    for `text`, cap height h, CENTERED at (cx, cy), horizontal. Unknown
    characters render as a rectangle and are recorded in `unknown`."""
    widths = []
    for ch in text:
        widths.append(FONT.get(ch, (0.55, None))[0])
    total_w = (sum(widths) + FONT_SPACING * (len(text) - 1)) * h
    x = cx - 0.5 * total_w
    y0 = cy - 0.5 * h
    out = []
    for k, ch in enumerate(text):
        w, strokes = FONT.get(ch, (0.55, None))
        if strokes is None:
            unknown.add(ch)
            strokes = [(True, [(0.0, 0.0), (0.55, 0.0), (0.55, 1.0), (0.0, 1.0)])]
        for (closed, pts) in strokes:
            out.append((closed, [(x + p[0] * h, y0 + p[1] * h) for p in pts]))
        x += (w + FONT_SPACING) * h
    return out, total_w


def poly_centroid(pts):
    """ported verbatim from board_final_dxf.py:510-522"""
    a2 = cx = cy = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        cr = x1 * y2 - x2 * y1
        a2 += cr
        cx += (x1 + x2) * cr
        cy += (y1 + y2) * cr
    if abs(a2) < 1e-12:
        return (sum(p[0] for p in pts) / m, sum(p[1] for p in pts) / m)
    return (cx / (3.0 * a2), cy / (3.0 * a2))


def signed_inside(pts, px, py):
    """ported verbatim from board_final_dxf.py:525-547 -- distance to the
    polygon boundary, positive inside (convex bases)."""
    m = len(pts)
    best = 1e18
    inside = False
    j = m - 1
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[j]
        if (y1 > py) != (y2 > py):
            xint = x1 + (py - y1) * (x2 - x1) / (y2 - y1)
            if px < xint:
                inside = not inside
        vx, vy = x2 - x1, y2 - y1
        L2 = vx * vx + vy * vy
        t = 0.0 if L2 < 1e-18 else max(0.0, min(1.0, ((px - x1) * vx + (py - y1) * vy) / L2))
        d = math.hypot(px - (x1 + t * vx), py - (y1 + t * vy))
        if d < best:
            best = d
        j = i
    return best if inside else -best


def seg_point_dist(px, py, ax, ay, bx, by):
    """ported verbatim from board_final_dxf.py:550-554"""
    vx, vy = bx - ax, by - ay
    L2 = vx * vx + vy * vy
    t = 0.0 if L2 < 1e-18 else max(0.0, min(1.0, ((px - ax) * vx + (py - ay) * vy) / L2))
    return math.hypot(px - (ax + t * vx), py - (ay + t * vy))


def place_id(text, base, pins, h0, h_min, margin, pin_clear, unknown):
    """ported verbatim from board_final_dxf.py:557-597 -- fit `text`
    inside convex polygon `base` (mm), horizontal, auto-shrinking from h0
    to h_min, dodging `pins` [(x,y)..]. Returns (strokes, height,
    forced)."""
    ccx, ccy = poly_centroid(base)
    offsets = [(0.0, 0.0), (0.0, 4.0), (0.0, -4.0), (4.0, 0.0), (-4.0, 0.0),
               (0.0, 8.0), (0.0, -8.0), (6.0, 4.0), (-6.0, 4.0),
               (6.0, -4.0), (-6.0, -4.0)]

    def ok(strokes):
        for (_c, pts) in strokes:
            for (px, py) in pts:
                if signed_inside(base, px, py) < margin:
                    return False
        for (hx, hy) in pins:
            for (_c, pts) in strokes:
                ring = pts + ([pts[0]] if _c else [])
                for k in range(len(ring) - 1):
                    if seg_point_dist(hx, hy, ring[k][0], ring[k][1],
                                      ring[k + 1][0], ring[k + 1][1]) < pin_clear:
                        return False
        return True

    h = h0
    while h >= h_min - 1e-9:
        for (ox, oy) in offsets:
            strokes, _w = id_strokes(text, ccx + ox, ccy + oy, h, unknown)
            if ok(strokes):
                return strokes, h, False
        h *= 0.85
    # floor: keep the least-bad placement at h_min
    best = None
    best_score = -1e18
    for (ox, oy) in offsets:
        strokes, _w = id_strokes(text, ccx + ox, ccy + oy, h_min, unknown)
        score = min(signed_inside(base, px, py)
                    for (_c, pts) in strokes for (px, py) in pts)
        if score > best_score:
            best_score = score
            best = strokes
    return best, h_min, True


# ============================================================
# Deboss frame for Rust text_solids (DejaVu Sans Bold, cap-height sized)
# ============================================================

# Advance widths of DejaVu Sans Bold (font units, unitsPerEm 2048) for the
# characters an ID / coordinate line can contain, read from the bundled
# font's hmtx table (DejaVuSans-Bold.ttf); the 'H' glyph's height is 1493
# units = the cap height `text_solids` sizes by. Used ONLY to estimate the
# rendered block width so the deboss stays inside the placed block.
DEJAVU_BOLD_CAP_UNITS = 1493.0
DEJAVU_BOLD_ADVANCE = {
    "0": 1425, "1": 1425, "2": 1425, "3": 1425, "4": 1425, "5": 1425, "6": 1425,
    "7": 1425, "8": 1425, "9": 1425, "A": 1585, "B": 1561, "C": 1503, "D": 1700,
    "E": 1399, "F": 1399, "G": 1681, "H": 1714, "I": 762, "J": 762, "K": 1587,
    "L": 1305, "M": 2038, "N": 1714, "O": 1741, "P": 1501, "Q": 1741, "R": 1577,
    "S": 1475, "T": 1397, "U": 1663, "V": 1585, "W": 2259, "X": 1579, "Y": 1483,
    "Z": 1485, " ": 713, "-": 850,
}


def dejavu_line_width(text, size):
    """Estimated DejaVu Sans Bold advance width of `text` at cap height
    `size` (sum of advances; the ink is slightly narrower)."""
    total = 0
    for ch in text:
        adv = DEJAVU_BOLD_ADVANCE.get(ch.upper())
        if adv is None:
            raise ValueError("deboss text %r: no DejaVu metric for %r" % (text, ch))
        total += adv
    return total * size / DEJAVU_BOLD_CAP_UNITS


def deboss_frame(lines, tx, ty, th, orient, bw, bh, z_face=0.0, under=1.0,
                 line_gap=LINE_GAP, mirror=True):
    """The text_solids frame for one part's deboss.

    Reproduces labels.py build_deboss_font's transform chain (labels.py:
    1116-1124): the glyph block is uniformly scaled to fit the ladder's
    (bw, bh) block, centered on the block, rotated by `orient`, MIRRORED
    across the X axis (y -> -y, so it reads from below), and translated to
    (tx, ty, z_face - under). Here the same chain is expressed as a plane:
    origin at z = z_face + under on the base face, x = the baseline
    direction, y flipped, so x cross y points -z and text_solids extruding
    depth 2*under downward covers z in [z_face - under, z_face + under].
    Returns (origin, xaxis, yaxis, size)."""
    n_l = len(lines)
    widest = max(dejavu_line_width(s, 1.0) for s in lines)
    size = th
    if widest > 1e-12 and widest * size > bw:
        size = bw / widest  # fit_scale: height first, capped by width
    block_w = widest * size
    block_h = ((n_l - 1) * line_gap + 1.0) * size
    # text_solids block in its own frame: first baseline at y=0 from x=0,
    # later lines stack downward -> bbox x [0, block_w], y [-(block_h-size), size]
    c_local = (0.5 * block_w, size - 0.5 * block_h)

    def M(x, y):
        rx, ry = rot_pt(x, y, orient)
        return (rx, -ry) if mirror else (rx, ry)

    mcx, mcy = M(c_local[0], c_local[1])
    ox, oy = tx - mcx, ty - mcy
    xa = M(1.0, 0.0)
    ya = M(0.0, 1.0)
    # (+ 0.0 turns the -0.0 a mirrored zero leaves into a canonical 0.0)
    return ((ox, oy, z_face + under),
            (xa[0] + 0.0, xa[1] + 0.0, 0.0), (ya[0] + 0.0, ya[1] + 0.0, 0.0), size)




def _cell_points(cell):
    """A cell arrives either as a closed curve from the engine (a
    cicada.Polyline-like object with `.points`, the Voronoi cell after the
    per-cell shrink) or as a plain list of points (offline tests, the
    production-layout lists). Both become a list of 3-tuples."""
    pts = getattr(cell, "points", None)
    if pts is None:
        pts = cell
    return [(float(p[0]), float(p[1]), float(p[2]) if len(p) > 2 else 0.0) for p in pts]


def cell_poly(cell):
    pts = [(p[0], p[1]) for p in _cell_points(cell)]
    return clean_poly(pts)


def poly_area(pts):
    a = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        a += x1 * y2 - x2 * y1
    return 0.5 * abs(a)


@cicada.node(
    title="Wall Labels",
    description="zone IDs, deboss text frames, ghost outlines and board engraving strokes for every part (ported labels.py + board_final_dxf.py).",
)
def wall_labels(
    cells: "[Closed<Curve>]",
    centroids: "[Point]",
    directions: "[Vector]",
    board_min: "Point",
    text_height: "Number" = 5.0,
    min_text_height: "Number" = 2.5,
    edge_margin: "Number" = 2.0,
    edge_margin_min: "Number" = 1.0,
    outline_scale: "Number" = 0.75,
    pin_spacing: "Number" = 12.0,
    pin_clearance: "Number" = 3.7,
    zone_cols: "Integer" = 3,
    zone_rows: "Integer" = 3,
    zone_letters: "Text" = "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    board_text_height: "Number" = 5.0,
    board_text_min_height: "Number" = 2.5,
    board_text_margin: "Number" = 1.5,
    board_pin_clear: "Number" = 2.8,
    deboss_under: "Number" = 1.0,
    ids_expected: "[Text]" = [],
) -> {
    "ids": "[Text]",
    "zones": "[Text]",
    "deboss_text": "[Text]",
    "deboss_plane": "[Plane]",
    "deboss_size": "[Number]",
    "deboss_mode": "[Text]",
    "ghosts": "[[Point]]",
    "board_strokes": "[[Point]]",
    "board_strokes_closed": "[Boolean]",
    "notes": "[Text]",
}:
    n = len(centroids)
    if not (len(cells) == len(directions) == n):
        raise ValueError("wall_labels: list lengths differ (cells %d, centroids %d, directions %d)"
                         % (len(cells), n, len(directions)))
    if n == 0:
        raise ValueError("wall_labels: no parts")
    P = LabelParams(text_height, min_text_height, edge_margin, edge_margin_min,
                    pin_spacing, pin_clearance, zone_cols, zone_rows, zone_letters)
    dat_x0, dat_y0 = float(board_min[0]), float(board_min[1])

    polys = []
    for i, cell in enumerate(cells):
        poly = cell_poly(cell)
        if poly is None:
            raise ValueError("wall_labels: cell %d has fewer than 3 distinct vertices" % i)
        polys.append(poly)
    areas = [poly_area(p) for p in polys]

    # ---- zones + IDs (labels.py:1235-1266) --------------------------
    xs = [c[0] for c in centroids]
    ys = [c[1] for c in centroids]
    x0, x1 = min(xs), max(xs)
    y0, y1 = min(ys), max(ys)
    seams = []
    if P.ZONE_COLS > 1:
        span = x1 - x0
        seams = [x0 + span * (k + 1) / float(P.ZONE_COLS) for k in range(P.ZONE_COLS - 1)]
    row_bounds = []
    if P.ZONE_ROWS > 1:
        span = y1 - y0
        row_bounds = [y0 + span * (k + 1) / float(P.ZONE_ROWS) for k in range(P.ZONE_ROWS - 1)]
    if len(P.ZONE_LETTERS) < P.ZONE_COLS * P.ZONE_ROWS:
        raise ValueError("wall_labels: zone_letters too short for a %dx%d grid"
                         % (P.ZONE_COLS, P.ZONE_ROWS))
    zones_list = [zone_of(c[0], c[1], seams, row_bounds, P) for c in centroids]
    ordinals = assign_ordinals(centroids, zones_list)
    ids = ["%s%d" % (zones_list[i], ordinals[i]) for i in range(n)]
    if ids_expected:
        if len(ids_expected) != n:
            raise ValueError("wall_labels: ids_expected has %d entries for %d parts"
                             % (len(ids_expected), n))
        bad = [(i, ids[i], ids_expected[i]) for i in range(n) if ids[i] != str(ids_expected[i])]
        if bad:
            raise ValueError(
                "wall_labels: %d IDs differ from the expected production IDs, first: %s"
                % (len(bad), ", ".join("part %d computed %s expected %s" % b for b in bad[:6])))
    if len(set(ids)) != n:
        raise ValueError("wall_labels: IDs are not unique")

    # ---- deboss placement ladder (labels.py:1301-1367) -----------------
    deboss_text = []
    deboss_plane = []
    deboss_size = []
    deboss_mode = []
    mode_counts = {"ok3": 0, "ok2": 0, "ok1": 0, "relaxed": 0, "tiny": 0, "forced": 0}
    forced_ids = []
    worst_violation = 0.0
    heights_used = []
    for i in range(n):
        cx, cy, cz = centroids[i]
        dx, dy = unit_xy(directions[i][0], directions[i][1])
        d = equiv_dia(areas[i])
        xm, ym = coords_mm(cx, cy, dat_x0, dat_y0)
        pid = ids[i]
        lines3 = [pid, "%04d" % xm, "%04d" % ym]
        lines2 = [pid, "%04d %04d" % (xm, ym)]
        lines1 = [pid]
        tx, ty, th, mode, orient, lines, viol = place_label(
            cx, cy, d, dx, dy, lines3, lines2, lines1, polys[i], P)
        mode_counts[mode] += 1
        if mode == "forced":
            forced_ids.append(pid)
            if viol > worst_violation:
                worst_violation = viol
        heights_used.append(th)
        bw, bh = block_dims(lines, th)
        origin, xa, ya, size = deboss_frame(lines, tx, ty, th, orient, bw, bh,
                                            z_face=cz, under=float(deboss_under))
        deboss_text.append("\n".join(lines))
        deboss_plane.append(cicada.Plane(origin, xa, ya))
        deboss_size.append(size)
        deboss_mode.append(mode)

    # ---- ghost outlines: cell scaled about its centroid (labels.py:1418-1421)
    ghosts = []
    for i in range(n):
        cx, cy, cz = centroids[i]
        ghosts.append([(cx + (px - cx) * outline_scale, cy + (py - cy) * outline_scale, cz)
                       for (px, py) in polys[i]])

    # ---- board TEXT: single-line IDs fitted per cell (board_final_dxf.py:874-893)
    # computed in the physical-datum mm frame exactly like production
    # (bases_mm), then returned in model coordinates.
    strokes_out = []
    closed_out = []
    unknown = set()
    board_forced = []
    h_lo, h_hi = 1e18, 0.0
    for i in range(n):
        cx, cy, cz = centroids[i]
        base_mm = [(x - dat_x0, y - dat_y0) for (x, y) in polys[i]]
        dx, dy = unit_xy(directions[i][0], directions[i][1])
        holes, _off = _pin_holes(cx, cy, equiv_dia(areas[i]), dx, dy, P)
        pins_mm = [(hx - dat_x0, hy - dat_y0) for (hx, hy) in holes]
        strokes, h_used, forced = place_id(
            ids[i], base_mm, pins_mm, float(board_text_height), float(board_text_min_height),
            float(board_text_margin), float(board_pin_clear), unknown)
        if forced:
            board_forced.append(ids[i])
        h_lo = min(h_lo, h_used)
        h_hi = max(h_hi, h_used)
        for (closed, pts) in strokes:
            strokes_out.append([(x + dat_x0, y + dat_y0, cz) for (x, y) in pts])
            closed_out.append(bool(closed))
    if unknown:
        raise ValueError("wall_labels: the board stroke FONT has no glyph for %s"
                         % sorted(unknown))

    zone_counts = {}
    for z in zones_list:
        zone_counts[z] = zone_counts.get(z, 0) + 1
    notes = [
        "Parts: %d | Zones %dx%d: %s" % (n, P.ZONE_COLS, P.ZONE_ROWS,
                                         " ".join("%s=%d" % (k, zone_counts[k]) for k in sorted(zone_counts))),
        "Deboss TextH used: %.1f-%.1f mm | 3L: %d | 2L: %d | 1L: %d | Relaxed: %d | Tiny: %d | Forced: %d"
        % (min(heights_used), max(heights_used), mode_counts["ok3"], mode_counts["ok2"],
           mode_counts["ok1"], mode_counts["relaxed"], mode_counts["tiny"], mode_counts["forced"]),
        "Datum: (%.1f, %.1f) (BOARD CORNER)" % (dat_x0, dat_y0),
        "Board TEXT: %d IDs -> %d polylines, heights %.1f-%.1f mm, %d forced%s"
        % (n, len(strokes_out), h_lo, h_hi, len(board_forced),
           (": " + ", ".join(board_forced[:8])) if board_forced else ""),
    ]
    if forced_ids:
        notes.append("!!! %d FORCED deboss labels (worst shortfall %.1f mm): %s"
                     % (len(forced_ids), worst_violation, ", ".join(forced_ids[:8])))
    return {
        "ids": ids,
        "zones": zones_list,
        "deboss_text": deboss_text,
        "deboss_plane": deboss_plane,
        "deboss_size": deboss_size,
        "deboss_mode": deboss_mode,
        "ghosts": ghosts,
        "board_strokes": strokes_out,
        "board_strokes_closed": closed_out,
        "notes": notes,
    }
