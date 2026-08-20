"""Shared helpers for the engine-wide offline tests (python -m unittest
discover -s tools -p "test_*.py"): install the offline `cicada` stub,
import the wall's script nodes (examples/wall/scripts/*.py) by path,
locate the optional reference material.

No engine, no network, no wall-clock: everything here is deterministic.
"""

import importlib.util
import os
import sys
import types

TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))      # <repo>/tools
REPO_DIR = os.path.dirname(TOOLS_DIR)
WALL_DIR = os.path.join(REPO_DIR, "examples", "wall")       # the wall project (pipeline dir)
SCRIPTS_DIR = os.path.join(WALL_DIR, "scripts")
INPUTS_DIR = os.path.join(WALL_DIR, "inputs")
GOLDEN_DIR = os.path.join(WALL_DIR, "golden", "production")

# The wall repo (READ ONLY) -- only used when present, for the optional
# production cross-checks; the unit tests never require it.
WALL_REPO = os.path.join(os.path.dirname(REPO_DIR),
                         "3D Print Stuff", "Lorenz LED wall")
WALL_EXPORT = os.path.join(WALL_REPO, "export", "solenoid_art_export_1.4.1")


def install_stub():
    """Make `import cicada` resolve to the offline stub (idempotent)."""
    if "cicada" not in sys.modules or not hasattr(sys.modules["cicada"], "Mesh"):
        spec = importlib.util.spec_from_file_location(
            "cicada", os.path.join(TOOLS_DIR, "_cicada_stub.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        sys.modules["cicada"] = module
    return sys.modules["cicada"]


_SCRIPTS = {}


def load_module_from_source(module_name, path):
    """Load a script file as a module the way the engine's worker does
    (crates/cicada-script/src/worker.py `_load`): exec the compiled source
    into a bare module, never the import machinery. Two reasons: the tests
    then exercise exactly the loading path the engine uses, and no
    `__pycache__/` is ever written beside the served project's scripts
    (examples/wall/scripts/ is a project directory inside a synced tree, and
    test by-products do not belong there)."""
    with open(path, "r", encoding="utf-8") as f:
        source = f.read()
    module = types.ModuleType(module_name)
    module.__file__ = path
    exec(compile(source, path, "exec"), module.__dict__)
    return module


def load_script(name):
    """Import examples/wall/scripts/<name>.py as a module (cached)."""
    install_stub()
    if name in _SCRIPTS:
        return _SCRIPTS[name]
    path = os.path.join(SCRIPTS_DIR, name + ".py")
    module = load_module_from_source("wall_script_" + name, path)
    _SCRIPTS[name] = module
    return module


def settings_dir():
    """Where the Bambu reference projects live: examples/wall/inputs/bambu
    (committed), else the wall repo root (read only), else None."""
    for cand in (os.path.join(INPUTS_DIR, "bambu"), WALL_REPO):
        if (os.path.isfile(os.path.join(cand, "example_settings.3mf"))
                and os.path.isfile(os.path.join(cand, "example_settings_x1c.3mf"))):
            return cand
    return None


def layout_path():
    p = os.path.join(INPUTS_DIR, "layout.json")
    return p if os.path.isfile(p) else None


def regular_polygon(cx, cy, r, n, z=0.0, phase=0.0):
    import math
    return [(cx + r * math.cos(phase + 2 * math.pi * k / n),
             cy + r * math.sin(phase + 2 * math.pi * k / n), z) for k in range(n)]


def mesh_arrays(mesh):
    """cicada.Mesh -> (verts, tris) lists."""
    p = mesh.positions
    t = mesh.indices
    verts = [(p[3 * i], p[3 * i + 1], p[3 * i + 2]) for i in range(len(p) // 3)]
    tris = [(t[3 * i], t[3 * i + 1], t[3 * i + 2]) for i in range(len(t) // 3)]
    return verts, tris


def make_settings_dir(folder):
    """Write synthetic Bambu settings references (example_settings.3mf for
    the H2, example_settings_x1c.3mf for the X1C) carrying exactly the keys
    the ported harvest reads, with the production values: H2 bed 330x320,
    master extruder 2 reaching x in [25, 330], heights [320, 325], filament
    map "2"; X1C bed 256x256 with the 18x28 mm front-left bed_exclude_area.
    Returns `folder`."""
    import json
    import zipfile
    os.makedirs(folder, exist_ok=True)
    proc = {"top_shell_layers": "3", "top_shell_thickness": "0",
            "bottom_shell_layers": "2", "bottom_shell_thickness": "0",
            "sparse_infill_density": "0%"}
    rng = ('<?xml version="1.0" encoding="utf-8"?>\n<objects>\n <object id="1">\n'
           '  <range min_z="0" max_z="4">\n   <option opt_key="extruder">0</option>\n'
           '   <option opt_key="layer_height">0.2</option>\n'
           '   <option opt_key="sparse_infill_density">25%</option>\n'
           '   <option opt_key="wall_loops">3</option>\n  </range>\n </object>\n</objects>\n')

    def write(name, ps, fmaps):
        with zipfile.ZipFile(os.path.join(folder, name), "w") as zf:
            zf.writestr("Metadata/project_settings.config", json.dumps(ps, indent=4))
            zf.writestr("Metadata/layer_config_ranges.xml", rng)
            zf.writestr("Metadata/model_settings.config",
                        '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n  <plate>\n'
                        '    <metadata key="plater_id" value="1"/>\n'
                        '    <metadata key="filament_maps" value="%s"/>\n  </plate>\n</config>\n' % fmaps)

    h2 = dict(proc)
    h2.update({"printable_area": ["0x0", "330x0", "330x320", "0x320"],
               "extruder_printable_area": ["0x0,325x0,325x320,0x320", "25x0,330x0,330x320,25x320"],
               "master_extruder_id": "2", "extruder_printable_height": ["320", "325"],
               "bed_exclude_area": [], "printer_model": "Bambu Lab H2C",
               "nozzle_diameter": ["0.4", "0.4"]})
    write("example_settings.3mf", h2, "2")
    x1c = dict(proc)
    x1c.update({"printable_area": ["0x0", "256x0", "256x256", "0x256"],
                "bed_exclude_area": ["0x0", "18x0", "18x28", "0x28"],
                "printer_model": "Bambu Lab X1 Carbon", "nozzle_diameter": ["0.4"]})
    write("example_settings_x1c.3mf", x1c, "1 1 1 1")
    return folder


def rigid_motion(source, target):
    """The rigid motion carrying plane `source` onto `target` (both
    cicada.Plane with orthonormal x, y in the XY plane, z up): returns
    f(point) -> point, mirroring what Rust `orient` does."""
    import math

    def unit(v):
        L = math.sqrt(v[0] ** 2 + v[1] ** 2 + v[2] ** 2)
        return (v[0] / L, v[1] / L, v[2] / L)

    def cross(a, b):
        return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])

    sx, sy = unit(source.x), unit(source.y)
    sz = cross(sx, sy)
    tx, ty = unit(target.x), unit(target.y)
    tz = cross(tx, ty)
    so, to = source.origin, target.origin

    def f(p):
        d = (p[0] - so[0], p[1] - so[1], p[2] - so[2])
        a = d[0] * sx[0] + d[1] * sx[1] + d[2] * sx[2]
        b = d[0] * sy[0] + d[1] * sy[1] + d[2] * sy[2]
        c = d[0] * sz[0] + d[1] * sz[1] + d[2] * sz[2]
        return (to[0] + a * tx[0] + b * ty[0] + c * tz[0],
                to[1] + a * tx[1] + b * ty[1] + c * tz[1],
                to[2] + a * tx[2] + b * ty[2] + c * tz[2])

    return f


def synthetic_wall(n=40, seed=1, span=(2300.0, 1100.0)):
    """A deterministic little wall: regular-polygon cells, centroids,
    heights, lean lengths, bins. Returns a dict of lists."""
    import random
    rnd = random.Random(seed)
    cells, cents, heights, leans, bins = [], [], [], [], []
    cols = int(math_ceil(n ** 0.5))
    pitch_x = span[0] / cols
    pitch_y = span[1] / max(1, (n + cols - 1) // cols)
    for i in range(n):
        cx = 40.0 + (i % cols) * pitch_x + rnd.uniform(-3, 3)
        cy = 40.0 + (i // cols) * pitch_y + rnd.uniform(-3, 3)
        r = rnd.uniform(18.0, 32.0)
        k = rnd.randint(5, 8)
        cells.append(regular_polygon(cx, cy, r, k, phase=rnd.random()))
        cents.append((cx, cy, 0.0))
        h = rnd.uniform(20.0, 120.0)
        heights.append(h)
        leans.append(1.125 * (h - 0.1))
        bins.append(rnd.randint(0, 4))
    return {"cells": cells, "centroids": cents, "heights": heights,
            "lean_lengths": leans, "bins": bins}


def math_ceil(x):
    import math
    return math.ceil(x)
