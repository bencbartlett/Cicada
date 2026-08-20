# Wall corpus: the board CNC DXF exporter (stage 6, docs/15). EFFECTFUL.
#
# Ported from the wall repo's board_final_dxf.py (T3b): dxf_document (the
# R12 writer shared with labels.py), join_open_plines, and the layer
# assembly of board_final_dxf.py:922-942 that produced the shipped
# board_postprocessed.dxf:
#   OUTLINES  75%-scaled ghost cell outlines (closed polylines)
#   TEXT      single-line part IDs in the connected stroke FONT
#             (regenerated text path: board_final_dxf.py:874-893)
#   PINHOLES  one CIRCLE per pin hole, HoleDia 3.1 -> r 1.55: 2 per part
#             (coil-captured parts skipped, ExcludeIdx) + the coil ring
#             holes
#   BOARDCUT  the finished board rectangle (0,0)-(W,H)
#   STOCK     the 97 x 49 in MDF sheet centered on the board (reference)
# Every coordinate is in mm from the PHYSICAL board corner (board_min),
# the CNC datum; %.3f. Production's writer quirk is reproduced on purpose:
# the LAYER table lists only the layers that were KEPT from the frozen
# board.dxf (OUTLINES; SEAM when present) plus PINHOLES / BOARDCUT / STOCK
# -- the regenerated TEXT layer is referenced by 3828 entities but never
# declared in the table (board_final_dxf.py:941-942), and the shipped file
# carries exactly that. Line endings are CRLF explicitly (production
# wrote in Windows text mode); the HEADER section is the bare
# "0/SECTION/2/HEADER/0/ENDSEC".
#
# join_open_plines (board_final_dxf.py:600-657) is ported for completeness
# but is OFF by default: production's TEXT regeneration path never joined
# (the FONT glyphs are already plunge-minimal single polylines).

import math
import os

import cicada

IN = 25.4  # mm per inch (board_final_dxf.py:690)


def _pipeline_dir():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_path(path):
    path = str(path)
    if os.path.isabs(path):
        return path
    return os.path.normpath(os.path.join(_pipeline_dir(), path))


def dxf_document(layers, entities):
    """ported verbatim from board_final_dxf.py:660-680 (== labels.py:903-922)
    -- R12 entities, coordinates in mm; entities = (layer, kind, data)
    with kind 'circle' (x, y, r) or 'pline' (closed, pts)."""
    L = []
    L.append("0\nSECTION\n2\nHEADER\n0\nENDSEC")
    L.append("0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n%d" % len(layers))
    for name in layers:
        L.append("0\nLAYER\n2\n%s\n70\n0\n62\n7\n6\nCONTINUOUS" % name)
    L.append("0\nENDTAB\n0\nENDSEC")
    L.append("0\nSECTION\n2\nENTITIES")
    for (layer, kind, data) in entities:
        if kind == "circle":
            x, y, r = data
            L.append("0\nCIRCLE\n8\n%s\n10\n%.3f\n20\n%.3f\n30\n0.0\n40\n%.3f" % (layer, x, y, r))
        elif kind == "pline":
            closed, pts = data
            L.append("0\nPOLYLINE\n8\n%s\n66\n1\n70\n%d" % (layer, 1 if closed else 0))
            for (x, y) in pts:
                L.append("0\nVERTEX\n8\n%s\n10\n%.3f\n20\n%.3f\n30\n0.0" % (layer, x, y))
            L.append("0\nSEQEND")
    L.append("0\nENDSEC\n0\nEOF")
    return "\n".join(L) + "\n"


def join_open_plines(entities, layer, tol=0.005):
    """ported verbatim from board_final_dxf.py:600-657 -- chain OPEN
    polylines on `layer` whose endpoints coincide (within tol) into
    continuous polylines; only degree-2 junctions join. Returns
    (new_entities, n_before, n_after)."""
    strokes = []
    others = []
    for e in entities:
        if e[0] == layer and e[1] == "pline" and not e[2][0] and len(e[2][1]) >= 2:
            strokes.append(list(e[2][1]))
        else:
            others.append(e)

    def key(p):
        return (int(round(p[0] / tol)), int(round(p[1] / tol)))

    ends = {}
    for i, pts in enumerate(strokes):
        for end in (0, 1):
            ends.setdefault(key(pts[-end]), []).append((i, end))

    used = [False] * len(strokes)
    merged = []
    for i in range(len(strokes)):
        if used[i]:
            continue
        used[i] = True
        chain = list(strokes[i])
        for direction in (1, 0):   # extend tail, then head
            while True:
                tip = chain[-1] if direction == 1 else chain[0]
                cands = [(j, e) for (j, e) in ends.get(key(tip), [])
                         if not used[j]]
                node_degree = len([1 for (j, e) in ends.get(key(tip), [])])
                if len(cands) != 1 or node_degree > 2:
                    break
                j, _e = cands[0]
                nxt = strokes[j]
                used[j] = True
                if key(nxt[0]) == key(tip):
                    seg = nxt[1:]
                elif key(nxt[-1]) == key(tip):
                    seg = list(reversed(nxt))[1:]
                else:
                    break
                if direction == 1:
                    chain.extend(seg)
                else:
                    chain = list(reversed(seg)) + chain
        closed = len(chain) > 3 and key(chain[0]) == key(chain[-1])
        if closed:
            chain = chain[:-1]
        merged.append((layer, "pline", (closed, chain)))
    return others + merged, len(strokes), len(merged)


def board_entities(ghosts, strokes, strokes_closed, holes, skip_holes, coil_holes,
                   board_min, board_max, hole_dia, stock_width_in, stock_height_in,
                   join_text=False):
    """The entity list + layer table of the shipped board_postprocessed.dxf
    (board_final_dxf.py:752-942 minus the file I/O and cross-checks that
    compared against the frozen board.dxf). All inputs in MODEL mm; the
    output coordinates are shifted to the physical-board datum. Returns
    (layers, entities, notes)."""
    dx0, dy0 = float(board_min[0]), float(board_min[1])
    W = float(board_max[0]) - dx0
    H = float(board_max[1]) - dy0
    if W <= 0.0 or H <= 0.0:
        raise ValueError("export_dxf: board_min/board_max do not span a rectangle")
    notes = []

    entities = []
    # OUTLINES (kept from the production board.dxf: labels.py:1529-1534).
    # Production's ghosts came from Rhino closed polylines whose point list
    # repeats the first vertex at the end, and that duplicate was written
    # into the file (closed flag AND a closing vertex) -- reproduced here.
    for i, g in enumerate(ghosts):
        pts = [((p[0] - dx0), (p[1] - dy0)) for p in g]
        if len(pts) < 3:
            raise ValueError("export_dxf: ghost %d has %d vertices" % (i, len(pts)))
        if math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]) > 1e-9:
            pts.append(pts[0])
        entities.append(("OUTLINES", "pline", (True, pts)))
    # TEXT (regenerated single-line IDs, board_final_dxf.py:892-893)
    if len(strokes) != len(strokes_closed):
        raise ValueError("export_dxf: %d strokes but %d closed flags" % (len(strokes), len(strokes_closed)))
    for k, s in enumerate(strokes):
        pts = [((p[0] - dx0), (p[1] - dy0)) for p in s]
        if len(pts) < 2:
            raise ValueError("export_dxf: stroke %d has %d vertices" % (k, len(pts)))
        entities.append(("TEXT", "pline", (bool(strokes_closed[k]), pts)))
    n_text_raw = len(strokes)
    if join_text and n_text_raw:
        entities, _nb, _na = join_open_plines(entities, "TEXT")

    # PINHOLES: part holes (2 per part, ExcludeIdx pairs skipped) then coil
    if len(holes) % 2 == 1:
        raise ValueError("export_dxf: holes count %d is odd (expected 2 per part)" % len(holes))
    n_parts = len(holes) // 2
    if skip_holes and len(skip_holes) != n_parts:
        raise ValueError("export_dxf: skip_holes has %d flags for %d parts (2 holes each)"
                         % (len(skip_holes), n_parts))
    drills = []
    for k, p in enumerate(holes):
        if skip_holes and skip_holes[k // 2]:
            continue
        drills.append((p[0] - dx0, p[1] - dy0))
    n_part_holes = len(drills)
    for p in coil_holes:
        drills.append((p[0] - dx0, p[1] - dy0))
    r_mm = 0.5 * float(hole_dia)
    outside = [k for k, (hx, hy) in enumerate(drills)
               if hx < r_mm or hy < r_mm or hx > W - r_mm or hy > H - r_mm]
    if outside:
        raise ValueError("export_dxf: %d drill(s) fall outside the finished board (first indices %s) "
                         "-- datum or units are wrong" % (len(outside), outside[:6]))
    close_pairs = 0
    cell = {}
    for k, (hx, hy) in enumerate(drills):
        key = (int(hx // 10.0), int(hy // 10.0))
        for ox in (-1, 0, 1):
            for oy in (-1, 0, 1):
                for j in cell.get((key[0] + ox, key[1] + oy), ()):
                    jx, jy = drills[j]
                    if math.hypot(hx - jx, hy - jy) < hole_dia:
                        close_pairs += 1
        cell.setdefault(key, []).append(k)
    if close_pairs:
        notes.append("%d drill pairs closer than one diameter (overlapping holes)" % close_pairs)
    for (hx, hy) in drills:
        entities.append(("PINHOLES", "circle", (hx, hy, r_mm)))

    # stock sheet, centered on the finished board (auto-orient)
    sw = float(stock_width_in) * IN
    sh = float(stock_height_in) * IN
    if (sw < W or sh < H) and (sh >= W and sw >= H):
        sw, sh = sh, sw
        notes.append("stock orientation swapped to fit the board")
    if sw < W or sh < H:
        notes.append("STOCK %.1fx%.1f mm is SMALLER than the finished board %.1fx%.1f mm" % (sw, sh, W, H))
    mx = 0.5 * (sw - W)
    my = 0.5 * (sh - H)
    entities.append(("BOARDCUT", "pline", (True, [(0.0, 0.0), (W, 0.0), (W, H), (0.0, H)])))
    entities.append(("STOCK", "pline", (True, [(-mx, -my), (W + mx, -my), (W + mx, H + my), (-mx, H + my)])))

    # layer table: kept layers that have entities (OUTLINES; the TEXT layer
    # is regenerated and -- production quirk -- NOT declared) + the new ones
    keep_layers = ("OUTLINES", "SEAM")
    layers = [lay for lay in keep_layers if any(e[0] == lay for e in entities)]
    layers += ["PINHOLES", "BOARDCUT", "STOCK"]
    notes.insert(0, "%d drills (%d part + %d coil, %d excluded parts) at %.3f mm | board %.1fx%.1f mm | "
                    "stock %.0fx%.0f in (margins %.1f/%.1f mm) | %d outlines, %d text | layers: %s" % (
                        len(drills), n_part_holes, len(coil_holes),
                        sum(1 for s in (skip_holes or []) if s), float(hole_dia), W, H,
                        float(stock_width_in), float(stock_height_in), mx, my,
                        len(ghosts), len([e for e in entities if e[0] == "TEXT"]), ",".join(layers)))
    return layers, entities, notes


@cicada.node(
    title="Export DXF",
    description="write the board CNC DXF (OUTLINES ghosts, TEXT ids, PINHOLES, BOARDCUT, STOCK) in physical-datum mm (ported board_final_dxf.py).",
    effectful=True,
)
def export_dxf(
    ghosts: "[[Point]]",
    strokes: "[[Point]]",
    strokes_closed: "[Boolean]",
    holes: "[Point]",
    coil_holes: "[Point]",
    board_min: "Point",
    board_max: "Point",
    skip_holes: "[Boolean]" = [],
    hole_dia: "Number" = 3.1,
    stock_width_in: "Number" = 97.0,
    stock_height_in: "Number" = 49.0,
    join_text: "Boolean" = False,
    path: "Text" = "out/board.dxf",
) -> None:
    layers, entities, _notes = board_entities(
        ghosts, strokes, strokes_closed, holes, skip_holes, coil_holes,
        board_min, board_max, hole_dia, stock_width_in, stock_height_in, join_text)
    doc = dxf_document(layers, entities)
    full = resolve_path(path)
    folder = os.path.dirname(full)
    if folder:
        os.makedirs(folder, exist_ok=True)
    # CRLF explicitly: production wrote the file in Windows text mode.
    with open(full, "wb") as f:
        f.write(doc.replace("\n", "\r\n").encode("ascii"))
    return None
