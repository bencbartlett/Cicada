# Rendering and interop

Two renderers, one boundary: **Cicada owns the fast truthful preview;
Blender owns beauty.** Interchange is boring on purpose.

## Real-time viewer (in Cicada)

Requirements, in priority order:

1. **Instanced rendering** for thousands of objects — the wall scene is
   ~1,500 solids plus previews, which both Grasshopper and Rhino handled
   badly. Same-mesh-different-transform must be one draw call.
2. **Backward picking**: click geometry → producing node + element index +
   part ID light up. Launch requirement (see architecture doc).
3. **Per-node preview toggles** with display cost shown next to compute
   cost in the profiler — display is a first-class edge.
4. Live updates from the scheduler: partial results appear as stages
   complete; a cancelled solve leaves the last coherent frame.
5. Section/measure/isolate as cheap inspection tools (later).

**v1: three.js in the browser — web-first, Onshape-style.** The Rust
engine serves the app and streams mesh buffers over binary WebSocket
frames (`cicada serve`, local or remote); the desktop app comes later
as a thin Tauri wrapper bundling a local engine — a packaging
exercise, not a second codebase. Instanced meshes make wall-scale
scenes trivial; an ID-buffer pass gives backward picking (instance →
node + element index + part ID); canvas, params panel, inspectors, and
viewport dock in one window — itself a usability win over the GH/Rhino
split.

**v2 candidate** if the browser ceiling is hit on real scenes: a native
wgpu viewer sharing the engine's GPU compute path. The web viewport
already gives remote/tablet dashboards for free. Decide on evidence
from v0.1 usage, not upfront.

## Blender bridge (photorealistic renders)

Blender is the render backend, not a modeling dependency:

- **Transport: USD** (primary; glTF secondary for web previews). Each
  Cicada axis element becomes a prim; attributes carry `part_id`, axis
  names/indices, and material bin. Instancing maps to USD point instancers
  where geometry repeats.
- **Materials by convention**: Cicada assigns material *names* (e.g. the
  wall's five filament bins); a Blender-side template .blend maps names →
  shader setups. Cicada never speaks shader graphs.
- **Headless driving**: `pip install bpy` gives scriptable Cycles without
  opening the UI. A `cicada render` command exports USD, applies the
  template, sets camera/resolution/samples from typed render presets in
  the pipeline file, and writes PNG/EXR. Cameras defined in Cicada
  (bookmarkable from the viewer) export with the scene.
- **Interactive path**: the same USD opens in the Blender UI for manual
  lighting/compositing; re-export updates geometry in place (stable prim
  paths from part IDs, so Blender-side edits survive).
- **Later, optional**: a live-link add-on (socket push into a running
  Blender session). File-based round-trip first; it is 90% of the value.

## Exchange formats

| Format | Direction | Via | Notes |
|---|---|---|---|
| 3MF (Bambu project flavor) | out | ported wall-repo writer | Multi-plate, per-object filaments, height ranges, per-pool slicer policy — production-proven |
| STL/3MF (plain) | out | trimesh/Manifold | Generic mesh export |
| DXF | out | ported wall-repo writer | CNC: holes/outlines/text layers, datum discipline |
| STEP | in/out | OCCT (`opencascade-rs`) | The B-rep interchange; OCCT's best open feature |
| .3dm | in/out | rhino3dm | Rhino interop without Rhino |
| USD / glTF | out | usd-core (Python side) / gltf (Rust) | Blender bridge + web preview; not a hot path |
| SVG | out | small writer | Laser/plot workflows |
| **.gh import** | in | GH_IO.dll via pythonnet | The format is cracked (raw-DEFLATE GH_Archive; base64 script source; `ScriptParamAccess` per input). A migration importer can recover component code, wiring, and access modes from existing definitions — the wall project's audit tooling is the seed |

The wall-repo exporters (Bambu 3MF, DXF, manifests) and the .gh importer
run as Python 3 script nodes first — they are production-proven code —
and promote to Rust only if profiling ever cares; export is not a hot
path.

## CSV/data surface

Manifests, trackers, and BOMs are first-class outputs (the wall workflow
runs on them). Any axis can be dumped as a table with its IDs; any table
can be joined back by ID. Spreadsheets are an interop format, not a UI.
