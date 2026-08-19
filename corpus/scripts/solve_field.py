# Wall corpus: the magnetic field solver (stage 6, docs/15).
#
# Ported from the wall repo's magnetic_field.py (GhPython). Physics per
# wire, superposed (magnetic_field.py:12-22):
#
#     B(p) = s * I * ( -(py - wy), (px - wx), 0 ) / (r^2 + CoreRadius^2)
#
# s = +1 for current OUT of the page (counterclockwise), -1 INTO the page.
# The CoreRadius^2 softening kills the 1/r singularity. Only the pure
# evaluation loop is ported (magnetic_field.py:321-401); the Rhino preview
# outputs (V, Lines, Traj3D, TrajDisplay, SamplePts) are not nodes.
#
# Pure stdlib Python 3, deterministic: the same points + wires give the
# same bits on every run (the engine memoizes on input hashes).

import math

import cicada


def field_at(px, py, wires, core2):
    """ported verbatim from magnetic_field.py:324-350 (the per-point
    superposition; `wires` = [(wx, wy, signed_current)])."""
    bx = 0.0
    by = 0.0
    best_d2 = None
    for (wx, wy, s_i) in wires:
        dx = px - wx
        dy = py - wy
        r2 = dx * dx + dy * dy
        if best_d2 is None or r2 < best_d2:
            best_d2 = r2
        denom = r2 + core2
        if denom < 1e-12:
            continue
        # Out-of-page current circulates counterclockwise:
        # B ~ z_hat x rel = (-dy, dx)
        c = s_i / denom
        bx += -dy * c
        by += dx * c
    return bx, by, best_d2


def solve(points, wires, core_radius, influence_radius, falloff_power):
    """ported verbatim from magnetic_field.py:321-401 minus the Rhino
    types: returns (unit_dirs, mags, weights). Degenerate field -> unit
    (1, 0) exactly as production's V_unit."""
    core2 = core_radius * core_radius
    dirs = []
    mags = []
    weights = []
    for (px, py, _pz) in points:
        bx, by, best_d2 = field_at(px, py, wires, core2)
        dist = math.sqrt(best_d2)
        # Distance-based weight so the piece can fade far from the wires.
        # 1 at the wires, approaching 0 far away.
        if influence_radius <= 1e-9:
            w = 1.0
        else:
            t = dist / influence_radius
            w = math.exp(-(t ** falloff_power))
        mag = math.sqrt(bx * bx + by * by)  # Rhino Vector3d.Length
        if mag > 1e-9:
            # adapted: Rhino's Vector3d.Unitize() == divide by the length
            ux, uy = bx / mag, by / mag
        else:
            ux, uy = 1.0, 0.0
        dirs.append((ux, uy))
        mags.append(mag)
        weights.append(w)
    return dirs, mags, weights


@cicada.node(
    title="Solve Field",
    description="2D magnetic field of straight wires through the wall plane: unit direction + magnitude per point (ported magnetic_field.py).",
)
def solve_field(
    points: "[Point]",
    wires_out: "[Point]",
    wires_in: "[Point]",
    current: "Number" = 1.0,
    core_radius: "Number" = 0.0,
    influence_radius: "Number" = 0.0,
    falloff_power: "Number" = 2.0,
) -> {"directions": "[Vector]", "magnitudes": "[Number]", "weights": "[Number]"}:
    """core_radius / influence_radius <= 0 select the production defaults
    (5% / 75% of the larger extent of the points' bounding box,
    magnetic_field.py:275-281); falloff_power <= 0 -> 2.0."""
    if not points:
        raise ValueError("solve_field: no points")
    if not wires_out and not wires_in:
        # magnetic_field.py substitutes a demo wire pair here; the corpus
        # refuses instead — demo wires would silently produce a wrong wall.
        raise ValueError("solve_field: no wires (wires_out and wires_in both empty)")
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    width = max(xs) - min(xs)
    height = max(ys) - min(ys)
    if width < 1e-9:
        width = 1.0
    if height < 1e-9:
        height = 1.0
    max_dim = max(width, height)
    if core_radius <= 0.0:
        core_radius = 0.05 * max_dim
    if influence_radius <= 0.0:
        influence_radius = 0.75 * max_dim
    if falloff_power <= 0.0:
        falloff_power = 2.0
    # Assemble wire list: (x, y, signed current) -- one current for
    # every wire (magnetic_field.py:301-315, broadcast branch).
    wires = []
    for (wx, wy, _wz) in wires_out:
        wires.append((wx, wy, float(current)))
    for (wx, wy, _wz) in wires_in:
        wires.append((wx, wy, -float(current)))
    dirs, mags, weights = solve(points, wires, core_radius, influence_radius, falloff_power)
    return {
        "directions": [cicada.Vector(ux, uy, 0.0) for (ux, uy) in dirs],
        "magnitudes": mags,
        "weights": weights,
    }
