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

**v1: polyscope** — Python-native, mesh/points/curves out of the box,
imgui side panels for the params UI, days-not-weeks to first pixels.

**v2 candidates** if polyscope's ceiling is hit: a wgpu-based native
viewer, or a three.js web view (which would also give remote/tablet
dashboards). Decide on evidence from Brood I usage, not upfront.

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
| STEP | in/out | OCCT (build123d) | The B-rep interchange; OCCT's best open feature |
| .3dm | in/out | rhino3dm | Rhino interop without Rhino |
| USD / glTF | out | usd-core / pygltf | Blender bridge + web preview |
| SVG | out | small writer | Laser/plot workflows |
| **.gh import** | in | GH_IO.dll via pythonnet | The format is cracked (raw-DEFLATE GH_Archive; base64 script source; `ScriptParamAccess` per input). A migration importer can recover component code, wiring, and access modes from existing definitions — the wall project's audit tooling is the seed |

## CSV/data surface

Manifests, trackers, and BOMs are first-class outputs (the wall workflow
runs on them). Any axis can be dumped as a table with its IDs; any table
can be joined back by ID. Spreadsheets are an interop format, not a UI.
