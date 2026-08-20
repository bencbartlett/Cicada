# Wall corpus: the frozen production layout loader (stage 6, docs/15).
#
# Reads examples/wall/inputs/layout.json (written by examples/wall/tools/
# extract_layout.py from the production exports; schema in the stage-6
# contract section 3) and fans it out as typed lists the rest of wall.cic
# consumes.
# Pure stdlib Python 3; deterministic; every missing or malformed field is
# a loud error, never a default (AGENTS.md: no silent fallbacks).
#
# Frames: model mm, z up, origin = workable-area bottom-left; the physical
# board corner sits at board_min (= (-25.4, -25.4) on the production wall).
# All 2D layout coordinates come out as 3-tuples with z = 0.

import json
import os

import cicada

_SCHEMA_KEYS = ("units", "workable", "board", "wires", "coil_board_points", "parts",
                "seeds", "cell_scales")
_PART_KEYS = ("idx", "id", "bin", "cell", "centroid", "lean", "lean_length",
              "height", "exported", "coil")


def _pipeline_dir():
    """examples/wall/ — the pipeline directory; relative paths resolve
    against it (scripts live in examples/wall/scripts/, docs/10 section 5)."""
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_path(path):
    """Absolute paths pass through; relative ones resolve against the
    pipeline directory (examples/wall/), never the process cwd."""
    path = str(path)
    if os.path.isabs(path):
        return path
    return os.path.normpath(os.path.join(_pipeline_dir(), path))


def _xy(value, what):
    if (not isinstance(value, (list, tuple)) or len(value) < 2
            or not all(isinstance(c, (int, float)) for c in value[:2])):
        raise ValueError("layout.json: %s must be [x, y], got %r" % (what, value))
    return (float(value[0]), float(value[1]), 0.0)


def load_layout(path):
    """Parse + validate layout.json. Returns the dict; raises ValueError
    with the offending key on any schema violation."""
    full = resolve_path(path)
    with open(full, "r", encoding="utf-8") as f:
        data = json.load(f)
    for key in _SCHEMA_KEYS:
        if key not in data:
            raise ValueError("layout.json (%s): missing top-level key %r" % (full, key))
    if data["units"] != "mm":
        raise ValueError("layout.json: units must be 'mm', got %r" % (data["units"],))
    for rect in ("workable", "board"):
        if "min" not in data[rect] or "max" not in data[rect]:
            raise ValueError("layout.json: %r needs min and max" % rect)
    parts = data["parts"]
    if not isinstance(parts, list) or not parts:
        raise ValueError("layout.json: parts must be a non-empty list")
    for i, part in enumerate(parts):
        for key in _PART_KEYS:
            if key not in part:
                raise ValueError("layout.json: parts[%d] missing %r" % (i, key))
        if part["idx"] != i:
            raise ValueError("layout.json: parts[%d].idx is %r (parts must be in idx order)"
                             % (i, part["idx"]))
        if len(part["cell"]) < 3:
            raise ValueError("layout.json: parts[%d].cell has %d vertices (< 3)"
                             % (i, len(part["cell"])))
        if part["coil"] not in (None, 1, 2):
            raise ValueError("layout.json: parts[%d].coil must be null, 1 or 2, got %r"
                             % (i, part["coil"]))
        if not isinstance(part["exported"], bool):
            raise ValueError("layout.json: parts[%d].exported must be a boolean" % i)
    for w in data["wires"]:
        for key in ("center", "current"):
            if key not in w:
                raise ValueError("layout.json: a wire is missing %r" % key)
        if float(w["current"]) == 0.0:
            raise ValueError("layout.json: a wire has zero current (sign decides in/out)")
    n = len(parts)
    if len(data["seeds"]) != n:
        raise ValueError("layout.json: %d seeds for %d parts (seeds are index-aligned with parts)"
                         % (len(data["seeds"]), n))
    if len(data["cell_scales"]) != n:
        raise ValueError("layout.json: %d cell_scales for %d parts" % (len(data["cell_scales"]), n))
    for i, sc in enumerate(data["cell_scales"]):
        if not isinstance(sc, (int, float)) or not (0.0 < float(sc) <= 1.0):
            raise ValueError("layout.json: cell_scales[%d] must be in (0, 1], got %r" % (i, sc))
    return data


@cicada.node(
    title="Wall Layout",
    description="the frozen production wall layout (inputs/layout.json) as typed lists: the Voronoi seeds, per-cell shrink, heights and lean lengths the pipeline consumes, plus the production cells/centroids/leans/ids it is checked against.",
)
def wall_layout(path: "Text" = "inputs/layout.json") -> {
    "seeds": "[Point]",
    "cell_scales": "[Number]",
    "cells_production": "[[Point]]",
    "centroids_production": "[Point]",
    "heights": "[Number]",
    "lean_lengths": "[Number]",
    "leans_production": "[Vector]",
    "bins": "[Integer]",
    "exported": "[Boolean]",
    "coil_captured": "[Boolean]",
    "ids_production": "[Text]",
    "wires_out": "[Point]",
    "wires_in": "[Point]",
    "coil_board_points": "[Point]",
    "board_min": "Point",
    "board_max": "Point",
    "workable_min": "Point",
    "workable_max": "Point",
}:
    data = load_layout(path)
    parts = data["parts"]
    seeds = [_xy(v, "seeds[%d]" % i) for i, v in enumerate(data["seeds"])]
    cell_scales = [float(sc) for sc in data["cell_scales"]]
    cells = [[_xy(v, "parts[%d].cell[]" % i) for v in p["cell"]] for i, p in enumerate(parts)]
    centroids = [_xy(p["centroid"], "parts[%d].centroid" % i) for i, p in enumerate(parts)]
    heights = [float(p["height"]) for p in parts]
    lean_lengths = [float(p["lean_length"]) for p in parts]
    leans = [cicada.Vector(float(p["lean"][0]), float(p["lean"][1]), 0.0) for p in parts]
    bins = [int(p["bin"]) for p in parts]
    exported = [bool(p["exported"]) for p in parts]
    coil_captured = [p["coil"] is not None for p in parts]
    ids = [str(p.get("production_id", p["id"])) for p in parts]
    # Wires: current > 0 flows OUT of the page (counterclockwise field),
    # < 0 INTO the page — the magnetic_field.py WiresOut / WiresIn split.
    wires_out = [_xy(w["center"], "wire center") for w in data["wires"] if float(w["current"]) > 0.0]
    wires_in = [_xy(w["center"], "wire center") for w in data["wires"] if float(w["current"]) < 0.0]
    coil_pts = [_xy(v, "coil_board_points[]") for v in data["coil_board_points"]]
    return {
        "seeds": seeds,
        "cell_scales": cell_scales,
        "cells_production": cells,
        "centroids_production": centroids,
        "heights": heights,
        "lean_lengths": lean_lengths,
        "leans_production": leans,
        "bins": bins,
        "exported": exported,
        "coil_captured": coil_captured,
        "ids_production": ids,
        "wires_out": wires_out,
        "wires_in": wires_in,
        "coil_board_points": coil_pts,
        "board_min": _xy(data["board"]["min"], "board.min"),
        "board_max": _xy(data["board"]["max"], "board.max"),
        "workable_min": _xy(data["workable"]["min"], "workable.min"),
        "workable_max": _xy(data["workable"]["max"], "workable.max"),
    }
