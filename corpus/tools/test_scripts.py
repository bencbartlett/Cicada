"""Unit tests for the corpus Python script nodes (corpus/scripts/*.py),
run offline with the `cicada` stub:

    python -m unittest discover -s corpus/tools -p "test_*.py"

Deterministic: synthetic walls from seeded generators, no wall-clock, no
network, no engine. The optional production cross-checks live in
test_production_crosscheck.py (skipped when the wall repo is absent).
"""

import json
import math
import os
import random
import shutil
import tempfile
import unittest
import zipfile

import test_support as ts

cicada = ts.install_stub()


def tmpdir():
    d = tempfile.mkdtemp(prefix="cicada-corpus-test-")
    return d


# ============================================================
# solve_field
# ============================================================

class SolveFieldTest(unittest.TestCase):
    def setUp(self):
        self.sf = ts.load_script("solve_field")

    def test_single_wire_circulates_counterclockwise_and_is_unit(self):
        out = self.sf.solve_field([(10.0, 0.0, 0.0), (0.0, 10.0, 0.0), (-10.0, 0.0, 0.0)],
                                  [(0.0, 0.0, 0.0)], [], 1.0, 0.1)
        d = out["directions"]
        # out-of-page current: B ~ z x r: at +x the field points +y
        self.assertAlmostEqual(d[0].x, 0.0, places=12)
        self.assertAlmostEqual(d[0].y, 1.0, places=12)
        self.assertAlmostEqual(d[1].x, -1.0, places=12)
        self.assertAlmostEqual(d[2].y, -1.0, places=12)
        for v in d:
            self.assertAlmostEqual(math.hypot(v.x, v.y), 1.0, places=12)
            self.assertEqual(v.z, 0.0)
        # |B| = I / (r^2 + core^2)
        self.assertAlmostEqual(out["magnitudes"][0], 10.0 / (100.0 + 0.01), places=12)
        self.assertEqual(len(out["weights"]), 3)

    def test_in_wire_reverses_and_current_scales(self):
        a = self.sf.solve_field([(10.0, 0.0, 0.0)], [], [(0.0, 0.0, 0.0)], 2.0, 0.1)
        b = self.sf.solve_field([(10.0, 0.0, 0.0)], [(0.0, 0.0, 0.0)], [], 1.0, 0.1)
        self.assertAlmostEqual(a["directions"][0].y, -1.0, places=12)
        self.assertAlmostEqual(a["magnitudes"][0], 2.0 * b["magnitudes"][0], places=12)

    def test_degenerate_field_points_plus_x(self):
        # midway between an antiparallel pair the fields ADD, so use two
        # equal OUT wires with the point exactly between: B cancels
        out = self.sf.solve_field([(0.0, 0.0, 0.0)], [(-5.0, 0.0, 0.0), (5.0, 0.0, 0.0)], [], 1.0, 0.1)
        self.assertEqual((out["directions"][0].x, out["directions"][0].y), (1.0, 0.0))
        self.assertAlmostEqual(out["magnitudes"][0], 0.0, places=15)

    def test_auto_core_radius_is_five_percent_of_the_larger_extent(self):
        pts = [(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (0.0, 40.0, 0.0)]
        auto = self.sf.solve_field(pts, [(50.0, 20.0, 0.0)], [], 1.0, 0.0)
        explicit = self.sf.solve_field(pts, [(50.0, 20.0, 0.0)], [], 1.0, 5.0)
        self.assertEqual(auto["magnitudes"], explicit["magnitudes"])

    def test_refuses_without_wires_or_points(self):
        with self.assertRaises(ValueError):
            self.sf.solve_field([(0.0, 0.0, 0.0)], [], [], 1.0, 0.1)
        with self.assertRaises(ValueError):
            self.sf.solve_field([], [(0.0, 0.0, 0.0)], [], 1.0, 0.1)

    def test_deterministic(self):
        pts = [(random.Random(3).uniform(0, 2000), random.Random(4).uniform(0, 1000), 0.0) for _ in range(5)]
        a = self.sf.solve_field(pts, [(600.0, 600.0, 0.0)], [(1800.0, 600.0, 0.0)], 1.0, 0.1)
        b = self.sf.solve_field(pts, [(600.0, 600.0, 0.0)], [(1800.0, 600.0, 0.0)], 1.0, 0.1)
        self.assertEqual(a, b)


# ============================================================
# tip_caps
# ============================================================

class TipCapsTest(unittest.TestCase):
    def setUp(self):
        self.tc = ts.load_script("tip_caps")

    def test_cap_has_base_vertex_count_and_hits_the_triangle_corners(self):
        for k in (5, 6, 7, 8, 9):
            cell = ts.regular_polygon(100.0, 50.0, 20.0, k, phase=0.3)
            out = self.tc.tip_caps([cell], [(100.0, 50.0, 0.0)], [cicada.Vector(0.6, 0.8, 0.0)],
                                   [30.0], [40.0], 1.8)
            cap = out[0]
            self.assertEqual(len(cap), k)
            self.assertTrue(all(abs(p[2] - 40.0) < 1e-12 for p in cap))
            ax, ay = 100.0 + 0.6 * 30.0, 50.0 + 0.8 * 30.0
            tri = self.tc.tri_corners(ax, ay, 0.6, 0.8, 1.8, 1.0, 0.0, True)
            for c in tri:
                self.assertTrue(any(math.hypot(p[0] - c[0], p[1] - c[1]) < 1e-9 for p in cap),
                                "corner %r not hit by cap %r" % (c, cap))
            # every cap vertex lies within the triangle's circumradius of the apex
            for p in cap:
                self.assertLessEqual(math.hypot(p[0] - ax, p[1] - ay), 1.8 + 1e-9)
            # consecutive vertices never coincide (valid loft sections)
            for i in range(k):
                a, b = cap[i], cap[(i + 1) % k]
                self.assertGreater(math.hypot(a[0] - b[0], a[1] - b[1]), 1e-6)

    def test_nose_points_along_the_direction(self):
        cell = ts.regular_polygon(0.0, 0.0, 20.0, 6)
        ux, uy = 1.0 / math.sqrt(2.0), 1.0 / math.sqrt(2.0)
        cap = self.tc.tip_caps([cell], [(0.0, 0.0, 0.0)], [cicada.Vector(ux, uy, 0.0)], [10.0], [30.0])[0]
        ax, ay = 10.0 * ux, 10.0 * uy
        nose = max(cap, key=lambda p: (p[0] - ax) * ux + (p[1] - ay) * uy)
        self.assertAlmostEqual((nose[0] - ax) * ux + (nose[1] - ay) * uy, 1.8, places=9)

    def test_edge_mode_and_winding_preserved(self):
        cell = ts.regular_polygon(0.0, 0.0, 20.0, 7)
        cw = list(reversed(cell))
        for base in (cell, cw):
            cap = self.tc.tip_caps([base], [(0.0, 0.0, 0.0)], [cicada.Vector(0.0, 1.0, 0.0)], [5.0], [30.0],
                                   corner_snap=False)[0]
            self.assertEqual(len(cap), 7)
            sa_base = self.tc.signed_area([(p[0], p[1]) for p in base])
            sa_cap = self.tc.signed_area([(p[0], p[1]) for p in cap])
            self.assertEqual(sa_base > 0, sa_cap > 0)

    def test_scaled_cell_cap_is_the_production_mini_cell(self):
        cell = ts.regular_polygon(100.0, 50.0, 20.0, 7, phase=0.2)
        cap = self.tc.tip_caps([cell], [(100.0, 50.0, 0.0)], [cicada.Vector(0.6, 0.8, 0.0)],
                               [30.0], [40.0], cell_scale=0.07)[0]
        self.assertEqual(len(cap), 7)
        ax, ay = 100.0 + 0.6 * 30.0, 50.0 + 0.8 * 30.0
        for p, q in zip(cell, cap):
            self.assertAlmostEqual(q[0], ax + 0.07 * (p[0] - 100.0), places=12)
            self.assertAlmostEqual(q[1], ay + 0.07 * (p[1] - 50.0), places=12)
            self.assertEqual(q[2], 40.0)
        # the scaled cap's centroid is the apex
        sa = self.tc.signed_area([(p[0], p[1]) for p in cap])
        self.assertAlmostEqual(abs(sa), 0.07 ** 2 * abs(self.tc.signed_area([(p[0], p[1]) for p in cell])), places=9)

    def test_degenerate_direction_points_plus_y_and_bad_inputs_raise(self):
        cell = ts.regular_polygon(0.0, 0.0, 20.0, 6)
        cap = self.tc.tip_caps([cell], [(0.0, 0.0, 0.0)], [cicada.Vector(0.0, 0.0, 0.0)], [0.0], [30.0])[0]
        nose = max(cap, key=lambda p: p[1])
        self.assertAlmostEqual(nose[1], 1.8, places=9)
        with self.assertRaises(ValueError):
            self.tc.tip_caps([cell], [], [], [], [], 1.8)
        with self.assertRaises(ValueError):
            self.tc.tip_caps([cell], [(0.0, 0.0, 0.0)], [cicada.Vector(1.0, 0.0, 0.0)], [1.0], [10.0], 0.0)


# ============================================================
# wall_layout
# ============================================================

def synthetic_layout(n=6):
    parts = []
    for i in range(n):
        cx, cy = 100.0 + 60.0 * i, 80.0
        parts.append({"idx": i, "id": "A%d" % (i + 1), "zone": "A", "bin": i % 5,
                      "cell": [[cx - 20, cy - 20], [cx + 20, cy - 20], [cx + 20, cy + 20], [cx - 20, cy + 20]],
                      "centroid": [cx, cy], "lean": [0.6, 0.8], "lean_length": 12.5 + i,
                      "height": 30.0 + i, "exported": i != 2, "coil": 1 if i == 2 else None,
                      "production_id": "A%d" % (i + 1)})
    return {
        "source": "synthetic test", "units": "mm",
        "workable": {"min": [0, 0], "max": [2387.6, 1168.4]},
        "board": {"min": [-25.4, -25.4], "max": [2413.0, 1193.8]},
        "stock": {"min": [-38.1, -38.1], "max": [2425.7, 1206.5]},
        "wires": [{"center": [600.0, 600.0], "radius": 50.0, "current": 1.0},
                  {"center": [1800.0, 600.0], "radius": 50.0, "current": -1.0}],
        "coil_board_points": [[600.0 + 25.0 * math.cos(k * math.pi / 3), 600.0 + 25.0 * math.sin(k * math.pi / 3)]
                              for k in range(6)],
        "parts": parts,
        "seeds": [[100.0 + 60.0 * i + 1.5, 78.0] for i in range(n)],
        "cell_scales": [0.9 for _ in range(n)],
    }


class WallLayoutTest(unittest.TestCase):
    def setUp(self):
        self.wl = ts.load_script("wall_layout")
        self.dir = tmpdir()

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def write(self, data):
        p = os.path.join(self.dir, "layout.json")
        with open(p, "w", encoding="utf-8") as f:
            json.dump(data, f)
        return p

    def test_loads_and_fans_out(self):
        out = self.wl.wall_layout(self.write(synthetic_layout()))
        self.assertEqual(len(out["cells_production"]), 6)
        self.assertEqual(out["cells_production"][0][0], (80.0, 60.0, 0.0))
        self.assertEqual(out["centroids_production"][1], (160.0, 80.0, 0.0))
        self.assertEqual(out["seeds"][0], (101.5, 78.0, 0.0))
        self.assertEqual(out["cell_scales"], [0.9] * 6)
        self.assertEqual(out["heights"][2], 32.0)
        self.assertEqual(out["lean_lengths"][3], 15.5)
        self.assertEqual(out["bins"], [0, 1, 2, 3, 4, 0])
        self.assertEqual(out["exported"], [True, True, False, True, True, True])
        self.assertEqual(out["coil_captured"], [False, False, True, False, False, False])
        self.assertEqual(out["ids_production"][4], "A5")
        self.assertEqual(out["wires_out"], [(600.0, 600.0, 0.0)])
        self.assertEqual(out["wires_in"], [(1800.0, 600.0, 0.0)])
        self.assertEqual(len(out["coil_board_points"]), 6)
        self.assertEqual(out["board_min"], (-25.4, -25.4, 0.0))
        self.assertEqual(out["workable_max"], (2387.6, 1168.4, 0.0))
        self.assertEqual(out["leans_production"][0], cicada.Vector(0.6, 0.8, 0.0))

    def test_relative_path_resolves_against_the_pipeline_dir(self):
        self.assertEqual(self.wl.resolve_path("inputs/layout.json"),
                         os.path.normpath(os.path.join(ts.CORPUS_DIR, "inputs", "layout.json")))
        self.assertEqual(self.wl.resolve_path(self.dir), self.dir)

    def test_schema_violations_are_loud(self):
        good = synthetic_layout()
        bad = json.loads(json.dumps(good))
        del bad["parts"][0]["lean_length"]
        with self.assertRaises(ValueError):
            self.wl.wall_layout(self.write(bad))
        bad = json.loads(json.dumps(good))
        bad["parts"][1]["idx"] = 7
        with self.assertRaises(ValueError):
            self.wl.wall_layout(self.write(bad))
        bad = json.loads(json.dumps(good))
        bad["units"] = "in"
        with self.assertRaises(ValueError):
            self.wl.wall_layout(self.write(bad))
        bad = json.loads(json.dumps(good))
        bad["wires"][0]["current"] = 0.0
        with self.assertRaises(ValueError):
            self.wl.wall_layout(self.write(bad))
        bad = json.loads(json.dumps(good))
        bad["parts"][0]["coil"] = 3
        with self.assertRaises(ValueError):
            self.wl.wall_layout(self.write(bad))
        with self.assertRaises(OSError):
            self.wl.wall_layout(os.path.join(self.dir, "missing.json"))


# ============================================================
# wall_labels
# ============================================================

class WallLabelsTest(unittest.TestCase):
    def setUp(self):
        self.wl = ts.load_script("wall_labels")
        self.sf = ts.load_script("solve_field")
        self.wall = ts.synthetic_wall(60, seed=7)
        self.dirs = self.sf.solve_field(self.wall["centroids"], [(600.0, 600.0, 0.0)],
                                        [(1800.0, 600.0, 0.0)], 1.0, 0.1)["directions"]
        self.out = self.wl.wall_labels(self.wall["cells"], self.wall["centroids"], self.dirs,
                                       (-25.4, -25.4, 0.0))

    def test_ids_unique_zone_prefixed_and_ordinal_reading_order(self):
        ids = self.out["ids"]
        self.assertEqual(len(set(ids)), 60)
        for pid, zone in zip(ids, self.out["zones"]):
            self.assertTrue(pid.startswith(zone))
            self.assertTrue(pid[1:].isdigit())
            self.assertGreaterEqual(int(pid[1:]), 1)
        # column-major zones: leftmost centroids are A/B/C
        xs = [c[0] for c in self.wall["centroids"]]
        leftmost = min(range(60), key=lambda i: xs[i])
        self.assertIn(self.out["zones"][leftmost], "ABC")
        # within a zone the ordinal 1 is in the top band
        for z in set(self.out["zones"]):
            idxs = [i for i in range(60) if self.out["zones"][i] == z]
            first = next(i for i in idxs if ids[i] == z + "1")
            top_y = max(self.wall["centroids"][i][1] for i in idxs)
            bot_y = min(self.wall["centroids"][i][1] for i in idxs)
            rows = int(round(math.sqrt(len(idxs) * max(top_y - bot_y, 1e-9) /
                                       max(max(xs[i] for i in idxs) - min(xs[i] for i in idxs), 1e-9))))
            band = (top_y - bot_y) / max(rows, 1)
            self.assertLessEqual(top_y - self.wall["centroids"][first][1], band + 1e-9)

    def test_assign_ordinals_is_a_permutation_per_zone(self):
        cents = [(x, y, 0.0) for (x, y) in [(0, 0), (10, 0), (20, 0), (0, 10), (10, 10), (20, 10)]]
        zones = ["A"] * 6
        ords = self.wl.assign_ordinals(cents, zones)
        self.assertEqual(sorted(ords), [1, 2, 3, 4, 5, 6])
        # top band first, left to right
        self.assertEqual(ords[3:], [1, 2, 3])
        self.assertEqual(ords[:3], [4, 5, 6])

    def test_deboss_text_lines_and_coordinates(self):
        for i in range(60):
            lines = self.out["deboss_text"][i].split("\n")
            self.assertEqual(lines[0], self.out["ids"][i])
            mode = self.out["deboss_mode"][i]
            cx, cy, _ = self.wall["centroids"][i]
            xm, ym = self.wl.coords_mm(cx, cy, -25.4, -25.4)
            if mode == "ok3":
                self.assertEqual(lines, [self.out["ids"][i], "%04d" % xm, "%04d" % ym])
            elif mode == "ok2":
                self.assertEqual(lines, [self.out["ids"][i], "%04d %04d" % (xm, ym)])
            else:
                self.assertIn(mode, ("ok1", "relaxed", "tiny", "forced"))

    def test_deboss_frame_is_mirrored_on_the_base_face(self):
        # normal = x cross y must point -z; origin on z = +1 (under)
        for pl in self.out["deboss_plane"]:
            x, y = pl.x, pl.y
            nz = x[0] * y[1] - x[1] * y[0]
            self.assertAlmostEqual(nz, -1.0, places=12)
            self.assertEqual(pl.origin[2], 1.0)
            self.assertEqual(x[2], 0.0)
            self.assertEqual(y[2], 0.0)
            self.assertAlmostEqual(math.hypot(x[0], x[1]), 1.0, places=12)
            self.assertAlmostEqual(x[0] * y[0] + x[1] * y[1], 0.0, places=12)

    def test_deboss_block_sits_inside_the_ladder_block_and_inside_the_cell(self):
        # re-run the ladder for one part and check the DejaVu block estimate
        # maps to the same center and fits the stroke-font block
        P = self.wl.LabelParams()
        for i in (0, 7, 23, 41):
            cx, cy, cz = self.wall["centroids"][i]
            dx, dy = self.wl.unit_xy(self.dirs[i][0], self.dirs[i][1])
            poly = self.wl.cell_poly(self.wall["cells"][i])
            d = self.wl.equiv_dia(self.wl.poly_area(poly))
            xm, ym = self.wl.coords_mm(cx, cy, -25.4, -25.4)
            pid = self.out["ids"][i]
            tx, ty, th, mode, orient, lines, _v = self.wl.place_label(
                cx, cy, d, dx, dy, [pid, "%04d" % xm, "%04d" % ym], [pid, "%04d %04d" % (xm, ym)], [pid], poly, P)
            bw, bh = self.wl.block_dims(lines, th)
            origin, xa, ya, size = self.wl.deboss_frame(lines, tx, ty, th, orient, bw, bh)
            self.assertEqual(size, self.out["deboss_size"][i])
            self.assertLessEqual(size, th + 1e-12)
            n_l = len(lines)
            W = max(self.wl.dejavu_line_width(s, size) for s in lines)
            H = ((n_l - 1) * 1.35 + 1.0) * size
            self.assertLessEqual(W, bw + 1e-9)
            self.assertLessEqual(H, bh + 1e-9)
            # block corners in the text_solids frame -> world, must sit in
            # the ladder block (inflated by nothing) hence inside the cell
            corners = [(0.0, size), (W, size), (W, size - H), (0.0, size - H)]
            world = [(origin[0] + u * xa[0] + v * ya[0], origin[1] + u * xa[1] + v * ya[1]) for (u, v) in corners]
            we, he = self.wl.eff_dims(bw, bh, orient)
            for (wx, wy) in world:
                self.assertLessEqual(abs(wx - tx), 0.5 * we + 1e-9)
                self.assertLessEqual(abs(wy - ty), 0.5 * he + 1e-9)
                self.assertTrue(self.wl.point_in_convex(wx, wy, poly))
            # block center maps onto the placement center
            cxw = sum(p[0] for p in world) / 4.0
            cyw = sum(p[1] for p in world) / 4.0
            self.assertAlmostEqual(cxw, tx, places=9)
            self.assertAlmostEqual(cyw, ty, places=9)
            # mirrored: reading direction along +x world for orient 0 means
            # the text's y axis points -y
            if orient == 0:
                self.assertEqual((xa[0], xa[1]), (1.0, 0.0))
                self.assertEqual((ya[0], ya[1]), (0.0, -1.0))

    def test_ghosts_are_cells_scaled_about_the_centroid(self):
        for i in range(60):
            cx, cy, _ = self.wall["centroids"][i]
            cell = self.wall["cells"][i]
            g = self.out["ghosts"][i]
            self.assertEqual(len(g), len(cell))
            for p, q in zip(cell, g):
                self.assertAlmostEqual(q[0], cx + 0.75 * (p[0] - cx), places=12)
                self.assertAlmostEqual(q[1], cy + 0.75 * (p[1] - cy), places=12)
                self.assertEqual(q[2], 0.0)

    def test_board_strokes_use_the_font_and_stay_inside_the_cell(self):
        strokes = self.out["board_strokes"]
        closed = self.out["board_strokes_closed"]
        self.assertEqual(len(strokes), len(closed))
        # one polyline per character (every FONT glyph is ONE polyline)
        self.assertEqual(len(strokes), sum(len(pid) for pid in self.out["ids"]))
        k = 0
        for i in range(60):
            poly = self.wl.cell_poly(self.wall["cells"][i])
            for ch in self.out["ids"][i]:
                pts = strokes[k]
                self.assertEqual(closed[k], self.wl.FONT[ch][1][0][0])
                for p in pts:
                    self.assertGreaterEqual(self.wl.signed_inside(poly, p[0], p[1]), 0.0)
                    self.assertEqual(p[2], 0.0)
                k += 1

    def test_id_strokes_match_the_production_font_layout(self):
        unknown = set()
        strokes, w = self.wl.id_strokes("G75", 10.0, 20.0, 5.0, unknown)
        self.assertEqual(unknown, set())
        self.assertEqual(len(strokes), 3)
        self.assertAlmostEqual(w, (0.55 * 3 + 0.22 * 2) * 5.0)
        # the 'G' starts at the block's left edge, the cap top at cy + h/2
        self.assertAlmostEqual(min(p[0] for _c, pts in strokes for p in pts), 10.0 - 0.5 * w, places=12)
        self.assertAlmostEqual(max(p[1] for _c, pts in strokes for p in pts), 22.5, places=12)

    def test_ids_expected_mismatch_raises(self):
        wrong = list(self.out["ids"])
        wrong[5] = "Z999"
        with self.assertRaises(ValueError):
            self.wl.wall_labels(self.wall["cells"], self.wall["centroids"], self.dirs,
                                (-25.4, -25.4, 0.0), ids_expected=wrong)
        same = self.wl.wall_labels(self.wall["cells"], self.wall["centroids"], self.dirs,
                                   (-25.4, -25.4, 0.0), ids_expected=list(self.out["ids"]))
        self.assertEqual(same["ids"], self.out["ids"])

    def test_placement_ladder_covers_every_rung_on_tiny_cells(self):
        # a sliver cell forces the ladder down; nothing raises, every part
        # gets an id + a deboss frame
        cells = [ts.regular_polygon(50.0, 50.0, 4.0, 5), ts.regular_polygon(120.0, 50.0, 7.0, 6),
                 ts.regular_polygon(200.0, 50.0, 40.0, 8)]
        cents = [(50.0, 50.0, 0.0), (120.0, 50.0, 0.0), (200.0, 50.0, 0.0)]
        dirs = [cicada.Vector(1.0, 0.0, 0.0)] * 3
        out = self.wl.wall_labels(cells, cents, dirs, (0.0, 0.0, 0.0))
        self.assertEqual(len(out["ids"]), 3)
        self.assertIn(out["deboss_mode"][0], ("tiny", "forced", "relaxed", "ok1"))
        self.assertEqual(out["deboss_mode"][2], "ok3")
        self.assertTrue(any("Forced" in n or "FORCED" in n for n in out["notes"]))

    def test_deterministic(self):
        again = self.wl.wall_labels(self.wall["cells"], self.wall["centroids"], self.dirs, (-25.4, -25.4, 0.0))
        self.assertEqual(again, self.out)


# ============================================================
# pin_cutters
# ============================================================

class PinCuttersTest(unittest.TestCase):
    def setUp(self):
        self.pc = ts.load_script("pin_cutters")

    def test_profiles_are_ccw_and_match_production_constants(self):
        P = self.pc.PinParams()
        self.assertAlmostEqual(P.HOLE_DEPTH, 8.975)
        self.assertAlmostEqual(P.PROUD, 7.375)
        self.assertAlmostEqual(P.R_BORE, 1.7)
        prof = self.pc.rib_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.RIB_N)
        self.assertGreater(self.pc.signed_area2(prof), 0.0)
        radii = sorted(set(round(math.hypot(x, y), 6) for (x, y) in prof))
        self.assertEqual(radii, [1.56, 1.7])
        slot = self.pc.slot_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.SLOT_HALF)
        self.assertGreater(self.pc.signed_area2(slot), 0.0)
        # the cap arcs stop at a_end < 90 deg where the 45-degree drafts
        # meet the rails at y = +-r_rib, so |y| peaks below r_bore
        self.assertLess(max(abs(y) for (_x, y) in slot), 1.7)
        self.assertGreaterEqual(max(abs(y) for (_x, y) in slot), 1.56)
        self.assertAlmostEqual(max(x for (x, _y) in slot), 1.7 + 0.15)
        self.assertEqual(sum(1 for (_x, y) in slot if abs(abs(y) - 1.56) < 1e-9), 8)  # two 4-point rails
        cap = self.pc.capsule_pts(1.7, 0.15)
        self.assertEqual(len(cap), len(self.pc.capsule_pts(0.6, 0.15)))

    def test_triangulation_of_the_bore_profiles_and_random_star_polygons(self):
        P = self.pc.PinParams()
        for poly in (self.pc.rib_profile_pts(P.R_BORE, 1.56, 1.0, 3),
                     self.pc.slot_profile_pts(P.R_BORE, 1.56, 1.0, 0.15),
                     self.pc.capsule_pts(2.7, 0.15), self.pc.circle_pts(1.7, 48),
                     [(0, 0), (5, 0), (10, 0), (10, 10), (5, 10), (0, 10)],  # collinear runs
                     [(0, 0), (4, 0), (4, 1), (1, 1), (1, 3), (4, 3), (4, 4), (0, 4)]):  # C shape
            tris = self.pc.triangulate_polygon(poly)
            self.assertEqual(len(tris), len(poly) - 2)
            area = sum(0.5 * ((poly[b][0] - poly[a][0]) * (poly[c][1] - poly[a][1])
                              - (poly[c][0] - poly[a][0]) * (poly[b][1] - poly[a][1])) for (a, b, c) in tris)
            self.assertAlmostEqual(area, 0.5 * abs(self.pc.signed_area2(poly)), places=9)
            self.assertTrue(all(0.5 * ((poly[b][0] - poly[a][0]) * (poly[c][1] - poly[a][1])
                                       - (poly[c][0] - poly[a][0]) * (poly[b][1] - poly[a][1])) > 0
                                for (a, b, c) in tris) or self.pc.signed_area2(poly) < 0)
        rnd = random.Random(11)
        for _ in range(40):
            n = rnd.randint(5, 24)
            poly = []
            for k in range(n):
                a = 2 * math.pi * k / n
                r = rnd.uniform(0.3, 1.0)
                poly.append((r * math.cos(a), r * math.sin(a)))
            if rnd.random() < 0.5:
                poly.reverse()
            tris = self.pc.triangulate_polygon(poly)
            self.assertEqual(len(tris), n - 2)
            area = sum(abs(0.5 * ((poly[b][0] - poly[a][0]) * (poly[c][1] - poly[a][1])
                                  - (poly[c][0] - poly[a][0]) * (poly[b][1] - poly[a][1]))) for (a, b, c) in tris)
            self.assertAlmostEqual(area, 0.5 * abs(self.pc.signed_area2(poly)), places=9)
        with self.assertRaises(ValueError):
            self.pc.triangulate_polygon([(0, 0), (1, 0)])
        with self.assertRaises(ValueError):
            self.pc.triangulate_polygon([(0, 0), (1, 0), (2, 0)])

    def test_each_cutter_is_watertight_with_the_production_volumes_and_z_ranges(self):
        P = self.pc.PinParams()
        prof = self.pc.rib_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.RIB_N)
        slot = self.pc.slot_profile_pts(P.R_BORE, 0.5 * P.RIB_EFF, P.RIB_W, P.SLOT_HALF)
        meshes, holes, off = self.pc.part_cutters(10.0, 5.0, 0.0, 0.6, 0.8, 40.0, P, prof, slot)
        self.assertEqual(len(meshes), 6)
        self.assertEqual(off, 12.0)
        self.assertAlmostEqual(holes[1][0], 10.0 + 0.6 * 12.0)
        cone_h = (1.7 - 0.6) * math.tan(math.radians(60.0))
        expect_z = [(-1.0, 8.975), (-1.0, 1.0), (8.475, 8.975 + cone_h)] * 2
        for k, (verts, tris) in enumerate(meshes):
            self.assertTrue(self.pc.mesh_is_watertight(verts, tris), "cutter %d" % k)
            vol = self.pc.mesh_signed_volume(verts, tris)
            self.assertGreater(vol, 0.0)
            zs = [v[2] for v in verts]
            self.assertAlmostEqual(min(zs), expect_z[k][0], places=9)
            self.assertAlmostEqual(max(zs), expect_z[k][1], places=9)
        # bore prism volume = profile area x (HOLE_DEPTH + UNDER)
        v0 = self.pc.mesh_signed_volume(*meshes[0])
        self.assertAlmostEqual(v0, self.pc.poly_area(prof) * (P.HOLE_DEPTH + 1.0), places=6)
        # chamfer: r 2.7 cylinder 1 mm + frustum 2.7 -> 1.7 over 1 mm (48-gon)
        v1 = self.pc.mesh_signed_volume(*meshes[1])
        k48 = math.sin(2 * math.pi / 48) * 48 / (2 * math.pi)
        expect = k48 * (math.pi * 2.7 ** 2 * 1.0 + math.pi / 3.0 * 1.0 * (2.7 ** 2 + 2.7 * 1.7 + 1.7 ** 2))
        self.assertAlmostEqual(v1, expect, places=6)
        # the slot bore is wider than the round one along the lean axis
        slot_verts = meshes[3][0]
        ux, uy = 0.6, 0.8
        along = [((v[0] - holes[1][0]) * ux + (v[1] - holes[1][1]) * uy) for v in slot_verts]
        self.assertAlmostEqual(max(along), 1.7 + 0.15, places=9)

    def test_node_outputs_and_clamped_spacing(self):
        cells = [ts.regular_polygon(10.0, 5.0, 20.0, 6), ts.regular_polygon(60.0, 5.0, 12.0, 7)]
        out = self.pc.pin_cutters([(10.0, 5.0, 0.0), (60.0, 5.0, 0.0)],
                                  [cicada.Vector(0.6, 0.8, 0.0), cicada.Vector(1.0, 0.0, 0.0)], cells)
        self.assertEqual([len(c) for c in out["cutters"]], [6, 6])
        self.assertEqual(len(out["board_points"]), 4)
        self.assertEqual(out["board_points"][0], (10.0, 5.0, 0.0))
        area = self.pc.poly_area([(p[0], p[1]) for p in cells[1]])
        self.assertAlmostEqual(out["spacing"][1], min(12.0, 0.35 * self.pc.equiv_dia(area)))
        self.assertAlmostEqual(out["board_points"][3][0], 60.0 + out["spacing"][1])
        for part in out["cutters"]:
            for m in part:
                verts, tris = ts.mesh_arrays(m)
                self.assertTrue(self.pc.mesh_is_watertight(verts, tris))
        # hole_cone off -> 4 cutters; slot_half 0 -> round lean bore
        out2 = self.pc.pin_cutters([(10.0, 5.0, 0.0)], [cicada.Vector(1.0, 0.0, 0.0)], [cells[0]],
                                   hole_cone=False, slot_half=0.0)
        self.assertEqual(len(out2["cutters"][0]), 4)
        self.assertEqual(len(out2["cutters"][0][0].positions), len(out2["cutters"][0][2].positions))
        with self.assertRaises(ValueError):
            self.pc.pin_cutters([(0.0, 0.0, 0.0)], [cicada.Vector(1.0, 0.0, 0.0)], [cells[0]], rib_eff_dia=3.5)
        with self.assertRaises(ValueError):
            self.pc.pin_cutters([(0.0, 0.0, 0.0)], [], [cells[0]])

    def test_watertight_check_catches_holes_and_flips(self):
        verts, tris = self.pc.prism_mesh([(0, 0), (1, 0), (1, 1), (0, 1)], 0.0, 2.0)
        self.assertTrue(self.pc.mesh_is_watertight(verts, tris))
        self.assertAlmostEqual(self.pc.mesh_signed_volume(verts, tris), 2.0)
        self.assertFalse(self.pc.mesh_is_watertight(verts, tris[1:]))
        flipped = list(tris)
        a, b, c = flipped[0]
        flipped[0] = (a, c, b)
        self.assertFalse(self.pc.mesh_is_watertight(verts, flipped))
        cw = self.pc.prism_mesh([(0, 1), (1, 1), (1, 0), (0, 0)], 0.0, 2.0)
        self.assertAlmostEqual(self.pc.mesh_signed_volume(*cw), 2.0)


# ============================================================
# pack_plates
# ============================================================

class PackPlatesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pp = ts.load_script("pack_plates")
        cls.sf = ts.load_script("solve_field")
        cls.dir = tmpdir()
        cls.settings = ts.make_settings_dir(os.path.join(cls.dir, "bambu"))
        cls.wall = ts.synthetic_wall(48, seed=5)
        cls.dirs = cls.sf.solve_field(cls.wall["centroids"], [(600.0, 600.0, 0.0)],
                                      [(1800.0, 600.0, 0.0)], 1.0, 0.1)["directions"]
        cls.ids = ["P%d" % i for i in range(48)]
        cls.exported = [i % 9 != 4 for i in range(48)]
        cls.out = cls.pp.pack_plates(cls.wall["cells"], cls.wall["centroids"], cls.dirs,
                                     cls.wall["heights"], cls.wall["lean_lengths"], cls.ids,
                                     cls.wall["bins"], cls.exported, settings_dir=cls.settings)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def test_settings_harvest_reads_the_production_values(self):
        s = self.pp.load_settings(self.settings)
        self.assertEqual(s["h2"]["keepout"], (25.0, 0.0))
        self.assertEqual(s["h2"]["height_cap"], 325.0)
        self.assertEqual(s["h2"]["filament_maps"], "2")
        self.assertEqual(s["x1c"]["bed_exclude"], (0.0, 0.0, 18.0, 28.0))
        self.assertEqual(s["beds"], {"X1C": (256.0, 256.0), "H2": (330.0, 320.0)})
        self.assertEqual(s["h2"]["proc"]["sparse_infill_density"], "0%")
        self.assertIn("<range", s["h2"]["range_xml"])
        with self.assertRaises(ValueError):
            self.pp.load_settings(os.path.join(self.dir, "nope"))

    def test_excluded_parts_get_identity_frames_and_no_row(self):
        for i in range(48):
            if not self.exported[i]:
                self.assertEqual(self.out["plate"][i], 0)
                self.assertEqual(self.out["slot"][i], -1)
                self.assertEqual(self.out["printer"][i], "")
                self.assertEqual(self.out["file"][i], "")
                self.assertIsNone(self.out["manifest_rows"][i])
                self.assertEqual(self.out["part_frames"][i], self.out["plate_frames"][i])
            else:
                self.assertGreater(self.out["plate"][i], 0)
                self.assertGreaterEqual(self.out["slot"][i], 0)
                self.assertEqual(self.out["printer"][i], self.pp.pool_of_bin(self.wall["bins"][i]))
                self.assertEqual(self.out["file"][i], self.pp.pool_filename(self.wall["bins"][i]))
                self.assertTrue(self.out["manifest_rows"][i].startswith(self.ids[i] + ",%d," % i))

    def test_manifest_rows_are_in_plate_order_and_match_per_part_rows(self):
        m = self.out["manifest"]
        self.assertEqual(m[0], "id,idx,bin,printer,plate,file,x_mm,y_mm,w_mm,d_mm,height_mm,area_mm2,est_g")
        rows = [r for r in self.out["manifest_rows"] if r is not None]
        self.assertEqual(sorted(m[1:]), sorted(rows))
        plates = [int(r.split(",")[4]) for r in m[1:]]
        self.assertEqual(plates, sorted(plates))
        # slots count up within each plate in manifest order
        seen = {}
        for r in m[1:]:
            f = r.split(",")
            idx = int(f[1])
            p = int(f[4])
            self.assertEqual(self.out["slot"][idx], seen.get(p, 0))
            seen[p] = seen.get(p, 0) + 1
        self.assertEqual(len(self.out["plate_table"]), max(plates))
        # bins pack in order: plate numbers are monotone in bin
        bins_by_plate = {}
        for r in m[1:]:
            f = r.split(",")
            bins_by_plate.setdefault(int(f[4]), set()).add(int(f[2]))
        order = [min(bins_by_plate[p]) for p in sorted(bins_by_plate)]
        self.assertEqual(order, sorted(order))
        self.assertTrue(all(len(v) == 1 for v in bins_by_plate.values()))

    def test_frames_carry_the_lean_to_plus_y_and_land_on_the_packed_slot(self):
        """Apply the rigid motion source->target to the cell and apex: the
        lean must point +Y, the base stay on z = 0, and the footprint bbox
        center must sit at the manifest x_mm/y_mm relative to the plate
        center (+ the plate grid offset)."""
        m = {int(r.split(",")[1]): r.split(",") for r in self.out["manifest"][1:]}
        for i in range(48):
            if not self.exported[i]:
                continue
            f = ts.rigid_motion(self.out["part_frames"][i], self.out["plate_frames"][i])
            cx, cy, _ = self.wall["centroids"][i]
            ux, uy = self.dirs[i].x, self.dirs[i].y
            L = self.wall["lean_lengths"][i]
            apex = f((cx + ux * L, cy + uy * L, self.wall["heights"][i]))
            c2 = f((cx, cy, 0.0))
            self.assertAlmostEqual(c2[2], 0.0, places=9)
            self.assertAlmostEqual(apex[0] - c2[0], 0.0, places=9)  # lean -> +Y
            self.assertAlmostEqual(apex[1] - c2[1], L, places=9)
            self.assertAlmostEqual(apex[2], self.wall["heights"][i], places=9)
            pts = [f(p) for p in self.wall["cells"][i]]
            bx = 0.5 * (min(p[0] for p in pts) + max(p[0] for p in pts))
            by = 0.5 * (min(p[1] for p in pts) + max(p[1] for p in pts))
            row = m[i]
            pool = row[3]
            plate = int(row[4])
            local = self.out["plate_local"][i]
            n_in_file = len(set(int(r[4]) for r in m.values() if r[5] == row[5]))
            phys_w, phys_d = (256.0, 256.0) if pool == "X1C" else (330.0, 320.0)
            gx, gy = self.pp.plate_grid_pos(local - 1, n_in_file, 1.2 * phys_w, 1.2 * phys_d)
            x_shift = 0.0 if pool == "X1C" else 12.5
            y_shift = 7.5
            self.assertAlmostEqual(bx, gx + 0.5 * phys_w + x_shift + float(row[6]), delta=0.051)
            self.assertAlmostEqual(by, gy + 0.5 * phys_d + y_shift + float(row[7]), delta=0.051)
            self.assertAlmostEqual(max(p[0] for p in pts) - min(p[0] for p in pts), float(row[8]), delta=0.051)
            self.assertAlmostEqual(max(p[1] for p in pts) - min(p[1] for p in pts), float(row[9]), delta=0.051)
            self.assertGreater(plate, 0)

    def test_packed_footprints_keep_the_clearance_and_stay_on_the_plate(self):
        """Pairwise separation >= clearance between raw footprints on the
        same plate (the terrain guarantee), and every footprint inside the
        usable area of its plate."""
        by_plate = {}
        for i in range(48):
            if not self.exported[i]:
                continue
            f = ts.rigid_motion(self.out["part_frames"][i], self.out["plate_frames"][i])
            pts = [(p[0], p[1]) for p in (f(q) for q in self.wall["cells"][i])]
            by_plate.setdefault(self.out["plate"][i], []).append((i, pts))
        wl = ts.load_script("wall_labels")
        for plate, items in by_plate.items():
            for a in range(len(items)):
                for b in range(a + 1, len(items)):
                    pa, pb = items[a][1], items[b][1]
                    # min vertex-to-edge distance both ways (convex cells)
                    d = min(min(wl.seg_point_dist(p[0], p[1], q[k][0], q[k][1], q[(k + 1) % len(q)][0], q[(k + 1) % len(q)][1])
                                for p in P for k in range(len(q))) for (P, q) in ((pa, pb), (pb, pa)))
                    self.assertGreaterEqual(d, 3.0 - 1e-6, "plate %d parts %d/%d too close: %.3f" % (plate, items[a][0], items[b][0], d))

    def test_deterministic_and_loud_on_bad_inputs(self):
        again = self.pp.pack_plates(self.wall["cells"], self.wall["centroids"], self.dirs,
                                    self.wall["heights"], self.wall["lean_lengths"], self.ids,
                                    self.wall["bins"], self.exported, settings_dir=self.settings)
        self.assertEqual(again, self.out)
        with self.assertRaises(ValueError):
            self.pp.pack_plates(self.wall["cells"][:3], self.wall["centroids"], self.dirs,
                                self.wall["heights"], self.wall["lean_lengths"], self.ids,
                                self.wall["bins"], self.exported, settings_dir=self.settings)
        # an oversize part is a loud error, never a silent drop
        big = [ts.regular_polygon(100.0, 100.0, 200.0, 8)]
        with self.assertRaises(ValueError):
            self.pp.pack_plates(big, [(100.0, 100.0, 0.0)], [cicada.Vector(1.0, 0.0, 0.0)], [50.0], [10.0],
                                ["B1"], [0], [True], settings_dir=self.settings)

    def test_terrain_pack_and_footprint_profile_basics(self):
        square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
        bots, tops = self.pp.footprint_profile(square, 1.5, 1.0)
        self.assertEqual(len(bots), 13)
        self.assertTrue(all(abs(b + 1.5) < 1e-9 for b in bots))
        self.assertTrue(all(abs(t - 11.5) < 1e-9 for t in tops))
        items = [{"idx": k, "w": 10.0, "d": 10.0, "tip_y": 12.0, "bots": bots, "tops": tops} for k in range(5)]
        plates, oversize = self.pp.terrain_pack(items, 30.0, 30.0, 1.5, 1.0)
        self.assertEqual(oversize, [])
        # 2 x 2 squares fit a 30 x 30 plate (10 + 3 + 10 + 3 + 10 > 30): the
        # fifth opens a second plate (first-fit-decreasing, plate closes
        # only when nothing left fits)
        self.assertEqual([len(p) for p in plates], [4, 1])
        xs = sorted(x for (_i, x, _y) in plates[0])
        self.assertEqual(xs, [0.0, 0.0, 13.0, 13.0])  # 10 wide + 3 clearance
        self.assertEqual(sorted(set(round(y, 6) for (_i, _x, y) in plates[0])), [0.0, 13.0])
        self.assertEqual(plates[1], [(4, 0.0, 0.0)])
        # too tall for the depth -> oversize
        tall = [{"idx": 9, "w": 10.0, "d": 10.0, "tip_y": 40.0, "bots": bots, "tops": tops}]
        self.assertEqual(self.pp.terrain_pack(tall, 30.0, 30.0, 1.5, 1.0)[1], [9])

    def test_yaw_and_helpers(self):
        self.assertAlmostEqual(self.pp.yaw_angle_to_plus_y(1.0, 0.0), math.pi / 2)
        self.assertAlmostEqual(self.pp.yaw_angle_to_plus_y(0.0, 1.0), 0.0)
        x, y = self.pp.rot2(1.0, 0.0, 0.0, 0.0, self.pp.yaw_angle_to_plus_y(1.0, 0.0))
        self.assertAlmostEqual(x, 0.0)
        self.assertAlmostEqual(y, 1.0)
        self.assertEqual(self.pp.unit_xy(0.0, 0.0), (0.0, 1.0, True))
        self.assertEqual(self.pp.pool_filename(3), "plates_f3_teal_H2.3mf")
        self.assertEqual(self.pp.pool_filename(0), "plates_f0_emerald_X1C.3mf")
        self.assertEqual(self.pp.plate_grid_pos(3, 5, 307.2, 307.2), (0.0, -307.2))
        self.assertAlmostEqual(self.pp.pyramid_area([(0, 0), (2, 0), (2, 2), (0, 2)], 0.0, (1.0, 1.0, 0.0)), 4.0 + 4.0)


# ============================================================
# export_bambu
# ============================================================

def small_wall_meshes(pp, pc, wall, dirs, out):
    """Oriented stand-in meshes: the cell prism moved by the frames."""
    meshes = []
    for i in range(len(wall["cells"])):
        f = ts.rigid_motion(out["part_frames"][i], out["plate_frames"][i])
        verts, tris = pc.prism_mesh([(p[0], p[1]) for p in wall["cells"][i]], 0.0, wall["heights"][i])
        verts = [f(v) for v in verts]
        meshes.append(cicada.Mesh.from_triangles(verts, tris))
    return meshes


class ExportBambuTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.eb = ts.load_script("export_bambu")
        cls.pp = ts.load_script("pack_plates")
        cls.pc = ts.load_script("pin_cutters")
        cls.sf = ts.load_script("solve_field")
        cls.dir = tmpdir()
        cls.settings = ts.make_settings_dir(os.path.join(cls.dir, "bambu"))
        cls.wall = ts.synthetic_wall(30, seed=9)
        cls.dirs = cls.sf.solve_field(cls.wall["centroids"], [(600.0, 600.0, 0.0)],
                                      [(1800.0, 600.0, 0.0)], 1.0, 0.1)["directions"]
        cls.ids = ["Q%d" % i for i in range(30)]
        cls.exported = [i != 3 for i in range(30)]
        cls.packed = cls.pp.pack_plates(cls.wall["cells"], cls.wall["centroids"], cls.dirs,
                                        cls.wall["heights"], cls.wall["lean_lengths"], cls.ids,
                                        cls.wall["bins"], cls.exported, settings_dir=cls.settings)
        cls.meshes = small_wall_meshes(cls.pp, cls.pc, cls.wall, cls.dirs, cls.packed)
        cls.out = os.path.join(cls.dir, "out")
        cls.eb.export_bambu(cls.meshes, cls.ids, cls.wall["bins"], cls.exported, cls.packed["plate"],
                            cls.packed["slot"], cls.packed["manifest"], settings_dir=cls.settings,
                            out_dir=cls.out)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def test_files_manifest_and_fixed_timestamps(self):
        names = sorted(os.listdir(self.out))
        expected = sorted(set(self.packed["file"]) - {""}) + ["manifest.csv"]
        self.assertEqual(names, sorted(expected))
        with open(os.path.join(self.out, "manifest.csv"), "rb") as f:
            raw = f.read()
        self.assertEqual(raw, ("\r\n".join(self.packed["manifest"]) + "\r\n").encode("utf-8"))
        for fname in names:
            if not fname.endswith(".3mf"):
                continue
            zf = zipfile.ZipFile(os.path.join(self.out, fname))
            for info in zf.infolist():
                self.assertEqual(info.date_time, (1980, 1, 1, 0, 0, 0))
                self.assertEqual(info.compress_type, zipfile.ZIP_DEFLATED)
            zf.close()

    def test_structure_matches_production_conventions(self):
        fname = self.packed["file"][0] if self.packed["file"][0] else self.packed["file"][1]
        zf = zipfile.ZipFile(os.path.join(self.out, fname))
        names = zf.namelist()
        for req in ("[Content_Types].xml", "_rels/.rels", "3D/3dmodel.model", "3D/_rels/3dmodel.model.rels",
                    "Metadata/model_settings.config", "Metadata/slice_info.config", "Metadata/cut_information.xml",
                    "Metadata/filament_sequence.json", "Metadata/layer_config_ranges.xml", "Metadata/project_settings.config"):
            self.assertIn(req, names)
        b = int(fname.split("_")[1][1:])
        pool = self.pp.pool_of_bin(b)
        parts = [i for i in range(30) if self.exported[i] and self.wall["bins"][i] == b]
        n = len(parts)
        self.assertEqual(len([x for x in names if x.startswith("3D/Objects/")]), n)
        root = zf.read("3D/3dmodel.model").decode()
        self.assertIn('<object id="%d" p:UUID="%08x-61cb-4c03-9d28-80fed5dfa1dc"' % (2 * n, n), root)
        ms = zf.read("Metadata/model_settings.config").decode()
        self.assertEqual(ms.count("<plate>"), len(set(self.packed["plate"][i] for i in parts)))
        self.assertIn('key="identify_id" value="100"', ms)
        self.assertIn('key="identify_id" value="%d"' % (99 + n), ms)
        if pool == "H2":
            self.assertIn('key="filament_maps" value="2"', ms)
        else:
            self.assertNotIn("filament_maps", ms)
        # object names are the ids, in plate order then slot order
        order = sorted(parts, key=lambda i: (self.packed["plate"][i], self.packed["slot"][i]))
        names_in_file = [m for m in __import__("re").findall(r'<object id="\d+">\s*<metadata key="name" value="([^"]*)"', ms)]
        self.assertEqual(names_in_file, [self.ids[i] for i in order])
        # layer ranges keyed 1..N
        lr = zf.read("Metadata/layer_config_ranges.xml").decode()
        self.assertEqual(lr.count("<object id="), n)
        self.assertIn('<object id="%d">' % n, lr)
        # embedded profile = reference with the proc overlay
        ps = json.loads(zf.read("Metadata/project_settings.config").decode("utf-8-sig"))
        self.assertEqual(ps["sparse_infill_density"], "0%")
        self.assertEqual(ps["printable_area"][2], "330x320" if pool == "H2" else "256x256")
        # item translations = bbox centers of the oriented meshes
        import re
        items = re.findall(r'<item objectid="(\d+)" p:UUID="[^"]*" transform="1 0 0 0 1 0 0 0 1 ([-\d.]+) ([-\d.]+) ([-\d.]+)"', root)
        self.assertEqual(len(items), n)
        for k, i in enumerate(order):
            verts, _t = ts.mesh_arrays(self.meshes[i])
            cx = 0.5 * (min(v[0] for v in verts) + max(v[0] for v in verts))
            cz = 0.5 * (min(v[2] for v in verts) + max(v[2] for v in verts))
            self.assertEqual(int(items[k][0]), 2 * (k + 1))
            self.assertAlmostEqual(float(items[k][1]), cx, places=3)
            self.assertAlmostEqual(float(items[k][3]), cz, places=3)
            self.assertAlmostEqual(cz, 0.5 * self.wall["heights"][i], places=6)
        # object mesh vertices are centered (bbox symmetric about 0)
        obj = zf.read("3D/Objects/object_1.model").decode()
        xs = [float(x) for x in re.findall(r'<vertex x="([-\d.]+)"', obj)]
        self.assertAlmostEqual(min(xs) + max(xs), 0.0, places=3)
        zf.close()

    def test_loud_on_inconsistent_inputs(self):
        with self.assertRaises(ValueError):
            self.eb.export_bambu(self.meshes[:5], self.ids, self.wall["bins"], self.exported,
                                 self.packed["plate"], self.packed["slot"], self.packed["manifest"],
                                 settings_dir=self.settings, out_dir=os.path.join(self.dir, "bad"))
        plates = list(self.packed["plate"])
        plates[0] = 0 if self.exported[0] else plates[0]
        with self.assertRaises(ValueError):
            self.eb.export_bambu(self.meshes, self.ids, self.wall["bins"], self.exported, plates,
                                 self.packed["slot"], self.packed["manifest"], settings_dir=self.settings,
                                 out_dir=os.path.join(self.dir, "bad"))
        with self.assertRaises(ValueError):
            self.eb.export_bambu(self.meshes, self.ids, self.wall["bins"], self.exported, self.packed["plate"],
                                 self.packed["slot"], ["not,a,header"], settings_dir=self.settings,
                                 out_dir=os.path.join(self.dir, "bad"))
        with self.assertRaises(ValueError):
            self.eb.export_bambu(self.meshes, self.ids, self.wall["bins"], self.exported, self.packed["plate"],
                                 self.packed["slot"], self.packed["manifest"],
                                 settings_dir=os.path.join(self.dir, "nope"), out_dir=os.path.join(self.dir, "bad"))

    def test_rewrite_is_byte_identical(self):
        other = os.path.join(self.dir, "out2")
        self.eb.export_bambu(self.meshes, self.ids, self.wall["bins"], self.exported, self.packed["plate"],
                             self.packed["slot"], self.packed["manifest"], settings_dir=self.settings, out_dir=other)
        for fname in os.listdir(self.out):
            with open(os.path.join(self.out, fname), "rb") as f:
                a = f.read()
            with open(os.path.join(other, fname), "rb") as f:
                b = f.read()
            self.assertEqual(a, b, fname)


# ============================================================
# export_dxf
# ============================================================

class ExportDxfTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ed = ts.load_script("export_dxf")
        cls.wl = ts.load_script("wall_labels")
        cls.pc = ts.load_script("pin_cutters")
        cls.sf = ts.load_script("solve_field")
        cls.nz = __import__("normalize")
        cls.dir = tmpdir()
        cls.wall = ts.synthetic_wall(30, seed=13)
        cls.dirs = cls.sf.solve_field(cls.wall["centroids"], [(600.0, 600.0, 0.0)],
                                      [(1800.0, 600.0, 0.0)], 1.0, 0.1)["directions"]
        cls.labels = cls.wl.wall_labels(cls.wall["cells"], cls.wall["centroids"], cls.dirs, (-25.4, -25.4, 0.0))
        cls.pins = cls.pc.pin_cutters(cls.wall["centroids"], cls.dirs, cls.wall["cells"])
        cls.coil = [(600.0 + 25.0 * math.cos(k * math.pi / 3), 600.0 + 25.0 * math.sin(k * math.pi / 3), 0.0)
                    for k in range(6)]
        cls.skip = [i == 4 for i in range(30)]
        cls.path = os.path.join(cls.dir, "out", "board.dxf")
        cls.ed.export_dxf(cls.labels["ghosts"], cls.labels["board_strokes"], cls.labels["board_strokes_closed"],
                          cls.pins["board_points"], cls.coil, (-25.4, -25.4, 0.0), (2413.0, 1193.8, 0.0),
                          skip_holes=cls.skip, path=cls.path)
        with open(cls.path, "rb") as f:
            cls.raw = f.read()
        cls.text = cls.raw.decode("ascii")
        cls.ents = cls.nz.parse_dxf(cls.text)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def test_header_crlf_and_layer_table_quirk(self):
        self.assertTrue(self.raw.startswith(b"0\r\nSECTION\r\n2\r\nHEADER\r\n0\r\nENDSEC\r\n"))
        self.assertNotIn(b"\n\n", self.raw.replace(b"\r\n", b"\n") + b"x")
        self.assertEqual(self.nz.dxf_layer_table(self.text), {"OUTLINES", "PINHOLES", "BOARDCUT", "STOCK"})
        self.assertTrue(self.raw.endswith(b"0\r\nENDSEC\r\n0\r\nEOF\r\n"))

    def test_entity_counts_and_order(self):
        by = {}
        for e in self.ents:
            by.setdefault(e[0], []).append(e)
        self.assertEqual(len(by["OUTLINES"]), 30)
        self.assertEqual(len(by["TEXT"]), len(self.labels["board_strokes"]))
        self.assertEqual(len(by["PINHOLES"]), 2 * 29 + 6)
        self.assertEqual(len(by["BOARDCUT"]), 1)
        self.assertEqual(len(by["STOCK"]), 1)
        layers_in_order = []
        for e in self.ents:
            if not layers_in_order or layers_in_order[-1] != e[0]:
                layers_in_order.append(e[0])
        self.assertEqual(layers_in_order, ["OUTLINES", "TEXT", "PINHOLES", "BOARDCUT", "STOCK"])

    def test_coordinates_are_datum_shifted_and_rounded(self):
        by = {}
        for e in self.ents:
            by.setdefault(e[0], []).append(e)
        # ghosts: closed, first vertex repeated, shifted by +25.4
        g0 = by["OUTLINES"][0]
        self.assertTrue(g0[2][0])
        pts = g0[2][1]
        self.assertEqual(len(pts), len(self.labels["ghosts"][0]) + 1)
        self.assertEqual(pts[0], pts[-1])
        self.assertAlmostEqual(pts[0][0], round(self.labels["ghosts"][0][0][0] + 25.4, 3), places=9)
        # circles: r 1.55, centers = board points + 25.4, skipped part absent
        circles = by["PINHOLES"]
        self.assertTrue(all(abs(c[2][2] - 1.55) < 1e-9 for c in circles))
        self.assertAlmostEqual(circles[0][2][0], round(self.pins["board_points"][0][0] + 25.4, 3), places=9)
        skipped = self.pins["board_points"][8:10]
        for c in circles:
            for s in skipped:
                self.assertGreater(math.hypot(c[2][0] - (s[0] + 25.4), c[2][1] - (s[1] + 25.4)), 1e-6)
        # coil holes are the last 6
        self.assertAlmostEqual(circles[-6][2][0], round(600.0 + 25.0 + 25.4, 3), places=9)
        # BOARDCUT / STOCK rectangles
        bc = by["BOARDCUT"][0][2][1]
        self.assertEqual(bc, [(0.0, 0.0), (2438.4, 0.0), (2438.4, 1219.2), (0.0, 1219.2)])
        st = by["STOCK"][0][2][1]
        self.assertEqual(st, [(-12.7, -12.7), (2451.1, -12.7), (2451.1, 1231.9), (-12.7, 1231.9)])
        # TEXT closed flags follow the strokes
        texts = by["TEXT"]
        self.assertEqual([t[2][0] for t in texts], list(self.labels["board_strokes_closed"]))
        # every numeric token has at most 3 decimals
        import re
        for tok in re.findall(r"\n(?:10|20|40)\r?\n([-\d.]+)", self.text):
            self.assertLessEqual(len(tok.split(".")[1]) if "." in tok else 0, 3)

    def test_loud_on_bad_inputs_and_identical_rewrite(self):
        with self.assertRaises(ValueError):
            self.ed.export_dxf(self.labels["ghosts"], self.labels["board_strokes"], self.labels["board_strokes_closed"][:-1],
                               self.pins["board_points"], self.coil, (-25.4, -25.4, 0.0), (2413.0, 1193.8, 0.0),
                               path=os.path.join(self.dir, "x.dxf"))
        with self.assertRaises(ValueError):  # a hole outside the board (bad datum)
            self.ed.export_dxf(self.labels["ghosts"], self.labels["board_strokes"], self.labels["board_strokes_closed"],
                               self.pins["board_points"], self.coil, (500.0, 500.0, 0.0), (2413.0, 1193.8, 0.0),
                               path=os.path.join(self.dir, "x.dxf"))
        with self.assertRaises(ValueError):  # odd hole count
            self.ed.export_dxf(self.labels["ghosts"], self.labels["board_strokes"], self.labels["board_strokes_closed"],
                               self.pins["board_points"][:-1], self.coil, (-25.4, -25.4, 0.0), (2413.0, 1193.8, 0.0),
                               path=os.path.join(self.dir, "x.dxf"))
        p2 = os.path.join(self.dir, "again", "board.dxf")
        self.ed.export_dxf(self.labels["ghosts"], self.labels["board_strokes"], self.labels["board_strokes_closed"],
                           self.pins["board_points"], self.coil, (-25.4, -25.4, 0.0), (2413.0, 1193.8, 0.0),
                           skip_holes=self.skip, path=p2)
        with open(p2, "rb") as f:
            self.assertEqual(f.read(), self.raw)

    def test_join_open_plines(self):
        ents = [("TEXT", "pline", (False, [(0, 0), (1, 0)])), ("TEXT", "pline", (False, [(1, 0), (1, 1)])),
                ("TEXT", "pline", (False, [(5, 5), (6, 5)])), ("OUTLINES", "pline", (True, [(0, 0), (1, 0), (1, 1)]))]
        out, nb, na = self.ed.join_open_plines(ents, "TEXT")
        self.assertEqual((nb, na), (3, 2))
        chains = [e for e in out if e[0] == "TEXT"]
        self.assertEqual(chains[0][2][1], [(0, 0), (1, 0), (1, 1)])
        # a closing chain becomes closed
        ring = [("TEXT", "pline", (False, [(0, 0), (1, 0)])), ("TEXT", "pline", (False, [(1, 0), (1, 1)])),
                ("TEXT", "pline", (False, [(1, 1), (0, 1)])), ("TEXT", "pline", (False, [(0, 1), (0, 0)]))]
        out, nb, na = self.ed.join_open_plines(ring, "TEXT")
        self.assertEqual(na, 1)
        self.assertTrue(out[0][2][0])
        self.assertEqual(len(out[0][2][1]), 4)


if __name__ == "__main__":
    unittest.main()
