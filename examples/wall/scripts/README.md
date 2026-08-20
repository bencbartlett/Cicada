# corpus/scripts — the wall pipeline's Python script nodes

Stage 6 (docs/15): the production wall-piece pipeline ported from the
Lorenz LED wall repo's GhPython components into Cicada Python script
nodes (docs/10 §5, script ABI of the stage-6 contract §1). Pure stdlib
Python 3 (3.10+), no numpy; every file is self-contained (the engine
hashes each script's source for its cache key, so scripts never import
each other — shared helpers are duplicated and marked). Ported functions
keep their production names and carry a
`ported verbatim from <file>:<lines>` or `adapted: <why>` note.

Conventions: model mm, z up, origin = the workable-area bottom-left
corner, the physical board corner at `board_min` (−25.4, −25.4); parts
stand on z = 0 with the cell as the base; all 2D layout data crosses the
wire as 3-tuples with z = 0. Cells arrive as `[Closed<Curve>]` — the
Voronoi cells after the per-cell shrink, built by the engine in
wall.cic (`cicada.Polyline`-like objects with `.points`); each script's
`_cell_points` also accepts plain point lists (offline tests). Relative paths (`inputs/…`, `out/…`)
resolve against the pipeline directory `corpus/` (the scripts' parent),
never the process cwd. Every malformed input is a loud error; nothing
falls back silently.

## Node list

| file | node | purity | ported from |
|---|---|---|---|
| `wall_layout.py` | `wall_layout` | pure | loader of `inputs/layout.json` (contract §3 schema) |
| `solve_field.py` | `solve_field` | pure | `magnetic_field.py` physics (lines 321–401) |
| `tip_caps.py` | `tip_caps` | pure | `tip_caps.py` core (signed_area, cyc_dist, tri_corners, cap_points, cap_points_corners) |
| `wall_labels.py` | `wall_labels` | pure | `labels.py` (GLYPHS/ADVANCE/line_width/layout_lines, coords_mm, zone_of, assign_ordinals, equiv_dia, unit_xy, the try_place/force_place ladder + helpers) and `board_final_dxf.py` (FONT, id_strokes, poly_centroid, signed_inside, seg_point_dist, place_id) |
| `pin_cutters.py` | `pin_cutters` | pure | `pin_holes.py` (hole_layout, coil_hole_layout, rib_profile_pts, capsule_pts, slot_profile_pts, rot_pts, §4 constants) + pure-Python mesh construction replacing the Rhino brep builders |
| `pack_plates.py` | `pack_plates` | pure | `plate_packer.py` (unit_xy, yaw_angle_to_plus_y, rot2, poly_area2d, pyramid_area, poly_y_at_x, footprint_profile, terrain_pack, color_slug/pool_of_bin/pool_filename, plate_grid_pos, pack_pipeline, harvest_settings/harvest_profile/profile_bed/find_settings/find_profile, the main flow 1505–1758) |
| `export_bambu.py` | `export_bambu` | **effectful** | `plate_packer.py` (model_xml-family writers: bbl_object_model_xml, bbl_root_model_xml, bbl_model_settings_xml, bbl_model_rels_xml, bbl_cut_information_xml, bbl_filament_sequence_json, center_mesh, write_bbl_3mf, layer_ranges_xml, overlay_profile + the settings harvest; export loop 1822–1986) |
| `export_dxf.py` | `export_dxf` | **effectful** | `board_final_dxf.py` (dxf_document — same writer as labels.py —, join_open_plines, the layer assembly 922–942) |

## Ports (exact kwargs and outputs)

### `wall_layout(path: Text = "inputs/layout.json")`
→ `seeds: [Point]` (the recovered Voronoi seeds, idx order), `cell_scales:
[Number]` (the per-cell shrink about the area centroid — the wall's
density modulation), `cells_production: [[Point]]`, `centroids_production:
[Point]` (checks, not inputs), `heights: [Number]`,
`lean_lengths: [Number]`, `leans_production: [Vector]` (check only),
`bins: [Integer]`, `exported: [Boolean]`, `coil_captured: [Boolean]`
(`coil != null`), `ids_production: [Text]`, `wires_out: [Point]`
(current > 0), `wires_in: [Point]` (current < 0), `coil_board_points:
[Point]`, `board_min/board_max: Point` (physical), `workable_min/max:
Point`. Validates the schema (units "mm", parts in idx order, coil ∈
{null,1,2}, non-zero wire currents …).

### `solve_field(points: [Point], wires_out: [Point], wires_in: [Point], current: Number = 1.0, core_radius: Number = 0.0, influence_radius: Number = 0.0, falloff_power: Number = 2.0)`
→ `directions: [Vector]` (unit XY, z = 0; degenerate field → (1, 0, 0)
exactly like production V_unit), `magnitudes: [Number]`, `weights:
[Number]`. `core_radius`/`influence_radius` ≤ 0 select the production
defaults (5 % / 75 % of the larger extent of the points' bounding box).
One `current` applies to every wire (the production broadcast branch).

### `tip_caps(cells: [Closed<Curve>], centroids: [Point], directions: [Vector], lean_lengths: [Number], heights: [Number], tip_radius: Number = 1.8, elongate: Number = 1.0, rotate_deg: Number = 0.0, corner_snap: Boolean = True, fuse: Number = 0.03, cell_scale: Number = 0.0)`
→ `[[Point]]` — one closed polygon per part at z = height with the
cell's vertex count and seam (loft vertex i ↔ i). Apex = centroid +
direction·lean_length. `cell_scale` > 0 replaces the triangle by the
cell scaled about the apex (see production findings: **0.07 reproduces
the printed 1.4.1 parts**).

### `wall_labels(cells: [Closed<Curve>], centroids, directions, board_min: Point, text_height = 5.0, min_text_height = 2.5, edge_margin = 2.0, edge_margin_min = 1.0, outline_scale = 0.75, pin_spacing = 12.0, pin_clearance = 3.7, zone_cols: Integer = 3, zone_rows: Integer = 3, zone_letters: Text = "ABC…Z", board_text_height = 5.0, board_text_min_height = 2.5, board_text_margin = 1.5, board_pin_clear = 2.8, deboss_under = 1.0, ids_expected: [Text] = [])`
→ `ids: [Text]`, `zones: [Text]`, `deboss_text: [Text]` ('\n'-joined
lines: ID, then `XXXX` / `YYYY` or `XXXX YYYY` mm coordinates from the
PHYSICAL datum where the ladder allows), `deboss_plane: [Plane]`,
`deboss_size: [Number]` (cap height for `text_solids`), `deboss_mode:
[Text]` (ok3/ok2/ok1/relaxed/tiny/forced), `ghosts: [[Point]]`
(0.75-scaled cells, model coords), `board_strokes: [[Point]]` (flat, part
order, model coords, z = 0), `board_strokes_closed: [Boolean]` (parallel
to `board_strokes` — the FONT's 0/8/D glyphs are closed polylines),
`notes: [Text]`. `ids_expected` (optional): refuse loudly if the computed
IDs differ from the production IDs. Zones use the centroid extent
(equal thirds), exactly as labels.py; the physical board corner is the
coordinate datum.

**Deboss frame** (for Rust `text_solids(text=each(labels.deboss_text),
plane=each(labels.deboss_plane), size=each(labels.deboss_size),
depth=2.0)`): origin on the base face at z = +1.0 (`deboss_under`), x =
baseline, y flipped so x × y = −z; depth 2.0 cuts z ∈ [−1, +1] — a 1 mm
deboss that reads correctly from BELOW (production MirrorPartText). The
block is centered on the ladder's placement (tx, ty) and `deboss_size`
shrinks the cap height so DejaVu Sans Bold (advance widths embedded)
stays inside the stroke-font block the ladder reserved (production did
the same uniform fit for Arial Black — `fit_scale`).

### `pin_cutters(centroids, directions, cells: [Closed<Curve>], bore = 3.4, rib_eff_dia = 3.12, rib_width = 1.0, rib_count: Integer = 3, chamfer = 1.0, relief = 1.6, pin_len = 15.875, board_depth = 8.5, pin_spacing = 12.0, slot_half = 0.15, hole_cone: Boolean = True)`
→ `cutters: [[Watertight<Mesh>]]` (per part, production order: centroid
bore [round locator with ribs: bore prism z ∈ [−1, 8.975], 45° mouth
chamfer r 2.7→1.7 over z ∈ [−1, 1], 60° cone ceiling r 1.7→0.6 from z =
8.475 to 10.880], then the lean bore [slot with two vanes: capsule
prism, capsule chamfer, capsule cone]), `board_points: [Point]` (2 per
part, flat: centroid, then centroid + spacing·direction), `spacing:
[Number]` (12 clamped to 0.35·d_equiv), `notes: [Text]`. Every cutter is
its own watertight mesh with outward winding (ear-clipped caps for the
non-convex rib/slot profiles; 48-segment rings for the round lofts;
watertightness + signed-volume self-check at build time).
Derived: PROUD = 7.375, HOLE_DEPTH = 8.975, UNDER = 1.0, CONE_TIP_R = 0.6.

### `pack_plates(cells: [Closed<Curve>], centroids, directions, heights, lean_lengths, ids: [Text], bins: [Integer], exported: [Boolean], settings_dir: Text = "inputs/bambu", apex_origins: [Point] = [], clearance = 3.0, step = 1.0, margin_front = 25.0, margin_side = 12.0, margin_back = 10.0, height_margin = 5.0, x1c_size = 256.0, x1c_max_z = 250.0, h2_width = 330.0, h2_depth = 320.0, h2_max_z = 325.0, area_per_hour = 25000.0, grams_per_area = 0.001)`
→ `part_frames: [Plane]` (source: origin = centroid at z = cz (0), x =
lean unit, y = z × x), `plate_frames: [Plane]` (target: the production
placement — yaw lean → +Y, base on z = 0, footprint bbox min at the
packed (x, y) in the usable-area frame + X/Y shifts + **the Bambu plate
grid offset**, so `orient`ed meshes sit in the 3MF world frame),
`plate: [Integer]` (global number, 0 excluded), `plate_local: [Integer]`
(1-based within the file, 0 excluded), `slot: [Integer]` (placement
order within the plate, −1 excluded), `printer: [Text]` ("X1C"/"H2"/""),
`file: [Text]` (`plates_f<bin>_<color>_<printer>.3mf` or ""),
`manifest_rows: [Text?]` (absent for excluded), `manifest: [Text]`
(header + rows in production plate order), `plate_table: [Text]`,
`notes: [Text]`. Reads `example_settings.3mf` / `example_settings_x1c.3mf`
from `settings_dir` for the H2 nozzle keep-out (25/0 mm), the height cap
(325), the X1 bed_exclude_area (18×28 front-left) and the embedded
profiles' printable areas (plate grid stride 1.2 × bed). Oversize parts
and missing settings are loud errors.

### `export_bambu(meshes: [Mesh], ids: [Text], bins: [Integer], exported: [Boolean], plates: [Integer], slots: [Integer], manifest: [Text], settings_dir: Text = "inputs/bambu", out_dir: Text = "out") -> None` *(effectful)*
Writes `<out_dir>/plates_f<bin>_<color>_<printer>.3mf` (one per bin
present) + `manifest.csv`. Object names = ids; mesh object ids 2k−1,
wrapper ids 2k, build items = centered meshes + world translation (bbox
center of the oriented mesh), `layer_config_ranges.xml` keyed 1..N,
`identify_id` from 100 per file, the H2 `filament_maps`, the embedded
profile = reference project_settings + the proc overlay. Object order =
plates by number, parts by slot (production order).

### `export_dxf(ghosts: [[Point]], strokes: [[Point]], strokes_closed: [Boolean], holes: [Point], coil_holes: [Point], board_min: Point, board_max: Point, skip_holes: [Boolean] = [], hole_dia = 3.1, stock_width_in = 97.0, stock_height_in = 49.0, join_text: Boolean = False, path: Text = "out/board.dxf") -> None` *(effectful)*
Writes the R12 board DXF in PHYSICAL-datum mm (%.3f): OUTLINES (ghosts,
closed, closing vertex repeated as production), TEXT (the FONT strokes),
PINHOLES (r = hole_dia/2: 2 per part except where `skip_holes[i]` — the
58 coil-captured parts in production — then the coil holes), BOARDCUT
(0,0)–(W,H), STOCK (97×49 in centered). Layer table reproduces the
production quirk: `OUTLINES, PINHOLES, BOARDCUT, STOCK` — **TEXT is used
but not declared** (board_final_dxf.py:941). CRLF line endings.

## Suggested wall.cic wiring (the integrator owns the file)

```
layout = wall_layout(path="inputs/layout.json")
# … voronoi(seeds=layout.seeds, boundary=board) → area centroids → scale by layout.cell_scales = cells
field  = solve_field(points=layout.seeds, wires_out=layout.wires_out, wires_in=layout.wires_in,
                     current=amps, core_radius=243.84)        # production: 0.1 × 96 in, at the seeds
caps   = tip_caps(cells=layout.cells, centroids=layout.centroids, directions=field.directions,
                  lean_lengths=layout.lean_lengths, heights=layout.heights, cell_scale=0.07)   # production caps
labels = wall_labels(cells=layout.cells, centroids=layout.centroids, directions=field.directions,
                     board_min=layout.board_min, ids_expected=layout.ids_production)
glyphs = text_solids(text=each(labels.deboss_text), plane=each(labels.deboss_plane),
                     size=each(labels.deboss_size), depth=2.0)
pins   = pin_cutters(centroids=layout.centroids, directions=field.directions, cells=layout.cells)
plates = pack_plates(cells=layout.cells, centroids=layout.centroids, directions=field.directions,
                     heights=layout.heights, lean_lengths=layout.lean_lengths, ids=labels.ids,
                     bins=layout.bins, exported=layout.exported, settings_dir="inputs/bambu",
                     step=2.0, h2_width=320.0)                        # the values production ran with
oriented = orient(geometry=each(carved), source=each(plates.part_frames), target=each(plates.plate_frames))
bambu  = export_bambu(meshes=oriented, ids=labels.ids, bins=layout.bins, exported=layout.exported,
                      plates=plates.plate, slots=plates.slot, manifest=plates.manifest,
                      settings_dir="inputs/bambu", out_dir="out")
dxf    = export_dxf(ghosts=labels.ghosts, strokes=labels.board_strokes, strokes_closed=labels.board_strokes_closed,
                    holes=pins.board_points, coil_holes=layout.coil_board_points, skip_holes=layout.coil_captured,
                    board_min=layout.board_min, board_max=layout.board_max, path="out/board.dxf")
```

## Deviations from production (declared)

1. **Deboss font**: production extruded Arial Black outlines; Cicada
   bundles DejaVu Sans Bold (contract §2). Same ported placement ladder,
   cap height fitted to the reserved block (production fitted Arial
   Black the same way), lines left-aligned from one origin (text_solids
   lays out left-aligned) instead of centered per line. Tessellation of
   the glyph solids differs by construction.
2. **ZIP timestamps** in the 3MFs are fixed at 1980-01-01 (production:
   wall-clock). Declared in the contract.
3. **Cutter tessellation**: round chamfer/cone lofts are 48-gons (Rhino
   meshed true circles); the carved result's bbox/volume are unaffected
   beyond the normalizer's tolerances.
4. `wall_labels` does not take `workable_min/max` (labels.py never used
   the workable rectangle — zones come from the centroid extent, the
   datum from the board rectangle).
5. Ghosts and board strokes are returned in MODEL coordinates (the
   contract sketch said physical-datum mm for strokes); `export_dxf`
   applies the datum shift to everything — one frame on the wires.
6. `pack_plates` emits `manifest` (full file lines in plate order) in
   addition to the per-part `manifest_rows`, plus `slot` / `plate_local`;
   `export_bambu` takes `slots` + `manifest` (not `files` /
   `manifest_rows`) — the exact object order and the byte-exact manifest
   need the within-plate order.
7. Production warned and continued on missing settings / oversize parts
   / no profile; these scripts refuse (AGENTS.md: fail loudly).
8. Not ported (out of scope): the reprint mode (OnlyIDs), PlateFilter,
   Limit, WritePlateMeta=False generic 3MF, the coupon, coil cylinder
   cutters, labels.py's own DXF (the shipped CNC file came from
   board_final_dxf.py's regenerated-TEXT path), the Rhino previews.

## Production findings (measured on the shipped 1.4.1 exports; see corpus/tools/test_production_crosscheck.py)

* **IDs**: zones + `assign_ordinals` reproduce all 1200 production IDs
  (checked against both manifests, 1142 parts).
* **Board DXF**: from a layout reconstructed out of the shipped
  board_postprocessed.dxf, the OUTLINES / PINHOLES / BOARDCUT / STOCK
  layers match within 0.001 mm and the TEXT layer matches for every part
  whose lean direction is known (the 58 coil-captured parts have no
  drills in the file, so their label offset cannot be reconstructed
  there).
* **3MF writer**: the non-mesh entries of the pristine plates_f3 file
  rebuild byte-exact from the production metadata; the H2 embedded
  profile (example_settings.3mf + proc overlay) is byte-exact.
* **Tip caps**: the printed parts carry the base cell scaled by exactly
  0.07 about the apex (the "scaled mini-cell cap"), not tip_caps'
  triangles → `tip_caps(cell_scale=0.07)`.
* **Packer parameters**: replaying the production footprints (from the
  H2 meshes) through `terrain_pack` reproduces the first placements of
  plate 42 and the 3MF item X/Z translations to 0.001 mm only with
  **PackStep = 2** and a usable H2 width of **271 mm** (= H2DWidth 320,
  keep-out 25/0, X_SHIFT 12.5 confirmed by the translations); the .cic
  must pass `step=2.0, h2_width=320.0`.
* **Packer inputs ≠ printed geometry**: in the shipped H2 meshes the pin
  pair and the apex sit 0.3–1.1° off +Y and the manifest `area_mm2`
  scatters ±47 mm² around the value computed from the printed apex — the
  production packer yawed and measured with Apexes/Centroids that differ
  from the printed tips/centroids by 1–3 mm (lateral) / ~1.5 mm (along
  the lean); the packing is chaotic in those inputs, so the production
  plate layout is NOT reproducible from the printed artifacts alone.
  `apex_origins` takes the recovered Voronoi seeds (the production
  Apexes were computed where the field was solved — at the seeds), which
  reproduces the production yaw to 0.005°; without it the manifest/3MF
  structural comparison fails on plate assignment (geometry per part
  still compares).

## Tests

```
python -m unittest discover -s corpus/tools -p "test_*.py"
```
Offline (the `cicada` module is stubbed by corpus/tools/_cicada_stub.py):
test_scripts.py (nodes), test_normalize.py (the normalizer),
test_production_crosscheck.py (read-only checks against the wall repo;
skipped when it is absent).
