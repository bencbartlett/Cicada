# Wall corpus: the Bambu Studio production-format 3MF exporter + manifest
# (stage 6, docs/15). EFFECTFUL: writes files, never memoized; runs only
# on explicit request (`cicada run --node bambu`, POST /api/run/bambu).
#
# Ported from the wall repo's plate_packer.py: the pure 3MF writers
# (bbl_object_model_xml, bbl_root_model_xml, bbl_model_settings_xml,
# bbl_model_rels_xml, bbl_cut_information_xml, bbl_filament_sequence_json,
# BBL_CONTENT_TYPES / BBL_SLICE_INFO / RELS, center_mesh, write_bbl_3mf,
# layer_ranges_xml, overlay_profile) and the settings harvest (shared
# verbatim with pack_plates.py), plus the per-file assembly of
# plate_packer.py:1822-1986 (export loop + manifest.csv).
#
# Inputs are the ORIENTED carved meshes (Rust `orient` from pack_plates'
# frames: plate-grid world coordinates, base on z = 0) for ALL parts plus
# the per-part ids / bins / exported flags / plate numbers / slots and the
# manifest lines from pack_plates. Object names = part ids; mesh object
# ids 2k-1, wrapper ids 2k, layer_config_ranges keyed 1..N, identify_id
# from 100 per file -- all as production.
#
# THE ONE DECLARED DEVIATION from production: every ZIP entry carries the
# FIXED date_time 1980-01-01 00:00:00 (production stamped wall-clock
# time), so the files are byte-reproducible. Line endings of manifest.csv
# are CRLF explicitly (production wrote it in Windows text mode).

import json
import os
import re
import zipfile

import cicada

X1C_BINS = 3
COLOR_NAMES = ["emerald", "forest_green", "sea_green", "teal", "sky_blue"]
PROC_KEYS = ("top_shell_layers", "top_shell_thickness",
             "bottom_shell_layers", "bottom_shell_thickness",
             "sparse_infill_density")
FIXED_ZIP_TIME = (1980, 1, 1, 0, 0, 0)


def _pipeline_dir():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_path(path):
    path = str(path)
    if os.path.isabs(path):
        return path
    return os.path.normpath(os.path.join(_pipeline_dir(), path))


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


# ---- Bambu Studio production-format writers (plate_packer.py:760-1010) --
# Structure replicated from a reference project saved by Bambu Studio
# 2.7.1.57: meshes live in per-part 3D/Objects/object_k.model files (mesh
# object id 2k-1); the root model has wrapper objects (id 2k) with
# components referencing them; build items carry world transforms with
# printable="1"; Metadata/model_settings.config maps instances to plates.

BBL_NS = ('xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" '
          'xmlns:BambuStudio="http://schemas.bambulab.com/package/2021" '
          'xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" '
          'requiredextensions="p"')

BBL_APP = "BambuStudio-02.07.01.57"


def bbl_object_model_xml(k, verts, tris):
    """ported verbatim from plate_packer.py:777-800 -- 3D/Objects/
    object_k.model: one mesh object, id 2k-1."""
    L = []
    L.append('<?xml version="1.0" encoding="UTF-8"?>')
    L.append('<model unit="millimeter" xml:lang="en-US" %s>' % BBL_NS)
    L.append(' <metadata name="BambuStudio:3mfVersion">1</metadata>')
    L.append(' <resources>')
    L.append('  <object id="%d" p:UUID="%08x-81cb-4c03-9d28-80fed5dfa1dc" type="model">'
             % (2 * k - 1, k))
    L.append('   <mesh>')
    L.append('    <vertices>')
    for (x, y, z) in verts:
        L.append('     <vertex x="%.4f" y="%.4f" z="%.4f"/>' % (x, y, z))
    L.append('    </vertices>')
    L.append('    <triangles>')
    for (a, b, c) in tris:
        L.append('     <triangle v1="%d" v2="%d" v3="%d"/>' % (a, b, c))
    L.append('    </triangles>')
    L.append('   </mesh>')
    L.append('  </object>')
    L.append(' </resources>')
    L.append(' <build/>')
    L.append('</model>')
    return "\n".join(L)


def bbl_root_model_xml(n_parts, item_translations):
    """ported verbatim from plate_packer.py:803-845 -- 3D/3dmodel.model:
    wrapper objects (id 2k) with components pointing at the object files;
    build items carry each part's FULL world position."""
    L = []
    L.append('<?xml version="1.0" encoding="UTF-8"?>')
    L.append('<model unit="millimeter" xml:lang="en-US" %s>' % BBL_NS)
    L.append(' <metadata name="Application">%s</metadata>' % BBL_APP)
    L.append(' <metadata name="BambuStudio:3mfVersion">1</metadata>')
    L.append(' <metadata name="Copyright"></metadata>')
    L.append(' <metadata name="CreationDate">2026-08-04</metadata>')
    L.append(' <metadata name="Description"></metadata>')
    L.append(' <metadata name="Designer"></metadata>')
    L.append(' <metadata name="DesignerCover"></metadata>')
    L.append(' <metadata name="DesignerUserId"></metadata>')
    L.append(' <metadata name="License"></metadata>')
    L.append(' <metadata name="ModificationDate">2026-08-04</metadata>')
    L.append(' <metadata name="Origin"></metadata>')
    L.append(' <metadata name="ProfileCover"></metadata>')
    L.append(' <metadata name="ProfileDescription"></metadata>')
    L.append(' <metadata name="ProfileTitle"></metadata>')
    L.append(' <metadata name="Title"></metadata>')
    L.append(' <resources>')
    for k in range(1, n_parts + 1):
        L.append('  <object id="%d" p:UUID="%08x-61cb-4c03-9d28-80fed5dfa1dc" type="model">'
                 % (2 * k, k))
        L.append('   <components>')
        L.append('    <component p:path="/3D/Objects/object_%d.model" objectid="%d" '
                 'p:UUID="%04x0000-b206-40ff-9872-83e8017abed1" '
                 'transform="1 0 0 0 1 0 0 0 1 0 0 0"/>' % (k, 2 * k - 1, k))
        L.append('   </components>')
        L.append('  </object>')
    L.append(' </resources>')
    L.append(' <build p:UUID="2c7c17d8-22b5-4d84-8835-1976022ea369">')
    for k in range(1, n_parts + 1):
        (ix, iy, iz) = item_translations[k - 1]
        L.append('  <item objectid="%d" p:UUID="%08x-b1ec-4553-aec9-835e5b724bb4" '
                 'transform="1 0 0 0 1 0 0 0 1 %.3f %.3f %.3f" printable="1"/>'
                 % (2 * k, 2 * k, ix, iy, iz))
    L.append(' </build>')
    L.append('</model>')
    return "\n".join(L)


def bbl_model_settings_xml(names, face_counts, plate_map, assemble_pos,
                           filament_maps=None):
    """ported verbatim from plate_packer.py:848-897 -- Metadata/
    model_settings.config: object entries (wrapper id 2k, part id 2k-1),
    plate blocks with instances (identify_id from 100), assemble block."""
    IDENTITY = "1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1"
    L = []
    L.append('<?xml version="1.0" encoding="UTF-8"?>')
    L.append('<config>')
    for k in range(1, len(names) + 1):
        L.append('  <object id="%d">' % (2 * k))
        L.append('    <metadata key="name" value="%s"/>' % names[k - 1])
        L.append('    <metadata key="extruder" value="1"/>')
        L.append('    <metadata face_count="%d"/>' % face_counts[k - 1])
        L.append('    <part id="%d" subtype="normal_part">' % (2 * k - 1))
        L.append('      <metadata key="name" value="%s"/>' % names[k - 1])
        L.append('      <metadata key="matrix" value="%s"/>' % IDENTITY)
        L.append('      <mesh_stat face_count="%d" edges_fixed="0" degenerate_facets="0" '
                 'facets_removed="0" facets_reversed="0" backwards_edges="0"/>'
                 % face_counts[k - 1])
        L.append('    </part>')
        L.append('  </object>')
    ident = 100
    for pi, part_ks in enumerate(plate_map):
        L.append('  <plate>')
        L.append('    <metadata key="plater_id" value="%d"/>' % (pi + 1))
        L.append('    <metadata key="plater_name" value=""/>')
        L.append('    <metadata key="locked" value="false"/>')
        L.append('    <metadata key="filament_map_mode" value="Auto For Flush"/>')
        if filament_maps:
            L.append('    <metadata key="filament_maps" value="%s"/>' % filament_maps)
        for k in part_ks:
            L.append('    <model_instance>')
            L.append('      <metadata key="object_id" value="%d"/>' % (2 * k))
            L.append('      <metadata key="instance_id" value="0"/>')
            L.append('      <metadata key="identify_id" value="%d"/>' % ident)
            L.append('    </model_instance>')
            ident += 1
        L.append('  </plate>')
    L.append('  <assemble>')
    for k in range(1, len(names) + 1):
        (ax, ay, az) = assemble_pos[k - 1]
        L.append('   <assemble_item object_id="%d" instance_id="0" '
                 'transform="1 0 0 0 1 0 0 0 1 %.3f %.3f %.3f" offset="0 0 0" />'
                 % (2 * k, ax, ay, az))
    L.append('  </assemble>')
    L.append('</config>')
    return "\n".join(L)


BBL_CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
    ' <Default Extension="rels" ContentType='
    '"application/vnd.openxmlformats-package.relationships+xml"/>\n'
    ' <Default Extension="model" ContentType='
    '"application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n'
    ' <Default Extension="png" ContentType="image/png"/>\n'
    ' <Default Extension="gcode" ContentType="text/x.gcode"/>\n'
    '</Types>'
)

BBL_SLICE_INFO = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<config>\n'
    '  <header>\n'
    '    <header_item key="X-BBL-Client-Type" value="slicer"/>\n'
    '    <header_item key="X-BBL-Client-Version" value="02.07.01.57"/>\n'
    '  </header>\n'
    '</config>\n'
)

RELS = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
    ' <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type='
    '"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
    '</Relationships>'
)


def bbl_model_rels_xml(n_parts):
    """ported verbatim from plate_packer.py:923-931"""
    L = []
    L.append('<?xml version="1.0" encoding="UTF-8"?>')
    L.append('<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">')
    for k in range(1, n_parts + 1):
        L.append(' <Relationship Target="/3D/Objects/object_%d.model" Id="rel-%d" '
                 'Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>' % (k, k))
    L.append('</Relationships>')
    return "\n".join(L)


def bbl_cut_information_xml(n_parts):
    """ported verbatim from plate_packer.py:934-943"""
    L = []
    L.append('<?xml version="1.0" encoding="utf-8"?>')
    L.append('<objects>')
    for k in range(1, n_parts + 1):
        L.append(' <object id="%d">' % k)
        L.append('  <cut_id id="0" check_sum="1" connectors_cnt="0"/>')
        L.append(' </object>')
    L.append('</objects>')
    return "\n".join(L)


def bbl_filament_sequence_json(n_plates):
    """ported verbatim from plate_packer.py:946-950"""
    entries = []
    for p in range(1, n_plates + 1):
        entries.append('"plate_%d":{"nozzle_sequence":[],"optimal_assignment":[],"sequence":[]}' % p)
    return "{" + ",".join(entries) + "}"


def center_mesh(verts):
    """ported verbatim from plate_packer.py:953-964 -- recenter vertices on
    their bbox center (Bambu convention). Returns (centered_verts, (cx,
    cy, cz))."""
    if not verts:
        return verts, (0.0, 0.0, 0.0)
    xs = [v[0] for v in verts]
    ys = [v[1] for v in verts]
    zs = [v[2] for v in verts]
    cx = 0.5 * (min(xs) + max(xs))
    cy = 0.5 * (min(ys) + max(ys))
    cz = 0.5 * (min(zs) + max(zs))
    return [(x - cx, y - cy, z - cz) for (x, y, z) in verts], (cx, cy, cz)


def _zip_write(zf, name, data):
    """adapted: zipfile.writestr with a FIXED timestamp (the declared
    deviation; production stamped wall-clock time) and writestr's default
    file mode, deflated."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    info = zipfile.ZipInfo(name, date_time=FIXED_ZIP_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o600 << 16
    zf.writestr(info, data)


def write_bbl_3mf(path, parts, plate_map, profile_bytes=None,
                  ranges_xml=None, filament_maps=None):
    """ported verbatim from plate_packer.py:967-1010 (adapted: _zip_write
    for fixed timestamps) -- Bambu Studio production-format multi-plate
    project. parts: list of (name, verts, tris, world_translation) in
    part-k order -- verts CENTERED at the part origin; plate_map: per
    plate, 1-based part indices."""
    n = len(parts)
    names = [p[0] for p in parts]
    face_counts = [len(p[2]) for p in parts]
    translations = [p[3] for p in parts]
    assemble_pos = translations
    zf = zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED)
    try:
        _zip_write(zf, "[Content_Types].xml", BBL_CONTENT_TYPES)
        _zip_write(zf, "_rels/.rels", RELS)
        _zip_write(zf, "3D/3dmodel.model", bbl_root_model_xml(n, translations))
        _zip_write(zf, "3D/_rels/3dmodel.model.rels", bbl_model_rels_xml(n))
        for k in range(1, n + 1):
            (_name, verts, tris, _tr) = parts[k - 1]
            _zip_write(zf, "3D/Objects/object_%d.model" % k,
                       bbl_object_model_xml(k, verts, tris))
        _zip_write(zf, "Metadata/model_settings.config",
                   bbl_model_settings_xml(names, face_counts, plate_map,
                                          assemble_pos, filament_maps))
        _zip_write(zf, "Metadata/slice_info.config", BBL_SLICE_INFO)
        _zip_write(zf, "Metadata/cut_information.xml", bbl_cut_information_xml(n))
        _zip_write(zf, "Metadata/filament_sequence.json", bbl_filament_sequence_json(len(plate_map)))
        if ranges_xml:
            _zip_write(zf, "Metadata/layer_config_ranges.xml", ranges_xml)
        if profile_bytes:
            _zip_write(zf, "Metadata/project_settings.config", profile_bytes)
    finally:
        zf.close()
    return path


# ---- settings reference (plate_packer.py:1013-1166) ----------------

def harvest_profile(path):
    """ported verbatim from plate_packer.py:1013-1032"""
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


def harvest_settings(path):
    """ported verbatim from plate_packer.py:1066-1136"""
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


def overlay_profile(profile_bytes, proc):
    """ported verbatim from plate_packer.py:1139-1150 -- overlay the
    harvested process keys onto an embedded profile."""
    if not profile_bytes or not proc:
        return profile_bytes
    try:
        data = json.loads(profile_bytes.decode("utf-8-sig"))
        for k, v in proc.items():
            data[k] = v
        return json.dumps(data).encode("utf-8")
    except Exception:
        return profile_bytes


def layer_ranges_xml(n_parts, range_block):
    """ported verbatim from plate_packer.py:1153-1166 -- the reference
    project's height-range block replicated for every part, keyed by the
    SEQUENTIAL 1-based object index."""
    L = ['<?xml version="1.0" encoding="utf-8"?>', '<objects>']
    for k in range(1, n_parts + 1):
        L.append(' <object id="%d">' % k)
        L.append("  " + range_block)
        L.append(' </object>')
    L.append('</objects>')
    return "\n".join(L)


def find_settings(settings_dir, filename):
    """adapted from plate_packer.py:1356-1370 (one directory)."""
    cand = os.path.join(settings_dir, filename)
    s = harvest_settings(cand)
    if s is not None:
        return s, cand
    return None, None


def find_profile(settings_dir, pool, settings_path, settings_x1c_path):
    """adapted from plate_packer.py:1768-1795 (one directory; raises
    instead of warning when nothing is found)."""
    cands = []
    if pool == "X1C":
        if settings_x1c_path:
            cands.append(settings_x1c_path)
        cands.append(os.path.join(settings_dir, "reference_x1c.3mf"))
        cands.append(os.path.join(settings_dir, "bambu_project_settings_x1c.json"))
    elif settings_path:
        cands.append(settings_path)
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
    for pool in ("X1C", "H2"):
        profiles[pool] = find_profile(settings_dir, pool, s_path, sx_path)
    return {"dir": settings_dir, "h2": S, "h2_path": s_path, "x1c": SX, "x1c_path": sx_path,
            "profiles": profiles}


def pool_ref(settings, pool, key):
    """ported verbatim from plate_packer.py:1378-1387 -- per-pool settings
    lookup with cross-fallback."""
    own = settings["x1c"] if pool == "X1C" else settings["h2"]
    other = settings["h2"] if pool == "X1C" else settings["x1c"]
    for s in (own, other):
        if s is not None and s.get(key):
            return s[key]
    return None


def mesh_arrays(mesh):
    """A cicada.Mesh (or anything with .positions / .indices flat arrays)
    -> (verts [(x, y, z)], tris [(a, b, c)])."""
    pos = mesh.positions
    idx = mesh.indices
    if len(pos) % 3 or len(idx) % 3:
        raise ValueError("export_bambu: malformed mesh buffers (%d positions, %d indices)"
                         % (len(pos), len(idx)))
    verts = [(float(pos[3 * i]), float(pos[3 * i + 1]), float(pos[3 * i + 2]))
             for i in range(len(pos) // 3)]
    tris = [(int(idx[3 * i]), int(idx[3 * i + 1]), int(idx[3 * i + 2]))
            for i in range(len(idx) // 3)]
    return verts, tris


def assemble_files(meshes, ids, bins, exported, plates, slots, settings):
    """Per output file: the (name, centered verts, tris, translation) list
    in production object order (plates by number, parts by slot) and the
    plate_map. Returns [(filename, pool, bbl_parts, plate_map)] in bin
    order (plate_packer.py:1822-1946)."""
    n = len(meshes)
    by_bin = {}
    for i in range(n):
        if not exported[i]:
            continue
        if plates[i] <= 0 or slots[i] < 0:
            raise ValueError("export_bambu: part %d (%s) is exported but has plate %d / slot %d"
                             % (i, ids[i], plates[i], slots[i]))
        by_bin.setdefault(bins[i], {}).setdefault(plates[i], []).append((slots[i], i))
    out = []
    for b in sorted(by_bin):
        pool = pool_of_bin(b)
        bbl_parts = []
        plate_map = []
        for number in sorted(by_bin[b]):
            items = sorted(by_bin[b][number])
            seen = set()
            part_ks = []
            for (slot, i) in items:
                if slot in seen:
                    raise ValueError("export_bambu: duplicate slot %d on plate %d" % (slot, number))
                seen.add(slot)
                verts, tris = mesh_arrays(meshes[i])
                if not verts or not tris:
                    raise ValueError("export_bambu: part %d (%s) has an empty mesh" % (i, ids[i]))
                # Bambu convention: mesh centered at its own origin, build
                # item carries the full world position (the oriented mesh
                # already sits in the plate-grid world frame).
                cverts, center = center_mesh(verts)
                bbl_parts.append((ids[i], cverts, tris, center))
                part_ks.append(len(bbl_parts))
            plate_map.append(part_ks)
        out.append((pool_filename(b), pool, bbl_parts, plate_map))
    return out


@cicada.node(
    title="Export Bambu",
    description="write the five Bambu Studio production-format multi-plate 3MF files and manifest.csv (ported plate_packer.py writers; fixed zip timestamps).",
    effectful=True,
)
def export_bambu(
    meshes: "[Mesh]",
    ids: "[Text]",
    bins: "[Integer]",
    exported: "[Boolean]",
    plates: "[Integer]",
    slots: "[Integer]",
    manifest: "[Text]",
    settings_dir: "Text" = "inputs/bambu",
    out_dir: "Text" = "out",
) -> None:
    n = len(meshes)
    if not (len(ids) == len(bins) == len(exported) == len(plates) == len(slots) == n):
        raise ValueError(
            "export_bambu: list lengths differ (meshes %d, ids %d, bins %d, exported %d, "
            "plates %d, slots %d)" % (n, len(ids), len(bins), len(exported), len(plates), len(slots)))
    if not manifest or not manifest[0].startswith("id,idx,bin,"):
        raise ValueError("export_bambu: manifest must start with the header line")
    settings = load_settings(settings_dir)
    folder = resolve_path(out_dir)
    os.makedirs(folder, exist_ok=True)
    files = assemble_files(meshes, ids, bins, exported, plates, slots, settings)
    if not files:
        raise ValueError("export_bambu: no exported parts")
    for (fname, pool, bbl_parts, plate_map) in files:
        profile_bytes, _prof_path = settings["profiles"][pool]
        profile_use = overlay_profile(profile_bytes, pool_ref(settings, pool, "proc"))
        ranges = None
        range_block = pool_ref(settings, pool, "range_xml")
        if range_block:
            ranges = layer_ranges_xml(len(bbl_parts), range_block)
        fmaps = None
        if settings["h2"] and pool == "H2":
            fmaps = settings["h2"].get("filament_maps")
        write_bbl_3mf(os.path.join(folder, fname), bbl_parts, plate_map, profile_use, ranges, fmaps)
    # manifest.csv: production wrote "\n".join(rows) + "\n" in Windows text
    # mode -> CRLF; written explicitly so the bytes match on any platform.
    with open(os.path.join(folder, "manifest.csv"), "wb") as f:
        f.write(("\r\n".join(manifest) + "\r\n").encode("utf-8"))
    return None
