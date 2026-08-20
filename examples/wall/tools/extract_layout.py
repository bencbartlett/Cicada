#!/usr/bin/env python3
"""Extract the frozen production wall layout from the wall repo's artifacts.

Dev tool, NOT a pipeline node (stage 6; lives in examples/wall/tools). Reads the production
exports of the Lorenz LED wall (READ ONLY) and writes

  (paths below are relative to examples/wall/ — the --wall-dir)
  inputs/layout.json                         the frozen layout (schema: docs /
                                             stage-6 contract section 3)
  golden/production/board_postprocessed.dxf  (copy)
  golden/production/manifest.csv             (copy of export 1.4.1)
  golden/production/coil_manifest.csv        (copy, the idx-bearing one)
  golden/production/plates_summary.json      per-object geometric
                                             summary of the 5 production plate
                                             3MFs + the 2 coil 3MFs
  golden/production/extraction_report.json   every check, statistic
                                             and anomaly this tool measured

Sources of truth (export 1.4.1 unless stated):
  board_postprocessed.dxf   OUTLINES = 1200 ghost cells (cells scaled 0.75
                            about their centroid) in PHYSICAL-datum mm (model
                            mm + 25.4); PINHOLES = 2296 circles: per exported
                            part the centroid pin + the lean pin, plus 2 x 6
                            coil ring holes; BOARDCUT/STOCK rectangles.
  manifest.csv (1.4.1)      1137 exported parts: id, idx, bin, plate, heights.
  manifest.csv (1.4)        1142 rows: the 5 parts dropped in 1.4.1.
  export/coil_manifest.csv  58 coil-captured parts + 2 cylinders, with idx.
  plates_f*_v1.4.1_.3mf     object meshes named by part id: height =
                            zmax - zmin, lean_length = |cap centroid - base
                            centroid| (parts are yawed so the lean points ~+Y).
  plates_f0/f1/f2 (1.4)     meshes of the 5 dropped parts.
  export/coil_1.3mf, coil_2.3mf   captured parts in true relative positions
                            (translated only) -> direct lean vectors; 1.4's
                            coil_2.3mf for I59 (see the report: it is missing
                            from every later coil_2 export).

Dependencies: none beyond the Python 3 standard library (pure Python is as
fast as numpy here and keeps the summation order, hence the output, fixed).
recover_seeds.py (optional, separate) needs numpy and likes scipy.

Deterministic: sorted keys, fixed float formatting (%.4f for mm, %.6f for unit
vectors), re-runnable (overwrites its outputs).

Usage:
  python examples/wall/tools/extract_layout.py [--wall-repo DIR] [--wall-dir DIR] [--quiet]
  python examples/wall/tools/recover_seeds.py     # optional: Voronoi seeds + per-cell shrink
  python examples/wall/tools/extract_layout.py    # second pass: keeps the seeds and re-derives the
                                                  # field-based fallbacks (trimmed coil parts) AT the seeds
Two passes are only needed once; afterwards both tools are idempotent (byte-identical outputs).
"""

from __future__ import annotations

import argparse
import csv
import datetime as _dt
import io
import json
import math
import os
import re
import shutil
import sys
import zipfile

IN = 25.4
GHOST_SCALE = 0.75
CAP_SCALE_EXPECTED = 0.07  # production caps = the cell scaled by 7 % (measured)
CORE_RADIUS = 0.1  # contract: solve_field(core_radius=0.1)
PIN_SPACING = 12.0
PIN_CLAMP = 0.35  # x cell equivalent diameter
ZONE_COLS = 3
ZONE_ROWS = 3
ZONE_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"

DEFAULT_WALL = (r"C:\Users\benja\Dropbox\Random Projects\3D Print Stuff"
                r"\Lorenz LED wall")
EXPORT_141 = os.path.join("export", "solenoid_art_export_1.4.1")
EXPORT_14 = os.path.join("export", "solenoid_art_export_1.4")
PLATE_FILES_141 = [
    "plates_f0_emerald_X1C_v1.4.1_.3mf",
    "plates_f1_forest_green_X1C_v1.4.1_.3mf",
    "plates_f2_sea_green_X1C_v1.4.1_.3mf",
    "plates_f3_teal_H2_v1.4.1_.3mf",
    "plates_f4_sky_blue_H2_v1.4.1_.3mf",
]
PLATE_FILES_14 = [
    "plates_f0_emerald_X1C_v1.4.3mf",
    "plates_f1_forest_green_X1C_v1.4.3mf",
    "plates_f2_sea_green_X1C_v1.4.3mf",
]
COIL_FILES = ["coil_1.3mf", "coil_2.3mf"]


# ---------------------------------------------------------------------------
# small geometry
# ---------------------------------------------------------------------------

def poly_centroid(pts):
    """(cx, cy, signed_area) of a closed polygon given as [(x, y), ...]."""
    a2 = cx = cy = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        cr = x1 * y2 - x2 * y1
        a2 += cr
        cx += (x1 + x2) * cr
        cy += (y1 + y2) * cr
    if abs(a2) < 1e-18:
        raise ValueError("degenerate polygon")
    return cx / (3.0 * a2), cy / (3.0 * a2), 0.5 * a2


def point_in_poly(px, py, poly):
    inside = False
    m = len(poly)
    j = m - 1
    for i in range(m):
        x1, y1 = poly[i]
        x2, y2 = poly[j]
        if (y1 > py) != (y2 > py):
            xint = x1 + (py - y1) * (x2 - x1) / (y2 - y1)
            if px < xint:
                inside = not inside
        j = i
    return inside


def convex_hull(pts):
    """Andrew monotone chain, CCW, collinear points dropped."""
    P = sorted(set((float(x), float(y)) for x, y in pts))
    if len(P) <= 2:
        return P

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lo = []
    for p in P:
        while len(lo) >= 2 and cross(lo[-2], lo[-1], p) <= 0:
            lo.pop()
        lo.append(p)
    up = []
    for p in reversed(P):
        while len(up) >= 2 and cross(up[-2], up[-1], p) <= 0:
            up.pop()
        up.append(p)
    return lo[:-1] + up[:-1]


def simplify_collinear(poly, tol):
    """Drop vertices closer than tol to the chord of their neighbours."""
    out = list(poly)
    changed = True
    while changed and len(out) > 3:
        changed = False
        for i in range(len(out)):
            a, b, c = out[i - 1], out[i], out[(i + 1) % len(out)]
            L = math.hypot(c[0] - a[0], c[1] - a[1])
            if L <= 0:
                d = 0.0
            else:
                d = abs((c[0] - a[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (c[1] - a[1])) / L
            if d < tol:
                out.pop(i)
                changed = True
                break
    return out


def best_cyclic_rotation(src, dst):
    """Rotation angle (about the origin) mapping polygon src onto dst with the
    best cyclic vertex correspondence (same winding). Returns (max_residual,
    angle_rad, shift). Both polygons are given about their centroids."""
    n = len(src)
    best = None
    if len(dst) != n:
        return (float("inf"), 0.0, 0)
    for shift in range(n):
        d = dst[shift:] + dst[:shift]
        num = sum(s[0] * t[1] - s[1] * t[0] for s, t in zip(src, d))
        den = sum(s[0] * t[0] + s[1] * t[1] for s, t in zip(src, d))
        ang = math.atan2(num, den)
        ca, sa = math.cos(ang), math.sin(ang)
        res = max(math.hypot(ca * s[0] - sa * s[1] - t[0], sa * s[0] + ca * s[1] - t[1])
                  for s, t in zip(src, d))
        if best is None or res < best[0]:
            best = (res, ang, shift)
    return best


def unit(dx, dy):
    L = math.hypot(dx, dy)
    if L < 1e-12:
        raise ValueError("zero-length vector")
    return dx / L, dy / L


def angle_between_deg(ax, ay, bx, by):
    return math.degrees(math.atan2(ax * by - ay * bx, ax * bx + ay * by))


def percentile(sorted_vals, q):
    if not sorted_vals:
        return None
    k = (len(sorted_vals) - 1) * q
    lo = int(math.floor(k))
    hi = int(math.ceil(k))
    if lo == hi:
        return sorted_vals[lo]
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * (k - lo)


def stats_of(vals):
    s = sorted(vals)
    if not s:
        return {"count": 0}
    return {
        "count": len(s),
        "min": s[0],
        "max": s[-1],
        "median": percentile(s, 0.5),
        "p90": percentile(s, 0.9),
        "p99": percentile(s, 0.99),
        "mean": sum(s) / len(s),
    }


# ---------------------------------------------------------------------------
# DXF (the labels.py / board_final_dxf.py R12 dialect)
# ---------------------------------------------------------------------------

def parse_dxf(text):
    """Entities as (layer, kind, data) in file order: ("pline", (closed, pts))
    and ("circle", (x, y, r)). Also returns the layer table names."""
    lines = [ln.strip() for ln in text.splitlines()]
    ents = []
    layer_table = []
    cur = None
    vert = None
    circ = None
    in_layer_def = False
    i = 0
    while i + 1 < len(lines):
        code, val = lines[i], lines[i + 1]
        i += 2
        if code == "0":
            if vert is not None and cur is not None and vert[1] is not None:
                cur[2].append((vert[0], vert[1]))
            vert = None
            circ = None
            in_layer_def = (val == "LAYER")
            if val == "POLYLINE":
                cur = ["0", False, []]
            elif val == "VERTEX":
                vert = [None, None]
            elif val == "SEQEND":
                if cur is not None:
                    ents.append((cur[0], "pline", (cur[1], cur[2])))
                cur = None
            elif val == "CIRCLE":
                circ = {"layer": "0", "x": None, "y": None, "r": 0.0}
                ents.append(circ)
            continue
        if in_layer_def and code == "2":
            layer_table.append(val)
            in_layer_def = False
            continue
        if circ is not None:
            if code == "8":
                circ["layer"] = val
            elif code == "10":
                circ["x"] = float(val)
            elif code == "20":
                circ["y"] = float(val)
            elif code == "40":
                circ["r"] = float(val)
        elif vert is not None:
            if code == "10":
                vert[0] = float(val)
            elif code == "20":
                vert[1] = float(val)
        elif cur is not None:
            if code == "8" and not cur[2]:
                cur[0] = val
            elif code == "70":
                cur[1] = bool(int(val) & 1)
    out = []
    for e in ents:
        if isinstance(e, dict):
            if e["x"] is None or e["y"] is None:
                raise ValueError("CIRCLE without center in DXF")
            out.append((e["layer"], "circle", (e["x"], e["y"], e["r"])))
        else:
            out.append(e)
    return out, layer_table


# ---------------------------------------------------------------------------
# labels.py port: zones + banded reading-order ordinals (pure)
# ---------------------------------------------------------------------------

def zone_letters_for(cents):
    xs = [c[0] for c in cents]
    ys = [c[1] for c in cents]
    x0, x1 = min(xs), max(xs)
    y0, y1 = min(ys), max(ys)
    seams = [x0 + (x1 - x0) * (k + 1) / float(ZONE_COLS) for k in range(ZONE_COLS - 1)]
    rows = [y0 + (y1 - y0) * (k + 1) / float(ZONE_ROWS) for k in range(ZONE_ROWS - 1)]

    def col_of(x):
        c = 0
        for s in seams:
            if x > s:
                c += 1
        return min(c, ZONE_COLS - 1)

    def row_top_of(y):
        rb = 0
        for b in rows:
            if y > b:
                rb += 1
        rb = min(rb, ZONE_ROWS - 1)
        return ZONE_ROWS - 1 - rb

    return [ZONE_LETTERS[col_of(c[0]) * ZONE_ROWS + row_top_of(c[1])] for c in cents]


def assign_ordinals(cents, zones):
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
        rows = max(1, min(rows, nz))
        band = zh / rows

        def band_key(i, _zy1=zy1, _band=band, _rows=rows):
            r = int((_zy1 - cents[i][1]) / _band)
            if r > _rows - 1:
                r = _rows - 1
            return (r, cents[i][0], -cents[i][1], i)

        for k, i in enumerate(sorted(idxs, key=band_key)):
            out[i] = k + 1
    return out


# ---------------------------------------------------------------------------
# 3MF reading (Bambu production format as written by plate_packer.py /
# coil_groups.py, and as re-saved by Bambu Studio)
# ---------------------------------------------------------------------------

_VERTEX_RE = re.compile(r'<vertex x="([^"]+)" y="([^"]+)" z="([^"]+)"')
_TRI_RE = re.compile(r'<triangle v1="(\d+)" v2="(\d+)" v3="(\d+)"')


def mesh_arrays(xml_text):
    vs = _VERTEX_RE.findall(xml_text)
    ts = _TRI_RE.findall(xml_text)
    V = [(float(a), float(b), float(c)) for a, b, c in vs]
    T = [(int(a), int(b), int(c)) for a, b, c in ts]
    return V, T


def mesh_stats(V, T):
    """bbox min/max, signed volume (mm^3, fixed summation order -> deterministic), counts, and the
    vertex sets at zmin / zmax (XY lists) for base/cap analysis."""
    xs = [v[0] for v in V]
    ys = [v[1] for v in V]
    zs = [v[2] for v in V]
    vol = 0.0
    for a, b, c in T:
        ax, ay, az = V[a]
        bx, by, bz = V[b]
        cx, cy, cz = V[c]
        vol += ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx) + az * (bx * cy - by * cx)
    vol /= 6.0
    zmin, zmax = min(zs), max(zs)
    return {
        "bbox_min": [min(xs), min(ys), zmin],
        "bbox_max": [max(xs), max(ys), zmax],
        "volume": vol,
        "vertices": len(V),
        "triangles": len(T),
        "base_xy": [(v[0], v[1]) for v in V if abs(v[2] - zmin) < 1e-4],
        "cap_xy": [(v[0], v[1]) for v in V if abs(v[2] - zmax) < 1e-4],
    }


def read_3mf(path):
    """Parse one Bambu project 3MF: returns a dict with file-level metadata and
    an ordered list of objects (model_settings.config order = plate order)
    carrying name, ids, mesh path, build translation, plate number, extruder."""
    z = zipfile.ZipFile(path)
    names = z.namelist()
    root = z.read("3D/3dmodel.model").decode("utf-8")
    ms = z.read("Metadata/model_settings.config").decode("utf-8")

    def meta(name):
        m = re.search(r'<metadata name="%s">([^<]*)</metadata>' % re.escape(name), root)
        return m.group(1) if m else None

    # wrapper object id -> mesh path
    comp = {}
    for m in re.finditer(r'<object id="(\d+)"[^>]*>\s*<components>\s*<component p:path="([^"]+)" objectid="(\d+)"', root):
        comp[m.group(1)] = (m.group(2).lstrip("/"), m.group(3))
    # build items: wrapper id -> transform (12 floats)
    items = {}
    for m in re.finditer(r'<item objectid="(\d+)"[^>]*transform="([^"]+)"', root):
        items[m.group(1)] = [float(v) for v in m.group(2).split()]
    # model_settings objects (in file order), plates, assemble
    objs = []
    for m in re.finditer(r'<object id="(\d+)">(.*?)</object>', ms, re.S):
        oid, body = m.group(1), m.group(2)
        name = re.search(r'<metadata key="name" value="([^"]*)"/>', body)
        extr = re.search(r'<metadata key="extruder" value="([^"]*)"/>', body)
        fc = re.search(r'<metadata face_count="(\d+)"/>', body)
        part = re.search(r'<part id="(\d+)"', body)
        extra = {}
        for key in ("source_file", "source_object_id", "source_volume_id"):
            mm = re.search(r'<metadata key="%s" value="([^"]*)"/>' % key, body)
            if mm:
                extra[key] = mm.group(1)
        objs.append({
            "object_id": int(oid),
            "part_id": int(part.group(1)) if part else None,
            "name": name.group(1) if name else None,
            "extruder": int(extr.group(1)) if extr else None,
            "face_count_declared": int(fc.group(1)) if fc else None,
            "source": extra,
        })
    plate_of = {}
    plates = []
    for m in re.finditer(r'<plate>(.*?)</plate>', ms, re.S):
        body = m.group(1)
        pid = int(re.search(r'<metadata key="plater_id" value="(\d+)"/>', body).group(1))
        fmaps = re.search(r'<metadata key="filament_maps" value="([^"]*)"/>', body)
        members = [int(v) for v in re.findall(r'<metadata key="object_id" value="(\d+)"/>', body)]
        for oid in members:
            plate_of[oid] = pid
        plates.append({"plater_id": pid, "filament_maps": fmaps.group(1) if fmaps else None,
                       "object_ids": members})
    assemble = {}
    for m in re.finditer(r'<assemble_item object_id="(\d+)" instance_id="\d+" transform="([^"]+)"', ms):
        assemble[int(m.group(1))] = [float(v) for v in m.group(2).split()]
    # project settings (JSON)
    proj = {}
    if "Metadata/project_settings.config" in names:
        try:
            proj = json.loads(z.read("Metadata/project_settings.config").decode("utf-8"))
        except Exception as e:  # loud but non-fatal: the summary says so
            proj = {"_error": "project_settings.config unreadable: %s" % e}
    pristine = (not any(n.startswith("Metadata/plate_") and n.endswith(".png") for n in names)
                and "source_file" not in ms)
    return {
        "zip": z,
        "entries": names,
        "application": meta("Application"),
        "creation_date": meta("CreationDate"),
        "modification_date": meta("ModificationDate"),
        "designer_user_id": meta("DesignerUserId"),
        "pristine": pristine,
        "comp": comp,
        "items": items,
        "objects": objs,
        "plates": plates,
        "plate_of": plate_of,
        "assemble": assemble,
        "project_settings": proj,
    }


def project_settings_summary(ps):
    keys = ["printer_settings_id", "print_settings_id", "printer_model", "nozzle_diameter",
            "filament_settings_id", "filament_colour", "filament_type", "filament_ids",
            "printable_area", "printable_height", "bed_exclude_area",
            "top_shell_layers", "bottom_shell_layers", "sparse_infill_density",
            "wall_loops", "layer_height"]
    out = {}
    for k in keys:
        if k in ps:
            out[k] = ps[k]
    if "_error" in ps:
        out["_error"] = ps["_error"]
    return out


# ---------------------------------------------------------------------------
# deterministic JSON writer: sorted keys, %.4f for floats (unit vectors %.6f)
# ---------------------------------------------------------------------------

class F6(float):
    """A float that serializes with 6 decimals (unit-vector components)."""


def _fmt_float(v):
    if v != v or v in (float("inf"), float("-inf")):
        return "null"
    if isinstance(v, F6):
        s = "%.6f" % v
    else:
        s = "%.4f" % v
    if s == "-0.0000" or s == "-0.000000":
        s = s[1:]
    return s


def dump_json(obj, indent=1, level=0):
    pad = " " * (indent * level)
    pad1 = " " * (indent * (level + 1))
    if isinstance(obj, dict):
        if not obj:
            return "{}"
        parts = []
        for k in sorted(obj.keys()):
            parts.append(pad1 + json.dumps(str(k)) + ": " + dump_json(obj[k], indent, level + 1))
        return "{\n" + ",\n".join(parts) + "\n" + pad + "}"
    if isinstance(obj, (list, tuple)):
        if not obj:
            return "[]"
        # compact numeric lists on one line
        if all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in obj):
            return "[" + ", ".join(_fmt_float(v) if isinstance(v, float) else str(v) for v in obj) + "]"
        if all(isinstance(v, (list, tuple)) and all(isinstance(w, (int, float)) and not isinstance(w, bool) for w in v) for v in obj):
            return "[" + ", ".join(dump_json(v, indent, level + 1) for v in obj) + "]"
        parts = [pad1 + dump_json(v, indent, level + 1) for v in obj]
        return "[\n" + ",\n".join(parts) + "\n" + pad + "]"
    if isinstance(obj, bool) or obj is None:
        return json.dumps(obj)
    if isinstance(obj, int):
        return str(obj)
    if isinstance(obj, float):
        return _fmt_float(obj)
    return json.dumps(str(obj))


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


# ---------------------------------------------------------------------------
# main extraction
# ---------------------------------------------------------------------------

class Loud(Exception):
    pass


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--wall-repo", default=os.environ.get("CICADA_WALL_REPO", DEFAULT_WALL),
                    help="the wall project repo (read only); env CICADA_WALL_REPO overrides the default")
    ap.add_argument("--wall-dir", default=os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")),
                    help="the examples/wall/ directory to write inputs/ and golden/production/ into")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)

    def log(msg):
        if not args.quiet:
            print(msg)

    wall = args.wall_repo
    exp141 = os.path.join(wall, EXPORT_141)
    exp14 = os.path.join(wall, EXPORT_14)
    for p in (wall, exp141, exp14, os.path.join(wall, "export", "coil_manifest.csv")):
        if not os.path.exists(p):
            raise Loud("missing input: %s" % p)

    report = {"tool": "examples/wall/tools/extract_layout.py", "checks": {}, "anomalies": [], "stats": {}}
    anomalies = report["anomalies"]

    # ---- DXF -------------------------------------------------------------
    dxf_path = os.path.join(exp141, "board_postprocessed.dxf")
    with open(dxf_path, "r", encoding="utf-8") as f:
        dxf_text = f.read()
    ents, layer_table = parse_dxf(dxf_text)
    by = {}
    for e in ents:
        by.setdefault(e[0], []).append(e)
    counts = {k: len(v) for k, v in sorted(by.items())}
    log("DXF entities per layer: %s; layer table: %s" % (counts, layer_table))
    report["checks"]["dxf_entity_counts"] = counts
    report["checks"]["dxf_layer_table"] = layer_table
    missing_layers = sorted(set(by) - set(layer_table))
    if missing_layers:
        anomalies.append("DXF layer table lacks layers that carry entities: %s (board_final_dxf.py only "
                         "lists OUTLINES/SEAM when it regenerates TEXT; CAM imports still show the layer)" % missing_layers)
    if len(by.get("OUTLINES", [])) != 1200:
        raise Loud("expected 1200 OUTLINES, got %d" % len(by.get("OUTLINES", [])))
    if len(by.get("PINHOLES", [])) != 2296:
        raise Loud("expected 2296 PINHOLES, got %d" % len(by.get("PINHOLES", [])))
    radii = sorted(set(round(c[2][2], 6) for c in by["PINHOLES"]))
    if radii != [1.55]:
        raise Loud("PINHOLES radii %s, expected [1.55]" % radii)

    def rect_of(layer):
        (closed, pts) = by[layer][0][2]
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        return [min(xs) - IN, min(ys) - IN], [max(xs) - IN, max(ys) - IN]

    board_min, board_max = rect_of("BOARDCUT")
    stock_min, stock_max = rect_of("STOCK")
    exp_board = ([-IN, -IN], [96 * IN - IN, 48 * IN - IN])
    if any(abs(a - b) > 1e-6 for a, b in zip(board_min + board_max, exp_board[0] + exp_board[1])):
        raise Loud("BOARDCUT rectangle %s/%s is not the 96x48 in board at (-25.4,-25.4)" % (board_min, board_max))
    exp_stock = ([-IN - 12.7, -IN - 12.7], [96 * IN - IN + 12.7, 48 * IN - IN + 12.7])
    if any(abs(a - b) > 1e-6 for a, b in zip(stock_min + stock_max, exp_stock[0] + exp_stock[1])):
        raise Loud("STOCK rectangle %s/%s is not the 97x49 in sheet centred on the board" % (stock_min, stock_max))
    workable = {"min": [0.0, 0.0], "max": [94 * IN, 46 * IN]}

    # cells: ghost -> unscale about the polygon centroid -> model datum
    cells = []
    ghost_repeat_closing = 0
    for e in by["OUTLINES"]:
        closed, pts = e[2]
        if not closed:
            raise Loud("OUTLINES polyline not flagged closed")
        if math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]) < 1e-9:
            pts = pts[:-1]
            ghost_repeat_closing += 1
        cx, cy, area = poly_centroid(pts)
        if area <= 0:
            raise Loud("ghost outline is not CCW")
        cell = [(cx + (x - cx) / GHOST_SCALE - IN, cy + (y - cy) / GHOST_SCALE - IN) for (x, y) in pts]
        ccx, ccy, carea = poly_centroid(cell)
        cells.append({"ghost": [(x - IN, y - IN) for (x, y) in pts], "cell": cell,
                      "centroid": (ccx, ccy), "area": carea})
    report["checks"]["ghost_outlines_with_repeated_closing_vertex"] = ghost_repeat_closing
    n = len(cells)
    cents = [c["centroid"] for c in cells]

    # pins
    holes = [(c[2][0] - IN, c[2][1] - IN) for c in by["PINHOLES"]]
    # grid index for speed
    grid = {}
    for k, (hx, hy) in enumerate(holes):
        grid.setdefault((int(hx // 20.0), int(hy // 20.0)), []).append(k)

    def near(x, y, radius):
        gx, gy = int(x // 20.0), int(y // 20.0)
        span = int(math.ceil(radius / 20.0)) + 1
        out = []
        for ox in range(-span, span + 1):
            for oy in range(-span, span + 1):
                for k in grid.get((gx + ox, gy + oy), ()):
                    if math.hypot(holes[k][0] - x, holes[k][1] - y) <= radius:
                        out.append(k)
        return out

    centroid_pin = [None] * n
    lean_pin = [None] * n
    centroid_pin_dist = []
    used = set()
    for i, c in enumerate(cells):
        cx, cy = c["centroid"]
        cand = near(cx, cy, 0.05)
        if len(cand) > 1:
            raise Loud("part %d: %d holes within 0.05 mm of the centroid" % (i, len(cand)))
        if cand:
            centroid_pin[i] = cand[0]
            used.add(cand[0])
            centroid_pin_dist.append(math.hypot(holes[cand[0]][0] - cx, holes[cand[0]][1] - cy))
    for i, c in enumerate(cells):
        if centroid_pin[i] is None:
            continue
        cx, cy = c["centroid"]
        inside = [k for k in near(cx, cy, 30.0) if k != centroid_pin[i] and point_in_poly(holes[k][0], holes[k][1], c["cell"])]
        if len(inside) != 1:
            raise Loud("part %d: expected exactly one lean pin inside the cell, found %d" % (i, len(inside)))
        lean_pin[i] = inside[0]
        used.add(inside[0])
    rest = [k for k in range(len(holes)) if k not in used]
    if len(rest) != 12:
        raise Loud("expected 12 coil ring holes left over, got %d" % len(rest))
    # two rings of 6
    rings = []
    pool = list(rest)
    while pool:
        seed = pool[0]
        grp = [k for k in pool if math.hypot(holes[k][0] - holes[seed][0], holes[k][1] - holes[seed][1]) < 200.0]
        pool = [k for k in pool if k not in grp]
        rings.append(grp)
    if len(rings) != 2 or any(len(g) != 6 for g in rings):
        raise Loud("coil ring clustering failed: %s" % [len(g) for g in rings])
    rings.sort(key=lambda g: min(holes[k][0] for k in g))  # left ring first
    wires = []
    coil_board_points = []
    for g in rings:
        P = [holes[k] for k in g]
        cx = sum(p[0] for p in P) / 6.0
        cy = sum(p[1] for p in P) / 6.0
        rr = [math.hypot(p[0] - cx, p[1] - cy) for p in P]
        ring_r = sum(rr) / 6.0
        if max(rr) - min(rr) > 0.01:
            raise Loud("coil ring radii not uniform: %s" % rr)
        angs = sorted(math.degrees(math.atan2(p[1] - cy, p[0] - cx)) for p in P)
        wires.append({"center": [cx, cy], "radius": 2.0 * ring_r, "ring_radius": ring_r,
                      "hole_angles_deg": angs})
        # production order: angles 0, 60, ... (pin_holes.coil_hole_layout, phase 0)
        P_sorted = sorted(P, key=lambda p: (math.degrees(math.atan2(p[1] - cy, p[0] - cx)) + 360.0) % 360.0)
        coil_board_points.extend([[p[0], p[1]] for p in P_sorted])
    report["checks"]["coil_rings"] = [{"center": w["center"], "ring_radius": w["ring_radius"],
                                      "cylinder_radius": w["radius"], "hole_angles_deg": w["hole_angles_deg"]} for w in wires]
    report["checks"]["centroid_pin_match_max_mm"] = max(centroid_pin_dist)
    report["checks"]["parts_with_pins"] = sum(1 for p in centroid_pin if p is not None)

    # lean unit vectors + spacing from the production pins
    lean = [None] * n
    spacing = [None] * n
    for i in range(n):
        if lean_pin[i] is None:
            continue
        cx, cy = cells[i]["centroid"]
        dx, dy = holes[lean_pin[i]][0] - cx, holes[lean_pin[i]][1] - cy
        L = math.hypot(dx, dy)
        lean[i] = (dx / L, dy / L)
        spacing[i] = L
    sp = [s for s in spacing if s is not None]
    report["checks"]["pin_spacing_mm"] = stats_of(sp)
    # clamp rule check: spacing = min(12, 0.35 * d_equiv)
    clamp_err = []
    for i in range(n):
        if spacing[i] is None:
            continue
        d_eq = 2.0 * math.sqrt(cells[i]["area"] / math.pi)
        clamp_err.append(abs(spacing[i] - min(PIN_SPACING, PIN_CLAMP * d_eq)))
    report["checks"]["pin_spacing_rule_max_err_mm"] = max(clamp_err)
    if max(clamp_err) > 0.01:
        anomalies.append("pin spacing deviates from min(12, 0.35*d_equiv) by up to %.4f mm" % max(clamp_err))

    # ---- seeds from a previous recover_seeds.py run (second pass) -----------------------------------------------
    # recover_seeds.py writes seeds/keep/cell_scales into layout.json; when they exist and the cells are unchanged,
    # the field-derived fallbacks below (lean / height of trimmed coil parts) are evaluated AT THE SEEDS, which is
    # where the production solve evaluated its field (exact), instead of at the centroids (~0.1 deg off)
    layout_path_early = os.path.join(args.wall_dir, "inputs", "layout.json")
    old_layout = None
    if os.path.exists(layout_path_early):
        try:
            with open(layout_path_early, "r", encoding="utf-8") as f:
                cand = json.load(f)
            if ("seeds" in cand and "keep" in cand and "cell_scales" in cand and len(cand.get("parts", [])) == n
                    and len(cand["seeds"]) >= n and len(cand["cell_scales"]) == n
                    and all(len(op["cell"]) == len(c["cell"]) and
                            all(abs(a[0] - b[0]) < 1e-4 and abs(a[1] - b[1]) < 1e-4 for a, b in zip(op["cell"], c["cell"]))
                            for op, c in zip(cand["parts"], cells))):
                old_layout = cand
                log("found seeds/keep/cell_scales from recover_seeds.py in the existing layout.json (cells unchanged): "
                    "field-derived fallbacks are evaluated at the seeds")
            elif "seeds" in cand:
                log("existing layout.json carries seeds but its cells differ: they are dropped (re-run recover_seeds.py)")
        except Exception as e:
            log("existing layout.json unreadable (%s); ignoring it" % e)
    eval_pts = [tuple(old_layout["seeds"][i]) for i in range(n)] if old_layout else list(cents)
    eval_pts_name = "seeds" if old_layout else "centroids"

    # ---- manifests -------------------------------------------------------
    def load_csv(p):
        with open(p, newline="", encoding="utf-8") as f:
            return list(csv.DictReader(f))

    m141 = load_csv(os.path.join(exp141, "manifest.csv"))
    m14 = load_csv(os.path.join(exp14, "manifest.csv"))
    coil = load_csv(os.path.join(wall, "export", "coil_manifest.csv"))
    if "idx" not in coil[0]:
        raise Loud("export/coil_manifest.csv has no idx column")
    if len(m141) != 1137 or len(m14) != 1142 or len(coil) != 60:
        raise Loud("manifest row counts %d/%d/%d, expected 1137/1142/60" % (len(m141), len(m14), len(coil)))
    by_id_141 = {r["id"]: r for r in m141}
    by_id_14 = {r["id"]: r for r in m14}
    prod_id = {}
    prod_bin = {}
    for r in m14:
        prod_id[int(r["idx"])] = r["id"]
        prod_bin[int(r["idx"])] = int(r["bin"])
    coil_of = {}
    coil_rows = {}
    for r in coil:
        if r["idx"] == "-":
            continue
        i = int(r["idx"])
        if i in prod_id:
            raise Loud("idx %d is both in manifest 1.4 and coil manifest" % i)
        prod_id[i] = r["id"]
        prod_bin[i] = int(r["bin"])
        coil_of[i] = int(r["cylinder"])
        coil_rows[i] = r
    if sorted(prod_id) != list(range(n)):
        raise Loud("manifest idx union is not 0..%d" % (n - 1))
    dropped_ids = [r["id"] for r in m14 if r["id"] not in by_id_141]
    if len(dropped_ids) != 5:
        raise Loud("expected 5 parts dropped in 1.4.1, got %d" % len(dropped_ids))
    for r in m141:
        o = by_id_14[r["id"]]
        for k in ("idx", "bin", "height_mm", "w_mm", "d_mm", "area_mm2"):
            if o[k] != r[k]:
                raise Loud("manifest 1.4 vs 1.4.1 disagree on %s for %s" % (k, r["id"]))
    report["checks"]["dropped_in_1_4_1"] = [{"id": r["id"], "idx": int(r["idx"]), "bin": int(r["bin"]),
                                             "height_mm": float(r["height_mm"]), "plate_1_4": int(r["plate"]),
                                             "file_1_4": r["file"]} for r in m14 if r["id"] in dropped_ids]

    # IDs: recompute from the DXF centroids (labels.py port) and compare
    zones = zone_letters_for(cents)
    ords = assign_ordinals(cents, zones)
    ids = ["%s%d" % (z, o) for z, o in zip(zones, ords)]
    id_mism = [(i, ids[i], prod_id[i]) for i in range(n) if ids[i] != prod_id[i]]
    report["checks"]["ids_recomputed_vs_manifest_mismatches"] = len(id_mism)
    if id_mism:
        raise Loud("recomputed IDs disagree with the manifests at %d indices, e.g. %s" % (len(id_mism), id_mism[:5]))
    # pins present exactly for non-coil parts
    pin_set = set(i for i in range(n) if centroid_pin[i] is not None)
    if pin_set != set(range(n)) - set(coil_of):
        raise Loud("DXF pin presence does not match the coil manifest's captured set")
    report["checks"]["dxf_order_equals_pipeline_idx"] = ("verified: recomputed banded-reading-order IDs match all 1200 "
                                                          "manifest ids; the 58 outlines without pins are exactly the coil idx set")
    log("IDs verified (1200/1200); pins present for %d parts; coil-captured %d" % (len(pin_set), len(coil_of)))

    # ---- meshes ----------------------------------------------------------
    mesh_by_idx = {}       # idx -> dict(height, lean_length, yaw_dev_deg, source, ...)
    plates_summary = {"files": []}
    mesh_cell_fit_res = []
    id_to_idx = {v: k for k, v in prod_id.items()}

    def summarize_file(path, role, want_ids=None, frame="plate", offset=(0.0, 0.0)):
        """Parse every object of one 3MF; return per-object summaries."""
        era = "1.4" if role.startswith("1.4 ") else "1.4.1"
        info = read_3mf(path)
        z = info["zip"]
        file_rec = {
            "file": os.path.basename(path),
            "relative_path": os.path.relpath(path, wall).replace("\\", "/"),
            "role": role,
            "pristine_writer_output": info["pristine"],
            "provenance": ("pristine plate_packer.py / coil_groups.py writer output"
                           if info["pristine"] else "re-saved by Bambu Studio (object-level comparison only)"),
            "application": info["application"],
            "creation_date": info["creation_date"],
            "modification_date": info["modification_date"],
            "n_entries": len(info["entries"]),
            "n_objects": len(info["objects"]),
            "n_plates": len(info["plates"]),
            "plates": [{"plater_id": p["plater_id"], "filament_maps": p["filament_maps"],
                        "n_objects": len(p["object_ids"])} for p in info["plates"]],
            "project_settings": project_settings_summary(info["project_settings"]),
            "objects": [],
        }
        name_counts = {}
        for ob in info["objects"]:
            name_counts[ob["name"]] = name_counts.get(ob["name"], 0) + 1
        dups = sorted(k for k, v in name_counts.items() if v > 1)
        if dups:
            file_rec["duplicate_object_names"] = dups
            anomalies.append("%s: duplicate object names %s" % (os.path.basename(path), dups))
        for ob in info["objects"]:
            oid = str(ob["object_id"])
            if oid not in info["comp"]:
                raise Loud("%s: object %s has no component mesh" % (path, oid))
            mesh_path, _mesh_oid = info["comp"][oid]
            xform = info["items"].get(oid)
            if xform is None:
                raise Loud("%s: object %s has no build item" % (path, oid))
            tr = xform[9:12]
            if want_ids is not None and ob["name"] not in want_ids and not str(ob["name"]).startswith("COIL_"):
                # 1.4 files: only the wanted objects are read (the files are huge and not golden)
                continue
            xml = z.read(mesh_path).decode("utf-8")
            V, T = mesh_arrays(xml)
            st = mesh_stats(V, T)
            rec = {
                "name": ob["name"],
                "object_id": ob["object_id"],
                "part_id": ob["part_id"],
                "mesh_path": mesh_path,
                "plate": info["plate_of"].get(ob["object_id"]),
                "translation": [tr[0], tr[1], tr[2]],
                "assemble_translation": (info["assemble"][ob["object_id"]][9:12]
                                         if ob["object_id"] in info["assemble"] else None),
                "bbox_min": st["bbox_min"],
                "bbox_max": st["bbox_max"],
                "bbox_world_min": [st["bbox_min"][k] + tr[k] for k in range(3)],
                "bbox_world_max": [st["bbox_max"][k] + tr[k] for k in range(3)],
                "volume_mm3": st["volume"],
                "vertices": st["vertices"],
                "triangles": st["triangles"],
                "face_count_declared": ob["face_count_declared"],
                "extruder": ob["extruder"],
            }
            if ob["source"]:
                rec["bambu_source"] = ob["source"]
            if ob["face_count_declared"] is not None and ob["face_count_declared"] != st["triangles"]:
                anomalies.append("%s %s: declared face_count %d != %d triangles" % (
                    os.path.basename(path), ob["name"], ob["face_count_declared"], st["triangles"]))
            # part analysis
            name = ob["name"]
            if name in id_to_idx:
                idx = id_to_idx[name]
                cell = cells[idx]["cell"]
                ccen = cells[idx]["centroid"]
                # mesh vertices are written with 4 decimals, so collinear mid-edge vertices sit within ~1e-4 mm of the
                # edge; 0.003 keeps the shortest production cell edges (0.023 mm) and drops the mesher's edge splits
                hb = simplify_collinear(convex_hull(st["base_xy"]), 0.003) if len(st["base_xy"]) >= 3 else []
                hc = simplify_collinear(convex_hull(st["cap_xy"]), 0.003) if len(st["cap_xy"]) >= 3 else []
                base_valid = len(hb) >= 3 and abs(poly_centroid(hb)[2]) > 1.0
                part = {"idx": idx, "base_vertices": len(hb), "cell_vertices": len(cell), "frame": frame}
                ang = 0.0
                bc = None
                if base_valid:
                    bc = poly_centroid(hb)
                    src = [(x - ccen[0], y - ccen[1]) for x, y in cell]
                    dst = [(x - bc[0], y - bc[1]) for x, y in hb]
                    res, ang, _shift = best_cyclic_rotation(src, dst)
                    part["base_fit_residual_mm"] = res
                    part["yaw_applied_deg"] = math.degrees(ang)
                    part["base_full"] = res <= 0.01
                    if frame == "plate":
                        if res > 0.01:
                            anomalies.append("%s %s: mesh base does not match the DXF cell (residual %.4f mm, %d vs %d vertices)" % (
                                os.path.basename(path), name, res, len(hb), len(cell)))
                        mesh_cell_fit_res.append(res)
                else:
                    part["base_full"] = False
                if frame == "plate":
                    if not base_valid:
                        raise Loud("%s %s: no base polygon at zmin" % (path, name))
                    part["height"] = st["bbox_max"][2] - st["bbox_min"][2]
                    part["height_source"] = "mesh zmax - zmin"
                else:
                    # coil group: every base plane sits at z = 0 in the group frame (lowest base -> z = 0);
                    # a trimmed part keeps that plane, so world zmax is the apex height when the cap survives
                    part["world_zmin"] = st["bbox_min"][2] + tr[2]
                    part["world_zmax"] = st["bbox_max"][2] + tr[2]
                    part["base_on_bed"] = abs(part["world_zmin"]) < 0.01
                    if base_valid and part["base_full"]:
                        # translation check: mesh base centroid (world) - offset == model centroid
                        part["base_translation_err_mm"] = math.hypot(bc[0] + tr[0] - offset[0] - ccen[0],
                                                                     bc[1] + tr[1] - offset[1] - ccen[1])
                        if part["base_translation_err_mm"] > 0.01:
                            anomalies.append("%s %s: base centroid off by %.4f mm from the DXF centroid + group offset" % (
                                os.path.basename(path), name, part["base_translation_err_mm"]))
                if len(hc) >= 3:
                    cc = poly_centroid(hc)
                    scale = math.sqrt(abs(cc[2]) / abs(cells[idx]["area"]))
                    part["cap_vertices"] = len(hc)
                    part["cap_scale_implied"] = scale
                    if len(hc) == len(cell) and abs(scale - CAP_SCALE_EXPECTED) <= 0.003:
                        kind = "scaled_cell_0.07"  # export 1.4.1 geometry
                    elif len(hc) == 3 and len(cell) != 3 and era == "1.4":
                        kind = "triangle"          # export 1.4 tip_caps.py geometry (its centroid is NOT on the lean axis)
                    else:
                        kind = "trimmed_or_unknown"
                    part["cap_kind"] = kind
                    if kind != "trimmed_or_unknown":
                        if frame == "plate":
                            Lxy = math.hypot(cc[0] - bc[0], cc[1] - bc[1])
                            ca, sa = math.cos(-ang), math.sin(-ang)
                            mx = ca * (cc[0] - bc[0]) - sa * (cc[1] - bc[1])
                            my = sa * (cc[0] - bc[0]) + ca * (cc[1] - bc[1])
                        else:
                            cap_world = (cc[0] + tr[0] - offset[0], cc[1] + tr[1] - offset[1])
                            mx, my = cap_world[0] - ccen[0], cap_world[1] - ccen[1]
                            Lxy = math.hypot(mx, my)
                            part["height"] = part["world_zmax"]
                            part["height_source"] = "coil mesh world zmax (base plane z = 0)"
                        cap_dir = unit(mx, my)
                        if lean[idx] is not None:
                            part["lean_vs_pins_deg"] = angle_between_deg(lean[idx][0], lean[idx][1], mx, my)
                        if kind == "scaled_cell_0.07":
                            part["lean_length"] = Lxy
                            part["lean_length_source"] = "%s cap centroid (%s)" % (frame, kind)
                            part["lean_model_frame"] = cap_dir
                        else:
                            # 1.4 triangle caps: measured for the record only (the 1.4.1 geometry is the scaled cell)
                            part["lean_length_triangle_cap"] = Lxy
                            part["lean_model_frame_triangle_cap"] = cap_dir
                        if frame == "plate":
                            lean_dir = lean[idx] if lean[idx] is not None else cap_dir
                            yaw_lean = math.atan2(lean_dir[0], lean_dir[1])
                            part["yaw_minus_lean_yaw_deg"] = math.degrees(math.atan2(math.sin(ang - yaw_lean), math.cos(ang - yaw_lean)))
                else:
                    part["cap_kind"] = "missing"
                rec["part"] = part
                if idx not in mesh_by_idx:
                    mesh_by_idx[idx] = dict(part, role=role, file=os.path.basename(path))
            elif name not in ("COIL_1", "COIL_2"):
                anomalies.append("%s: object name %r is not a production part id" % (os.path.basename(path), name))
            else:
                cx = 0.5 * (st["bbox_min"][0] + st["bbox_max"][0]) + tr[0]
                cy = 0.5 * (st["bbox_min"][1] + st["bbox_max"][1]) + tr[1]
                rec["cylinder"] = {"center_world": [cx, cy], "radius_from_bbox":
                                   0.25 * ((st["bbox_max"][0] - st["bbox_min"][0]) + (st["bbox_max"][1] - st["bbox_min"][1])),
                                   "world_zmin": st["bbox_min"][2] + tr[2], "world_zmax": st["bbox_max"][2] + tr[2]}
            file_rec["objects"].append(rec)
        return file_rec, info

    if True:  # meshes: the plates, the dropped parts, the coil groups
        for fn in PLATE_FILES_141:
            p = os.path.join(exp141, fn)
            log("reading %s ..." % fn)
            rec, info = summarize_file(p, "1.4.1 production plates")
            # manifest cross-check: every object is a 1.4.1 manifest row of this file stem and plate
            stem = fn.replace("_v1.4.1_", "")
            rows = [r for r in m141 if r["file"] == stem]
            names = [o["name"] for o in rec["objects"]]
            if sorted(names) != sorted(r["id"] for r in rows):
                anomalies.append("%s: object names differ from manifest rows of %s" % (fn, stem))
            plate_mism = 0
            for o in rec["objects"]:
                r = by_id_141.get(o["name"])
                if r is None:
                    continue
                local_plate = int(r["plate"]) - min(int(x["plate"]) for x in rows) + 1
                o["manifest"] = {"idx": int(r["idx"]), "plate_global": int(r["plate"]), "plate_local": local_plate,
                                 "x_mm": float(r["x_mm"]), "y_mm": float(r["y_mm"]), "w_mm": float(r["w_mm"]),
                                 "d_mm": float(r["d_mm"]), "height_mm": float(r["height_mm"]),
                                 "area_mm2": float(r["area_mm2"]), "est_g": float(r["est_g"])}
                if o["plate"] != local_plate:
                    plate_mism += 1
                if "part" in o:
                    dh = abs(o["part"]["height"] - float(r["height_mm"]))
                    o["part"]["height_vs_manifest_mm"] = o["part"]["height"] - float(r["height_mm"])
                    if dh > 0.051:
                        anomalies.append("%s %s: mesh height %.4f vs manifest %.1f" % (fn, o["name"], o["part"]["height"], float(r["height_mm"])))
            rec["manifest_plate_mismatches"] = plate_mism
            if plate_mism:
                anomalies.append("%s: %d objects sit on a different plate than the manifest says" % (fn, plate_mism))
            plates_summary["files"].append(rec)
        # the 5 dropped parts: 1.4 plates (re-saved) -- only those objects are read
        dropped_set = set(dropped_ids)
        for fn in PLATE_FILES_14:
            p = os.path.join(exp14, fn)
            log("reading %s (dropped parts only) ..." % fn)
            rec, info = summarize_file(p, "1.4 plates (dropped-part heights only)", want_ids=dropped_set)
            rec["objects_read"] = "only the parts dropped in 1.4.1: %s" % sorted(o["name"] for o in rec["objects"])
            for o in rec["objects"]:
                r = by_id_14[o["name"]]
                o["manifest_1_4"] = {"idx": int(r["idx"]), "plate_global": int(r["plate"]), "height_mm": float(r["height_mm"])}
                if "part" in o:
                    o["part"]["height_vs_manifest_mm"] = o["part"]["height"] - float(r["height_mm"])
            plates_summary["files"].append(rec)
        # coil files: the group is rigidly translated so the cylinder axis sits at the plate centre and the
        # lowest base at z = 0 -> offset (model -> world) = cylinder world centre - DXF ring centre
        def cylinder_world_center(path, k):
            info = read_3mf(path)
            for ob in info["objects"]:
                if ob["name"] == "COIL_%d" % k:
                    mesh_path, _ = info["comp"][str(ob["object_id"])]
                    tr = info["items"][str(ob["object_id"])][9:12]
                    V, T = mesh_arrays(info["zip"].read(mesh_path).decode("utf-8"))
                    st = mesh_stats(V, T)
                    return (0.5 * (st["bbox_min"][0] + st["bbox_max"][0]) + tr[0],
                            0.5 * (st["bbox_min"][1] + st["bbox_max"][1]) + tr[1])
            raise Loud("%s: no COIL_%d object" % (path, k))

        ring_of_coil = {}
        for k in (1, 2):
            parts_k = [i for i, c in coil_of.items() if c == k]
            mx = sum(cents[i][0] for i in parts_k) / len(parts_k)
            my = sum(cents[i][1] for i in parts_k) / len(parts_k)
            ring_of_coil[k] = min(range(2), key=lambda r: math.hypot(mx - wires[r]["center"][0], my - wires[r]["center"][1]))
        if ring_of_coil[1] == ring_of_coil[2]:
            raise Loud("both coil groups map to the same DXF ring")
        for k in (1, 2):
            wires[ring_of_coil[k]]["coil"] = k

        def read_coil_file(path, role, k, want_ids=None):
            cw = cylinder_world_center(path, k)
            ring = wires[ring_of_coil[k]]
            offset = (cw[0] - ring["center"][0], cw[1] - ring["center"][1])
            rec, info = summarize_file(path, role, want_ids=want_ids, frame="coil", offset=offset)
            rec["group_offset_model_to_world"] = [offset[0], offset[1]]
            rec["coil"] = k
            cyl = [o for o in rec["objects"] if "cylinder" in o]
            if len(cyl) != 1:
                raise Loud("%s: expected one cylinder object" % path)
            ring["cylinder_radius_from_mesh_bbox"] = cyl[0]["cylinder"]["radius_from_bbox"]
            ring["cylinder_height_from_mesh"] = cyl[0]["cylinder"]["world_zmax"] - cyl[0]["cylinder"]["world_zmin"]
            if abs(ring["cylinder_radius_from_mesh_bbox"] - ring["radius"]) > 0.05:
                anomalies.append("%s: cylinder radius from the mesh bbox %.4f differs from 2x ring radius %.4f" % (
                    os.path.basename(path), ring["cylinder_radius_from_mesh_bbox"], ring["radius"]))
            return rec

        for k, fn in enumerate(COIL_FILES, start=1):
            p = os.path.join(wall, "export", fn)
            log("reading %s ..." % fn)
            rec = read_coil_file(p, "coil group (root export, pristine)", k)
            names = set(o["name"] for o in rec["objects"])
            expected = set(r["id"] for r in coil if r["cylinder"] == str(k) and r["idx"] != "-")
            missing = sorted(expected - names)
            extra = sorted(names - expected - {"COIL_%d" % k})
            rec["coil_manifest_parts_missing_from_file"] = missing
            rec["objects_not_in_coil_manifest"] = extra
            if missing or extra:
                anomalies.append("%s: coil manifest parts missing from the file %s; extra objects %s" % (fn, missing, extra))
            plates_summary["files"].append(rec)
        # parts missing from the root coil exports (I59): the 1.4 coil exports
        missing_coil = [i for i in coil_of if i not in mesh_by_idx]
        if missing_coil:
            for k, fn in enumerate(COIL_FILES, start=1):
                want = set(prod_id[i] for i in missing_coil if coil_of[i] == k)
                if not want:
                    continue
                p = os.path.join(exp14, fn)
                log("reading %s from export 1.4 for %s ..." % (fn, sorted(want)))
                rec = read_coil_file(p, "1.4 coil group (fallback for parts missing from later coil exports)", k, want_ids=want)
                if rec["objects"]:
                    plates_summary["files"].append(rec)
        still = [prod_id[i] for i in coil_of if i not in mesh_by_idx]
        if still:
            raise Loud("no mesh found anywhere for coil parts %s" % still)
        coil_caps = {}
        for i in coil_of:
            m = mesh_by_idx[i]
            coil_caps[m.get("cap_kind", "missing")] = coil_caps.get(m.get("cap_kind", "missing"), 0) + 1
        report["checks"]["coil_part_cap_kinds"] = coil_caps
        report["checks"]["coil_parts_with_measured_height"] = sum(1 for i in coil_of if "height" in mesh_by_idx[i])
        bt = [m["base_translation_err_mm"] for m in mesh_by_idx.values() if "base_translation_err_mm" in m]
        if bt:
            report["checks"]["coil_base_translation_err_mm"] = stats_of(bt)
        report["checks"]["mesh_base_vs_dxf_cell_fit_residual_mm"] = stats_of(mesh_cell_fit_res)

    # ---- physics: wire signs + core radius from the production lean directions ----
    # production lean: pins (1142); coil-captured parts: the cap-centroid direction of the coil mesh when the
    # cap survived the cylinder trim (cap_kind scaled_cell_0.07 / triangle)
    lean_all = list(lean)
    lean_source = ["pins" if lean[i] is not None else None for i in range(n)]
    for i in coil_of:
        m = mesh_by_idx.get(i)
        if m is not None and "lean_model_frame" in m:
            lean_all[i] = m["lean_model_frame"]
            lean_source[i] = "coil mesh cap"

    def field_at(px, py, signs, core):
        bx = by_ = 0.0
        c2 = core * core
        for w, s in zip(wires, signs):
            dx = px - w["center"][0]
            dy = py - w["center"][1]
            c = s / (dx * dx + dy * dy + c2)
            bx += -dy * c
            by_ += dx * c
        return bx, by_

    def angle_errors(signs, core, which=None, pts=None):
        pts = eval_pts if pts is None else pts
        errs = []
        for i in range(n):
            if lean_all[i] is None or (which is not None and lean_source[i] != which):
                continue
            bx, by_ = field_at(pts[i][0], pts[i][1], signs, core)
            errs.append(abs(angle_between_deg(lean_all[i][0], lean_all[i][1], bx, by_)))
        return errs

    combos = {}
    combo_signs = {}
    for signs in ((1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (-1.0, -1.0)):
        key = "left%+d_right%+d" % (int(signs[0]), int(signs[1]))
        combo_signs[key] = signs
        combos[key] = stats_of(angle_errors(signs, CORE_RADIUS))
    best_key = min(combos, key=lambda k: combos[k]["median"])
    best_signs = combo_signs[best_key]
    if abs(best_signs[0] + best_signs[1]) > 0:
        anomalies.append("best-matching wire currents are NOT antiparallel: %s" % best_key)
    for w, s in zip(wires, best_signs):
        w["current"] = s
    # core radius scan (wire positions fixed at the ring centres, antiparallel unit currents): golden section on
    # log(core) minimising the median |angle error|
    def median_err(core):
        return percentile(sorted(angle_errors(best_signs, core)), 0.5)

    lo, hi = math.log(1.0), math.log(2000.0)
    gr = (math.sqrt(5.0) - 1.0) / 2.0
    c1 = hi - gr * (hi - lo)
    c2 = lo + gr * (hi - lo)
    f1, f2 = median_err(math.exp(c1)), median_err(math.exp(c2))
    for _ in range(60):
        if f1 < f2:
            hi, c2, f2 = c2, c1, f1
            c1 = hi - gr * (hi - lo)
            f1 = median_err(math.exp(c1))
        else:
            lo, c1, f1 = c1, c2, f2
            c2 = lo + gr * (hi - lo)
            f2 = median_err(math.exp(c2))
    core_fit = math.exp(0.5 * (lo + hi))
    err_contract = stats_of(angle_errors(best_signs, CORE_RADIUS))
    err_fit = stats_of(angle_errors(best_signs, core_fit))
    # a round-number candidate: 0.1 x the physical board width (96 in)
    core_candidate = 0.1 * 96 * IN
    err_candidate = stats_of(angle_errors(best_signs, core_candidate))
    fstats = {
        "evaluation_points": eval_pts_name,
        "by_sign_combo_at_contract_core_0.1": combos,
        "chosen_signs": best_key,
        "core_radius_contract": CORE_RADIUS,
        "at_contract_core": err_contract,
        "core_radius_fit": core_fit,
        "at_fitted_core": err_fit,
        "core_radius_candidate_0.1x96in": core_candidate,
        "at_candidate_core": err_candidate,
        "by_lean_source_at_fitted_core": {str(k): stats_of(angle_errors(best_signs, core_fit, which=k))
                                          for k in sorted(set(s for s in lean_source if s is not None))},
        "pins_only_at_candidate_core": stats_of(angle_errors(best_signs, core_candidate, which="pins")),
        "pins_only_at_contract_core": stats_of(angle_errors(best_signs, CORE_RADIUS, which="pins")),
    }
    if old_layout:
        fstats["at_centroids_for_reference"] = {"at_contract_core": stats_of(angle_errors(best_signs, CORE_RADIUS, pts=cents)),
                                                "at_candidate_core": stats_of(angle_errors(best_signs, core_candidate, pts=cents)),
                                                "pins_only_at_candidate_core": stats_of(angle_errors(best_signs, core_candidate, which="pins", pts=cents))}
        fstats["note"] = ("evaluated at the recovered Voronoi seeds (recover_seeds.py): the production field was solved at the "
                          "seeds with CoreRadius 0.1 x 96 in = 243.84 mm; the residual there is pin-rounding noise")
    else:
        fstats["note"] = ("evaluated at the cell centroids: the production directions are reproduced only with a large core radius "
                          "(~0.1 x board width) and the residual is not pin-rounding noise -- the production field was evaluated "
                          "at the Voronoi seeds (run recover_seeds.py, then this tool again, to see the exact match)")
    report["stats"]["field_direction_vs_production_lean_deg"] = fstats
    if err_contract["max"] > 1.0:
        anomalies.append("ported physics (at the %s) with the contract core radius %.3g vs production lean: median %.3f deg, max %.3f deg; "
                         "at the fitted core radius %.2f mm: median %.4f, max %.4f" % (
                             eval_pts_name, CORE_RADIUS, err_contract["median"], err_contract["max"], core_fit, err_fit["median"], err_fit["max"]))
    log("field signs: %s; |angle err| at the %s: core 0.1: median %.3f max %.3f; fitted core %.2f: median %.4f max %.4f" % (
        best_key, eval_pts_name, err_contract["median"], err_contract["max"], core_fit, err_fit["median"], err_fit["max"]))

    # ---- small dense least squares (no numpy dependency) -------------------------
    def polyfit(xs, ys, deg):
        m = deg + 1
        A = [[0.0] * m for _ in range(m)]
        bvec = [0.0] * m
        for x, y in zip(xs, ys):
            pw = [1.0]
            for _ in range(2 * deg):
                pw.append(pw[-1] * x)
            for r in range(m):
                bvec[r] += pw[r] * y
                for c in range(m):
                    A[r][c] += pw[r + c]
        # gaussian elimination with partial pivoting
        for col in range(m):
            piv = max(range(col, m), key=lambda r: abs(A[r][col]))
            A[col], A[piv] = A[piv], A[col]
            bvec[col], bvec[piv] = bvec[piv], bvec[col]
            if abs(A[col][col]) < 1e-300:
                raise Loud("singular normal equations in polyfit")
            for r in range(m):
                if r == col:
                    continue
                f = A[r][col] / A[col][col]
                for c in range(col, m):
                    A[r][c] -= f * A[col][c]
                bvec[r] -= f * bvec[col]
        return [bvec[r] / A[r][r] for r in range(m)]  # coefficients c0 + c1 x + c2 x^2 ...

    def polyval(coef, x):
        out = 0.0
        for c in reversed(coef):
            out = out * x + c
        return out

    def polyfit_scaled(xs, ys, deg):
        """Least squares on the normalised variable u = (x - x0) / xs_ -- keeps the normal equations
        well conditioned for degree 5 at x ~ 100. Returns (coef_low_to_high_in_u, x0, xs_)."""
        x0 = 0.5 * (min(xs) + max(xs))
        xs_ = max(0.5 * (max(xs) - min(xs)), 1e-12)
        coef = polyfit([(x - x0) / xs_ for x in xs], ys, deg)
        return coef, x0, xs_

    def polyval_scaled(fit, x):
        coef, x0, xs_ = fit
        return polyval(coef, (x - x0) / xs_)

    # ---- heights per part ------------------------------------------------------
    heights = [None] * n
    height_src = [None] * n
    lean_len = [None] * n
    lean_len_src = [None] * n
    for i in range(n):
        m = mesh_by_idx.get(i)
        if m is not None and "height" in m:
            heights[i] = m["height"]
            height_src[i] = "mesh %s (%s)" % (m["file"], m["height_source"])
        if m is not None and "lean_length" in m:
            lean_len[i] = m["lean_length"]
            lean_len_src[i] = "mesh %s (%s)" % (m["file"], m["lean_length_source"])
        if heights[i] is None and i not in coil_of:
            heights[i] = float(by_id_14[prod_id[i]]["height_mm"])
            height_src[i] = "manifest 1.4 (0.1 mm)"
    hdiff = []
    for i in range(n):
        if i in coil_of or not height_src[i].startswith("mesh"):
            continue
        hdiff.append(heights[i] - float(by_id_14[prod_id[i]]["height_mm"]))
    report["checks"]["mesh_height_minus_manifest_mm"] = stats_of(hdiff) if hdiff else None
    if hdiff and max(abs(d) for d in hdiff) > 0.051:
        anomalies.append("mesh heights differ from the manifest by more than rounding (max %.4f mm)" % max(abs(d) for d in hdiff))

    # height law H(|B|) at the fitted core radius over the exported parts (needed for trimmed coil parts)
    hm_pairs = []
    for i in range(n):
        if i in coil_of or heights[i] is None:
            continue
        bx, by_ = field_at(eval_pts[i][0], eval_pts[i][1], best_signs, core_fit)
        hm_pairs.append((math.log(math.hypot(bx, by_)), heights[i]))
    hlaw = {"n": len(hm_pairs), "core_radius": core_fit, "x": "log(|B|) with unit antiparallel currents at the %s" % eval_pts_name}
    hfit = None
    if len(hm_pairs) >= 10:
        for deg in (1, 3):
            fit = polyfit_scaled([x for x, _ in hm_pairs], [h for _, h in hm_pairs], deg)
            res = [h - polyval_scaled(fit, x) for x, h in hm_pairs]
            hlaw["poly%d" % deg] = {"coef_low_to_high_in_u": fit[0], "u": "(log|B| - %.6f) / %.6f" % (fit[1], fit[2]),
                                    "residual_abs_max": max(abs(r) for r in res),
                                    "residual_abs_median": percentile(sorted(abs(r) for r in res), 0.5)}
            if deg == 3:
                hfit = fit
    report["stats"]["height_vs_field_magnitude_law"] = hlaw
    h_est = []
    for i in range(n):
        if heights[i] is None:
            if hfit is None:
                raise Loud("no height data to fit the H(|B|) law for coil part idx %d" % i)
            bx, by_ = field_at(eval_pts[i][0], eval_pts[i][1], best_signs, core_fit)
            heights[i] = polyval_scaled(hfit, math.log(math.hypot(bx, by_)))
            height_src[i] = "estimated from the H(|B|) law at the %s (cap trimmed away in the coil mesh)" % eval_pts_name
            h_est.append(prod_id[i])
    report["checks"]["heights_estimated_from_field_law"] = h_est
    if h_est:
        anomalies.append("%d coil-captured parts have their apex trimmed away in every coil 3MF; heights estimated from the fitted "
                         "H(|B|) law (|residual| max %.3f, median %.3f mm over the exported parts): %s" % (
                             len(h_est), hlaw["poly3"]["residual_abs_max"], hlaw["poly3"]["residual_abs_median"], h_est))

    # ---- lean_length law L(H) over the parts with a measured (scaled-cell) cap ------------------------
    pairs = [(heights[i], lean_len[i]) for i in range(n)
             if lean_len[i] is not None and "scaled_cell" in lean_len_src[i] and i not in coil_of]
    law = {"n": len(pairs), "fitted_over": "exported parts with a measured 7%-scaled-cell cap (1.4.1 plate meshes)",
           "height_range": [min(h for h, _ in pairs), max(h for h, _ in pairs)] if pairs else None}
    lfit5 = None
    if len(pairs) >= 6:
        H = [h for h, _ in pairs]
        Lv = [L for _, L in pairs]
        lin = polyfit(H, Lv, 1)
        res = [L - polyval(lin, h) for h, L in pairs]
        law["linear"] = {"a": lin[1], "b": lin[0], "formula": "lean_length = %.6f * height + %.6f" % (lin[1], lin[0]),
                         "residual_abs_max": max(abs(r) for r in res), "residual_abs_median": percentile(sorted(abs(r) for r in res), 0.5)}
        if abs(lin[1]) > 1e-12:
            law["linear_as_a_times_height_minus_h0"] = {"a": lin[1], "h0": -lin[0] / lin[1]}
        quad = polyfit(H, Lv, 2)
        resq = [L - polyval(quad, h) for h, L in pairs]
        law["quadratic"] = {"a2": quad[2], "a1": quad[1], "a0": quad[0],
                            "formula": "lean_length = %.8f * height^2 + %.6f * height + %.6f" % (quad[2], quad[1], quad[0]),
                            "residual_abs_max": max(abs(r) for r in resq),
                            "residual_abs_median": percentile(sorted(abs(r) for r in resq), 0.5)}
        lfit5 = polyfit_scaled(H, Lv, 5)
        res5 = [L - polyval_scaled(lfit5, h) for h, L in pairs]
        law["poly5"] = {"coef_low_to_high_in_u": lfit5[0], "u": "(height - %.6f) / %.6f" % (lfit5[1], lfit5[2]),
                        "residual_abs_max": max(abs(r) for r in res5),
                        "residual_abs_median": percentile(sorted(abs(r) for r in res5), 0.5),
                        "note": "lean_length IS a smooth deterministic function of height (coil-part measurements land on the same "
                                "curve within 0.005 mm) but not a linear or quadratic one; no closed form found -- the degree-5 "
                                "polynomial reproduces it inside height_range; outside it extrapolates"}
        law["ratio_lean_over_height"] = stats_of([L / h for h, L in pairs])
        # coil parts with a measured cap: how far off the curve are they (verifies the coil frame handling)
        coil_dev = [abs(lean_len[i] - polyval_scaled(lfit5, heights[i])) for i in coil_of
                    if lean_len[i] is not None and "scaled_cell" in lean_len_src[i]
                    and law["height_range"][0] <= heights[i] <= law["height_range"][1]]
        law["coil_parts_in_range_abs_deviation_from_poly5"] = stats_of(coil_dev)
        hmin, hmax = min(H), max(H)
        nb = 12
        bins = [[] for _ in range(nb)]
        for h, L in pairs:
            k = min(nb - 1, int((h - hmin) / (hmax - hmin + 1e-12) * nb))
            bins[k].append((h, L))
        law["binned"] = [{"height_mid": hmin + (hmax - hmin) * (k + 0.5) / nb,
                          "n": len(bv), "height_mean": sum(h for h, _ in bv) / len(bv), "lean_mean": sum(L for _, L in bv) / len(bv),
                          "ratio_mean": sum(L / h for h, L in bv) / len(bv)} for k, bv in enumerate(bins) if bv]
    report["stats"]["lean_length_law"] = law
    l_est = []
    l_extrap = []
    for i in range(n):
        if lean_len[i] is None:
            if lfit5 is None:
                raise Loud("no lean_length data to fit a law from")
            lean_len[i] = polyval_scaled(lfit5, heights[i])
            lean_len_src[i] = "estimated from the poly5 L(height) law"
            l_est.append(prod_id[i])
            if not (law["height_range"][0] <= heights[i] <= law["height_range"][1]):
                l_extrap.append(prod_id[i])
    report["checks"]["lean_lengths_estimated_from_law"] = l_est
    report["checks"]["lean_lengths_extrapolated_below_fitted_height_range"] = l_extrap
    if l_est:
        anomalies.append("%d parts have no 1.4.1-geometry cap measurement (5 dropped parts with 1.4 triangle caps, trimmed coil "
                         "parts); lean_length from the poly5 L(height) law (|residual| max %.3f, median %.3f mm; %d of them below the "
                         "fitted height range -> extrapolated): %s" % (
                             len(l_est), law["poly5"]["residual_abs_max"], law["poly5"]["residual_abs_median"], len(l_extrap), l_est))
    tri = {prod_id[i]: {"lean_length_triangle_cap": mesh_by_idx[i]["lean_length_triangle_cap"],
                        "lean_length_used": lean_len[i],
                        "triangle_cap_direction_vs_pins_deg": mesh_by_idx[i].get("lean_vs_pins_deg")}
           for i in range(n) if i in mesh_by_idx and "lean_length_triangle_cap" in mesh_by_idx[i]}
    report["checks"]["triangle_caps_export_1_4"] = tri

    # cap geometry findings
    caps = [m for m in mesh_by_idx.values() if "cap_scale_implied" in m and m["role"].startswith("1.4.1")]
    if caps:
        report["stats"]["cap_scale_implied_1_4_1"] = stats_of([m["cap_scale_implied"] for m in caps])
        report["stats"]["cap_vertex_count_minus_cell_vertex_count_1_4_1"] = stats_of([m["cap_vertices"] - m["cell_vertices"] for m in caps])
        report["stats"]["cap_kinds_1_4_1"] = {k: sum(1 for m in caps if m["cap_kind"] == k) for k in sorted(set(m["cap_kind"] for m in caps))}
    caps14 = [m for m in mesh_by_idx.values() if "cap_scale_implied" in m and m["role"].startswith("1.4 ")]
    if caps14:
        report["stats"]["cap_kinds_1_4"] = {k: sum(1 for m in caps14 if m["cap_kind"] == k) for k in sorted(set(m["cap_kind"] for m in caps14))}
    yawdev = [m["yaw_minus_lean_yaw_deg"] for m in mesh_by_idx.values() if "yaw_minus_lean_yaw_deg" in m and m["role"].startswith("1.4.1")]
    if yawdev:
        report["stats"]["production_plate_yaw_minus_lean_yaw_deg"] = stats_of(yawdev)
        report["stats"]["production_plate_yaw_minus_lean_yaw_abs_deg"] = stats_of([abs(v) for v in yawdev])
        if max(abs(v) for v in yawdev) > 0.05:
            anomalies.append("production plate meshes are yawed so the lean is NOT exactly +Y: |yaw - lean yaw| up to %.3f deg "
                             "(median %.3f); plate_packer.py's Apexes input differed from the geometric apex" % (
                                 max(abs(v) for v in yawdev), percentile(sorted(abs(v) for v in yawdev), 0.5)))
    lvp = [m["lean_vs_pins_deg"] for m in mesh_by_idx.values() if "lean_vs_pins_deg" in m]
    if lvp:
        report["stats"]["mesh_lean_direction_vs_pin_lean_abs_deg"] = stats_of([abs(v) for v in lvp])
    # the production plate yaw explained (needs the seeds): plate_packer.py's Apexes were SEED-based apex points
    # (seed + lean_length * lean) while its Centroids were the cell centroids -> yaw sends (seed - centroid) +
    # lean_length * lean to +Y, not the lean itself
    yaw_explained = None
    if old_layout:
        devs = []
        for i, m in mesh_by_idx.items():
            if "yaw_applied_deg" not in m or not m["role"].startswith("1.4.1") or lean_len[i] is None:
                continue
            ln = lean_all[i] if lean_all[i] is not None else None
            if ln is None:
                continue
            sx, sy = eval_pts[i]
            vx = (sx - cents[i][0]) + lean_len[i] * ln[0]
            vy = (sy - cents[i][1]) + lean_len[i] * ln[1]
            pred = math.degrees(math.atan2(vx, vy))
            d = (m["yaw_applied_deg"] - pred + 180.0) % 360.0 - 180.0
            devs.append(abs(d))
        if devs:
            yaw_explained = stats_of(devs)
            report["stats"]["production_plate_yaw_minus_seed_apex_yaw_abs_deg"] = yaw_explained


    # ---- assemble layout.json ------------------------------------------------
    today = _dt.date.today().isoformat()
    parts = []
    field_lean = []
    for i in range(n):
        c = cells[i]
        ln = lean_all[i]
        if ln is None:
            # no pins and no usable coil-mesh cap: the ported field at the fitted core radius (flagged)
            bx, by_ = field_at(eval_pts[i][0], eval_pts[i][1], best_signs, core_fit)
            ln = unit(bx, by_)
            lean_source[i] = "field at fitted core at the %s (no production evidence)" % eval_pts_name
            field_lean.append(prod_id[i])
        parts.append({
            "idx": i,
            "id": prod_id[i],
            "zone": prod_id[i][0],
            "bin": prod_bin[i],
            "cell": [[x, y] for x, y in c["cell"]],
            "centroid": [c["centroid"][0], c["centroid"][1]],
            "lean": [F6(ln[0]), F6(ln[1])],
            "lean_length": lean_len[i],
            "height": heights[i],
            "exported": (prod_id[i] in by_id_141),
            "coil": coil_of.get(i),
            "production_id": prod_id[i],
        })
    report["checks"]["lean_directions_from_field"] = field_lean
    if field_lean:
        anomalies.append("%d coil-captured parts have neither pins nor a surviving cap: lean direction from the ported field at the "
                         "fitted core radius: %s" % (len(field_lean), field_lean))
    wires_out = [{"center": w["center"], "radius": w["radius"], "current": w["current"]} for w in wires]
    # order: current +1 (out) first, then -1 (in), as the contract example shows
    wires_out.sort(key=lambda w: -w["current"])
    cap_note = "n/a"
    if "cap_scale_implied_1_4_1" in report["stats"]:
        cap_note = "%.4f (min %.4f, max %.4f over %d parts)" % (
            report["stats"]["cap_scale_implied_1_4_1"]["median"], report["stats"]["cap_scale_implied_1_4_1"]["min"],
            report["stats"]["cap_scale_implied_1_4_1"]["max"], report["stats"]["cap_scale_implied_1_4_1"]["count"])
    fstats = report["stats"]["field_direction_vs_production_lean_deg"]
    notes = {
        "cells": "true cell polygon = DXF OUTLINES ghost un-scaled by 1/0.75 about the ghost polygon centroid, shifted by -25.4 mm "
                 "(physical -> model datum), CCW, production vertex order, repeated closing vertex dropped; DXF coordinates carry "
                 "3 decimals, so cell vertices are exact to ~0.0007 mm; every 1.4.1 plate mesh base matches its cell within %.4f mm" % (
                     report["checks"].get("mesh_base_vs_dxf_cell_fit_residual_mm", {}).get("max", float("nan"))),
        "centroid": "polygon centroid of the cell (= the production centroid pin within %.4f mm for every part with pins)" % max(centroid_pin_dist),
        "lean": "unit(lean pin - centroid pin) from the DXF PINHOLES for the %d parts with pins; coil-captured parts: the cap-centroid "
                "minus base-centroid direction of the coil 3MF mesh (translation-only frame) when the cap survived the trim (%d parts), "
                "else the ported field at the fitted core radius (%d parts, listed in the report)" % (
                    sum(1 for s in lean_source if s == "pins"), sum(1 for s in lean_source if s == "coil mesh cap"), len(field_lean)),
        "lean_length": "XY distance from the base polygon centroid to the 7%%-scaled-cell cap centroid of the production mesh "
                       "(1.4.1 plate meshes for the 1137 exported parts; coil 3MFs for captured parts with a surviving cap); the 5 dropped "
                       "parts (1.4 triangle caps) and the trimmed coil parts take the poly5 L(height) law (%d parts, listed in the report)" % (
                           len(report["checks"].get("lean_lengths_estimated_from_law", []))),
        "lean_length_law": ("lean_length IS a smooth deterministic function of height but not a linear one: linear %s (|residual| max %.3f mm, "
                            "median %.3f mm), quadratic |residual| max %.3f mm; a degree-5 polynomial in u = %s reproduces it to "
                            "|residual| max %.4f mm, median %.4f mm over %d exported parts (height %.2f..%.2f mm); lean/height runs "
                            "%.3f..%.3f; coefficients (low to high in u): %s; coil-part measurements land on the same curve within %.4f mm; "
                            "closed form not identified (see golden/production/extraction_report.json for the binned table)" % (
                                law.get("linear", {}).get("formula", "n/a"), law.get("linear", {}).get("residual_abs_max", float("nan")),
                                law.get("linear", {}).get("residual_abs_median", float("nan")),
                                law.get("quadratic", {}).get("residual_abs_max", float("nan")),
                                law.get("poly5", {}).get("u", "n/a"), law.get("poly5", {}).get("residual_abs_max", float("nan")),
                                law.get("poly5", {}).get("residual_abs_median", float("nan")), law["n"],
                                (law.get("height_range") or [float("nan")] * 2)[0], (law.get("height_range") or [float("nan")] * 2)[1],
                                law.get("ratio_lean_over_height", {}).get("min", float("nan")), law.get("ratio_lean_over_height", {}).get("max", float("nan")),
                                ", ".join("%.6f" % c for c in law.get("poly5", {}).get("coef_low_to_high_in_u", [])),
                                law.get("coil_parts_in_range_abs_deviation_from_poly5", {}).get("max", float("nan")))),
        "height": "zmax - zmin of the production mesh (4 decimals) for every exported/dropped part (manifests carry the same value "
                  "rounded to 0.1 mm); coil-captured parts: world zmax of the coil 3MF mesh (bases at z = 0) when the cap survived, "
                  "else estimated from the fitted H(|B|) law (%d parts, listed in the report)" % len(report["checks"].get("heights_estimated_from_field_law", [])),
        "caps": ("production 1.4.1 tip caps are the cell polygon scaled by %s -- NOT the tip_caps.py triangle (export 1.4 used "
                 "triangles); cap vertex count = cell vertex count; the cap centroid sits at centroid + lean_length * lean, z = height"
                 % cap_note),
        "wires": "cylinder centre = mean of its 6 coil ring holes (PINHOLES), radius = 2 x ring radius (pin_holes.py CoilPinRadiusFrac 0.5); "
                 "current signs chosen so the 2D Biot-Savart direction best matches the production lean: %s" % best_key,
        "field": ("2D Biot-Savart evaluated at the %s: with the contract core radius %.3g mm the direction misses the production lean "
                  "by median %.3f deg, max %.3f deg (pinned parts: median %.3f, max %.3f); with CoreRadius 0.1 x 96 in = %.2f mm "
                  "(fit %.2f) by median %.4f, max %.4f deg (pinned parts: median %.4f, max %.4f) -- the production solve used "
                  "CoreRadius 243.84 mm and evaluated the field at the Voronoi seeds (see notes.field_at_seeds once "
                  "recover_seeds.py has run)" % (
                      eval_pts_name, CORE_RADIUS, fstats["at_contract_core"]["median"], fstats["at_contract_core"]["max"],
                      fstats["pins_only_at_contract_core"]["median"], fstats["pins_only_at_contract_core"]["max"],
                      fstats["core_radius_candidate_0.1x96in"], core_fit,
                      fstats["at_candidate_core"]["median"], fstats["at_candidate_core"]["max"],
                      fstats["pins_only_at_candidate_core"]["median"], fstats["pins_only_at_candidate_core"]["max"])),
        "plate_yaw": ("the production plates are NOT yawed so the lean points +Y (|yaw - lean yaw| median %.3f, max %.3f deg); they are "
                      "yawed so that (seed - centroid) + lean_length * lean points +Y: plate_packer.py's Apexes were seed-based apex "
                      "points while its Centroids were the cell centroids%s" % (
                          report["stats"].get("production_plate_yaw_minus_lean_yaw_abs_deg", {}).get("median", float("nan")),
                          report["stats"].get("production_plate_yaw_minus_lean_yaw_abs_deg", {}).get("max", float("nan")),
                          (" (verified against all %d plate meshes: |yaw - predicted| median %.4f, max %.4f deg)" % (
                              yaw_explained["count"], yaw_explained["median"], yaw_explained["max"])) if yaw_explained
                          else " (run recover_seeds.py, then this tool again, to verify this against the plate meshes)")),
        "coil_board_points": "the 12 production coil ring holes (PINHOLES), model mm, left ring first then right ring, each ordered by angle from +X",
        "exported": "true for the 1137 rows of manifest 1.4.1; false for the 58 coil-captured parts and the 5 parts dropped in 1.4.1 (%s)" % ", ".join(sorted(dropped_ids)),
        "idx": "DXF OUTLINES order = production pipeline index; verified by recomputing labels.py's banded-reading-order IDs from the "
               "centroids (1200/1200 match the manifests) and by the coil idx set being exactly the pin-less outlines",
    }

    layout = {
        "source": "wall repo export 1.4.1 (board_postprocessed.dxf, manifest.csv, plates_*_v1.4.1_.3mf) + export 1.4 manifest/plates "
                  "(5 dropped parts) + export/coil_manifest.csv, coil_*.3mf; extracted %s by examples/wall/tools/extract_layout.py" % today,
        "units": "mm",
        "workable": workable,
        "board": {"min": board_min, "max": board_max},
        "stock": {"min": stock_min, "max": stock_max},
        "wires": wires_out,
        "coil_board_points": coil_board_points,
        "parts": parts,
        "counts": {
            "parts": n,
            "exported": sum(1 for p in parts if p["exported"]),
            "coil": sum(1 for p in parts if p["coil"] is not None),
            "coil_1": sum(1 for p in parts if p["coil"] == 1),
            "coil_2": sum(1 for p in parts if p["coil"] == 2),
            "dropped": sum(1 for p in parts if not p["exported"] and p["coil"] is None),
            "wires": len(wires_out),
            "coil_board_points": len(coil_board_points),
            "pin_holes_in_dxf": len(holes),
        },
        "notes": notes,
    }
    report["counts"] = layout["counts"]
    report["sources"] = {
        "dxf": os.path.relpath(dxf_path, wall).replace("\\", "/"),
        "manifest_1_4_1": EXPORT_141.replace("\\", "/") + "/manifest.csv",
        "manifest_1_4": EXPORT_14.replace("\\", "/") + "/manifest.csv",
        "coil_manifest": "export/coil_manifest.csv",
        "heights": {k: sum(1 for s in height_src if s == k) for k in sorted(set(height_src))},
        "lean_lengths": {k: sum(1 for s in lean_len_src if s == k) for k in sorted(set(lean_len_src))},
        "lean_directions": {str(k): sum(1 for s in lean_source if s == k) for k in sorted(set(lean_source), key=str)},
    }
    report["extracted"] = today

    # ---- write ------------------------------------------------------------------
    wall_dir = args.wall_dir
    inputs = os.path.join(wall_dir, "inputs")
    golden = os.path.join(wall_dir, "golden", "production")
    os.makedirs(inputs, exist_ok=True)
    os.makedirs(golden, exist_ok=True)
    layout_path = os.path.join(inputs, "layout.json")
    # carry the recover_seeds.py outputs forward (validated against the cells above)
    if old_layout:
        layout["seeds"] = old_layout["seeds"]
        layout["keep"] = old_layout["keep"]
        layout["cell_scales"] = [F6(v) for v in old_layout["cell_scales"]]
        for k in ("seeds", "cell_scales", "field_at_seeds"):
            if k in old_layout.get("notes", {}):
                layout["notes"][k] = old_layout["notes"][k]
    write_text(layout_path, dump_json(layout) + "\n")
    shutil.copyfile(dxf_path, os.path.join(golden, "board_postprocessed.dxf"))
    shutil.copyfile(os.path.join(exp141, "manifest.csv"), os.path.join(golden, "manifest.csv"))
    shutil.copyfile(os.path.join(wall, "export", "coil_manifest.csv"), os.path.join(golden, "coil_manifest.csv"))
    if True:
        plates_summary["source"] = ("per-object summaries of the production 3MFs (export 1.4.1 plates, the coil groups from export/, "
                                    "and the 1.4 objects used as fallbacks); extracted %s by examples/wall/tools/extract_layout.py" % today)
        plates_summary["notes"] = {
            "pristine_writer_output": "files with no Metadata/plate_*.png thumbnails and no Bambu 'source_file' object metadata are "
                                      "byte-for-byte writer output (plate_packer.py / coil_groups.py); the others were re-saved by "
                                      "Bambu Studio (float32-rounded vertices, thumbnails, re-ordered objects) -- object-level comparison only",
            "coordinates": "mesh bbox in the object's local (bbox-centred) frame; translation = the build item's world position; "
                           "bbox_world = bbox + translation; volume = signed tetrahedron sum (mm^3)",
            "part": "per part: height = zmax - zmin; lean_length = |cap centroid - base centroid| (XY); yaw_applied_deg = rotation "
                    "that maps the DXF cell onto the mesh base (fit residual reported); yaw_minus_lean_yaw_deg = how far the "
                    "production orientation is from 'lean -> +Y'",
        }
        write_text(os.path.join(golden, "plates_summary.json"), dump_json(plates_summary) + "\n")
    write_text(os.path.join(golden, "extraction_report.json"), dump_json(report) + "\n")
    log("wrote %s" % layout_path)
    log("counts: %s" % layout["counts"])
    if anomalies:
        log("ANOMALIES (%d):" % len(anomalies))
        for a in anomalies:
            log("  - " + a)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Loud as e:
        print("extract_layout: REFUSED: %s" % e, file=sys.stderr)
        sys.exit(2)
