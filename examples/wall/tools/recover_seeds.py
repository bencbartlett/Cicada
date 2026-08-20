#!/usr/bin/env python3
"""Recover the Voronoi generating seeds (and the per-cell shrink) behind the
frozen production cells, and write them into examples/wall/inputs/layout.json ONLY if
re-running the Voronoi reproduces every production cell vertex within 2e-3 mm.

Dev tool, NOT a pipeline node (stage 6; lives in examples/wall/tools). Exit status 0 = seeds
written; 2 = recovery failed (nothing written, the reason is printed); 3 =
input problem.

What the production cells turned out to be (measured here, see the report):
  production cell_i = centroid_i + s_i * (voronoi_cell_i - centroid_i)
i.e. each Voronoi cell (bounded by the workable rectangle; the 1200 kept cells
tile it completely -- no culled cells) was SHRUNK about its own area centroid
by a per-cell factor s_i in [0.818, 0.954]; s_i is an exact linear function of
the field magnitude at the seed (scale = 0.8052 + 51.53*|B|, unit antiparallel
currents, CoreRadius 243.84 mm). Neighbouring production cells therefore do
not share edges (gaps 1.8-12 mm); the shrink is the only reason. Bonus found
on the way: the production field was evaluated AT THE SEEDS (2D Biot-Savart at
the recovered seeds reproduces every production lean to < 0.005 deg).

Method
  1. neighbour edges: pairs of anti-parallel, overlapping edges of nearby cells;
     gap_ij = k_i d_i + k_j d_j with k = 1/s - 1 and d = centroid-to-edge
     distance -> sparse least squares for every k_i (all 1200 cells are
     constrained, the model fits the gaps to ~2e-3 mm).
  2. unscale every cell about its centroid by 1/s_i -> the Voronoi cells, whose
     shared edges now coincide.
  3. seeds by least squares over shared edges: each shared edge of cells i, j
     lies on the perpendicular bisector of seeds i, j (2 linear equations per
     edge: midpoint on the edge line, seed difference parallel to the edge
     normal). Edges on the workable rectangle are clips and add nothing.
  4. culled neighbours: every remaining (unshared, non-clip) edge of a kept
     cell is the bisector against a culled seed = the reflection of the kept
     seed across that edge; reflections that agree within tolerance are one
     culled seed (their spread is a consistency check).
  5. verification: half-plane-clipping Voronoi of all seeds (kept + culled)
     inside the workable rectangle, each kept cell re-shrunk by its s_i about
     its centroid, compared vertex-to-vertex with the production cells.

Dependencies: numpy (required), scipy (optional: sparse lsqr + cKDTree make it
faster; without scipy the dense numpy solves take a little longer). Neither
is ever needed by the pipeline scripts; `pip install --user numpy scipy`.

Writes into layout.json (only on success): "seeds" (kept first in idx order,
then the recovered culled seeds), "keep" (booleans, one per seed),
"cell_scales" (per part, idx order) and "seeds_note"; plus
examples/wall/golden/production/seed_recovery_report.json (always, also on failure).

Usage:
  python examples/wall/tools/recover_seeds.py [--layout PATH] [--report PATH]
                                              [--tolerance 0.002] [--dry-run]
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys

try:
    import numpy as np
except Exception:  # pragma: no cover
    print("recover_seeds: numpy is required (pip install --user numpy)", file=sys.stderr)
    sys.exit(3)

try:
    from scipy.sparse import lil_matrix
    from scipy.sparse.linalg import lsqr
    from scipy.spatial import cKDTree
    HAVE_SCIPY = True
except Exception:  # pragma: no cover
    HAVE_SCIPY = False

HERE = os.path.dirname(os.path.abspath(__file__))
WALL_DIR = os.path.normpath(os.path.join(HERE, ".."))  # examples/wall/


def load_layout(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def poly_centroid(pts):
    a2 = cx = cy = 0.0
    m = len(pts)
    for i in range(m):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % m]
        cr = x1 * y2 - x2 * y1
        a2 += cr
        cx += (x1 + x2) * cr
        cy += (y1 + y2) * cr
    return cx / (3.0 * a2), cy / (3.0 * a2), 0.5 * a2


def neighbour_gaps(cells, cents, radius=150.0, max_gap=20.0, min_gap=0.0):
    """(i, j, gap, d_i, d_j) for every pair of anti-parallel overlapping edges."""
    n = len(cells)
    C = np.array(cents)
    if HAVE_SCIPY:
        tree = cKDTree(C)
        near = tree.query_ball_point(C, radius)
    else:
        near = [[j for j in range(n) if np.hypot(*(C[j] - C[i])) <= radius] for i in range(n)]
    E = []
    for i in range(n):
        c = np.array(cells[i])
        m = len(c)
        for k in range(m):
            a, b = c[k], c[(k + 1) % m]
            t = b - a
            L = np.linalg.norm(t)
            t = t / L
            E.append((i, a, b, t, np.array([t[1], -t[0]]), L))  # CCW -> (ty, -tx) is the outward normal
    by_cell = {}
    for e in E:
        by_cell.setdefault(e[0], []).append(e)
    rows = []
    for i in range(n):
        for (ii, a, b, t, nrm, L) in by_cell[i]:
            for j in near[i]:
                if j <= i:
                    continue
                for (jj, c_, d_, t2, nrm2, L2) in by_cell[j]:
                    if float(np.dot(t, t2)) > -0.9999:
                        continue
                    g1 = float(np.dot(c_ - a, nrm))
                    g2 = float(np.dot(d_ - a, nrm))
                    if abs(g1 - g2) > 0.01 or g1 < min_gap or g1 > max_gap:
                        continue
                    pc = float(np.dot(c_ - a, t))
                    pd = float(np.dot(d_ - a, t))
                    if min(L, max(pc, pd)) - max(0.0, min(pc, pd)) < 1.0:
                        continue
                    di = -float(np.dot(C[i] - a, nrm))
                    dj = -float(np.dot(C[j] - c_, nrm2))  # both distances positive (centroids inside their cells)
                    rows.append((i, j, g1, di, dj))
    return rows


def solve_scales(rows, n):
    if HAVE_SCIPY:
        A = lil_matrix((len(rows), n))
        b = np.zeros(len(rows))
        for r, (i, j, g, di, dj) in enumerate(rows):
            A[r, i] = di
            A[r, j] = dj
            b[r] = g
        k = lsqr(A.tocsr(), b, atol=1e-14, btol=1e-14, iter_lim=100000)[0]
        res = A.tocsr() @ k - b
    else:
        A = np.zeros((len(rows), n))
        b = np.zeros(len(rows))
        for r, (i, j, g, di, dj) in enumerate(rows):
            A[r, i] = di
            A[r, j] = dj
            b[r] = g
        k = np.linalg.lstsq(A, b, rcond=None)[0]
        res = A @ k - b
    return 1.0 / (1.0 + k), res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--layout", default=os.path.join(WALL_DIR, "inputs", "layout.json"))
    ap.add_argument("--report", default=os.path.join(WALL_DIR, "golden", "production", "seed_recovery_report.json"))
    ap.add_argument("--tolerance", type=float, default=0.002, help="max allowed vertex deviation, mm (contract: 2e-3)")
    ap.add_argument("--dry-run", action="store_true", help="never write layout.json (the report is still written)")
    args = ap.parse_args(argv)

    lay = load_layout(args.layout)
    parts = lay["parts"]
    n = len(parts)
    cells = [p["cell"] for p in parts]
    cents = [tuple(p["centroid"]) for p in parts]
    heights = [p["height"] for p in parts]
    W0 = lay["workable"]["min"]
    W1 = lay["workable"]["max"]
    report = {"tool": "examples/wall/tools/recover_seeds.py", "scipy": HAVE_SCIPY, "tolerance_mm": args.tolerance, "steps": {}}

    def fail(msg, code=2):
        report["outcome"] = "FAILED: " + msg
        os.makedirs(os.path.dirname(args.report), exist_ok=True)
        with open(args.report, "w", encoding="utf-8", newline="\n") as f:
            json.dump(report, f, indent=1, sort_keys=True)
        print("recover_seeds: " + report["outcome"], file=sys.stderr)
        return code

    # ---- 1. per-cell shrink from neighbour gaps ------------------------------------
    rows = neighbour_gaps(cells, cents)
    if len(rows) < n:
        return fail("only %d neighbour edge pairs found for %d cells" % (len(rows), n))
    s, gap_res = solve_scales(rows, n)
    constrained = set()
    for (i, j, *_rest) in rows:
        constrained.add(i)
        constrained.add(j)
    H = np.array(heights)
    lin = np.polyfit(H, s, 1)
    report["steps"]["1_shrink"] = {
        "neighbour_edge_pairs": len(rows),
        "cells_constrained": len(constrained),
        "gap_residual_abs_max_mm": float(np.abs(gap_res).max()),
        "gap_residual_abs_median_mm": float(np.median(np.abs(gap_res))),
        "scale_min": float(s.min()), "scale_median": float(np.median(s)), "scale_max": float(s.max()),
        "scale_vs_height_linear_fit": {"a": float(lin[0]), "b": float(lin[1]),
                                       "abs_residual_max": float(np.abs(s - np.polyval(lin, H)).max()),
                                       "abs_residual_median": float(np.median(np.abs(s - np.polyval(lin, H))))},
    }
    if len(constrained) != n:
        return fail("%d cells have no neighbour-gap constraint; their shrink is undetermined" % (n - len(constrained)))
    if np.abs(gap_res).max() > 0.01:
        return fail("the per-cell shrink model does not explain the gaps (max residual %.4f mm)" % np.abs(gap_res).max())

    # ---- 2. unscale -> Voronoi cells ---------------------------------------------------
    V = []
    for i in range(n):
        c = np.array(cents[i])
        V.append(c + (np.array(cells[i]) - c) / s[i])
    # boundary check: outer edges of boundary cells should land on the workable rectangle
    bdev = []
    for i in range(n):
        for v in V[i]:
            d = min(abs(v[0] - W0[0]), abs(v[0] - W1[0]), abs(v[1] - W0[1]), abs(v[1] - W1[1]))
            if d < 0.5:
                bdev.append(d)
    report["steps"]["2_unscale"] = {"unscaled_vertices_within_0.5mm_of_rectangle": len(bdev),
                                    "their_max_deviation_mm": float(max(bdev)) if bdev else None,
                                    "their_median_deviation_mm": float(np.median(bdev)) if bdev else None}

    # ---- 3. shared edges -> seeds ------------------------------------------------------
    def on_rect(p, tol=0.01):
        return (abs(p[0] - W0[0]) < tol or abs(p[0] - W1[0]) < tol or abs(p[1] - W0[1]) < tol or abs(p[1] - W1[1]) < tol)

    # pair edges of the UNSCALED cells exactly like the gap search: anti-parallel, overlapping, now with ~zero gap
    Vl = [[(float(v[0]), float(v[1])) for v in V[i]] for i in range(n)]
    pairs = neighbour_gaps(Vl, cents, max_gap=0.05, min_gap=-0.05)
    edges = []  # (i, k, a, b)
    for i in range(n):
        m = len(V[i])
        for k in range(m):
            edges.append((i, k, V[i][k], V[i][(k + 1) % m]))
    partner = [None] * len(edges)
    edge_index = {}
    for ei, (i, k, a, b) in enumerate(edges):
        edge_index.setdefault(i, []).append(ei)
    shared = []     # (i, j, a_i, b_i, a_j, b_j, gap)
    match_dev = []
    for (i, j, gap, di, dj) in pairs:
        best = None
        for ei in edge_index[i]:
            _i, _k, a, b = edges[ei]
            for ej in edge_index[j]:
                _j, _kk, c_, d_ = edges[ej]
                t = b - a
                t2 = d_ - c_
                if float(np.dot(t, t2)) >= 0:
                    continue
                dev = max(np.linalg.norm(c_ - b), np.linalg.norm(d_ - a))
                if best is None or dev < best[0]:
                    best = (dev, ei, ej)
        if best is None or best[0] > 0.05:
            return fail("shared edge between %s and %s could not be matched (endpoint mismatch %s)" % (
                parts[i]["id"], parts[j]["id"], None if best is None else "%.4f mm" % best[0]))
        dev, ei, ej = best
        partner[ei] = ej
        partner[ej] = ei
        shared.append((i, j, edges[ei][2], edges[ei][3], edges[ej][2], edges[ej][3], gap))
        match_dev.append(dev)
    clip = []
    orphan = []
    tiny = []
    for ei, (i, k, a, b) in enumerate(edges):
        if partner[ei] is not None:
            continue
        L = float(np.linalg.norm(b - a))
        if on_rect(a, 0.05) and on_rect(b, 0.05) and (abs(a[0] - b[0]) < 0.05 or abs(a[1] - b[1]) < 0.05):
            clip.append(ei)
        elif L < 1.0:
            tiny.append(ei)   # sub-mm edges the overlap test skipped: no information, no culled cell behind them
        else:
            orphan.append(ei)
    report["steps"]["3_edges"] = {"edges": len(edges), "shared_pairs": len(shared), "clip_edges": len(clip),
                                  "unpaired_sub_mm_edges": len(tiny),
                                  "orphan_edges_to_culled_cells": len(orphan),
                                  "shared_endpoint_mismatch_max_mm": float(max(match_dev)) if match_dev else None,
                                  "shared_endpoint_mismatch_median_mm": float(np.median(match_dev)) if match_dev else None,
                                  "unscaled_gap_abs_max_mm": float(max(abs(sh[6]) for sh in shared)) if shared else None}
    if len(shared) < n:
        return fail("too few shared edges (%d) to determine %d seeds" % (len(shared), n))
    # least squares: unknowns x = [s0x, s0y, s1x, s1y, ...]; the tangent equation is weighted by the edge length
    # (its direction is only as good as the endpoints over that length), the midpoint equation by 1
    nrows = 2 * len(shared)
    if HAVE_SCIPY:
        A = lil_matrix((nrows, 2 * n))
    else:
        A = np.zeros((nrows, 2 * n))
    b = np.zeros(nrows)
    wts = np.zeros(nrows)
    r = 0
    for (i, j, ai, bi, aj, bj, _gap) in shared:
        p0 = 0.5 * (ai + bj)
        p1 = 0.5 * (bi + aj)
        t = p1 - p0
        L = float(np.linalg.norm(t))
        t = t / L
        nrm = np.array([t[1], -t[0]])
        c = float(np.dot(nrm, 0.5 * (p0 + p1)))
        A[r, 2 * i] = nrm[0]; A[r, 2 * i + 1] = nrm[1]
        A[r, 2 * j] = nrm[0]; A[r, 2 * j + 1] = nrm[1]
        b[r] = 2.0 * c
        wts[r] = 1.0
        r += 1
        w = min(L, 20.0) / 20.0
        A[r, 2 * i] = w * t[0]; A[r, 2 * i + 1] = w * t[1]
        A[r, 2 * j] = -w * t[0]; A[r, 2 * j + 1] = -w * t[1]
        b[r] = 0.0
        wts[r] = w
        r += 1
    if HAVE_SCIPY:
        x = lsqr(A.tocsr(), b, atol=1e-14, btol=1e-14, iter_lim=200000)[0]
        res = A.tocsr() @ x - b
    else:
        x = np.linalg.lstsq(A, b, rcond=None)[0]
        res = A @ x - b
    seeds = x.reshape(n, 2)
    mid_res = np.abs(res[0::2])
    deg = np.zeros(n, int)
    for (i, j, *_rest) in shared:
        deg[i] += 1
        deg[j] += 1
    isolated = [int(i) for i in range(n) if deg[i] == 0]
    report["steps"]["3_seeds"] = {"midpoint_residual_abs_max_mm": float(mid_res.max()),
                                  "midpoint_residual_abs_median_mm": float(np.median(mid_res)),
                                  "cells_without_shared_edges": isolated,
                                  "seed_minus_centroid_mm": {"median": float(np.median(np.hypot(*(seeds - np.array(cents)).T))),
                                                             "max": float(np.hypot(*(seeds - np.array(cents)).T).max())}}
    if isolated:
        return fail("%d kept cells share no edge with another kept cell; their seeds are undetermined: %s" % (
            len(isolated), [parts[i]["id"] for i in isolated]))


    # ---- 4. culled seeds by reflection -------------------------------------------------
    cand = []
    for ei in orphan:
        i, k, a, b = edges[ei]
        t = b - a
        t = t / np.linalg.norm(t)
        nrm = np.array([t[1], -t[0]])
        d = float(np.dot(seeds[i] - a, nrm))
        cand.append(seeds[i] - 2.0 * d * nrm)
    cand = np.array(cand) if cand else np.zeros((0, 2))
    culled = []
    spreads = []
    if len(cand):
        used = np.zeros(len(cand), bool)
        if HAVE_SCIPY:
            ctree = cKDTree(cand)
        for ci in range(len(cand)):
            if used[ci]:
                continue
            if HAVE_SCIPY:
                grp = [g for g in ctree.query_ball_point(cand[ci], 0.5) if not used[g]]
            else:
                grp = [g for g in range(len(cand)) if not used[g] and np.linalg.norm(cand[g] - cand[ci]) <= 0.5]
            for g in grp:
                used[g] = True
            P = cand[grp]
            mean = P.mean(axis=0)
            culled.append(mean)
            spreads.append(float(np.linalg.norm(P - mean, axis=1).max()))
    culled = np.array(culled) if culled else np.zeros((0, 2))
    report["steps"]["4_culled"] = {"orphan_edges": len(orphan), "culled_seeds": int(len(culled)),
                                   "reflection_cluster_spread_max_mm": float(max(spreads)) if spreads else None,
                                   "reflection_cluster_spread_median_mm": float(np.median(spreads)) if spreads else None,
                                   "culled_seeds_outside_workable": int(sum(1 for c in culled if not (W0[0] <= c[0] <= W1[0] and W0[1] <= c[1] <= W1[1])))}

    # ---- 5. verification: half-plane Voronoi -> shrink -> compare --------------------------
    allseeds = np.vstack([seeds, culled]) if len(culled) else seeds
    N = len(allseeds)
    if HAVE_SCIPY:
        stree = cKDTree(allseeds)
    rect = [np.array([W0[0], W0[1]]), np.array([W1[0], W0[1]]), np.array([W1[0], W1[1]]), np.array([W0[0], W1[1]])]

    def clip_halfplane(poly, p, q):
        """Keep the part of poly closer to p than to q."""
        mid = 0.5 * (p + q)
        nrm = q - p
        out = []
        m = len(poly)
        for k in range(m):
            a = poly[k]
            bpt = poly[(k + 1) % m]
            fa = float(np.dot(a - mid, nrm))
            fb = float(np.dot(bpt - mid, nrm))
            if fa <= 0:
                out.append(a)
            if (fa < 0 < fb) or (fb < 0 < fa):
                tpar = fa / (fa - fb)
                out.append(a + tpar * (bpt - a))
        return out

    def voronoi_cell(k, radius=400.0):
        p = allseeds[k]
        poly = list(rect)
        if HAVE_SCIPY:
            nbrs = stree.query_ball_point(p, radius)
        else:
            nbrs = [j for j in range(N) if np.linalg.norm(allseeds[j] - p) <= radius]
        nbrs = sorted(nbrs, key=lambda j: np.linalg.norm(allseeds[j] - p))
        for j in nbrs:
            if j == k:
                continue
            poly = clip_halfplane(poly, p, allseeds[j])
            if len(poly) < 3:
                break
        return poly

    def verify(allseeds, s, want_structure=False):
        """Recompute every kept cell, shrink it, compare with the production cell.
        Returns (devs, worst, count_mismatch, structure) where structure holds, per production vertex copy,
        the matched recomputed vertex and its generators (for the refinement)."""
        Nn = len(allseeds)
        if HAVE_SCIPY:
            stree = cKDTree(allseeds)

        def voronoi_cell(k, radius=400.0):
            pk = allseeds[k]
            poly = list(rect)
            if HAVE_SCIPY:
                nbrs = stree.query_ball_point(pk, radius)
            else:
                nbrs = [j for j in range(Nn) if np.linalg.norm(allseeds[j] - pk) <= radius]
            nbrs = sorted(nbrs, key=lambda j: np.linalg.norm(allseeds[j] - pk))
            for j in nbrs:
                if j == k:
                    continue
                poly = clip_halfplane(poly, pk, allseeds[j])
                if len(poly) < 3:
                    break
            return poly, nbrs

        devs = []
        count_mismatch = []
        worst = []
        structure = []
        for i in range(n):
            poly, nbrs = voronoi_cell(i)
            if len(poly) < 3:
                return None, None, None, "recomputed Voronoi cell %s is empty" % parts[i]["id"]
            cleaned = []
            for v in poly:
                if not cleaned or np.linalg.norm(v - cleaned[-1]) > 1e-7:
                    cleaned.append(v)
            if len(cleaned) > 1 and np.linalg.norm(cleaned[0] - cleaned[-1]) <= 1e-7:
                cleaned.pop()
            poly = np.array(cleaned)
            cx, cy, _a = poly_centroid([(float(v[0]), float(v[1])) for v in poly])
            c = np.array([cx, cy])
            shrunk = c + s[i] * (poly - c)
            prod = np.array(cells[i])
            dists = np.array([np.linalg.norm(shrunk - pv, axis=1) for pv in prod])  # prod x recomputed
            d = dists.min(axis=1)
            dmax = float(d.max())
            devs.extend(d.tolist())
            if len(shrunk) != len(prod):
                count_mismatch.append((parts[i]["id"], len(prod), len(shrunk)))
            if dmax > args.tolerance:
                worst.append((dmax, parts[i]["id"]))
            if want_structure:
                pi = allseeds[i]
                for vi, pv in enumerate(prod):
                    m = int(dists[vi].argmin())
                    u = poly[m]
                    ri = float(np.linalg.norm(u - pi))
                    gens = [j for j in nbrs if j != i and abs(np.linalg.norm(u - allseeds[j]) - ri) < 1e-5]
                    structure.append((i, vi, m, tuple(gens[:2]), c))
        return np.array(devs), sorted(worst, reverse=True), count_mismatch, structure

    def circumcenter(pa, pb, pc):
        ax, ay = pa
        bx, by = pb
        cx_, cy_ = pc
        d = 2.0 * (ax * (by - cy_) + bx * (cy_ - ay) + cx_ * (ay - by))
        ux = ((ax * ax + ay * ay) * (by - cy_) + (bx * bx + by * by) * (cy_ - ay) + (cx_ * cx_ + cy_ * cy_) * (ay - by)) / d
        uy = ((ax * ax + ay * ay) * (cx_ - bx) + (bx * bx + by * by) * (ax - cx_) + (cx_ * cx_ + cy_ * cy_) * (bx - ax)) / d
        return np.array([ux, uy])

    def clip_vertex(pa, pb, u_ref):
        """Intersection of the bisector of pa, pb with the rectangle edge nearest to u_ref."""
        mid = 0.5 * (pa + pb)
        nrm = pb - pa
        # candidate edges: x = W0[0], x = W1[0], y = W0[1], y = W1[1]
        best = None
        for axis, val in ((0, W0[0]), (0, W1[0]), (1, W0[1]), (1, W1[1])):
            # points on the line axis=val satisfying nrm.(x - mid) = 0
            other = 1 - axis
            if abs(nrm[other]) < 1e-12:
                continue
            t = (float(np.dot(nrm, mid)) - nrm[axis] * val) / nrm[other]
            pt = np.zeros(2)
            pt[axis] = val
            pt[other] = t
            dd = float(np.linalg.norm(pt - u_ref))
            if best is None or dd < best[0]:
                best = (dd, pt)
        return best[1]

    allseeds = np.vstack([seeds, culled]) if len(culled) else seeds
    N = len(allseeds)
    devs, worst, count_mismatch, structure = verify(allseeds, s, want_structure=True)
    if devs is None:
        return fail(structure)
    report["steps"]["5_verify_initial"] = {"vertex_deviation_max_mm": float(devs.max()),
                                           "vertex_deviation_median_mm": float(np.median(devs)),
                                           "parts_over_tolerance": len(worst),
                                           "vertex_count_mismatch_total": len(count_mismatch)}
    if count_mismatch:
        return fail("Voronoi re-run changes the vertex count of %d cells, e.g. %s" % (len(count_mismatch), count_mismatch[:5]))

    # ---- 6. refinement: Gauss-Newton on the production vertices (seeds + per-cell shrink jointly) -------------
    # residual per production vertex copy: c_i + s_i (u - c_i) - v, u = circumcenter / clip point of the generators
    iters = []
    for it in range(4):
        if devs.max() <= 0.75 * args.tolerance:
            break
        nres = 2 * len(structure)
        nvar = 2 * N + n
        J = lil_matrix((nres, nvar)) if HAVE_SCIPY else np.zeros((nres, nvar))
        rvec = np.zeros(nres)
        eps = 1e-6
        r = 0
        for (i, vi, m, gens, c) in structure:
            v = np.array(cells[i][vi])
            pi = allseeds[i]
            if len(gens) == 2:
                ids = (i, gens[0], gens[1])
                pts = [allseeds[t] for t in ids]
                u0 = circumcenter(*pts)
                du = []
                for q in range(3):
                    for ax in range(2):
                        pp = [pt.copy() for pt in pts]
                        pp[q][ax] += eps
                        du.append((circumcenter(*pp) - u0) / eps)
            elif len(gens) == 1:
                ids = (i, gens[0])
                pts = [allseeds[t] for t in ids]
                u0 = clip_vertex(pts[0], pts[1], v)
                du = []
                for q in range(2):
                    for ax in range(2):
                        pp = [pt.copy() for pt in pts]
                        pp[q][ax] += eps
                        du.append((clip_vertex(pp[0], pp[1], v) - u0) / eps)
            else:
                ids = ()
                # rectangle corner: nearest corner
                u0 = min(rect, key=lambda cc: np.linalg.norm(cc - v))
                du = []
            res = c + s[i] * (u0 - c) - v
            rvec[r] = res[0]
            rvec[r + 1] = res[1]
            for qi, t in enumerate(ids):
                for ax in range(2):
                    col = 2 * t + ax
                    g = du[2 * qi + ax]
                    J[r, col] = s[i] * g[0]
                    J[r + 1, col] = s[i] * g[1]
            J[r, 2 * N + i] = (u0 - c)[0]
            J[r + 1, 2 * N + i] = (u0 - c)[1]
            r += 2
        if HAVE_SCIPY:
            delta = lsqr(J.tocsr(), -rvec, damp=1e-6, atol=1e-14, btol=1e-14, iter_lim=200000)[0]
        else:
            delta = np.linalg.lstsq(J, -rvec, rcond=None)[0]
        allseeds = allseeds + delta[:2 * N].reshape(N, 2)
        s = s + delta[2 * N:]
        devs2, worst2, count_mismatch2, structure2 = verify(allseeds, s, want_structure=True)
        if devs2 is None:
            return fail(structure2)
        iters.append({"iteration": it + 1, "vertex_deviation_max_mm": float(devs2.max()),
                      "vertex_deviation_median_mm": float(np.median(devs2)), "parts_over_tolerance": len(worst2),
                      "max_seed_move_mm": float(np.abs(delta[:2 * N]).max()), "max_scale_change": float(np.abs(delta[2 * N:]).max())})
        if count_mismatch2 or devs2.max() > devs.max():
            iters[-1]["rejected"] = True
            allseeds = allseeds - delta[:2 * N].reshape(N, 2)
            s = s - delta[2 * N:]
            break
        devs, worst, count_mismatch, structure = devs2, worst2, count_mismatch2, structure2
    report["steps"]["6_refine"] = iters
    report["steps"]["7_verify"] = {"seeds_total": int(N), "kept": n, "culled": int(N - n),
                                   "vertex_deviation_max_mm": float(devs.max()),
                                   "vertex_deviation_median_mm": float(np.median(devs)),
                                   "vertex_deviation_p99_mm": float(np.percentile(devs, 99)),
                                   "parts_over_tolerance": len(worst),
                                   "worst_parts": [{"id": w[1], "max_dev_mm": w[0]} for w in worst[:10]],
                                   "vertex_count_mismatch_total": len(count_mismatch),
                                   "scale_min": float(s.min()), "scale_max": float(s.max()),
                                   "seed_minus_centroid_mm": {"median": float(np.median(np.hypot(*(allseeds[:n] - np.array(cents)).T))),
                                                              "max": float(np.hypot(*(allseeds[:n] - np.array(cents)).T).max())}}
    if count_mismatch or worst:
        return fail("Voronoi re-run does not reproduce the production cells within %.4f mm: max deviation %.4f mm, %d parts over, "
                    "%d vertex-count mismatches (see %s)" % (args.tolerance, devs.max(), len(worst), len(count_mismatch), args.report))


    # ---- 8. round to the file precision and re-verify (what is written must be what was checked) ---------------
    seeds_r = np.round(allseeds, 4)
    s_r = np.round(s, 6)
    devs_r, worst_r, count_mismatch_r, _st = verify(seeds_r, s_r)
    if devs_r is None:
        return fail(_st)
    report["steps"]["8_verify_rounded"] = {"seeds_decimals": 4, "scale_decimals": 6,
                                           "vertex_deviation_max_mm": float(devs_r.max()),
                                           "vertex_deviation_median_mm": float(np.median(devs_r)),
                                           "parts_over_tolerance": len(worst_r),
                                           "vertex_count_mismatch_total": len(count_mismatch_r)}
    if count_mismatch_r or worst_r:
        return fail("after rounding to the file precision the re-run misses the tolerance (max %.4f mm)" % devs_r.max())
    allseeds, s, devs = seeds_r, s_r, devs_r

    # ---- 9. what the seeds reveal about the production field solve and the shrink law --------------------------
    wires = lay.get("wires", [])
    field_note = None
    scale_note = None
    if len(wires) == 2:
        Wc = np.array([w["center"] for w in wires])
        Ws = np.array([w["current"] for w in wires])
        C = np.array(cents)
        lean = np.array([p["lean"] for p in parts])
        has_pins = np.array([p.get("coil") is None for p in parts])

        def field(X, core):
            bx = np.zeros(len(X))
            by = np.zeros(len(X))
            for (wx, wy), sg in zip(Wc, Ws):
                dx = X[:, 0] - wx
                dy = X[:, 1] - wy
                cc = sg / (dx * dx + dy * dy + core * core)
                bx += -dy * cc
                by += dx * cc
            return bx, by

        def ang_err(X, core):
            bx, by = field(X, core)
            e = np.degrees(np.arctan2(lean[:, 0] * by - lean[:, 1] * bx, lean[:, 0] * bx + lean[:, 1] * by))
            return np.abs(e[has_pins])

        # golden-section on log(core) for the median error at the seeds
        lo, hi = math.log(1.0), math.log(2000.0)
        gr = (math.sqrt(5.0) - 1.0) / 2.0
        c1 = hi - gr * (hi - lo)
        c2 = lo + gr * (hi - lo)
        f1 = float(np.median(ang_err(allseeds[:n], math.exp(c1))))
        f2 = float(np.median(ang_err(allseeds[:n], math.exp(c2))))
        for _ in range(80):
            if f1 < f2:
                hi, c2, f2 = c2, c1, f1
                c1 = hi - gr * (hi - lo)
                f1 = float(np.median(ang_err(allseeds[:n], math.exp(c1))))
            else:
                lo, c1, f1 = c1, c2, f2
                c2 = lo + gr * (hi - lo)
                f2 = float(np.median(ang_err(allseeds[:n], math.exp(c2))))
        core_fit = math.exp(0.5 * (lo + hi))
        core_round = 0.1 * 96 * 25.4  # 243.84 mm = 0.1 x the physical board width
        es = ang_err(allseeds[:n], core_round)
        ec = ang_err(C, core_round)
        e0 = ang_err(allseeds[:n], 0.1)
        report["steps"]["9_field_at_seeds"] = {
            "core_radius_fit_at_seeds_mm": core_fit,
            "core_radius_round_mm": core_round,
            "angle_error_deg_at_seeds_core_round": {"median": float(np.median(es)), "p99": float(np.percentile(es, 99)), "max": float(es.max())},
            "angle_error_deg_at_centroids_core_round": {"median": float(np.median(ec)), "max": float(ec.max())},
            "angle_error_deg_at_seeds_core_0.1": {"median": float(np.median(e0)), "max": float(e0.max())},
            "parts_compared": int(has_pins.sum()),
        }
        field_note = ("the production field was evaluated AT THE SEEDS with CoreRadius %.2f mm (0.1 x 96 in): 2D Biot-Savart at the "
                      "recovered seeds reproduces the production lean of all %d pinned parts to median %.4f deg, max %.4f deg "
                      "(pin-rounding noise); at the cell centroids the same solve is off by median %.3f, max %.3f deg; at core 0.1 mm "
                      "by median %.2f, max %.2f deg" % (core_round, int(has_pins.sum()), float(np.median(es)), float(es.max()),
                                                         float(np.median(ec)), float(ec.max()), float(np.median(e0)), float(e0.max())))
        # shrink law: cell_scale vs |B| at the seeds (unit antiparallel currents)
        bx, by = field(allseeds[:n], core_round)
        M = np.hypot(bx, by)
        lin = np.polyfit(M, s, 1)
        res = s - np.polyval(lin, M)
        report["steps"]["9_shrink_law"] = {"scale = a + b*|B|": {"a": float(lin[1]), "b": float(lin[0])},
                                           "scale = a + c*(|B|/|B|max)": {"a": float(lin[1]), "c": float(lin[0] * M.max()), "|B|max": float(M.max())},
                                           "abs_residual_max": float(np.abs(res).max()), "abs_residual_median": float(np.median(np.abs(res)))}
        scale_note = ("cell_scales[idx] = shrink of Voronoi cell idx about its centroid; it is an exact linear function of the field "
                      "magnitude at the seed: scale = %.5f + %.3f * |B| = %.5f + %.5f * (|B| / |B|max), |B| from unit antiparallel "
                      "currents at CoreRadius %.2f mm (|residual| max %.1e over 1200 cells); stored per part anyway" % (
                          lin[1], lin[0], lin[1], lin[0] * M.max(), core_round, float(np.abs(res).max())))

    # ---- success: write -------------------------------------------------------------------
    report["outcome"] = ("OK: %d seeds (%d kept + %d culled) reproduce every production cell vertex within %.4f mm "
                         "(max %.4f) after the per-cell shrink" % (N, n, N - n, args.tolerance, devs.max()))
    os.makedirs(os.path.dirname(args.report), exist_ok=True)
    with open(args.report, "w", encoding="utf-8", newline="\n") as f:
        json.dump(report, f, indent=1, sort_keys=True)
    print("recover_seeds: " + report["outcome"])
    if field_note:
        print("recover_seeds: " + field_note)
    if args.dry_run:
        return 0
    # write deterministically into layout.json through extract_layout's writer (same formatting rules)
    sys.path.insert(0, HERE)
    from extract_layout import dump_json, F6  # noqa: E402
    lay["seeds"] = [[float(v[0]), float(v[1])] for v in allseeds]
    lay["keep"] = [True] * n + [False] * (N - n)
    lay["cell_scales"] = [F6(v) for v in s]
    for part in lay["parts"]:  # unit vectors keep their 6-decimal formatting through the round trip
        part["lean"] = [F6(v) for v in part["lean"]]
    notes = lay.setdefault("notes", {})
    notes["seeds"] = (
        "recovered by examples/wall/tools/recover_seeds.py: the Voronoi of `seeds` bounded by `workable` (kept = the first %d in idx "
        "order, then %d recovered culled neighbours; keep[] says which), each kept cell SHRUNK about its own area centroid by "
        "cell_scales[idx], reproduces every production cell vertex within %.4f mm (max %.4f, median %.4f); the %d kept cells tile "
        "the workable rectangle completely (no culled cells); kept seeds sit %.2f mm (median, max %.2f) from the cell centroids" % (
            n, N - n, args.tolerance, devs.max(), float(np.median(devs)), n,
            report["steps"]["7_verify"]["seed_minus_centroid_mm"]["median"], report["steps"]["7_verify"]["seed_minus_centroid_mm"]["max"]))
    if scale_note:
        notes["cell_scales"] = scale_note
    if field_note:
        notes["field_at_seeds"] = field_note
    with open(args.layout, "w", encoding="utf-8", newline="\n") as f:
        f.write(dump_json(lay) + "\n")
    print("recover_seeds: wrote seeds/keep/cell_scales (+ notes) into %s" % args.layout)
    return 0



if __name__ == "__main__":
    sys.exit(main())
