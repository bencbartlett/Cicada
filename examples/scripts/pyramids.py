# Script nodes beyond numbers (stage 6): Python builds MESHES, returns
# several outputs at once, and writes files as an effectful exporter.
#
# Three things this file demonstrates (doc 10 §5; the worker header in
# crates/cicada-script/src/worker.py is the ABI reference):
#   - `-> {"meshes": "[Watertight<Mesh>]", "volumes": "[Number]"}`: a dict
#     literal return annotation = multi-output; the dialect reads the
#     ports as `pyr.meshes` / `pyr.volumes`. The function must return a
#     dict with exactly those keys (missing/extra keys are red).
#   - `cicada.Mesh.from_triangles(vertices, triangles)`: the mesh carrier.
#     Declaring `Watertight<Mesh>` makes the host CHECK watertightness on
#     every returned mesh — a leaky pyramid is red with counts, never a
#     silent pass into a boolean.
#   - `@cicada.node(..., effectful=True)` + `-> None`: an exporter. It
#     never auto-runs and is never served from cache; `--node table` (or
#     the node's Run button in the app) is the explicit action.
#
# Pure Python 3, no packages — runs on any interpreter.

import csv
import sys

import cicada


@cicada.node(
    title="Pyramids",
    description="a square pyramid per point: watertight meshes plus their volumes.",
)
def pyramids(
    points: "[Point]",
    heights: "[Number]",
    base: "Number" = 2.0,
) -> {"meshes": "[Watertight<Mesh>]", "volumes": "[Number]"}:
    if len(points) != len(heights):
        raise ValueError(
            "points and heights differ in length: %d vs %d" % (len(points), len(heights))
        )
    half = base / 2.0
    meshes = []
    volumes = []
    for (x, y, z), height in zip(points, heights):
        vertices = [
            (x - half, y - half, z),
            (x + half, y - half, z),
            (x + half, y + half, z),
            (x - half, y + half, z),
            (x, y, z + height),
        ]
        # Counter-clockwise seen from outside: the base faces -z, the
        # four sides fan to the apex.
        triangles = [
            (0, 2, 1),
            (0, 3, 2),
            (0, 1, 4),
            (1, 2, 4),
            (2, 3, 4),
            (3, 0, 4),
        ]
        meshes.append(cicada.Mesh.from_triangles(vertices, triangles))
        volumes.append(base * base * height / 3.0)
    return {"meshes": meshes, "volumes": volumes}


@cicada.node(
    title="Export CSV",
    description="one row per mesh (index, vertex/triangle counts, volume) to a CSV file.",
    effectful=True,
)
def export_csv(meshes: "[Mesh]", volumes: "[Number]", path: "Text") -> None:
    if len(meshes) != len(volumes):
        raise ValueError(
            "meshes and volumes differ in length: %d vs %d" % (len(meshes), len(volumes))
        )
    with open(path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["index", "vertices", "triangles", "volume"])
        for index, (mesh, volume) in enumerate(zip(meshes, volumes)):
            writer.writerow([index, mesh.vertex_count, mesh.triangle_count, "%.6f" % volume])
    # Say WHERE it landed (wall lesson 7) — the worker's stderr is the
    # terminal; relative paths resolve against the pipeline's directory.
    sys.stderr.write("export_csv: wrote %d row(s) to %s\n" % (len(meshes), path))
