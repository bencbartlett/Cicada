# Wall corpus: color-batched polygonal plate packing (stage 6, docs/15).
#
# Ported from the wall repo's plate_packer.py (T4): the PURE core --
# unit_xy, yaw_angle_to_plus_y, rot2, poly_area2d, pyramid_area,
# poly_y_at_x, footprint_profile, terrain_pack, color_slug/pool_of_bin/
# pool_filename, plate_grid_pos, pack_pipeline, the settings harvest
# (harvest_settings, harvest_profile, profile_bed, find_settings/
# find_profile adapted to one settings directory) and the main orient +
# measure + manifest + plate-table flow (plate_packer.py:1505-1758).
# Nothing Rhino: the part geometry is moved by Rust `orient` from the
# per-part source/target frames this node emits, and export_bambu.py
# writes the files.
#
# Frames (stage-6 contract section 4): part_frames = source plane per
# part -- origin at the centroid on z = 0, x = the lean unit, y = z cross
# x; plate_frames = target plane -- production's per-part transform
# (plate_packer.py:1880-1885: yaw so the apex lean points +Y, base on
# z = 0, footprint bbox min at the packed (x, y) in the usable-area frame,
# the X/Y shifts for nozzle keep-out / front margin) PLUS the Bambu plate
# grid offset (plate_grid_pos, stride 1.2 x the embedded profile's bed),
# so the oriented meshes sit in the 3MF file's world coordinates and
# export_bambu.py centers them exactly like production (center_mesh +
# item translation = plate origin + local center). Excluded parts
# (exported = false: coil-captured, dropped) get target = source, an
# identity motion, plate 0, slot -1, empty printer/file, absent manifest
# row.
#
# Params (plate_packer.py:1325-1353, mm): clearance 3, step 1, usable X1C
# (256 - 2*12, 256 - 25 - 10), H2 (330 - 2*12 - keepout, 320 - 25 - 10),
# margins front 25 / side 12 / back 10, height margin 5, TIP_PAD 2,
# AreaPerHour 25000, GramsPerArea 0.001; identify_id (in export_bambu)
# from 100 global. The nozzle keep-out, the H2 height cap and the X1
# bed_exclude_area come from the reference projects in `settings_dir`
# (example_settings.3mf / example_settings_x1c.3mf), exactly as the
# production packer harvested them.

import json
import math
import os
import re
import zipfile

import cicada

X1C_BINS = 3  # bins 0..2 pack to the X1C bed; the rest to the H2 bed
COLOR_NAMES = ["emerald", "forest_green", "sea_green", "teal", "sky_blue"]
TIP_PAD = 2.0  # plate_packer.py:1341 (mm)
PROC_KEYS = ("top_shell_layers", "top_shell_thickness",
             "bottom_shell_layers", "bottom_shell_thickness",
             "sparse_infill_density")


def _pipeline_dir():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_path(path):
    """Relative paths resolve against the pipeline directory (corpus/)."""
    path = str(path)
    if os.path.isabs(path):
        return path
    return os.path.normpath(os.path.join(_pipeline_dir(), path))


# ============================================================
# Pure geometry helpers (plate_packer.py:421-466)
# ============================================================

def unit_xy(dx, dy):
    """ported verbatim from plate_packer.py:421-425 (no lean: treat as +Y)"""
    L = math.hypot(dx, dy)
    if L < 1e-9:
        return 0.0, 1.0, True  # no lean: keep as-is, treat as +Y
    return dx / L, dy / L, False


def yaw_angle_to_plus_y(ux, uy):
    """ported verbatim from plate_packer.py:428-430 -- rotation angle
    about Z sending unit (ux, uy) to (0, +1)."""
    return math.atan2(ux, uy)


def rot2(px, py, cx, cy, ang):
    """ported verbatim from plate_packer.py:433-438"""
    c = math.cos(ang)
    s = math.sin(ang)
    dx = px - cx
    dy = py - cy
    return (cx + c * dx - s * dy, cy + s * dx + c * dy)


def poly_area2d(pts):
    """ported verbatim from plate_packer.py:441-448"""
    a = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i][0], pts[i][1]
        x2, y2 = pts[(i + 1) % m][0], pts[(i + 1) % m][1]
        a += x1 * y2 - x2 * y1
    return 0.5 * abs(a)


def pyramid_area(base_pts, base_z, apex):
    """ported verbatim from plate_packer.py:451-466 -- total shell area:
    base polygon + lateral triangles to the apex."""
    area = poly_area2d(base_pts)
    ax, ay, az = apex
    m = len(base_pts)
    for i in range(m):
        x1, y1 = base_pts[i][0], base_pts[i][1]
        x2, y2 = base_pts[(i + 1) % m][0], base_pts[(i + 1) % m][1]
        ux, uy, uz = x2 - x1, y2 - y1, 0.0
        vx, vy, vz = ax - x1, ay - y1, az - base_z
        cxp = uy * vz - uz * vy
        cyp = uz * vx - ux * vz
        czp = ux * vy - uy * vx
        area += 0.5 * math.sqrt(cxp * cxp + cyp * cyp + czp * czp)
    return area


# ============================================================
# Pure packing: polygon column profiles + terrain heightfield
# (plate_packer.py:473-635)
# ============================================================

def poly_y_at_x(pts, x):
    """ported verbatim from plate_packer.py:473-502"""
    lo = None
    hi = None
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i][0], pts[i][1]
        x2, y2 = pts[(i + 1) % m][0], pts[(i + 1) % m][1]
        if x1 > x2:
            x1, x2, y1, y2 = x2, x1, y2, y1
        if x < x1 - 1e-9 or x > x2 + 1e-9:
            continue
        if x2 - x1 < 1e-12:
            ys = (y1, y2)
        else:
            t = (x - x1) / (x2 - x1)
            if t < 0.0:
                t = 0.0
            elif t > 1.0:
                t = 1.0
            ys = (y1 + t * (y2 - y1),)
        for y in ys:
            if lo is None or y < lo:
                lo = y
            if hi is None or y > hi:
                hi = y
    if lo is None:
        return None
    return lo, hi


def footprint_profile(pts, half_gap, step):
    """ported verbatim from plate_packer.py:505-547 -- per-column occupied
    [yBot, yTop] of the polygon inflated by half_gap; pts are LOCAL (bbox
    min corner at (0, 0)). Returns (bots, tops)."""
    BIG = 1e18
    xs = [p[0] for p in pts]
    w = max(xs)
    m = int(math.ceil((w + 2.0 * half_gap) / step - 1e-9))
    if m < 1:
        m = 1
    bots = []
    tops = []
    for j in range(m):
        wl = j * step - 2.0 * half_gap
        wr = (j + 1) * step
        lo = None
        hi = None
        for x in (max(wl, 0.0), min(wr, w)):
            r = poly_y_at_x(pts, x)
            if r is not None:
                if lo is None or r[0] < lo:
                    lo = r[0]
                if hi is None or r[1] > hi:
                    hi = r[1]
        for p in pts:
            if wl < p[0] < wr:
                if lo is None or p[1] < lo:
                    lo = p[1]
                if hi is None or p[1] > hi:
                    hi = p[1]
        if lo is None:
            bots.append(BIG)
            tops.append(-BIG)
        else:
            bots.append(lo - half_gap)
            tops.append(hi + half_gap)
    return bots, tops


def terrain_pack(items, usable_w, usable_d, half_gap, step, blocked=None):
    """ported verbatim from plate_packer.py:550-635 -- bottom-left terrain
    packing of polygon footprints, first-fit-decreasing; items pre-sorted
    (height desc = rake order). Returns (plates, oversize): plates =
    [[(idx, x, y)]], (x, y) = RAW bbox min corner in usable-area corner
    coordinates."""
    ncols = int(math.ceil((usable_w + 2.0 * half_gap) / step))
    hf0 = [-half_gap] * ncols
    for (zx0, _zy0, zx1, zy1) in (blocked or []):
        seed = zy1 - half_gap
        for c in range(ncols):
            if c * step - 2.0 * half_gap < zx1 and (c + 1) * step > zx0:
                if seed > hf0[c]:
                    hf0[c] = seed
    plates = []
    oversize = []
    todo = []
    for it in items:
        if (it["w"] > usable_w + 1e-9
                or max(it["d"], it["tip_y"]) > usable_d + 1e-9):
            oversize.append(it["idx"])
        else:
            todo.append(it)
    while todo:
        hf = list(hf0)
        hf_min = min(hf)
        placed = []
        rest = []
        for it in todo:
            bots = it["bots"]
            tops = it["tops"]
            limit = usable_d + 1e-9 - max(it["d"], it["tip_y"])
            # O(1) fail-fast: the lowest Y any slot could give
            if hf_min - min(bots) > limit:
                rest.append(it)
                continue
            mcols = len(bots)
            g_max = int(math.floor((usable_w - it["w"]) / step + 1e-9))
            if g_max > ncols - mcols:
                g_max = ncols - mcols
            best_y = None
            best_g = -1
            for g in range(0, g_max + 1):
                y = 0.0
                ok = True
                for j in range(mcols):
                    v = hf[g + j] - bots[j]
                    if v > y:
                        if v > limit:
                            ok = False
                            break
                        if best_y is not None and v >= best_y:
                            ok = False
                            break
                        y = v
                if ok and (best_y is None or y < best_y):
                    best_y = y
                    best_g = g
            if best_y is None:
                rest.append(it)
                continue
            placed.append((it["idx"], best_g * step, best_y))
            for j in range(mcols):
                t = best_y + tops[j]
                if t > hf[best_g + j]:
                    hf[best_g + j] = t
            hf_min = min(hf)
        if not placed:
            # nothing fit an EMPTY plate: everything left is stuck
            oversize.extend(it["idx"] for it in todo)
            break
        plates.append(placed)
        todo = rest
    return plates, oversize


# ============================================================
# Color bins -> printers / files (plate_packer.py:642-725)
# ============================================================

def color_slug(b):
    """ported verbatim from plate_packer.py:647-650"""
    if 0 <= b < len(COLOR_NAMES):
        return COLOR_NAMES[b]
    return "bin%d" % b


def pool_of_bin(b):
    """ported verbatim from plate_packer.py:653-654"""
    return "X1C" if b < X1C_BINS else "H2"


def pool_filename(b):
    """ported verbatim from plate_packer.py:657-658"""
    return "plates_f%d_%s_%s.3mf" % (b, color_slug(b), pool_of_bin(b))


def plate_grid_pos(k, n, stride_x, stride_y):
    """ported verbatim from plate_packer.py:676-683 -- Bambu Studio's plate
    grid: cols = ceil(sqrt(n)), row-major, rows advancing toward the
    front (-Y). Returns (x, y) of plate k's origin (0-based)."""
    cols = int(math.ceil(math.sqrt(n))) if n > 0 else 1
    row = k // cols
    col = k % cols
    return (col * stride_x, -row * stride_y)


def pack_pipeline(parts, params, progress=None):
    """ported verbatim from plate_packer.py:686-725 -- one group per color
    bin, height-sorted descending (rake); returns (plate_records,
    assignment, oversize). Deterministic."""
    hg = 0.5 * params["clearance"]
    step = params["step"]
    groups = {}
    for p in parts:
        groups.setdefault(p["bin"], []).append(p)

    plate_records = []
    oversize_all = []
    for b in sorted(groups.keys()):
        pool = pool_of_bin(b)
        uw, ud = params["usable"][pool]
        group = sorted(groups[b],
                       key=lambda p: (-p["h"], -p["d"], -p["w"], p["idx"]))
        if progress:
            progress("packing f%d (%d parts)" % (b, len(group)))
        plates, oversize = terrain_pack(
            group, uw, ud, hg, step,
            (params.get("blocked") or {}).get(pool))
        oversize_all.extend(oversize)
        for plate in plates:
            plate_records.append({"pool": pool, "bin": b, "items": plate})

    assignment = {}
    counters = {}
    number = 0
    for rec in plate_records:
        number += 1
        counters[rec["bin"]] = counters.get(rec["bin"], 0) + 1
        rec["number"] = number
        rec["local"] = counters[rec["bin"]]
        for (idx, x, y) in rec["items"]:
            assignment[idx] = (number, rec["bin"], x, y)
    return plate_records, assignment, oversize_all


# ============================================================
# Settings reference harvest (plate_packer.py:1013-1150, 1356-1387,
# 1768-1795) -- shared verbatim with export_bambu.py
# ============================================================

def harvest_profile(path):
    """ported verbatim from plate_packer.py:1013-1032 -- project settings
    bytes from a Bambu-saved project 3MF or a raw project_settings JSON."""
    try:
        zf = zipfile.ZipFile(path)
        try:
            return zf.read("Metadata/project_settings.config")
        finally:
            zf.close()
    except Exception:
        pass
    try:
        f = open(path, "rb")
        data = f.read()
        f.close()
        if b"printable_area" in data:
            return data
    except Exception:
        pass
    return None


def profile_bed(profile_bytes, default_w, default_d):
    """ported verbatim from plate_packer.py:1035-1056 -- PHYSICAL bed size
    from a harvested profile's printable_area."""
    try:
        data = json.loads(profile_bytes.decode("utf-8"))
        xs = []
        ys = []
        for s in data.get("printable_area", []):
            a, b = str(s).split("x")
            xs.append(float(a))
            ys.append(float(b))
        if xs and ys:
            w = max(xs) - min(xs)
            d = max(ys) - min(ys)
            if w > 0 and d > 0:
                return w, d
    except Exception:
        pass
    return default_w, default_d


def harvest_settings(path):
    """ported verbatim from plate_packer.py:1066-1136 -- pull the
    production settings out of a reference project: proc overrides,
    range_xml block, nozzle keepout (left, right), height_cap,
    filament_maps, bed_exclude. None if unreadable."""
    try:
        zf = zipfile.ZipFile(path)
    except Exception:
        return None
    out = {"proc": {}, "range_xml": None, "keepout": (0.0, 0.0),
           "height_cap": None, "filament_maps": None, "bed_exclude": None}
    try:
        ps = json.loads(zf.read("Metadata/project_settings.config").decode("utf-8-sig"))
        for k in PROC_KEYS:
            if k in ps:
                out["proc"][k] = ps[k]

        def bbox_x(pts):
            xs = [float(str(s).split("x")[0]) for s in pts]
            return min(xs), max(xs)

        try:
            areas = ps.get("extruder_printable_area") or []
            master = int(str(ps.get("master_extruder_id", "1")))
            if areas and 1 <= master <= len(areas):
                ex0, ex1 = bbox_x(str(areas[master - 1]).split(","))
                bed = ps.get("printable_area") or []
                bd0, bd1 = bbox_x(bed) if bed else (ex0, ex1)
                out["keepout"] = (max(0.0, ex0 - bd0), max(0.0, bd1 - ex1))
            hts = ps.get("extruder_printable_height") or []
            if hts and 1 <= master <= len(hts):
                out["height_cap"] = float(str(hts[master - 1]))
        except Exception:
            pass
        try:
            ex = [(float(str(s).split("x")[0]), float(str(s).split("x")[1]))
                  for s in (ps.get("bed_exclude_area") or [])]
            if ex:
                out["bed_exclude"] = (min(p[0] for p in ex), min(p[1] for p in ex),
                                      max(p[0] for p in ex), max(p[1] for p in ex))
        except Exception:
            pass
    except Exception:
        pass
    try:
        rx = zf.read("Metadata/layer_config_ranges.xml").decode("utf-8")
        m = re.search(r"<range\b.*?</range>", rx, re.DOTALL)
        if m:
            out["range_xml"] = m.group(0)
    except Exception:
        pass
    try:
        ms = zf.read("Metadata/model_settings.config").decode("utf-8")
        m = re.search(r'key="filament_maps" value="([^"]*)"', ms)
        if m:
            out["filament_maps"] = m.group(1)
    except Exception:
        pass
    try:
        zf.close()
    except Exception:
        pass
    return out


def find_settings(settings_dir, filename):
    """adapted from plate_packer.py:1356-1370 find_settings: the production
    search (explicit path, export folder, its parent) collapses to ONE
    directory, `settings_dir`. Returns (settings, path); (None, None)
    when the file is absent or unreadable."""
    cand = os.path.join(settings_dir, filename)
    s = harvest_settings(cand)
    if s is not None:
        return s, cand
    return None, None


def find_profile(settings_dir, pool, settings_path, settings_x1c_path):
    """adapted from plate_packer.py:1768-1795 find_profile: the same
    candidate chain searched in `settings_dir` only. Returns (bytes, path)
    or raises -- a file with no embedded project_settings shows no plates
    in Bambu Studio, and production only warned."""
    cands = []
    if pool == "X1C":
        if settings_x1c_path:
            cands.append(settings_x1c_path)  # the X1C settings reference IS an X1C project
        cands.append(os.path.join(settings_dir, "reference_x1c.3mf"))
        cands.append(os.path.join(settings_dir, "bambu_project_settings_x1c.json"))
    elif settings_path:
        cands.append(settings_path)  # the settings reference IS an H2C project
    cands.append(os.path.join(settings_dir, "reference_2plate.3mf"))
    cands.append(os.path.join(settings_dir, "bambu_project_settings_h2c.json"))
    cands.append(os.path.join(settings_dir, "bambu_project_settings_h2d.json"))
    for cand in cands:
        data = harvest_profile(cand)
        if data is not None:
            return data, cand
    raise ValueError("no Bambu project profile for the %s pool in %s (searched %s)"
                     % (pool, settings_dir, ", ".join(os.path.basename(c) for c in cands)))


def load_settings(settings_dir):
    """Both references + the per-pool embedded profiles and physical bed
    sizes. Returns a dict; raises when a pool's profile is missing."""
    settings_dir = resolve_path(settings_dir)
    if not os.path.isdir(settings_dir):
        raise ValueError("settings_dir %s is not a directory" % settings_dir)
    S, s_path = find_settings(settings_dir, "example_settings.3mf")
    SX, sx_path = find_settings(settings_dir, "example_settings_x1c.3mf")
    if S is None:
        raise ValueError("example_settings.3mf (the H2 settings reference) not found in %s" % settings_dir)
    if SX is None:
        raise ValueError("example_settings_x1c.3mf (the X1C settings reference) not found in %s" % settings_dir)
    profiles = {}
    beds = {}
    for pool in ("X1C", "H2"):
        data, path = find_profile(settings_dir, pool, s_path, sx_path)
        dflt_w = 256.0 if pool == "X1C" else 330.0
        dflt_d = 256.0 if pool == "X1C" else 320.0
        profiles[pool] = (data, path)
        beds[pool] = profile_bed(data, dflt_w, dflt_d)
    return {"dir": settings_dir, "h2": S, "h2_path": s_path, "x1c": SX, "x1c_path": sx_path,
            "profiles": profiles, "beds": beds}




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
    """plate_packer.py:1226-1247 outline_pts tail: [(x, y)], closing
    duplicate dropped, None below 3 vertices."""
    pts = [(p[0], p[1]) for p in _cell_points(cell)]
    if len(pts) >= 2 and math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]) < 1e-9:
        pts = pts[:-1]
    return pts if len(pts) >= 3 else None


def apex_and_lean(centroid, direction, lean_length, height, i, apex_origin=None):
    """The packer's apex and plate_packer.py's yaw unit
    unit_xy(apex - Centroids[i]) (plate_packer.py:1520-1525). The apex is
    `apex_origin` (when given — see the apex_origins port: production's
    Apexes were computed from the Voronoi SEEDS, where the field was
    solved) else the cell centroid, displaced lean_length along the field
    unit vector, at the part height. Returns (ax, ay, az, ux, uy,
    degenerate)."""
    cx, cy, cz = centroid
    ox, oy = (apex_origin[0], apex_origin[1]) if apex_origin is not None else (cx, cy)
    ux0, uy0 = direction[0], direction[1]
    L = math.hypot(ux0, uy0)
    if L < 1e-9:
        if lean_length > 1e-12:
            raise ValueError("pack_plates: part %d has a zero-length direction but lean_length %.4g"
                             % (i, lean_length))
        ax, ay = ox, oy
    else:
        ax = ox + ux0 / L * lean_length
        ay = oy + uy0 / L * lean_length
    az = cz + height
    ux, uy, degenerate = unit_xy(ax - cx, ay - cy)
    return ax, ay, az, ux, uy, degenerate


class PackParams(object):
    """adapted: plate_packer.py:1325-1344 module parameters (mm)."""

    def __init__(self, clearance=3.0, step=1.0, x1c_size=256.0, x1c_max_z=250.0,
                 h2_width=330.0, h2_depth=320.0, h2_max_z=325.0, margin_front=25.0,
                 margin_side=12.0, margin_back=10.0, height_margin=5.0,
                 area_per_hour=25000.0, grams_per_area=0.001):
        self.X1C_SIZE = float(x1c_size)
        self.X1C_MAXZ = float(x1c_max_z)
        self.H2D_W = float(h2_width)
        self.H2D_D = float(h2_depth)
        self.H2D_MAXZ = float(h2_max_z)
        self.M_FRONT = float(margin_front)
        self.M_SIDE = float(margin_side)
        self.M_BACK = float(margin_back)
        self.Y_SHIFT = 0.5 * (self.M_FRONT - self.M_BACK)
        self.H_MARGIN = float(height_margin)
        self.CLEARANCE = float(clearance)
        self.PACK_STEP = float(step)
        if self.PACK_STEP <= 0.0:
            self.PACK_STEP = 1.0
        self.AREA_PER_HOUR = float(area_per_hour)
        self.G_PER_AREA = float(grams_per_area)


def pack_wall(cells, centroids, directions, heights, lean_lengths, ids, bins, exported,
              settings, PP, apex_origins=None, bed_exclude=True):
    """The ported main flow (plate_packer.py:1505-1758) without Rhino.
    Returns a dict with everything the node outputs are built from.
    apex_origins: optional per-part origins of the packer's apexes (the
    production Apexes were seed-based); None -> the cell centroids."""
    n = len(centroids)
    notes = []
    HALF_GAP = 0.5 * PP.CLEARANCE
    parts_data = []
    geo = {}
    lean_units = []
    excluded = []
    no_lean = 0
    for i in range(n):
        cx, cy, cz = centroids[i]
        ax, ay, az, ux, uy, degenerate = apex_and_lean(
            centroids[i], directions[i], lean_lengths[i], heights[i], i,
            apex_origins[i] if apex_origins else None)
        lean_units.append((ux, uy))
        if not exported[i]:
            excluded.append(i)
            continue
        if degenerate:
            no_lean += 1
        ang = yaw_angle_to_plus_y(ux, uy)
        pts = outline_xy(cells[i])
        if pts is None:
            raise ValueError("pack_plates: part %d has a degenerate cell outline" % i)
        rpts = [rot2(p[0], p[1], cx, cy, ang) for p in pts]
        rax, ray = rot2(ax, ay, cx, cy, ang)
        xs = [p[0] for p in rpts]
        ys = [p[1] for p in rpts]
        minx = min(xs)
        miny = min(ys)
        w = max(xs) - minx
        d = max(ys) - miny
        lpts = [(p[0] - minx, p[1] - miny) for p in rpts]
        bots, tops = footprint_profile(lpts, HALF_GAP, PP.PACK_STEP)
        tip_y = (ray - miny) + TIP_PAD
        h = az - cz
        area = pyramid_area(pts, cz, (ax, ay, az))
        fp = poly_area2d(pts)
        geo[i] = (ang, minx, miny, w, d, cz, rax, ray, ux, uy)
        parts_data.append({"idx": i, "id": ids[i], "bin": bins[i],
                           "w": w, "d": d, "h": h, "tip_y": tip_y,
                           "bots": bots, "tops": tops,
                           "area": area, "fp": fp,
                           "g": area * PP.G_PER_AREA})
    if no_lean:
        notes.append("%d parts with no lean (kept as-is)" % no_lean)

    # nozzle keep-out + height cap from the settings reference: pack
    # H2 plates inside the MASTER nozzle's reachable area only
    SETTINGS = settings["h2"]
    SETTINGS_X1C = settings["x1c"]
    ko_l = SETTINGS["keepout"][0]
    ko_r = SETTINGS["keepout"][1]
    h2_ceiling = PP.H2D_MAXZ
    if SETTINGS.get("height_cap"):
        h2_ceiling = min(h2_ceiling, SETTINGS["height_cap"])
    X_SHIFT = {"X1C": 0.0, "H2": 0.5 * (ko_l - ko_r)}

    tall_x1c = [p for p in parts_data
                if pool_of_bin(p["bin"]) == "X1C" and p["h"] > PP.X1C_MAXZ - PP.H_MARGIN]
    if tall_x1c:
        notes.append("!!! %d X1C-color parts exceed the X1C height ceiling (%.0f mm): %s%s !!!" % (
            len(tall_x1c), PP.X1C_MAXZ - PP.H_MARGIN,
            ", ".join(p["id"] for p in tall_x1c[:8]), "..." if len(tall_x1c) > 8 else ""))
    tall_h2 = [p for p in parts_data
               if pool_of_bin(p["bin"]) == "H2" and p["h"] > h2_ceiling - PP.H_MARGIN]
    if tall_h2:
        notes.append("%d parts exceed the H2 (master nozzle) height ceiling!" % len(tall_h2))

    params = {
        "clearance": PP.CLEARANCE,
        "step": PP.PACK_STEP,
        "usable": {
            "X1C": (PP.X1C_SIZE - 2 * PP.M_SIDE, PP.X1C_SIZE - PP.M_FRONT - PP.M_BACK),
            "H2": (PP.H2D_W - 2 * PP.M_SIDE - ko_l - ko_r,
                   PP.H2D_D - PP.M_FRONT - PP.M_BACK),
        },
    }
    blocked = {"X1C": [], "H2": []}
    for pool_name, s_ref, off_x in (("X1C", SETTINGS_X1C, PP.M_SIDE),
                                    ("H2", SETTINGS, PP.M_SIDE + ko_l)):
        zone = (s_ref or {}).get("bed_exclude")
        if not zone:
            continue
        if not bed_exclude:
            notes.append("%s bed_exclude_area %s IGNORED (bed_exclude=False: the production "
                         "layout shows no block)" % (pool_name, zone))
            continue
        zx0, zy0, zx1, zy1 = [v for v in zone]
        uw_p, ud_p = params["usable"][pool_name]
        lx0, ly0 = zx0 - off_x, zy0 - PP.M_FRONT
        lx1, ly1 = zx1 - off_x, zy1 - PP.M_FRONT
        if ly0 > 1e-9:
            notes.append("%s bed_exclude_area does not touch the front edge -- unsupported shape, NOT packed around" % pool_name)
            continue
        if lx1 <= 0.0 or lx0 >= uw_p or ly1 <= 0.0:
            continue  # margins already clear the whole zone
        blocked[pool_name].append((max(0.0, lx0), ly0, min(uw_p, lx1), min(ud_p, ly1)))
    params["blocked"] = blocked

    plate_records, assignment, oversize = pack_pipeline(parts_data, params)
    if oversize:
        raise ValueError("pack_plates: %d parts too big for any plate: %s" % (
            len(oversize), ", ".join(str(s) for s in oversize[:8])))
    data_by_idx = dict((p["idx"], p) for p in parts_data)

    # plate grid offsets per plate (plate_packer.py:1822-1867): within a
    # file, plate k of n_recs at plate_grid_pos with stride 1.2 x the
    # embedded profile's bed
    grid = {}
    strides = {}
    bins_present = sorted(set(r["bin"] for r in plate_records))
    for b in bins_present:
        recs = [r for r in plate_records if r["bin"] == b]
        pool = pool_of_bin(b)
        phys_w, phys_d = settings["beds"][pool]
        bed_w = PP.X1C_SIZE if pool == "X1C" else PP.H2D_W
        bed_d = PP.X1C_SIZE if pool == "X1C" else PP.H2D_D
        if bed_w > phys_w + 1e-6 or bed_d > phys_d + 1e-6:
            notes.append("f%d packing area exceeds physical bed %.0fx%.0f" % (b, phys_w, phys_d))
        stride_x = 1.2 * phys_w
        stride_y = 1.2 * phys_d
        strides[pool] = (phys_w, phys_d)
        for k, rec in enumerate(recs):
            grid[rec["number"]] = plate_grid_pos(k, len(recs), stride_x, stride_y)

    # per-part placement
    placement = {}
    for rec in plate_records:
        pool = rec["pool"]
        uw, ud = params["usable"][pool]
        phys_w, phys_d = settings["beds"][pool]
        gx, gy = grid[rec["number"]]
        for slot, (idx, px, py) in enumerate(rec["items"]):
            ang, minx, miny, w, d, cz, rax, ray, ux, uy = geo[idx]
            cx, cy, _cz = centroids[idx]
            lx = 0.5 * phys_w + X_SHIFT[pool] + (px - 0.5 * uw) - minx
            ly = 0.5 * phys_d + PP.Y_SHIFT + (py - 0.5 * ud) - miny
            placement[idx] = {"number": rec["number"], "local": rec["local"], "slot": slot,
                              "pool": pool, "bin": rec["bin"], "px": px, "py": py,
                              "tx": gx + lx, "ty": gy + ly, "cz": cz, "ux": ux, "uy": uy,
                              "uw": uw, "ud": ud}

    # ---- manifest + plate table (plate_packer.py:1726-1758) ------------
    manifest = ["id,idx,bin,printer,plate,file,x_mm,y_mm,w_mm,d_mm,height_mm,area_mm2,est_g"]
    rows_by_idx = {}
    plate_table = []
    plate_stats = {}
    for rec in plate_records:
        b = rec["bin"]
        pool = rec["pool"]
        uw, ud = params["usable"][pool]
        fname = pool_filename(b)
        tot_g = 0.0
        tot_a = 0.0
        tot_fp = 0.0
        max_h = 0.0
        for (idx, px, py) in rec["items"]:
            p = data_by_idx[idx]
            tot_g += p["g"]
            tot_a += p["area"]
            tot_fp += p["fp"]
            if p["h"] > max_h:
                max_h = p["h"]
            row = "%s,%d,%d,%s,%d,%s,%.1f,%.1f,%.1f,%.1f,%.1f,%.0f,%.1f" % (
                p["id"], p["idx"], p["bin"], pool, rec["number"], fname,
                (px + 0.5 * p["w"] - 0.5 * uw),
                (py + 0.5 * p["d"] - 0.5 * ud),
                p["w"], p["d"], p["h"], p["area"], p["g"])
            manifest.append(row)
            rows_by_idx[idx] = row
        hours = tot_a / PP.AREA_PER_HOUR
        fill = tot_fp / (uw * ud) if uw * ud > 0 else 0.0
        plate_stats[rec["number"]] = (tot_g, max_h, hours)
        flag = " | BACKUP SPOOL" if tot_g > 900.0 else ""
        plate_table.append(
            "plate %02d | f%d %s %s #%02d | %d parts | maxH %.0f mm | fill %d%% | ~%.1f h | ~%.0f g%s" % (
                rec["number"], b, color_slug(b), pool, rec["local"],
                len(rec["items"]), max_h, round(fill * 100.0), hours, tot_g, flag))

    bits = []
    for b in bins_present:
        recs_b = [r for r in plate_records if r["bin"] == b]
        parts_b = sum(len(r["items"]) for r in recs_b)
        hours_b = sum(plate_stats[r["number"]][2] for r in recs_b)
        bits.append("f%d %s (%s): %d parts / %d plates (~%.0f h)" % (
            b, color_slug(b), pool_of_bin(b), parts_b, len(recs_b), hours_b))
    notes.insert(0, "Parts: %d placed / %d in | %d excluded | %s" % (
        len(placement), n, len(excluded), " | ".join(bits) if bits else "no plates"))
    notes.append("settings: x1c:%s, h2:%s (keep-out L%.0f/R%.0f mm%s)" % (
        os.path.basename(settings["x1c_path"]), os.path.basename(settings["h2_path"]),
        ko_l, ko_r, (", fmap %s" % SETTINGS["filament_maps"]) if SETTINGS.get("filament_maps") else ""))
    return {"placement": placement, "manifest": manifest, "rows_by_idx": rows_by_idx,
            "plate_table": plate_table, "notes": notes, "plate_records": plate_records,
            "params": params, "X_SHIFT": X_SHIFT, "grid": grid, "geo": geo,
            "lean_units": lean_units}


@cicada.node(
    title="Pack Plates",
    description="color-batched polygonal plate packing: per-part source/target frames for orient, plate numbers, files, manifest rows and the plate table (ported plate_packer.py).",
)
def pack_plates(
    cells: "[Closed<Curve>]",
    centroids: "[Point]",
    directions: "[Vector]",
    heights: "[Number]",
    lean_lengths: "[Number]",
    ids: "[Text]",
    bins: "[Integer]",
    exported: "[Boolean]",
    settings_dir: "Text" = "inputs/bambu",
    apex_origins: "[Point]" = [],
    bed_exclude: "Boolean" = True,
    clearance: "Number" = 3.0,
    step: "Number" = 1.0,
    margin_front: "Number" = 25.0,
    margin_side: "Number" = 12.0,
    margin_back: "Number" = 10.0,
    height_margin: "Number" = 5.0,
    x1c_size: "Number" = 256.0,
    x1c_max_z: "Number" = 250.0,
    h2_width: "Number" = 330.0,
    h2_depth: "Number" = 320.0,
    h2_max_z: "Number" = 325.0,
    area_per_hour: "Number" = 25000.0,
    grams_per_area: "Number" = 0.001,
) -> {
    "part_frames": "[Plane]",
    "plate_frames": "[Plane]",
    "plate": "[Integer]",
    "plate_local": "[Integer]",
    "slot": "[Integer]",
    "printer": "[Text]",
    "file": "[Text]",
    "manifest_rows": "[Text?]",
    "manifest": "[Text]",
    "plate_table": "[Text]",
    "notes": "[Text]",
}:
    """apex_origins: OPTIONAL per-part origins for the packer's apexes
    (apex = origin + lean_length * lean). Production's `Apexes` input was
    computed where the field was solved — at the Voronoi SEEDS, not the
    cell centroids — so its yaw unit_xy(apex - Centroids) sends
    (seed - centroid) + lean_length * lean to +Y (the shipped H2 meshes
    confirm this to 0.005 degrees; the printed pin pair and cap sit a
    median 0.57 degrees off +Y). Wire the recovered seeds here to
    reproduce the production packing; empty (default) uses the cell
    centroids — a declared deviation that changes the yaw by up to ~7
    degrees, w/d by fractions of a mm, and therefore the plate layout.
    bed_exclude: pack around the X1-series front-left bed_exclude_area
    harvested from the X1C reference (18 x 28 mm; the slicer refuses to
    print there). The SHIPPED production layout shows no such block (its
    first parts sit at the usable-area corner: plate_packer.py only searched
    ExportPath and its parent for example_settings_x1c.3mf, which lived in
    the repo root), so wall.cic passes False to reproduce it; True is the
    correct setting for a new layout."""
    n = len(centroids)
    if apex_origins and len(apex_origins) != n:
        raise ValueError("pack_plates: apex_origins has %d entries for %d parts" % (len(apex_origins), n))
    if not (len(cells) == len(directions) == len(heights) == len(lean_lengths)
            == len(ids) == len(bins) == len(exported) == n):
        raise ValueError(
            "pack_plates: list lengths differ (cells %d, centroids %d, directions %d, heights %d, "
            "lean_lengths %d, ids %d, bins %d, exported %d)" % (
                len(cells), n, len(directions), len(heights), len(lean_lengths), len(ids),
                len(bins), len(exported)))
    if n == 0:
        raise ValueError("pack_plates: no parts")
    PP = PackParams(clearance, step, x1c_size, x1c_max_z, h2_width, h2_depth, h2_max_z,
                    margin_front, margin_side, margin_back, height_margin,
                    area_per_hour, grams_per_area)
    settings = load_settings(settings_dir)
    result = pack_wall(cells, centroids, directions, heights, lean_lengths, ids, bins,
                       exported, settings, PP, apex_origins if apex_origins else None,
                       bool(bed_exclude))
    placement = result["placement"]

    part_frames = []
    plate_frames = []
    plate = []
    plate_local = []
    slot = []
    printer = []
    files = []
    rows = []
    for i in range(n):
        cx, cy, cz = centroids[i]
        ux, uy = result["lean_units"][i]  # (0, 1) when the part has no lean
        # source: the part's own frame at its centroid on the base plane
        # (z = cz, 0 on the wall); the target drops the base to z = 0.
        src = cicada.Plane((cx, cy, cz), (ux, uy, 0.0), (-uy, ux, 0.0))
        part_frames.append(src)
        if i in placement:
            pl = placement[i]
            # yaw sends the lean unit to +Y: x' = (0, 1), y' = z x x' = (-1, 0)
            plate_frames.append(cicada.Plane((pl["tx"] + cx, pl["ty"] + cy, 0.0),
                                             (0.0, 1.0, 0.0), (-1.0, 0.0, 0.0)))
            plate.append(pl["number"])
            plate_local.append(pl["local"])
            slot.append(pl["slot"])
            printer.append(pl["pool"])
            files.append(pool_filename(pl["bin"]))
            rows.append(result["rows_by_idx"][i])
        else:
            plate_frames.append(src)
            plate.append(0)
            plate_local.append(0)
            slot.append(-1)
            printer.append("")
            files.append("")
            rows.append(None)
    return {
        "part_frames": part_frames,
        "plate_frames": plate_frames,
        "plate": plate,
        "plate_local": plate_local,
        "slot": slot,
        "printer": printer,
        "file": files,
        "manifest_rows": rows,
        "manifest": result["manifest"],
        "plate_table": result["plate_table"],
        "notes": result["notes"],
    }
