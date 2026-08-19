"""Unit tests for corpus/tools/normalize.py (the 3MF / DXF / manifest
normalizer + compare). Offline, deterministic: the fixtures are built
with the corpus writers themselves."""

import json
import os
import shutil
import tempfile
import unittest
import zipfile

import normalize as nz
import test_support as ts

cicada = ts.install_stub()


def write(path, text, binary=False):
    if binary:
        with open(path, "wb") as f:
            f.write(text)
    else:
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    return path


class ManifestCompareTest(unittest.TestCase):
    HEADER = "id,idx,bin,printer,plate,file,x_mm,y_mm,w_mm,d_mm,height_mm,area_mm2,est_g"
    ROWS = ["C1,0,0,X1C,1,plates_f0_emerald_X1C.3mf,-96.0,-87.0,40.1,47.1,69.6,7079,7.1",
            "A7,3,0,X1C,1,plates_f0_emerald_X1C.3mf,-41.7,-85.9,60.6,49.2,68.9,8771,8.8",
            "B2,9,3,H2,2,plates_f3_teal_H2.3mf,14.7,-86.8,45.4,47.4,68.8,8062,8.1"]

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="cicada-nz-")
        self.ref = write(os.path.join(self.dir, "ref.csv"), "\r\n".join([self.HEADER] + self.ROWS) + "\r\n")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def run_compare(self, rows, crlf=True):
        nl = "\r\n" if crlf else "\n"
        ours = write(os.path.join(self.dir, "ours.csv"), nl.join([self.HEADER] + rows) + nl)
        rep = nz.Report("t")
        v = nz.compare_manifest(ours, self.ref, rep)
        return v, rep.text()

    def test_identical_passes(self):
        v, text = self.run_compare(self.ROWS)
        self.assertEqual(v, nz.PASS)
        self.assertIn("byte-exact", text)

    def test_reordered_rows_are_declared_noise(self):
        v, text = self.run_compare([self.ROWS[1], self.ROWS[0], self.ROWS[2]])
        self.assertEqual(v, nz.NOISE)
        self.assertIn("row-order", text)

    def test_line_endings_are_declared_noise(self):
        v, text = self.run_compare(self.ROWS, crlf=False)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("line-endings", text)

    def test_last_digit_flip_is_noise_bigger_is_fail(self):
        flipped = list(self.ROWS)
        flipped[0] = flipped[0].replace("-96.0", "-96.1").replace(",7079,", ",7080,")
        v, text = self.run_compare(flipped)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("numeric-rounding", text)
        bad = list(self.ROWS)
        bad[0] = bad[0].replace("-96.0", "-96.3")
        v, text = self.run_compare(bad)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("beyond", text)

    def test_structural_column_change_fails(self):
        bad = list(self.ROWS)
        bad[2] = bad[2].replace(",3,H2,2,", ",3,H2,3,")
        v, text = self.run_compare(bad)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("structural", text)
        v, text = self.run_compare(self.ROWS[:2])
        self.assertEqual(v, nz.FAIL)
        self.assertIn("row-set", text)


class DxfCompareTest(unittest.TestCase):
    def setUp(self):
        self.ed = ts.load_script("export_dxf")
        self.dir = tempfile.mkdtemp(prefix="cicada-nz-")
        self.layers = ["OUTLINES", "PINHOLES", "BOARDCUT", "STOCK"]
        self.entities = [
            ("OUTLINES", "pline", (True, [(10.0, 10.0), (20.0, 10.0), (15.0, 18.0), (10.0, 10.0)])),
            ("TEXT", "pline", (False, [(12.0, 12.0), (13.0, 14.0), (14.0, 12.0)])),
            ("TEXT", "pline", (True, [(30.0, 12.0), (33.0, 12.0), (33.0, 17.0), (30.0, 17.0)])),
            ("PINHOLES", "circle", (15.0, 13.0, 1.55)),
            ("PINHOLES", "circle", (25.0, 13.0, 1.55)),
            ("BOARDCUT", "pline", (True, [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)])),
            ("STOCK", "pline", (True, [(-5.0, -5.0), (105.0, -5.0), (105.0, 55.0), (-5.0, 55.0)])),
        ]
        self.ref = write(os.path.join(self.dir, "ref.dxf"),
                         self.ed.dxf_document(self.layers, self.entities).replace("\n", "\r\n"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def compare(self, layers, entities, crlf=True, tol=2e-3):
        doc = self.ed.dxf_document(layers, entities)
        ours = write(os.path.join(self.dir, "ours.dxf"), doc.replace("\n", "\r\n") if crlf else doc)
        rep = nz.Report("t")
        v = nz.compare_dxf(ours, self.ref, tol, rep)
        return v, rep.text()

    def test_parse_roundtrip(self):
        with open(self.ref, "r") as f:
            ents = nz.parse_dxf(f.read())
        self.assertEqual(len(ents), len(self.entities))
        self.assertEqual(ents[0][0], "OUTLINES")
        self.assertEqual(ents[0][2][0], True)
        self.assertEqual(ents[0][2][1], self.entities[0][2][1])
        self.assertEqual(ents[3], ("PINHOLES", "circle", (15.0, 13.0, 1.55)))
        self.assertEqual(ents[1][2][0], False)
        with open(self.ref, "r") as f:
            self.assertEqual(nz.dxf_layer_table(f.read()), set(self.layers))

    def test_identical_passes_and_lf_is_noise(self):
        v, text = self.compare(self.layers, self.entities)
        self.assertEqual(v, nz.PASS)
        self.assertIn("HEADER**: PASS", text)
        v, text = self.compare(self.layers, self.entities, crlf=False)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("line-endings", text)

    def test_sub_tolerance_rounding_is_noise_and_beyond_fails(self):
        ents = list(self.entities)
        ents[3] = ("PINHOLES", "circle", (15.001, 13.0, 1.55))
        v, text = self.compare(self.layers, ents)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("rounding:PINHOLES", text)
        ents[3] = ("PINHOLES", "circle", (15.01, 13.0, 1.55))
        v, text = self.compare(self.layers, ents)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("geometry:PINHOLES", text)

    def test_counts_layer_table_and_closed_flags(self):
        v, text = self.compare(self.layers, self.entities[:-1])
        self.assertEqual(v, nz.FAIL)
        self.assertIn("count:STOCK", text)
        v, text = self.compare(self.layers + ["TEXT"], self.entities)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("layer-table", text)
        ents = list(self.entities)
        ents[1] = ("TEXT", "pline", (True, ents[1][2][1]))
        v, text = self.compare(self.layers, ents)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("closed-flag", text)
        ents = list(self.entities)
        ents[1] = ("TEXT", "pline", (False, ents[1][2][1] + [(15.0, 13.0)]))
        v, text = self.compare(self.layers, ents)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("shape mismatches", text)


class ThreeMfCompareTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.eb = ts.load_script("export_bambu")
        cls.pc = ts.load_script("pin_cutters")
        cls.dir = tempfile.mkdtemp(prefix="cicada-nz-")
        cls.profile = json.dumps({"printable_area": ["0x0", "256x0", "256x256", "0x256"], "sparse_infill_density": "0%"}).encode()
        cls.ranges = cls.eb.layer_ranges_xml(3, '<range min_z="0" max_z="4">\n   <option opt_key="wall_loops">3</option>\n  </range>')
        cls.parts = cls.make_parts(1.0)
        cls.ref = cls.write_3mf("ref.3mf", cls.parts)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    @classmethod
    def make_parts(cls, scale, shift=0.0):
        parts = []
        for k, (name, w, h) in enumerate((("A1", 10.0, 20.0), ("B2", 12.0, 15.0), ("C3", 8.0, 30.0))):
            verts, tris = cls.pc.prism_mesh([(0, 0), (w, 0), (w, w), (0, w)], 0.0, h)
            verts = [(x * scale, y * scale, z * scale) for (x, y, z) in verts]
            cverts, center = cls.eb.center_mesh(verts)
            parts.append((name, cverts, tris, (center[0] + 30.0 * k + shift, center[1], center[2])))
        return parts

    @classmethod
    def write_3mf(cls, name, parts, plate_map=None):
        path = os.path.join(cls.dir, name)
        cls.eb.write_bbl_3mf(path, parts, plate_map or [[1, 2], [3]], cls.profile, cls.ranges, None)
        return path

    def compare(self, path, ref=None, level="xml"):
        rep = nz.Report("t")
        v = nz.compare_3mf(path, ref or self.ref, level=level, report=rep)
        return v, rep.text()

    def test_identical_passes_at_both_levels_and_via_summary(self):
        ours = self.write_3mf("same.3mf", self.parts)
        self.assertEqual(self.compare(ours)[0], nz.PASS)
        self.assertEqual(self.compare(ours, level="bytes")[0], nz.PASS)
        summary = nz.summarize_3mf(self.ref)
        self.assertEqual(summary["format"], "cicada-corpus-3mf-summary-1")
        self.assertEqual([o["name"] for o in summary["objects"]], ["A1", "B2", "C3"])
        self.assertEqual(summary["plates"], [[1, 2], [3]])
        self.assertEqual(summary["objects"][0]["volume"], 2000.0)
        self.assertEqual(summary["objects"][0]["bbox"], [[-5.0, -5.0, -10.0], [5.0, 5.0, 10.0]])
        self.assertEqual(summary["objects"][0]["triangles"], 12)
        sj = os.path.join(self.dir, "ref_summary.json")
        with open(sj, "w", encoding="utf-8") as f:
            json.dump(summary, f)
        v, text = self.compare(ours, ref=sj)
        self.assertEqual(v, nz.PASS)
        self.assertIn("byte-exact after normalization", text)

    def test_small_geometry_differences_are_noise_large_fail(self):
        ours = self.write_3mf("tiny.3mf", self.make_parts(1.0002))
        v, text = self.compare(ours)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("geometry-noise", text)
        self.assertIn("3D/3dmodel.model**: PASS", text)  # translations normalized out of the byte compare
        # bytes level: mesh entries differ -> FAIL with the declared-noise class named
        v, text = self.compare(ours, level="bytes")
        self.assertEqual(v, nz.FAIL)
        self.assertIn("bytes:objects", text)
        ours = self.write_3mf("big.3mf", self.make_parts(1.05))
        v, text = self.compare(ours)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("bbox", text)

    def test_structure_changes_fail(self):
        ours = self.write_3mf("plates.3mf", self.parts, plate_map=[[1], [2, 3]])
        v, text = self.compare(ours)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("plate-map", text)
        parts = [("Z9",) + p[1:] for p in self.parts[:1]] + self.parts[1:]
        ours = self.write_3mf("names.3mf", parts)
        v, text = self.compare(ours)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("object-names", text)
        shifted = self.write_3mf("shift.3mf", self.make_parts(1.0, shift=0.5))
        v, text = self.compare(shifted)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("translation", text)

    def test_resaved_reference_declares_its_noise_but_keeps_hard_checks(self):
        # A reference re-saved by Bambu Studio: thumbnail entries, one object
        # nudged by hand. Entry list / XML / translation -> declared noise;
        # names, plates, bbox, volume stay hard checks.
        shifted = self.write_3mf("moved.3mf", self.make_parts(1.0, shift=0.5))
        resaved = os.path.join(self.dir, "resaved.3mf")
        with zipfile.ZipFile(shifted) as src, zipfile.ZipFile(resaved, "w", zipfile.ZIP_DEFLATED) as dst:
            for item in src.infolist():
                dst.writestr(item, src.read(item.filename))
            dst.writestr("Metadata/plate_1.png", b"\x89PNG not really")
        ours = self.write_3mf("ours.3mf", self.parts)
        v, text = self.compare(ours, ref=resaved)
        self.assertEqual(v, nz.NOISE)
        self.assertIn("RE-SAVED by Bambu Studio", text)
        self.assertIn("reference-resaved", text)
        self.assertIn("moved by hand", text)
        # ...but a wrong part id still fails against a re-saved reference
        parts = [("Z9",) + p[1:] for p in self.parts[:1]] + self.parts[1:]
        bad = self.write_3mf("badnames.3mf", parts)
        v, text = self.compare(bad, ref=resaved)
        self.assertEqual(v, nz.FAIL)
        self.assertIn("object-names", text)

    def test_date_metadata_is_stripped(self):
        # rewrite the ref with a different CreationDate -> still PASS
        src = zipfile.ZipFile(self.ref)
        path = os.path.join(self.dir, "dates.3mf")
        out = zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED)
        for info in src.infolist():
            data = src.read(info.filename)
            if info.filename == "3D/3dmodel.model":
                data = data.replace(b"2026-08-04", b"2031-01-01")
            out.writestr(info.filename, data)
        out.close()
        src.close()
        v, text = self.compare(path)
        self.assertEqual(v, nz.PASS)
        v, text = self.compare(path, level="bytes")
        self.assertEqual(v, nz.PASS)

    def test_cli_exit_codes(self):
        import contextlib
        import io
        ours = self.write_3mf("cli.3mf", self.make_parts(1.0002))
        report = os.path.join(self.dir, "r.md")
        sink = io.StringIO()
        with contextlib.redirect_stdout(sink):
            self.assertEqual(nz.main(["3mf", ours, self.ref, "--report", report]), 0)
            self.assertEqual(nz.main(["3mf", ours, self.ref, "--strict"]), 2)
            bad = self.write_3mf("cli_bad.3mf", self.make_parts(1.05))
            self.assertEqual(nz.main(["3mf", bad, self.ref]), 1)
        self.assertIn("**Overall: FAIL**", sink.getvalue())
        with open(report, "r", encoding="utf-8") as f:
            self.assertIn("**Overall: NOISE**", f.read())


class AllModeCoverageTest(unittest.TestCase):
    """Regression (adversarial review, stage 6): `all` mode enumerated only
    the OURS plates, so a dropped colour/printer bin (export_bambu writes
    fewer files, exits 0) passed unnoticed — a nightly false green."""

    @classmethod
    def setUpClass(cls):
        cls.eb = ts.load_script("export_bambu")
        cls.pc = ts.load_script("pin_cutters")
        cls.dir = tempfile.mkdtemp(prefix="cicada-nz-all-")
        cls.profile = json.dumps(
            {"printable_area": ["0x0", "256x0", "256x256", "0x256"], "sparse_infill_density": "0%"}
        ).encode()
        cls.ranges = cls.eb.layer_ranges_xml(
            3, '<range min_z="0" max_z="4">\n   <option opt_key="wall_loops">3</option>\n  </range>'
        )

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def _parts(self, k):
        name, w, h = (("A1", 10.0, 20.0), ("B2", 12.0, 15.0))[k % 2]
        verts, tris = self.pc.prism_mesh([(0, 0), (w, 0), (w, w), (0, w)], 0.0, h)
        cverts, center = self.eb.center_mesh(verts)
        return [(name, cverts, tris, (center[0], center[1], center[2]))]

    def _plate(self, d, base):
        path = os.path.join(d, base + ".3mf")
        self.eb.write_bbl_3mf(path, self._parts(0), [[1]], self.profile, self.ranges, None)
        return path

    def _run_all(self, ours, ref):
        import contextlib
        import io

        sink = io.StringIO()
        with contextlib.redirect_stdout(sink):
            code = nz.main(["all", "--ours", ours, "--ref", ref])
        return code, sink.getvalue()

    def test_all_fails_when_a_reference_plate_has_no_ours_file(self):
        bases = ["plates_f0_emerald_X1C", "plates_f1_forest_green_X1C"]
        ours = os.path.join(self.dir, "ours")
        ref = os.path.join(self.dir, "ref")
        os.makedirs(ours, exist_ok=True)
        os.makedirs(ref, exist_ok=True)
        # ref has two plate summaries + a DXF + a manifest; ours has both
        # plates + matching DXF/manifest → baseline passes.
        import contextlib
        import io

        for base in bases:
            p = self._plate(ours, base)
            with contextlib.redirect_stdout(io.StringIO()):
                nz.main(["summarize", p, "-o", os.path.join(ref, base + ".summary.json")])
        for d in (ours, ref):
            with open(os.path.join(d, "manifest.csv"), "w", newline="") as f:
                f.write("id\nA1\n")
        # a trivial matching DXF in both
        dxf = "0\nSECTION\n2\nHEADER\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nENDSEC\n0\nEOF\n"
        for d in (ours, ref):
            with open(os.path.join(d, "board.dxf" if d == ours else "board_postprocessed.dxf"), "w") as f:
                f.write(dxf)
        code, _ = self._run_all(ours, ref)
        self.assertEqual(code, 0, "baseline: both plates present → PASS/NOISE")
        # Now DROP one ours plate; the reference summary still exists.
        os.remove(os.path.join(ours, bases[1] + ".3mf"))
        code, text = self._run_all(ours, ref)
        self.assertEqual(code, 1, "a missing reference plate must FAIL")
        self.assertIn("plate coverage", text)
        self.assertIn(bases[1], text)


if __name__ == "__main__":
    unittest.main()
