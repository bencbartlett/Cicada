"""Optional cross-checks of the ported writers against the PRODUCTION
reference exports in the wall repo (READ ONLY). Skipped when the wall
repo is not present on this machine; the offline unit tests in
test_scripts.py / test_normalize.py never need it.

What is checked (cheap: no object meshes are parsed):
  * export_bambu's non-mesh 3MF entries rebuilt from the production
    metadata of the pristine plates_f3_teal_H2 file are BYTE-EXACT;
  * the H2 embedded profile (example_settings.3mf + proc overlay) equals
    the production project_settings.config byte for byte;
  * the production board_postprocessed.dxf has the layer-table quirk and
    the entity counts the corpus reproduces (TEXT used but undeclared),
    its TEXT glyphs are the ported FONT at cap height 5, BOARDCUT/STOCK
    are the ported constants;
  * the production manifest rows follow pool_filename / pool_of_bin and
    plate numbering is monotone in bin.
"""

import csv
import os
import re
import unittest
import zipfile

import test_support as ts

cicada = ts.install_stub()

F3 = os.path.join(ts.WALL_EXPORT, "plates_f3_teal_H2_v1.4.1_.3mf")
DXF = os.path.join(ts.WALL_EXPORT, "board_postprocessed.dxf")
MANIFEST = os.path.join(ts.WALL_EXPORT, "manifest.csv")
HAVE_WALL = (os.path.isfile(F3) and os.path.isfile(DXF) and os.path.isfile(MANIFEST)
             and os.path.isfile(os.path.join(ts.WALL_REPO, "example_settings.3mf")))


@unittest.skipUnless(HAVE_WALL, "wall repo reference exports not present (read-only optional check)")
class ProductionBambuTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.eb = ts.load_script("export_bambu")
        cls.z = zipfile.ZipFile(F3)
        cls.root = cls.z.read("3D/3dmodel.model").decode()
        cls.ms = cls.z.read("Metadata/model_settings.config").decode()
        cls.items = re.findall(
            r'<item objectid="(\d+)" p:UUID="[^"]*" transform="1 0 0 0 1 0 0 0 1 ([-\d.]+) ([-\d.]+) ([-\d.]+)" printable="1"/>',
            cls.root)
        cls.names = dict((int(i), nm) for i, nm in
                         re.findall(r'<object id="(\d+)">\s*<metadata key="name" value="([^"]*)"/>', cls.ms))
        cls.faces = [int(x) for x in re.findall(r'<metadata face_count="(\d+)"/>', cls.ms)]
        cls.plate_map = []
        for p in re.findall(r"<plate>(.*?)</plate>", cls.ms, re.DOTALL):
            cls.plate_map.append([int(x) // 2 for x in re.findall(r'key="object_id" value="(\d+)"', p)])
        cls.n = len(cls.items)
        cls.settings = cls.eb.load_settings(ts.WALL_REPO)

    @classmethod
    def tearDownClass(cls):
        cls.z.close()

    def test_non_mesh_entries_rebuild_byte_exact(self):
        n = self.n
        self.assertEqual(n, 258)
        names = [self.names[2 * k] for k in range(1, n + 1)]
        translations = [(float(a), float(b), float(c)) for (_o, a, b, c) in self.items]
        self.assertEqual(self.eb.bbl_root_model_xml(n, translations), self.root)
        fmaps = self.settings["h2"]["filament_maps"]
        self.assertEqual(fmaps, "2")
        self.assertEqual(self.eb.bbl_model_settings_xml(names, self.faces, self.plate_map, translations, fmaps), self.ms)
        self.assertEqual(self.eb.bbl_model_rels_xml(n), self.z.read("3D/_rels/3dmodel.model.rels").decode())
        self.assertEqual(self.eb.bbl_cut_information_xml(n), self.z.read("Metadata/cut_information.xml").decode())
        self.assertEqual(self.eb.bbl_filament_sequence_json(len(self.plate_map)),
                         self.z.read("Metadata/filament_sequence.json").decode())
        self.assertEqual(self.eb.BBL_CONTENT_TYPES, self.z.read("[Content_Types].xml").decode())
        self.assertEqual(self.eb.RELS, self.z.read("_rels/.rels").decode())
        self.assertEqual(self.eb.BBL_SLICE_INFO, self.z.read("Metadata/slice_info.config").decode())
        ranges = self.eb.layer_ranges_xml(n, self.settings["h2"]["range_xml"])
        self.assertEqual(ranges, self.z.read("Metadata/layer_config_ranges.xml").decode())

    def test_h2_profile_overlay_is_byte_exact(self):
        profile_bytes, path = self.settings["profiles"]["H2"]
        self.assertTrue(path.endswith("example_settings.3mf"))
        ours = self.eb.overlay_profile(profile_bytes, self.eb.pool_ref(self.settings, "H2", "proc"))
        self.assertEqual(ours, self.z.read("Metadata/project_settings.config"))

    def test_plate_grid_and_plate_count_match(self):
        # 15 plates, cols = ceil(sqrt(15)) = 4, stride 1.2 x 330 / 320:
        # the first object of plate k sits inside plate k's grid cell
        pp = ts.load_script("pack_plates")
        self.assertEqual(len(self.plate_map), 15)
        for k, ks in enumerate(self.plate_map):
            gx, gy = pp.plate_grid_pos(k, 15, 1.2 * 330.0, 1.2 * 320.0)
            (_o, tx, ty, _tz) = self.items[ks[0] - 1]
            self.assertTrue(gx <= float(tx) <= gx + 330.0, (k, tx, gx))
            self.assertTrue(gy <= float(ty) <= gy + 320.0, (k, ty, gy))

    def test_object_model_header_matches(self):
        obj = self.z.read("3D/Objects/object_1.model").decode()
        head = self.eb.bbl_object_model_xml(1, [], []).split("    <vertices>")[0]
        self.assertTrue(obj.startswith(head))
        self.assertTrue(obj.endswith(" </resources>\n <build/>\n</model>"))


@unittest.skipUnless(HAVE_WALL, "wall repo reference exports not present (read-only optional check)")
class ProductionDxfTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import normalize
        cls.nz = normalize
        cls.wl = ts.load_script("wall_labels")
        with open(DXF, "rb") as f:
            cls.raw = f.read()
        cls.text = cls.raw.decode("ascii")
        cls.ents = normalize.parse_dxf(cls.text)
        cls.by = {}
        for e in cls.ents:
            cls.by.setdefault(e[0], []).append(e)

    def test_layer_table_quirk_and_counts(self):
        self.assertEqual(self.nz.dxf_layer_table(self.text), {"OUTLINES", "PINHOLES", "BOARDCUT", "STOCK"})
        self.assertEqual(len(self.by["OUTLINES"]), 1200)
        self.assertEqual(len(self.by["TEXT"]), 3828)
        self.assertEqual(len(self.by["PINHOLES"]), 2296)  # 2 x (1200 - 58 coil-captured) + 12 coil holes
        self.assertEqual(len(self.by["BOARDCUT"]), 1)
        self.assertEqual(len(self.by["STOCK"]), 1)
        self.assertTrue(self.raw.startswith(b"0\r\nSECTION\r\n2\r\nHEADER\r\n0\r\nENDSEC\r\n"))

    def test_boardcut_stock_and_hole_radius_are_the_ported_constants(self):
        self.assertEqual(self.by["BOARDCUT"][0][2][1], [(0.0, 0.0), (2438.4, 0.0), (2438.4, 1219.2), (0.0, 1219.2)])
        self.assertEqual(self.by["STOCK"][0][2][1], [(-12.7, -12.7), (2451.1, -12.7), (2451.1, 1231.9), (-12.7, 1231.9)])
        self.assertTrue(all(abs(c[2][2] - 1.55) < 1e-9 for c in self.by["PINHOLES"]))
        # ghosts carry the closing duplicate vertex and the closed flag
        g = self.by["OUTLINES"][0]
        self.assertTrue(g[2][0])
        self.assertEqual(g[2][1][0], g[2][1][-1])

    def test_text_glyphs_are_the_ported_font_at_cap_height_5(self):
        # every TEXT polyline must be one FONT glyph (single polyline per
        # character) at some cap height h: match the shape by normalizing
        # to its own bbox origin and the glyph's cap height
        matched = 0
        closed_in_font = 0
        for e in self.by["TEXT"][:400]:
            closed, pts = e[2]
            ys = [p[1] for p in pts]
            xs = [p[0] for p in pts]
            h = None
            ok = False
            for ch, (adv, strokes) in self.wl.FONT.items():
                (gclosed, gpts) = strokes[0]
                if gclosed != closed or len(gpts) != len(pts):
                    continue
                gy = [p[1] for p in gpts]
                gx = [p[0] for p in gpts]
                span = max(gy) - min(gy)
                if span <= 0:
                    continue
                h_try = (max(ys) - min(ys)) / span
                x0 = min(xs) - min(gx) * h_try
                y0 = min(ys) - min(gy) * h_try
                if all(abs(x0 + p[0] * h_try - q[0]) < 2e-3 and abs(y0 + p[1] * h_try - q[1]) < 2e-3
                       for p, q in zip(gpts, pts)):
                    ok = True
                    h = h_try
                    break
            self.assertTrue(ok, "TEXT polyline is not a FONT glyph: %r" % (pts,))
            self.assertTrue(2.5 - 1e-6 <= h <= 5.0 + 1e-6, h)
            matched += 1
            if closed:
                closed_in_font += 1
        self.assertEqual(matched, 400)
        self.assertGreater(closed_in_font, 0)


@unittest.skipUnless(HAVE_WALL, "wall repo reference exports not present (read-only optional check)")
class ProductionManifestTest(unittest.TestCase):
    def test_rows_follow_the_ported_naming_and_plate_order(self):
        pp = ts.load_script("pack_plates")
        with open(MANIFEST, "r", newline="") as f:
            rows = list(csv.DictReader(f))
        self.assertEqual(len(rows), 1137)
        last_plate = 0
        last_bin = 0
        for r in rows:
            b = int(r["bin"])
            self.assertEqual(r["file"], pp.pool_filename(b))
            self.assertEqual(r["printer"], pp.pool_of_bin(b))
            p = int(r["plate"])
            self.assertGreaterEqual(p, last_plate)
            self.assertGreaterEqual(b, last_bin)
            last_plate, last_bin = p, b
            self.assertAlmostEqual(float(r["est_g"]), float(r["area_mm2"]) * 0.001, delta=0.1)  # both printed rounded
        with open(MANIFEST, "rb") as f:
            raw = f.read()
        self.assertIn(b"\r\n", raw)
        self.assertTrue(raw.startswith(b"id,idx,bin,printer,plate,file,x_mm,y_mm,w_mm,d_mm,height_mm,area_mm2,est_g\r\n"))


@unittest.skipUnless(HAVE_WALL, "wall repo reference exports not present (read-only optional check)")
class ProductionLabelsChainTest(unittest.TestCase):
    """End-to-end check of the labels -> pins -> DXF chain against the
    shipped board: the layout is RECONSTRUCTED from board_postprocessed.dxf
    itself (ghost outlines scaled back about their centroid = the cell,
    pin pairs = centroid + lean direction for the 1142 parts that have
    drills), then wall_labels + pin_cutters + export_dxf are run and the
    result compared with normalize.compare_dxf. IDs must equal the
    production manifests (1.4.1 and 1.4, 1142 parts with idx); OUTLINES,
    PINHOLES, BOARDCUT, STOCK must match within 2e-3 mm; TEXT must match
    for every part whose lean direction is known (the 58 coil-captured
    parts have no drills in the shipped file, so their direction -- and
    therefore their label offset -- cannot be recovered here)."""

    @classmethod
    def setUpClass(cls):
        import math
        import tempfile
        import normalize
        cls.nz = normalize
        cls.wl = ts.load_script("wall_labels")
        cls.pc = ts.load_script("pin_cutters")
        cls.ed = ts.load_script("export_dxf")
        with open(DXF, "rb") as f:
            text = f.read().decode("ascii")
        by = {}
        for e in normalize.parse_dxf(text):
            by.setdefault(e[0], []).append(e)
        ghosts = [e[2][1][:-1] for e in by["OUTLINES"]]
        cls.n = len(ghosts)
        cls.cents = []
        cls.cells = []
        for g in ghosts:
            cx, cy = cls.wl.poly_centroid(g)
            cls.cents.append((cx - 25.4, cy - 25.4, 0.0))
            cls.cells.append([(cx + (x - cx) / 0.75 - 25.4, cy + (y - cy) / 0.75 - 25.4, 0.0) for (x, y) in g])
        circ = [(c[2][0] - 25.4, c[2][1] - 25.4) for c in by["PINHOLES"]]
        part_pins = circ[:-12]
        cls.coil = [(x, y, 0.0) for (x, y) in circ[-12:]]
        cls.dirs = [None] * cls.n
        grid = {}
        for i, (cx, cy, _z) in enumerate(cls.cents):
            grid.setdefault((int(cx // 5), int(cy // 5)), []).append(i)
        for k in range(0, len(part_pins), 2):
            (x1, y1), (x2, y2) = part_pins[k], part_pins[k + 1]
            best = None
            for ox in (-1, 0, 1):
                for oy in (-1, 0, 1):
                    for i in grid.get((int(x1 // 5) + ox, int(y1 // 5) + oy), []):
                        d = math.hypot(cls.cents[i][0] - x1, cls.cents[i][1] - y1)
                        if best is None or d < best[0]:
                            best = (d, i)
            assert best is not None and best[0] < 0.01
            cls.dirs[best[1]] = (x2 - x1, y2 - y1)
        cls.directions = [cicada.Vector(d[0], d[1], 0.0) if d else cicada.Vector(1.0, 0.0, 0.0) for d in cls.dirs]
        cls.labels = cls.wl.wall_labels(cls.cells, cls.cents, cls.directions, (-25.4, -25.4, 0.0))
        cls.pins = cls.pc.pin_cutters(cls.cents, cls.directions, cls.cells)
        cls.dir = tempfile.mkdtemp(prefix="cicada-prod-")
        cls.out = os.path.join(cls.dir, "board.dxf")
        cls.ed.export_dxf(cls.labels["ghosts"], cls.labels["board_strokes"], cls.labels["board_strokes_closed"],
                          cls.pins["board_points"], cls.coil, (-25.4, -25.4, 0.0), (2413.0, 1193.8, 0.0),
                          skip_holes=[d is None for d in cls.dirs], path=cls.out)

    @classmethod
    def tearDownClass(cls):
        import shutil
        shutil.rmtree(cls.dir, ignore_errors=True)

    def test_reconstruction_is_the_production_part_set(self):
        self.assertEqual(self.n, 1200)
        self.assertEqual(sum(1 for d in self.dirs if d is None), 58)

    def test_ids_equal_both_production_manifests(self):
        for rel in (MANIFEST, os.path.join(ts.WALL_REPO, "export", "solenoid_art_export_1.4", "manifest.csv")):
            with open(rel, "r", newline="") as f:
                rows = list(csv.DictReader(f))
            bad = [(int(r["idx"]), r["id"], self.labels["ids"][int(r["idx"])])
                   for r in rows if self.labels["ids"][int(r["idx"])] != r["id"]]
            self.assertEqual(bad, [], "%s: %d id mismatches, first %s" % (rel, len(bad), bad[:5]))
        self.assertEqual(len(set(self.labels["ids"])), 1200)

    def test_dxf_layers_match_production_within_tolerance(self):
        rep = self.nz.Report("prod")
        self.nz.compare_dxf(self.out, DXF, 2e-3, rep)
        text = rep.text()
        for layer in ("OUTLINES", "PINHOLES", "BOARDCUT", "STOCK"):
            self.assertIn("**layer %s**: PASS" % layer, text)
        self.assertIn("**LAYER table**: PASS", text)
        self.assertIn("**HEADER**: PASS", text)
        self.assertIn("3828 entities", text)
        # TEXT: exact for every part with a known lean; mismatches only on
        # the 58 direction-less coil parts
        with open(DXF, "rb") as f:
            prod = [e for e in self.nz.parse_dxf(f.read().decode("ascii")) if e[0] == "TEXT"]
        owner = []
        for i, pid in enumerate(self.labels["ids"]):
            owner += [i] * len(pid)
        self.assertEqual(len(prod), len(owner))
        self.assertEqual(len(prod), len(self.labels["board_strokes"]))
        bad_parts = set()
        max_known = 0.0
        for k, (e, s) in enumerate(zip(prod, self.labels["board_strokes"])):
            pts = e[2][1]
            self.assertEqual(len(pts), len(s))
            d = max(max(abs(p[0] - (q[0] + 25.4)), abs(p[1] - (q[1] + 25.4))) for p, q in zip(pts, s))
            if d > 2e-3:
                bad_parts.add(owner[k])
            elif self.dirs[owner[k]] is not None:
                max_known = max(max_known, d)
        self.assertTrue(all(self.dirs[i] is None for i in bad_parts),
                        "TEXT differs on parts WITH known lean: %s" % sorted(i for i in bad_parts if self.dirs[i] is not None))
        self.assertLess(max_known, 2e-3)


if __name__ == "__main__":
    unittest.main()
