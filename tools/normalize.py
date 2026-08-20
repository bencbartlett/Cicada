#!/usr/bin/env python3
"""Normalize + compare a project's exporter outputs against its golden
references (stage-6 contract section 6; first user: examples/wall).

An engine-wide dev tool, NOT a pipeline node. Pure stdlib Python 3 (numpy is not used).

    normalize.py 3mf OURS.3mf REF.3mf|REF.json [--level xml|bytes] [--tol-bbox 0.02]
                 [--tol-volume-rel 0.005] [--tol-translation 0.01] [--report FILE] [--strict]
    normalize.py summarize FILE.3mf [-o summary.json]
    normalize.py dxf OURS.dxf REF.dxf [--tol 0.002] [--report FILE] [--strict]
    normalize.py manifest OURS.csv REF.csv [--report FILE] [--strict]
    normalize.py all --ours DIR --ref DIR [--report FILE] [--strict]

3MF, --level xml (default): the zip entry list (sorted) must match; every
non-mesh XML entry is compared byte-exact after normalizing the declared
noise -- <metadata name="CreationDate"/"ModificationDate"> values, zip
timestamps (container metadata, never part of the entry bytes), the build
item / assemble_item translations (compared numerically within
--tol-translation instead) and the face_count attributes (triangle counts
are REPORTED, not compared: the tessellation is the declared noise, Rhino
mesher vs Manifold). Object meshes are replaced by canonical summaries
(name, id, plate, translation, bbox to 1e-2 mm, volume to 1e-1 mm3,
triangle count) and compared bbox / volume within tolerance. The reference
may be a 3MF or a summary JSON produced by `summarize` (the production
files are 50-116 MB and live outside the repo; write their summaries once
with `normalize.py summarize <prod>.3mf -o examples/wall/golden/production/
plates_f<bin>_<color>_<printer>.summary.json` -- `all` mode looks for
exactly those names next to the production DXF + manifest).
3MF, --level bytes: strict -- every entry byte-exact (only the date
metadata stripped). Expected to fail vs production; the report says which
entries differ and why.
DXF: entities by layer: CIRCLE (center, r) and POLYLINE (vertices,
closed) compared numerically within --tol (2e-3 mm, the recovered-layout
rounding noise); counts per layer must match; the layer table is compared
as a set; the HEADER section byte-exact (line endings normalized).
manifest.csv: byte-exact after sorting rows by id (row order is plate
order; reported when it differs); when not exact, the numeric columns are
compared and last-digit flips within one unit of their printed precision
are classed as declared noise.

Verdicts: PASS (exact / within tolerance), NOISE (only declared-noise
classes differ), FAIL. Exit code: 0 for PASS and NOISE, 1 for FAIL; with
--strict, NOISE exits 2. The markdown report (stdout, or --report FILE)
lists per-file verdicts, counts, max deviations and every diff class with
its cause and whether it is declared noise.
"""

import argparse
import hashlib
import json
import os
import re
import sys
import zipfile

PASS, NOISE, FAIL = "PASS", "NOISE", "FAIL"
_ORDER = {PASS: 0, NOISE: 1, FAIL: 2}


def worst(verdicts):
    v = PASS
    for x in verdicts:
        if _ORDER[x] > _ORDER[v]:
            v = x
    return v


class Report(object):
    """Collects markdown lines + the overall verdict."""

    def __init__(self, title):
        self.lines = ["# %s" % title, ""]
        self.verdicts = []

    def section(self, title):
        self.lines.append("")
        self.lines.append("## %s" % title)
        self.lines.append("")

    def line(self, text=""):
        self.lines.append(text)

    def verdict(self, label, v, detail=""):
        self.verdicts.append(v)
        self.lines.append("- **%s**: %s%s" % (label, v, (" -- " + detail) if detail else ""))

    def diff_class(self, name, cause, declared_noise, detail=""):
        self.lines.append("  - diff class `%s`: %s%s [%s]" % (
            name, cause, (" -- " + detail) if detail else "",
            "declared noise" if declared_noise else "NOT declared noise"))

    def overall(self):
        return worst(self.verdicts) if self.verdicts else PASS

    def text(self):
        return "\n".join(self.lines + ["", "**Overall: %s**" % self.overall(), ""])


# ============================================================
# 3MF
# ============================================================

_DATE_RE = re.compile(r'(<metadata name="(?:CreationDate|ModificationDate)">)[^<]*(</metadata>)')
_ITEM_RE = re.compile(r'(<item objectid="(\d+)" p:UUID="[^"]*" transform=")([^"]*)(" printable="1"/>)')
_ASSEMBLE_RE = re.compile(r'(<assemble_item object_id="(\d+)" instance_id="0" transform=")([^"]*)(" offset="0 0 0" />)')
_FACE_RE = re.compile(r'face_count="(\d+)"')
_VERTEX_RE = re.compile(r'<vertex x="([-+0-9.eE]+)" y="([-+0-9.eE]+)" z="([-+0-9.eE]+)"')
_TRI_RE = re.compile(r'<triangle v1="(\d+)" v2="(\d+)" v3="(\d+)"')
_OBJECT_ENTRY_RE = re.compile(r'^3D/Objects/object_(\d+)\.model$')


def strip_dates(text):
    return _DATE_RE.sub(r"\1\2", text)


def _parse_transform(text):
    parts = text.split()
    if len(parts) != 12:
        raise ValueError("transform with %d numbers: %r" % (len(parts), text))
    return [float(p) for p in parts]


def normalize_root_model(text):
    """3D/3dmodel.model -> (text with item transforms blanked, {objectid: (tx,ty,tz)})."""
    trans = {}

    def sub(m):
        t = _parse_transform(m.group(3))
        trans[int(m.group(2))] = (t[9], t[10], t[11])
        return m.group(1) + "T" + m.group(4)

    return strip_dates(_ITEM_RE.sub(sub, text)), trans


def normalize_model_settings(text):
    """Metadata/model_settings.config -> (text with assemble transforms and
    face counts blanked, {object_id: (x,y,z)}, [face counts in order])."""
    trans = {}

    def sub(m):
        t = _parse_transform(m.group(3))
        trans[int(m.group(2))] = (t[9], t[10], t[11])
        return m.group(1) + "T" + m.group(4)

    faces = [int(x) for x in _FACE_RE.findall(text)]
    out = _ASSEMBLE_RE.sub(sub, text)
    out = _FACE_RE.sub('face_count="N"', out)
    return out, trans, faces


def mesh_summary(xml_text):
    """object_k.model -> (bbox_min, bbox_max, volume, triangle_count)."""
    verts = [(float(a), float(b), float(c)) for a, b, c in _VERTEX_RE.findall(xml_text)]
    tris = [(int(a), int(b), int(c)) for a, b, c in _TRI_RE.findall(xml_text)]
    if not verts or not tris:
        raise ValueError("object mesh without vertices or triangles")
    xs = [v[0] for v in verts]
    ys = [v[1] for v in verts]
    zs = [v[2] for v in verts]
    v6 = 0.0
    for (a, b, c) in tris:
        ax, ay, az = verts[a]
        bx, by, bz = verts[b]
        cx, cy, cz = verts[c]
        v6 += ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx) + az * (bx * cy - by * cx)
    return ((min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs)), v6 / 6.0, len(tris))


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def summarize_3mf(path):
    """The canonical summary of a Bambu production-format 3MF (the format
    examples/wall/golden/production/plates_summary.json holds per file)."""
    zf = zipfile.ZipFile(path)
    names = sorted(zf.namelist())
    root_raw = zf.read("3D/3dmodel.model").decode("utf-8")
    root_norm, item_trans = normalize_root_model(root_raw)
    ms_raw = zf.read("Metadata/model_settings.config").decode("utf-8")
    ms_norm, asm_trans, faces = normalize_model_settings(ms_raw)
    # object names + plate membership from model_settings
    obj_names = {}
    for m in re.finditer(r'<object id="(\d+)">\s*<metadata key="name" value="([^"]*)"/>', ms_raw):
        obj_names[int(m.group(1))] = m.group(2)
    extruders = {}
    for m in re.finditer(r'<object id="(\d+)">\s*<metadata key="name" value="[^"]*"/>\s*<metadata key="extruder" value="([^"]*)"/>', ms_raw):
        extruders[int(m.group(1))] = m.group(2)
    plate_of = {}
    plates = []
    for pi, block in enumerate(re.findall(r"<plate>(.*?)</plate>", ms_raw, re.DOTALL)):
        ks = []
        for oid in re.findall(r'key="object_id" value="(\d+)"', block):
            oid = int(oid)
            plate_of[oid] = pi + 1
            ks.append(oid // 2)
        plates.append(ks)
    objects = []
    for name in names:
        m = _OBJECT_ENTRY_RE.match(name)
        if not m:
            continue
        k = int(m.group(1))
        raw = zf.read(name)
        bmin, bmax, vol, ntri = mesh_summary(raw.decode("utf-8"))
        wrapper = 2 * k
        objects.append({
            "k": k,
            "id": wrapper,
            "name": obj_names.get(wrapper, ""),
            "plate": plate_of.get(wrapper, 0),
            "translation": [round(c, 3) for c in item_trans.get(wrapper, (0.0, 0.0, 0.0))],
            "bbox": [[round(c, 2) for c in bmin], [round(c, 2) for c in bmax]],
            "volume": round(vol, 1),
            "triangles": ntri,
            "extruder": extruders.get(wrapper, ""),
            "sha256_raw": _sha(raw),
        })
    objects.sort(key=lambda o: o["k"])
    xml = {}
    for name in names:
        if _OBJECT_ENTRY_RE.match(name):
            continue
        data = zf.read(name)
        if name == "3D/3dmodel.model":
            norm = root_norm.encode("utf-8")
        elif name == "Metadata/model_settings.config":
            norm = ms_norm.encode("utf-8")
        else:
            try:
                norm = strip_dates(data.decode("utf-8")).encode("utf-8")
            except UnicodeDecodeError:
                norm = data
        xml[name] = {"sha256_normalized": _sha(norm), "sha256_raw": _sha(data), "size": len(data)}
    summary = {
        "format": "cicada-corpus-3mf-summary-1",
        "file": os.path.basename(path),
        "entries": names,
        "plates": plates,
        "objects": objects,
        "xml": xml,
        "assemble_equals_items": all(
            asm_trans.get(k) == item_trans.get(k) for k in item_trans),
    }
    zf.close()
    return summary


def _load_ref_3mf(path):
    """Reference as (summary, texts or None) -- texts only when it is a 3MF."""
    if path.lower().endswith(".json"):
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        if data.get("format") != "cicada-corpus-3mf-summary-1":
            # allow a dict of several summaries keyed by file name
            raise ValueError("%s is not a normalize.py 3MF summary" % path)
        return data, None
    zf = zipfile.ZipFile(path)
    texts = {}
    for name in zf.namelist():
        if not _OBJECT_ENTRY_RE.match(name):
            texts[name] = zf.read(name)
    zf.close()
    return summarize_3mf(path), texts


def compare_3mf(ours, ref, level="xml", tol_bbox=0.02, tol_volume_rel=0.005, tol_volume_abs=1.0,
                tol_translation=0.01, report=None):
    """Returns the verdict; appends to `report` (a Report) when given."""
    rep = report or Report("3MF compare")
    rep.section("3MF: %s vs %s (level %s)" % (os.path.basename(ours), os.path.basename(ref), level))
    ours_sum = summarize_3mf(ours)
    ref_sum, ref_texts = _load_ref_3mf(ref)
    zf = zipfile.ZipFile(ours)
    our_texts = {n: zf.read(n) for n in zf.namelist() if not _OBJECT_ENTRY_RE.match(n)}
    our_dates = sorted(set(i.date_time for i in zf.infolist()))
    zf.close()
    verdicts = []

    # Provenance: the wall writer never writes thumbnails; a reference with
    # Metadata/*.png entries was opened and RE-SAVED by Bambu Studio (the
    # production X1C plate files were: thumbnails added, XML rewritten,
    # objects nudged by hand). Against such a reference the entry list, the
    # non-mesh XML and the build-item translations are NOT the writer's
    # output — they are reported as a declared class (`reference-resaved`)
    # instead of failing; object names, plate membership, bbox and volume
    # remain hard checks (a re-save does not change them).
    resaved = any(e.startswith("Metadata/") and e.endswith(".png") for e in ref_sum["entries"])
    if resaved:
        rep.line("- reference provenance: RE-SAVED by Bambu Studio (thumbnail entries present) -- "
                 "not pristine writer output; entry-list / XML / translation differences are "
                 "reported under `reference-resaved`, geometry and structure stay hard checks")

    # entry list
    if ours_sum["entries"] == ref_sum["entries"]:
        rep.verdict("zip entry list", PASS, "%d entries" % len(ours_sum["entries"]))
    else:
        missing = sorted(set(ref_sum["entries"]) - set(ours_sum["entries"]))
        extra = sorted(set(ours_sum["entries"]) - set(ref_sum["entries"]))
        if resaved:
            rep.verdict("zip entry list", NOISE, "missing %s; extra %s" % (missing[:5], extra[:5]))
            rep.diff_class("reference-resaved", "entries Bambu Studio added on re-save", True,
                           "%d missing in ours" % len(missing))
            verdicts.append(NOISE)
        else:
            rep.verdict("zip entry list", FAIL, "missing %s; extra %s" % (missing[:5], extra[:5]))
            rep.diff_class("entry-list", "different set of zip entries", False)
            verdicts.append(FAIL)
    rep.line("- zip timestamps (ours): %s (fixed 1980-01-01 is the declared deviation)" % (our_dates,))

    # non-mesh XML entries
    for name in sorted(set(ours_sum["xml"]) & set(ref_sum["xml"])):
        o = ours_sum["xml"][name]
        r = ref_sum["xml"][name]
        if level == "bytes":
            if our_texts is not None and ref_texts is not None:
                a = strip_dates(our_texts[name].decode("utf-8", "replace"))
                b = strip_dates(ref_texts[name].decode("utf-8", "replace"))
                same = a == b
            else:
                same = o["sha256_raw"] == r["sha256_raw"]
            if same:
                rep.verdict(name, PASS, "byte-exact (dates stripped)")
            else:
                why = "content differs"
                if name == "3D/3dmodel.model":
                    why = "build item translations (bbox centers: tessellation/layout noise) and/or structure"
                elif name == "Metadata/model_settings.config":
                    why = "face counts (tessellation) and/or assemble translations and/or structure"
                rep.verdict(name, FAIL, why)
                rep.diff_class("bytes:" + name, why, name in ("3D/3dmodel.model", "Metadata/model_settings.config"))
                verdicts.append(FAIL)
            continue
        if o["sha256_normalized"] == r["sha256_normalized"]:
            rep.verdict(name, PASS, "byte-exact after normalization")
        else:
            detail = "normalized content differs"
            if our_texts is not None and ref_texts is not None and name in ref_texts:
                a = our_texts[name].decode("utf-8", "replace")
                b = ref_texts[name].decode("utf-8", "replace")
                if name == "3D/3dmodel.model":
                    a, _ = normalize_root_model(a)
                    b, _ = normalize_root_model(b)
                elif name == "Metadata/model_settings.config":
                    a, _, _ = normalize_model_settings(a)
                    b, _, _ = normalize_model_settings(b)
                else:
                    a, b = strip_dates(a), strip_dates(b)
                al, bl = a.splitlines(), b.splitlines()
                first = next((i for i in range(min(len(al), len(bl))) if al[i] != bl[i]), min(len(al), len(bl)))
                detail = "first differing line %d: ours %r vs ref %r (%d vs %d lines)" % (
                    first + 1, al[first] if first < len(al) else "<eof>",
                    bl[first] if first < len(bl) else "<eof>", len(al), len(bl))
            if resaved:
                rep.verdict(name, NOISE, detail)
                rep.diff_class("reference-resaved", "XML rewritten by Bambu Studio on re-save", True, name)
                verdicts.append(NOISE)
            else:
                rep.verdict(name, FAIL, detail)
                rep.diff_class("xml:" + name, "non-mesh XML differs after normalization", False)
                verdicts.append(FAIL)

    # objects: pair by k (object order is plate order, slot order)
    ours_objs = {o["k"]: o for o in ours_sum["objects"]}
    ref_objs = {o["k"]: o for o in ref_sum["objects"]}
    if len(ours_objs) != len(ref_objs):
        rep.verdict("object count", FAIL, "%d vs %d" % (len(ours_objs), len(ref_objs)))
        rep.diff_class("object-count", "different number of objects", False)
        verdicts.append(FAIL)
    else:
        rep.verdict("object count", PASS, "%d objects, %d plates" % (len(ours_objs), len(ours_sum["plates"])))
    if level == "bytes":
        mesh_diff = [k for k in sorted(set(ours_objs) & set(ref_objs))
                     if ours_objs[k].get("sha256_raw") != ref_objs[k].get("sha256_raw")]
        if mesh_diff:
            rep.verdict("object mesh entries (bytes)", FAIL, "%d of %d object_k.model entries differ, first k=%s"
                        % (len(mesh_diff), len(ours_objs), mesh_diff[:5]))
            rep.diff_class("bytes:objects", "object meshes differ byte-wise (tessellation / layout noise)", True)
            verdicts.append(FAIL)
        else:
            rep.verdict("object mesh entries (bytes)", PASS, "all %d byte-exact" % len(ours_objs))
    if ours_sum["plates"] != ref_sum["plates"]:
        rep.verdict("plate membership", FAIL, "plate -> object lists differ")
        rep.diff_class("plate-map", "objects assigned to different plates / order", False)
        verdicts.append(FAIL)
    else:
        rep.verdict("plate membership", PASS)
    name_mismatch = [k for k in ours_objs if k in ref_objs and ours_objs[k]["name"] != ref_objs[k]["name"]]
    if name_mismatch:
        k = name_mismatch[0]
        rep.verdict("object names", FAIL, "%d differ, first k=%d: %s vs %s" % (
            len(name_mismatch), k, ours_objs[k]["name"], ref_objs[k]["name"]))
        rep.diff_class("object-names", "object names (part ids) differ", False)
        verdicts.append(FAIL)
    else:
        rep.verdict("object names", PASS)
    max_tr = 0.0
    max_bb = 0.0
    max_vol_rel = 0.0
    max_vol_abs = 0.0
    tri_our = 0
    tri_ref = 0
    bad_tr = bad_bb = bad_vol = 0
    moved = []
    for k in sorted(set(ours_objs) & set(ref_objs)):
        a, b = ours_objs[k], ref_objs[k]
        dtr = max(abs(x - y) for x, y in zip(a["translation"], b["translation"]))
        if dtr > tol_translation:
            moved.append((a["name"], dtr))
        dbb = max(abs(x - y) for x, y in zip(a["bbox"][0] + a["bbox"][1], b["bbox"][0] + b["bbox"][1]))
        dv = abs(a["volume"] - b["volume"])
        rel = dv / max(abs(b["volume"]), 1e-9)
        max_tr = max(max_tr, dtr)
        max_bb = max(max_bb, dbb)
        max_vol_abs = max(max_vol_abs, dv)
        max_vol_rel = max(max_vol_rel, rel)
        tri_our += a["triangles"]
        tri_ref += b["triangles"]
        if dtr > tol_translation:
            bad_tr += 1
        if dbb > tol_bbox:
            bad_bb += 1
        if dv > tol_volume_abs and rel > tol_volume_rel:
            bad_vol += 1
    if ours_objs and ref_objs:
        rep.verdict("item translations", PASS if bad_tr == 0 else (NOISE if resaved else FAIL),
                    "max |d| %.4f mm (tol %.3f), %d over" % (max_tr, tol_translation, bad_tr))
        rep.verdict("object bbox", PASS if bad_bb == 0 else FAIL,
                    "max |d| %.4f mm (tol %.3f), %d over" % (max_bb, tol_bbox, bad_bb))
        rep.verdict("object volume", PASS if bad_vol == 0 else FAIL,
                    "max |d| %.2f mm3, max rel %.5f (tol rel %.4f and abs %.1f), %d over" % (
                        max_vol_abs, max_vol_rel, tol_volume_rel, tol_volume_abs, bad_vol))
        rep.line("- triangle counts (REPORTED, not compared): ours %d, ref %d" % (tri_our, tri_ref))
        if tri_our != tri_ref:
            rep.diff_class("triangle-count", "tessellation differs (Rhino mesher vs Manifold)", True,
                           "ours %d vs ref %d" % (tri_our, tri_ref))
        if bad_tr:
            listed = ", ".join("%s %.1f mm" % m for m in moved[:12])
            if resaved:
                rep.diff_class("reference-resaved", "objects moved by hand in Bambu Studio after export "
                               "(the pristine manifest is the writer's placement)", True, listed)
                verdicts.append(NOISE)
            else:
                rep.diff_class("translation", "build item translations beyond tolerance", False, listed)
                verdicts.append(FAIL)
        if bad_bb:
            rep.diff_class("bbox", "object bounding boxes beyond tolerance", False)
            verdicts.append(FAIL)
        if bad_vol:
            rep.diff_class("volume", "object volumes beyond tolerance (deboss font + tessellation are the declared noise within tolerance)", False)
            verdicts.append(FAIL)
        if not (bad_tr or bad_bb or bad_vol) and (max_tr > 0 or max_bb > 0 or max_vol_abs > 0):
            verdicts.append(NOISE)
            rep.diff_class("geometry-noise", "sub-tolerance translation/bbox/volume differences", True,
                           "max translation %.4f, bbox %.4f mm, volume %.2f mm3" % (max_tr, max_bb, max_vol_abs))
    v = worst(verdicts) if verdicts else PASS
    rep.verdicts.append(v)
    rep.line("")
    rep.line("**File verdict: %s**" % v)
    if report is None:
        print(rep.text())
    return v


# ============================================================
# DXF
# ============================================================

def parse_dxf(text):
    """ported verbatim from board_final_dxf.py:365-423 -- entities as
    (layer, kind, data) in file order."""
    lines = [ln.strip() for ln in text.splitlines()]
    ents = []
    cur = None       # open polyline [layer, closed, pts]
    vert = None
    circ = None
    i = 0
    while i + 1 < len(lines):
        code, val = lines[i], lines[i + 1]
        i += 2
        if code == "0":
            if vert is not None and cur is not None and vert[1] is not None:
                cur[2].append((vert[0], vert[1]))
            vert = None
            circ = None
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
                try:
                    cur[1] = bool(int(val) & 1)
                except ValueError:
                    pass
    out = []
    for e in ents:
        if isinstance(e, dict):
            if e["x"] is not None and e["y"] is not None:
                out.append((e["layer"], "circle", (e["x"], e["y"], e["r"])))
        else:
            out.append(e)
    return out


def dxf_sections(text):
    """{'HEADER': [lines], 'TABLES': [...], 'ENTITIES': [...]} (line-ending agnostic)."""
    lines = [ln.strip() for ln in text.splitlines()]
    out = {}
    i = 0
    while i + 3 < len(lines):
        if lines[i] == "0" and lines[i + 1] == "SECTION" and lines[i + 2] == "2":
            name = lines[i + 3]
            j = i + 4
            body = []
            while j + 1 < len(lines) and not (lines[j] == "0" and lines[j + 1] == "ENDSEC"):
                body.append(lines[j])
                j += 1
            out[name] = body
            i = j + 2
        else:
            i += 1
    return out


def dxf_layer_table(text):
    return set(re.findall(r"(?m)^0\s*\r?\nLAYER\s*\r?\n2\s*\r?\n([^\r\n]+)", text))


def compare_dxf(ours, ref, tol=2e-3, report=None):
    rep = report or Report("DXF compare")
    rep.section("DXF: %s vs %s (tol %.4f mm)" % (os.path.basename(ours), os.path.basename(ref), tol))
    with open(ours, "rb") as f:
        a_text = f.read().decode("ascii", "replace")
    with open(ref, "rb") as f:
        b_text = f.read().decode("ascii", "replace")
    verdicts = []
    # HEADER byte-exact (line endings normalized)
    ha = "\n".join(dxf_sections(a_text).get("HEADER", ["<missing>"]))
    hb = "\n".join(dxf_sections(b_text).get("HEADER", ["<missing>"]))
    if ha == hb:
        rep.verdict("HEADER", PASS, "byte-exact")
    else:
        rep.verdict("HEADER", FAIL, "ours %r vs ref %r" % (ha[:60], hb[:60]))
        rep.diff_class("header", "HEADER section differs", False)
        verdicts.append(FAIL)
    crlf_a, crlf_b = "\r\n" in a_text, "\r\n" in b_text
    if crlf_a != crlf_b:
        rep.line("- line endings differ (ours %s, ref %s) -- normalized for every comparison"
                 % ("CRLF" if crlf_a else "LF", "CRLF" if crlf_b else "LF"))
        rep.diff_class("line-endings", "CRLF vs LF", True)
        verdicts.append(NOISE)
    # layer table as sets
    la, lb = dxf_layer_table(a_text), dxf_layer_table(b_text)
    if la == lb:
        rep.verdict("LAYER table", PASS, "%s" % sorted(la))
    else:
        rep.verdict("LAYER table", FAIL, "ours %s vs ref %s" % (sorted(la), sorted(lb)))
        rep.diff_class("layer-table", "declared layers differ (note production's quirk: TEXT is used but undeclared)", False)
        verdicts.append(FAIL)
    # entities per layer
    ea, eb = parse_dxf(a_text), parse_dxf(b_text)
    by_a, by_b = {}, {}
    for e in ea:
        by_a.setdefault(e[0], []).append(e)
    for e in eb:
        by_b.setdefault(e[0], []).append(e)
    for layer in sorted(set(by_a) | set(by_b)):
        A = by_a.get(layer, [])
        B = by_b.get(layer, [])
        kinds_a = (sum(1 for e in A if e[1] == "circle"), sum(1 for e in A if e[1] == "pline"))
        kinds_b = (sum(1 for e in B if e[1] == "circle"), sum(1 for e in B if e[1] == "pline"))
        if len(A) != len(B) or kinds_a != kinds_b:
            rep.verdict("layer %s counts" % layer, FAIL,
                        "ours %d (%d circles, %d plines) vs ref %d (%d circles, %d plines)"
                        % (len(A), kinds_a[0], kinds_a[1], len(B), kinds_b[0], kinds_b[1]))
            rep.diff_class("count:" + layer, "entity counts differ on layer %s" % layer, False)
            verdicts.append(FAIL)
            continue
        max_dev = 0.0
        over = 0
        shape = 0
        closed_diff = 0
        for ea_, eb_ in zip(A, B):
            if ea_[1] != eb_[1]:
                shape += 1
                continue
            if ea_[1] == "circle":
                d = max(abs(ea_[2][0] - eb_[2][0]), abs(ea_[2][1] - eb_[2][1]), abs(ea_[2][2] - eb_[2][2]))
            else:
                ca, pa = ea_[2]
                cb, pb = eb_[2]
                if ca != cb:
                    closed_diff += 1
                if len(pa) != len(pb):
                    shape += 1
                    continue
                d = 0.0
                for (x1, y1), (x2, y2) in zip(pa, pb):
                    d = max(d, abs(x1 - x2), abs(y1 - y2))
            max_dev = max(max_dev, d)
            if d > tol:
                over += 1
        ok = over == 0 and shape == 0 and closed_diff == 0
        rep.verdict("layer %s" % layer, PASS if ok else FAIL,
                    "%d entities, max |d| %.4f mm, %d over tol, %d shape mismatches, %d closed-flag mismatches"
                    % (len(A), max_dev, over, shape, closed_diff))
        if not ok:
            rep.diff_class("geometry:" + layer, "entities on %s differ beyond tolerance / in shape (index-paired: file order must be the production order)" % layer, False)
            verdicts.append(FAIL)
        elif max_dev > 0.0:
            rep.diff_class("rounding:" + layer, "sub-tolerance coordinate differences (recovered-layout rounding)", True,
                           "max %.4f mm" % max_dev)
            verdicts.append(NOISE)
    v = worst(verdicts) if verdicts else PASS
    rep.verdicts.append(v)
    rep.line("")
    rep.line("**File verdict: %s**" % v)
    if report is None:
        print(rep.text())
    return v


# ============================================================
# manifest.csv
# ============================================================

_MANIFEST_HEADER = "id,idx,bin,printer,plate,file,x_mm,y_mm,w_mm,d_mm,height_mm,area_mm2,est_g"
# printed precision per numeric column -> one unit in the last place
_MANIFEST_ULP = {"x_mm": 0.1, "y_mm": 0.1, "w_mm": 0.1, "d_mm": 0.1, "height_mm": 0.1,
                 "area_mm2": 1.0, "est_g": 0.1}


def _read_manifest(path):
    with open(path, "rb") as f:
        raw = f.read()
    text = raw.decode("utf-8")
    crlf = "\r\n" in text
    lines = [ln for ln in text.replace("\r\n", "\n").split("\n") if ln != ""]
    return raw, crlf, lines


def compare_manifest(ours, ref, report=None):
    rep = report or Report("manifest compare")
    rep.section("manifest: %s vs %s" % (os.path.basename(ours), os.path.basename(ref)))
    raw_a, crlf_a, la = _read_manifest(ours)
    raw_b, crlf_b, lb = _read_manifest(ref)
    verdicts = []
    if raw_a == raw_b:
        rep.verdict("bytes", PASS, "byte-exact, %d rows" % (len(la) - 1))
        v = PASS
        rep.line("")
        rep.line("**File verdict: %s**" % v)
        if report is None:
            print(rep.text())
        return v
    if not la or not lb or la[0] != _MANIFEST_HEADER or lb[0] != _MANIFEST_HEADER:
        rep.verdict("header", FAIL, "ours %r vs ref %r" % (la[:1], lb[:1]))
        rep.diff_class("header", "manifest header differs", False)
        v = FAIL
        rep.line("")
        rep.line("**File verdict: %s**" % v)
        if report is None:
            print(rep.text())
        return v
    if crlf_a != crlf_b:
        rep.diff_class("line-endings", "CRLF vs LF (ours %s, ref %s)" % (
            "CRLF" if crlf_a else "LF", "CRLF" if crlf_b else "LF"), True)
        verdicts.append(NOISE)
    rows_a = la[1:]
    rows_b = lb[1:]
    key = lambda r: r.split(",", 1)[0]
    sa = sorted(rows_a, key=key)
    sb = sorted(rows_b, key=key)
    if rows_a != rows_b and sa == sb:
        rep.diff_class("row-order", "same rows, different order (row order is plate order)", True)
        verdicts.append(NOISE)
    if sa == sb:
        rep.verdict("rows (sorted by id)", PASS, "%d rows identical" % len(sa))
    else:
        ids_a = set(key(r) for r in rows_a)
        ids_b = set(key(r) for r in rows_b)
        if ids_a != ids_b:
            rep.verdict("row set", FAIL, "%d rows vs %d; missing ids %s; extra ids %s" % (
                len(rows_a), len(rows_b), sorted(ids_b - ids_a)[:6], sorted(ids_a - ids_b)[:6]))
            rep.diff_class("row-set", "different set of part ids in the manifest", False)
            verdicts.append(FAIL)
        else:
            cols = _MANIFEST_HEADER.split(",")
            by_b = dict((key(r), r.split(",")) for r in rows_b)
            exact_bad = {}
            num_bad = {}
            num_noise = {}
            max_dev = {}
            for r in rows_a:
                fa = r.split(",")
                fb = by_b[key(r)]
                if len(fa) != len(cols) or len(fb) != len(cols):
                    exact_bad["<columns>"] = exact_bad.get("<columns>", 0) + 1
                    continue
                for c, x, y in zip(cols, fa, fb):
                    if x == y:
                        continue
                    if c in _MANIFEST_ULP:
                        try:
                            d = abs(float(x) - float(y))
                        except ValueError:
                            exact_bad[c] = exact_bad.get(c, 0) + 1
                            continue
                        max_dev[c] = max(max_dev.get(c, 0.0), d)
                        if d <= _MANIFEST_ULP[c] + 1e-9:
                            num_noise[c] = num_noise.get(c, 0) + 1
                        else:
                            num_bad[c] = num_bad.get(c, 0) + 1
                    else:
                        exact_bad[c] = exact_bad.get(c, 0) + 1
            if exact_bad:
                rep.verdict("structural columns", FAIL, "mismatches per column: %s" % exact_bad)
                rep.diff_class("structural", "id/idx/bin/printer/plate/file differ (packing or ids differ)", False)
                verdicts.append(FAIL)
            else:
                rep.verdict("structural columns", PASS, "id/idx/bin/printer/plate/file identical for all rows")
            if num_bad:
                rep.verdict("numeric columns", FAIL, "beyond one last-digit unit: %s; max deviations %s" % (num_bad, max_dev))
                rep.diff_class("numeric", "numeric columns differ beyond the printed precision", False)
                verdicts.append(FAIL)
            elif num_noise:
                rep.verdict("numeric columns", NOISE, "last-digit flips only: %s; max deviations %s" % (num_noise, max_dev))
                rep.diff_class("numeric-rounding", "last-digit flips of %.1f/%.0f fields (recovered-layout rounding)", True)
                verdicts.append(NOISE)
    if not verdicts:
        rep.diff_class("trailing-bytes", "rows identical but the raw bytes differ (BOM / trailing newline)", True)
        verdicts.append(NOISE)
    v = worst(verdicts)
    rep.verdicts.append(v)
    rep.line("")
    rep.line("**File verdict: %s**" % v)
    if report is None:
        print(rep.text())
    return v


# ============================================================
# CLI
# ============================================================

def _exit_code(verdict, strict):
    if verdict == FAIL:
        return 1
    if verdict == NOISE and strict:
        return 2
    return 0


def _emit(rep, path):
    text = rep.text()
    if path:
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print("report written to %s (overall %s)" % (path, rep.overall()))
    else:
        print(text)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("3mf", help="compare a Bambu 3MF against a 3MF or a summary JSON")
    p.add_argument("ours")
    p.add_argument("ref")
    p.add_argument("--level", choices=("xml", "bytes"), default="xml")
    p.add_argument("--tol-bbox", type=float, default=0.02)
    p.add_argument("--tol-volume-rel", type=float, default=0.005)
    p.add_argument("--tol-volume-abs", type=float, default=1.0)
    p.add_argument("--tol-translation", type=float, default=0.01)
    p.add_argument("--report")
    p.add_argument("--strict", action="store_true")
    p = sub.add_parser("summarize", help="write the canonical summary JSON of a 3MF")
    p.add_argument("file")
    p.add_argument("-o", "--output")
    p = sub.add_parser("dxf", help="compare two board DXFs")
    p.add_argument("ours")
    p.add_argument("ref")
    p.add_argument("--tol", type=float, default=2e-3)
    p.add_argument("--report")
    p.add_argument("--strict", action="store_true")
    p = sub.add_parser("manifest", help="compare two manifest.csv")
    p.add_argument("ours")
    p.add_argument("ref")
    p.add_argument("--report")
    p.add_argument("--strict", action="store_true")
    p = sub.add_parser("all", help="compare an output directory against a reference directory")
    p.add_argument("--ours", required=True, help="dir with plates_f*.3mf, board.dxf, manifest.csv")
    p.add_argument("--ref", required=True, help="dir with plates_summary.json or plates_f*.3mf, board_postprocessed.dxf, manifest.csv")
    p.add_argument("--level", choices=("xml", "bytes"), default="xml")
    p.add_argument("--report")
    p.add_argument("--strict", action="store_true")
    args = ap.parse_args(argv)

    if args.cmd == "summarize":
        s = summarize_3mf(args.file)
        text = json.dumps(s, indent=1, sort_keys=True)
        if args.output:
            with open(args.output, "w", encoding="utf-8", newline="\n") as f:
                f.write(text + "\n")
            print("summary of %s: %d objects, %d plates -> %s" % (args.file, len(s["objects"]), len(s["plates"]), args.output))
        else:
            print(text)
        return 0
    if args.cmd == "3mf":
        rep = Report("3MF equivalence")
        v = compare_3mf(args.ours, args.ref, args.level, args.tol_bbox, args.tol_volume_rel,
                        args.tol_volume_abs, args.tol_translation, rep)
        _emit(rep, args.report)
        return _exit_code(v, args.strict)
    if args.cmd == "dxf":
        rep = Report("DXF equivalence")
        v = compare_dxf(args.ours, args.ref, args.tol, rep)
        _emit(rep, args.report)
        return _exit_code(v, args.strict)
    if args.cmd == "manifest":
        rep = Report("manifest equivalence")
        v = compare_manifest(args.ours, args.ref, rep)
        _emit(rep, args.report)
        return _exit_code(v, args.strict)
    if args.cmd == "all":
        rep = Report("Wall corpus output equivalence")
        rep.line("ours: `%s`  " % os.path.abspath(args.ours))
        rep.line("ref: `%s`" % os.path.abspath(args.ref))
        verdicts = []
        plates = sorted(f for f in os.listdir(args.ours) if re.match(r"plates_f\d+_.*\.3mf$", f))
        if not plates:
            rep.verdict("3MF files", FAIL, "no plates_f*.3mf in %s" % args.ours)
            verdicts.append(FAIL)
        summary_json = None
        sj = os.path.join(args.ref, "plates_summary.json")
        if os.path.isfile(sj):
            with open(sj, "r", encoding="utf-8") as f:
                summary_json = json.load(f)
        for fname in plates:
            ours_path = os.path.join(args.ours, fname)
            ref_path = None
            base = re.sub(r"\.3mf$", "", fname)
            # 1. a per-file summary written by `normalize.py summarize`
            for cand in (base + ".summary.json", base + ".json"):
                if os.path.isfile(os.path.join(args.ref, cand)):
                    ref_path = os.path.join(args.ref, cand)
                    break
            # 2. the production 3MF itself (any suffix, e.g. _v1.4.1_)
            if ref_path is None:
                for cand in sorted(os.listdir(args.ref)):
                    if cand.endswith(".3mf") and cand.startswith(base):
                        ref_path = os.path.join(args.ref, cand)
                        break
            # 3. one plates_summary.json (normalize.py format, or a dict keyed by file name)
            if ref_path is None and summary_json is not None:
                entry = None
                if summary_json.get("format") == "cicada-corpus-3mf-summary-1":
                    entry = summary_json
                else:
                    for key_, val in summary_json.items():
                        if isinstance(val, dict) and str(key_).startswith(base):
                            entry = val
                            break
                if entry is not None:
                    tmp = os.path.join(args.ours, "." + base + ".ref-summary.json")
                    with open(tmp, "w", encoding="utf-8") as f:
                        json.dump(entry, f)
                    ref_path = tmp
            if ref_path is None:
                rep.section("3MF: %s" % fname)
                rep.verdict(fname, FAIL, "no reference found in %s: expected %s.summary.json (write it with "
                            "`normalize.py summarize <production.3mf> -o ...`), a %s*.3mf, or a plates_summary.json "
                            "in normalize.py's format" % (args.ref, base, base))
                verdicts.append(FAIL)
                continue
            verdicts.append(compare_3mf(ours_path, ref_path, args.level, report=rep))
            if ref_path.endswith(".ref-summary.json"):
                os.remove(ref_path)
        # Plate COVERAGE: every reference plate must have an ours file. The
        # loop above only iterates OUR plates, so a regression that drops a
        # whole colour/printer bin (export_bambu writes fewer files, exits 0)
        # would leave a reference plate unchecked and pass. Enumerate the
        # reference plate set (per-file summaries + any plates_f*.3mf) and
        # FAIL on any missing from ours. (Within-plate part loss is already
        # caught by compare_3mf's object-count check.)
        ours_bases = {re.sub(r"\.3mf$", "", f) for f in plates}
        ref_bases = set()
        for cand in os.listdir(args.ref):
            m = re.match(r"(plates_f\d+_.*?)(?:\.summary)?\.json$", cand)
            if m and re.match(r"plates_f\d+_", m.group(1)):
                ref_bases.add(m.group(1))
            m = re.match(r"(plates_f\d+_[^.]*)\.3mf$", cand)
            if m:
                ref_bases.add(m.group(1))
        missing = sorted(b for b in ref_bases if b not in ours_bases)
        rep.section("plate coverage")
        if missing:
            rep.verdict("plate coverage", FAIL,
                        "%d reference plate(s) have no ours file: %s" % (len(missing), missing))
            rep.diff_class("plate-coverage",
                           "the engine emitted fewer plates than production (a dropped colour/printer bin)",
                           False)
            verdicts.append(FAIL)
        else:
            rep.verdict("plate coverage", PASS,
                        "%d reference plate(s), all present in ours" % len(ref_bases))
        ours_dxf = os.path.join(args.ours, "board.dxf")
        ref_dxf = None
        for cand in ("board_postprocessed.dxf", "board.dxf"):
            if os.path.isfile(os.path.join(args.ref, cand)):
                ref_dxf = os.path.join(args.ref, cand)
                break
        if os.path.isfile(ours_dxf) and ref_dxf:
            verdicts.append(compare_dxf(ours_dxf, ref_dxf, report=rep))
        else:
            rep.section("DXF")
            rep.verdict("board.dxf", FAIL, "missing ours (%s) or ref (%s)" % (os.path.isfile(ours_dxf), bool(ref_dxf)))
            verdicts.append(FAIL)
        om = os.path.join(args.ours, "manifest.csv")
        rm = os.path.join(args.ref, "manifest.csv")
        if os.path.isfile(om) and os.path.isfile(rm):
            verdicts.append(compare_manifest(om, rm, report=rep))
        else:
            rep.section("manifest")
            rep.verdict("manifest.csv", FAIL, "missing ours (%s) or ref (%s)" % (os.path.isfile(om), os.path.isfile(rm)))
            verdicts.append(FAIL)
        _emit(rep, args.report)
        return _exit_code(worst(verdicts), args.strict)
    return 1


if __name__ == "__main__":
    sys.exit(main())
