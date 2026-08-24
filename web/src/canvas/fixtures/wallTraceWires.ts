/**
 * The wall in trace mode (docs/17 finding U6; the B2 review's first finding):
 * `examples/wall/wall.cic` under the server's auto-layout, as the router
 * sees it — the 70 wires' endpoints `wireEnds` computes from the graph and
 * the canvas's node positions (unit 24 px; React Flow's measured handle
 * geometry: overhang 3.5 px, row shift −1 px, so the row lattice's origin
 * is 5 px), read from the running app on 2026-08-24. Its busiest three-unit
 * gap — between the `caps`/`labels`/`pins` column and `glyphs`/`plates`/
 * `dxf` — carries 22 vertical runs, at most 8 deep at any height, on nine
 * lines a Z may take: the case that made the first router stack five wires
 * on one line. The layout is a snapshot, not a contract: if the auto-layout
 * changes, this stays a valid (and hard) input for the router.
 */
import type { TraceWire } from "../trace";

export const WALL_UNIT = 24;
export const WALL_ROW_ORIGIN = 5;
export const WALL_WIRES: TraceWire[] = [
  { id: "layout.workable_min->wmin.point", ends: { sx: 243.5, sy: 419, tx: 308.5, ty: 35 } },
  { id: "layout.workable_max->wmax.point", ends: { sx: 243.5, sy: 443, tx: 308.5, ty: 155 } },
  { id: "wmin.x->wx.start", ends: { sx: 531.5, sy: 35, tx: 620.5, ty: 35 } },
  { id: "wmax.x->wx.end", ends: { sx: 531.5, sy: 155, tx: 620.5, ty: 59 } },
  { id: "wmin.y->wy.start", ends: { sx: 531.5, sy: 59, tx: 620.5, ty: 131 } },
  { id: "wmax.y->wy.end", ends: { sx: 531.5, sy: 179, tx: 620.5, ty: 155 } },
  { id: "wx.out->board.x", ends: { sx: 843.5, sy: 35, tx: 908.5, ty: 59 } },
  { id: "wy.out->board.y", ends: { sx: 843.5, sy: 131, tx: 908.5, ty: 83 } },
  { id: "layout.seeds->voro.seeds", ends: { sx: 243.5, sy: 35, tx: 1196.5, ty: 35 } },
  { id: "board.out->voro.boundary", ends: { sx: 1131.5, sy: 35, tx: 1196.5, ty: 59 } },
  { id: "geom.centroid->cells.center", ends: { sx: 1707.5, sy: 59, tx: 1772.5, ty: 59 } },
  { id: "layout.cell_scales->cells.factor", ends: { sx: 243.5, sy: 59, tx: 1772.5, ty: 83 } },
  { id: "layout.seeds->field.points", ends: { sx: 243.5, sy: 35, tx: 308.5, ty: 275 } },
  { id: "layout.wires_out->field.wires_out", ends: { sx: 243.5, sy: 299, tx: 308.5, ty: 299 } },
  { id: "layout.wires_in->field.wires_in", ends: { sx: 243.5, sy: 323, tx: 308.5, ty: 323 } },
  { id: "amps.out->field.current", ends: { sx: 219.5, sy: 515, tx: 308.5, ty: 347 } },
  { id: "cells.out->caps.cells", ends: { sx: 1995.5, sy: 35, tx: 2060.5, ty: 35 } },
  { id: "geom.centroid->caps.centroids", ends: { sx: 1707.5, sy: 59, tx: 2060.5, ty: 59 } },
  { id: "field.directions->caps.directions", ends: { sx: 555.5, sy: 275, tx: 2060.5, ty: 83 } },
  { id: "layout.lean_lengths->caps.lean_lengths", ends: { sx: 243.5, sy: 155, tx: 2060.5, ty: 107 } },
  { id: "layout.heights->caps.heights", ends: { sx: 243.5, sy: 131, tx: 2060.5, ty: 131 } },
  { id: "caps.out->cap_rings.vertices", ends: { sx: 2283.5, sy: 35, tx: 2492.5, ty: 35 } },
  { id: "cap_rings.out->cap_curves.curve", ends: { sx: 2715.5, sy: 35, tx: 2828.5, ty: 35 } },
  { id: "cells.out->frusta.start", ends: { sx: 1995.5, sy: 35, tx: 3116.5, ty: 35 } },
  { id: "cap_curves.out->frusta.end", ends: { sx: 3051.5, sy: 35, tx: 3116.5, ty: 59 } },
  { id: "deboss.out->cut.deboss", ends: { sx: 219.5, sy: 683, tx: 308.5, ty: 491 } },
  { id: "cells.out->labels.cells", ends: { sx: 1995.5, sy: 35, tx: 2060.5, ty: 347 } },
  { id: "geom.centroid->labels.centroids", ends: { sx: 1707.5, sy: 59, tx: 2060.5, ty: 371 } },
  { id: "field.directions->labels.directions", ends: { sx: 555.5, sy: 275, tx: 2060.5, ty: 395 } },
  { id: "layout.board_min->labels.board_min", ends: { sx: 243.5, sy: 371, tx: 2060.5, ty: 419 } },
  { id: "deboss.out->labels.deboss_under", ends: { sx: 219.5, sy: 683, tx: 2060.5, ty: 779 } },
  { id: "layout.ids_production->labels.ids_expected", ends: { sx: 243.5, sy: 275, tx: 2060.5, ty: 803 } },
  { id: "labels.deboss_text->glyphs.text", ends: { sx: 2427.5, sy: 395, tx: 2492.5, ty: 131 } },
  { id: "labels.deboss_size->glyphs.size", ends: { sx: 2427.5, sy: 443, tx: 2492.5, ty: 155 } },
  { id: "cut.out->glyphs.depth", ends: { sx: 531.5, sy: 491, tx: 2492.5, ty: 179 } },
  { id: "labels.deboss_plane->glyphs.plane", ends: { sx: 2427.5, sy: 419, tx: 2492.5, ty: 203 } },
  { id: "geom.centroid->pins.centroids", ends: { sx: 1707.5, sy: 59, tx: 2060.5, ty: 875 } },
  { id: "field.directions->pins.directions", ends: { sx: 555.5, sy: 275, tx: 2060.5, ty: 899 } },
  { id: "cells.out->pins.cells", ends: { sx: 1995.5, sy: 35, tx: 2060.5, ty: 923 } },
  { id: "glyphs.out->cutters.a", ends: { sx: 2715.5, sy: 131, tx: 2828.5, ty: 107 } },
  { id: "pins.cutters->cutters.b", ends: { sx: 2283.5, sy: 875, tx: 2828.5, ty: 131 } },
  { id: "frusta.out->carved.mesh", ends: { sx: 3339.5, sy: 35, tx: 3404.5, ty: 35 } },
  { id: "cutters.out->carved.cutters", ends: { sx: 3051.5, sy: 107, tx: 3404.5, ty: 59 } },
  { id: "cells.out->plates.cells", ends: { sx: 1995.5, sy: 35, tx: 2492.5, ty: 347 } },
  { id: "geom.centroid->plates.centroids", ends: { sx: 1707.5, sy: 59, tx: 2492.5, ty: 371 } },
  { id: "field.directions->plates.directions", ends: { sx: 555.5, sy: 275, tx: 2492.5, ty: 395 } },
  { id: "layout.heights->plates.heights", ends: { sx: 243.5, sy: 131, tx: 2492.5, ty: 419 } },
  { id: "layout.lean_lengths->plates.lean_lengths", ends: { sx: 243.5, sy: 155, tx: 2492.5, ty: 443 } },
  { id: "labels.ids->plates.ids", ends: { sx: 2427.5, sy: 347, tx: 2492.5, ty: 467 } },
  { id: "layout.bins->plates.bins", ends: { sx: 243.5, sy: 203, tx: 2492.5, ty: 491 } },
  { id: "layout.exported->plates.exported", ends: { sx: 243.5, sy: 227, tx: 2492.5, ty: 515 } },
  { id: "layout.seeds->plates.apex_origins", ends: { sx: 243.5, sy: 35, tx: 2492.5, ty: 563 } },
  { id: "carved.out->oriented.geometry", ends: { sx: 3627.5, sy: 35, tx: 3692.5, ty: 35 } },
  { id: "plates.part_frames->oriented.source", ends: { sx: 2763.5, sy: 347, tx: 3692.5, ty: 59 } },
  { id: "plates.plate_frames->oriented.target", ends: { sx: 2763.5, sy: 371, tx: 3692.5, ty: 83 } },
  { id: "oriented.out->bambu.meshes", ends: { sx: 3915.5, sy: 35, tx: 3980.5, ty: 35 } },
  { id: "labels.ids->bambu.ids", ends: { sx: 2427.5, sy: 347, tx: 3980.5, ty: 59 } },
  { id: "layout.bins->bambu.bins", ends: { sx: 243.5, sy: 203, tx: 3980.5, ty: 83 } },
  { id: "layout.exported->bambu.exported", ends: { sx: 243.5, sy: 227, tx: 3980.5, ty: 107 } },
  { id: "plates.plate->bambu.plates", ends: { sx: 2763.5, sy: 395, tx: 3980.5, ty: 131 } },
  { id: "plates.slot->bambu.slots", ends: { sx: 2763.5, sy: 443, tx: 3980.5, ty: 155 } },
  { id: "plates.manifest->bambu.manifest", ends: { sx: 2763.5, sy: 539, tx: 3980.5, ty: 179 } },
  { id: "labels.ghosts->dxf.ghosts", ends: { sx: 2427.5, sy: 491, tx: 2492.5, ty: 971 } },
  { id: "labels.board_strokes->dxf.strokes", ends: { sx: 2427.5, sy: 515, tx: 2492.5, ty: 995 } },
  { id: "labels.board_strokes_closed->dxf.strokes_closed", ends: { sx: 2427.5, sy: 539, tx: 2492.5, ty: 1019 } },
  { id: "pins.board_points->dxf.holes", ends: { sx: 2283.5, sy: 899, tx: 2492.5, ty: 1043 } },
  { id: "layout.coil_board_points->dxf.coil_holes", ends: { sx: 243.5, sy: 347, tx: 2492.5, ty: 1067 } },
  { id: "layout.board_min->dxf.board_min", ends: { sx: 243.5, sy: 371, tx: 2492.5, ty: 1091 } },
  { id: "layout.board_max->dxf.board_max", ends: { sx: 243.5, sy: 395, tx: 2492.5, ty: 1115 } },
  { id: "layout.coil_captured->dxf.skip_holes", ends: { sx: 243.5, sy: 251, tx: 2492.5, ty: 1139 } },
];
