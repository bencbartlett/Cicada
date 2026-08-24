/**
 * The trace router (docs/16 §Canvas conventions — the `trace` wire display
 * mode; wave 4 B2, finding U6): PCB-style wires of our own in place of
 * React Flow's smooth-step path. Pure functions, unit-tested in
 * `trace.test.ts`; the React side — the per-render lane assignment and the
 * context the edges read it from — is `traceLanes.ts`.
 *
 * The shape of a trace. A wire leaves its source handle rightwards and
 * enters its target handle from the left (the canvas's only handle
 * orientation: inputs left, outputs right). Its runs are horizontal and
 * vertical; every 90° turn is cut at 45° — two 45° bends joined by a
 * diagonal, the PCB mitre — with legs of `TRACE_CORNER_UNITS` (one grid
 * unit; the corner's "radius"). Three routes:
 *
 *   - FORWARD (the Z): a target to the right, within `TRACE_STAIRCASE_UNITS`
 *     — a stub along the source's row, the cut, a vertical channel halfway
 *     between the nodes, the cut, a stub along the target's row. A jog
 *     shorter than two legs has no room for a vertical run and is one 45°
 *     diagonal; a level target is a straight line.
 *   - STAIR: a target further right — a short escape beside each node
 *     (`TRACE_ESCAPE_UNITS` + a leg), and the long run in a horizontal
 *     channel between the two rows, so a long run is never a stub pinned
 *     to a row of nodes; when every channel between (and on) its rows is
 *     taken it detours a leg or more beyond them. A level target is a
 *     straight line unless its row is taken.
 *   - BACK: a target to the left (or too close for two legs) — out, down
 *     (or up) beside the source, a horizontal channel leftwards, up (or
 *     down) beside the target, in; the channel halfway between the rows
 *     when six legs fit there, else three legs below both. Three, not two:
 *     a back route's turns come in same-sense pairs (a U-turn) whose two
 *     cuts would meet at 90° on a two-leg run, where the Z's and the
 *     stair's pairs have opposite senses and merge into one diagonal.
 *
 * Lanes. Every run occupies its line on a lattice of `TRACE_LANE_UNITS`
 * (¼ unit) — the horizontal lattice for rows, the vertical for channels —
 * over its extent. The STUBS are pinned to their port's row (a wire must
 * reach its handle) and are recorded first; the FREE runs — the Z's
 * vertical channel, the stair's channel and escapes, the back route's
 * three — are then placed, in one deterministic order (source position,
 * target position, then the wire id), each on the lattice line nearest its
 * natural place whose occupied extents it does not touch, so no two
 * parallel runs coincide and the picture never depends on render order.
 * Two wires out of one port share their stub until they part — a trunk,
 * as on a board. Where a gap is too narrow for full legs AND lanes, the
 * legs shrink toward `TRACE_MIN_LEG_UNITS` before any two runs are let
 * coincide (Ben's finding U6 ranks them: the radius "~1 unit", the overlap
 * "never"); only a gap narrower than the lanes need even so collapses the
 * runs onto the one line that fits (move the nodes apart). The router
 * knows no obstacles: a run may cross a node that lies in its way.
 */
import type { GraphView } from "../protocol/messages";

/** The legs of a 45° corner cut, in grid units (the corner's "radius"; docs/16). */
export const TRACE_CORNER_UNITS = 1;

/** The lane pitch, in grid units: parallel free runs are at least this far apart (docs/16). */
export const TRACE_LANE_UNITS = 0.25;

/** A stair's or back route's stub before its first cut, in grid units (beside the node, never through it). */
export const TRACE_ESCAPE_UNITS = 0.5;

/** A forward target further than this, in grid units, takes the stair (its long run in a free channel). */
export const TRACE_STAIRCASE_UNITS = 6;

/** The shortest a corner's legs shrink to, in grid units, when a gap cannot hold full legs and the lanes. */
export const TRACE_MIN_LEG_UNITS = 0.5;

/**
 * How far a lane search walks from the natural line, in lattice steps, before
 * it gives up and lets the run coincide (16 units).
 */
export const TRACE_LANE_STEPS = 64;

/**
 * A stair's or back route's verticals may move at most this many steps (2
 * units) — the bound its horizontal channel is checked against, so the
 * assignment stays exact without a second pass.
 */
const SIDE_STEPS = 8;

const EPS = 1e-6;

export interface Point {
  x: number;
  y: number;
}

/** One wire's endpoints in flow px: the source handle (leaves rightwards) and the target handle (entered from the left). */
export interface TraceEnds {
  sx: number;
  sy: number;
  tx: number;
  ty: number;
}

/**
 * Where a trace's free runs go (flow px, absolute). `forward`: the x of the
 * Z's vertical channel; `stair` and `back`: the x beside the source, the y
 * of the horizontal channel, the x beside the target. `tracePath` clamps
 * them into the ranges its own endpoints allow, so a route assigned from
 * the row model draws cleanly on React Flow's measured handles.
 */
export type TraceRoute =
  | { kind: "forward"; vx: number }
  | { kind: "stair"; vx1: number; hy: number; vx2: number }
  | { kind: "back"; vx1: number; hy: number; vx2: number };

/** A wire to route: its id (the lane order's last key) and its endpoints. */
export interface TraceWire {
  id: string;
  ends: TraceEnds;
}

/**
 * Forward when the target is at least two (possibly shrunken) legs to the
 * right — or, for a jog shorter than that, at least the jog's height (the
 * 45° diagonal fits).
 */
export function isForward(ends: TraceEnds, unit: number): boolean {
  const dx = ends.tx - ends.sx;
  const dy = Math.abs(ends.ty - ends.sy);
  return dx + EPS >= Math.min(2 * TRACE_MIN_LEG_UNITS * unit, dy);
}

/** The route a wire takes with no other wire on the canvas: channels at their natural places. */
export function naturalRoute(ends: TraceEnds, unit: number): TraceRoute {
  if (isForward(ends, unit)) {
    if (ends.tx - ends.sx <= TRACE_STAIRCASE_UNITS * unit + EPS) return { kind: "forward", vx: (ends.sx + ends.tx) / 2 };
    const escape = (TRACE_ESCAPE_UNITS + TRACE_CORNER_UNITS) * unit;
    return { kind: "stair", vx1: ends.sx + escape, hy: (ends.sy + ends.ty) / 2, vx2: ends.tx - escape };
  }
  const [lo, hi] = backChannelRange(ends, unit);
  // Between the rows when six legs fit there, else three legs below both.
  const hy = hi === Infinity ? lo : (lo + hi) / 2;
  const { vx1, vx2 } = backVerticals(ends, unit);
  return { kind: "back", vx1, hy, vx2 };
}

/** The y range a back route's horizontal channel may take: `[lo, hi]`, `hi = Infinity` below both rows. */
function backChannelRange(ends: TraceEnds, unit: number): [number, number] {
  const r = TRACE_CORNER_UNITS * unit;
  const top = Math.min(ends.sy, ends.ty);
  const bottom = Math.max(ends.sy, ends.ty);
  if (bottom - top + EPS >= 6 * r) return [top + 3 * r, bottom - 3 * r];
  return [bottom + 3 * r, Infinity];
}

/**
 * A back route's verticals: `TRACE_ESCAPE_UNITS` plus a leg beside each
 * node, pushed apart to keep the channel run between them at least three
 * legs (a target only just too close for a forward route would otherwise
 * leave a run too short for its U-turn's two cuts). `vx1Min` / `vx2Max` are
 * the bounds a lane may move them to.
 */
function backVerticals(ends: TraceEnds, unit: number): { vx1: number; vx2: number; vx1Min: number; vx2Max: number } {
  const r = TRACE_CORNER_UNITS * unit;
  const escape = (TRACE_ESCAPE_UNITS + TRACE_CORNER_UNITS) * unit;
  const mid = (ends.sx + ends.tx) / 2;
  const half = Math.max((ends.sx - ends.tx) / 2 + escape, 1.5 * r);
  return {
    vx1: mid + half,
    vx2: mid - half,
    vx1Min: Math.max(ends.sx + r, mid + 1.5 * r),
    vx2Max: Math.min(ends.tx - r, mid - 1.5 * r),
  };
}

/**
 * The x range a Z's vertical channel may take: each stub at least a
 * (shrunken) leg — or half the jog, when the jog is lower than that.
 */
function forwardChannelRange(ends: TraceEnds, unit: number): [number, number] {
  const dy = Math.abs(ends.ty - ends.sy);
  const half = Math.min(TRACE_MIN_LEG_UNITS * unit, dy / 2);
  return [ends.sx + half, ends.tx - half];
}

/** The x ranges a stair's two verticals may take: a full leg off each node, never past the middle. */
function stairVerticalRanges(ends: TraceEnds, unit: number): [[number, number], [number, number]] {
  const r = TRACE_CORNER_UNITS * unit;
  const mid = (ends.sx + ends.tx) / 2;
  return [
    [ends.sx + r, mid],
    [mid, ends.tx - r],
  ];
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/**
 * The orthogonal polyline of a route — the source, every corner, the target
 * — with the route's channels clamped into the ranges these endpoints allow.
 * Zero-length and straight-through corners are left for `chamfer` to drop.
 */
export function routeCorners(ends: TraceEnds, route: TraceRoute, unit: number): Point[] {
  const s = { x: ends.sx, y: ends.sy };
  const t = { x: ends.tx, y: ends.ty };
  if (route.kind === "forward") {
    if (Math.abs(ends.ty - ends.sy) < EPS) return [s, t];
    const [lo, hi] = forwardChannelRange(ends, unit);
    const vx = clamp(route.vx, lo, hi);
    return [s, { x: vx, y: s.y }, { x: vx, y: t.y }, t];
  }
  if (route.kind === "stair") {
    const [range1, range2] = stairVerticalRanges(ends, unit);
    const vx1 = clamp(route.vx1, range1[0], range1[1]);
    const vx2 = clamp(route.vx2, range2[0], range2[1]);
    const hy = stairChannelClamp(ends, route.hy, unit);
    return [s, { x: vx1, y: s.y }, { x: vx1, y: hy }, { x: vx2, y: hy }, { x: vx2, y: t.y }, t];
  }
  const bounds = backVerticals(ends, unit);
  const vx1 = Math.max(route.vx1, bounds.vx1Min);
  const vx2 = Math.min(route.vx2, bounds.vx2Max);
  const [lo, hi] = backChannelRange(ends, unit);
  const hy = clamp(route.hy, lo, hi);
  return [s, { x: vx1, y: s.y }, { x: vx1, y: hy }, { x: vx2, y: hy }, { x: vx2, y: t.y }, t];
}

/**
 * Whether a stair's channel at `hy` is well-formed for these endpoints:
 * each vertical is nothing or at least a leg (a shorter one would be a
 * kink, not a corner). A channel outside the two rows is a detour — a
 * leg or more beyond them; its long run then lies between two same-sense
 * turns, which a stair's run (over three legs, by its threshold) affords.
 */
function stairChannelOk(ends: TraceEnds, hy: number, unit: number): boolean {
  const r = TRACE_CORNER_UNITS * unit;
  const leg = (d: number) => d < EPS || d + EPS >= r;
  return leg(Math.abs(hy - ends.sy)) && leg(Math.abs(hy - ends.ty));
}

/** The nearest well-formed channel to `hy` for a stair: its own row(s), or a leg away from them. */
function stairChannelClamp(ends: TraceEnds, hy: number, unit: number): number {
  if (stairChannelOk(ends, hy, unit)) return hy;
  const r = TRACE_CORNER_UNITS * unit;
  const top = Math.min(ends.sy, ends.ty);
  const bottom = Math.max(ends.sy, ends.ty);
  const candidates = [top, bottom, top + r, bottom - r, top - r, bottom + r].filter((y) => stairChannelOk(ends, y, unit));
  return candidates.reduce((best, y) => (Math.abs(y - hy) < Math.abs(best - hy) ? y : best), candidates[0]!);
}

function same(a: Point, b: Point): boolean {
  return Math.abs(a.x - b.x) < EPS && Math.abs(a.y - b.y) < EPS;
}

/** Drop repeated points and straight-through (collinear, same-direction) corners. */
function simplify(points: readonly Point[]): Point[] {
  const out: Point[] = [];
  for (const p of points) {
    const b = out[out.length - 1];
    if (b !== undefined && same(b, p)) continue;
    const a = out[out.length - 2];
    if (a !== undefined && b !== undefined) {
      const cross = (b.x - a.x) * (p.y - b.y) - (b.y - a.y) * (p.x - b.x);
      const dot = (b.x - a.x) * (p.x - b.x) + (b.y - a.y) * (p.y - b.y);
      if (Math.abs(cross) < EPS && dot > 0) {
        out[out.length - 1] = p;
        continue;
      }
    }
    out.push(p);
  }
  return out;
}

/**
 * Cut every corner of an orthogonal polyline at 45°: legs of `r`, shortened
 * only where a run is too short for two full legs (half the run per corner;
 * a stub may be consumed whole). A run shorter than two legs between
 * opposite-sense turns leaves its two cuts meeting — one 45° diagonal.
 */
export function chamfer(polyline: readonly Point[], r: number): Point[] {
  const corners = simplify(polyline);
  const n = corners.length - 1;
  if (n < 1) return corners;
  const out: Point[] = [corners[0]!];
  const push = (p: Point) => {
    const last = out[out.length - 1]!;
    if (!same(last, p)) out.push(p);
  };
  for (let i = 1; i < n; i += 1) {
    const prev = corners[i - 1]!;
    const here = corners[i]!;
    const next = corners[i + 1]!;
    const lenIn = Math.hypot(here.x - prev.x, here.y - prev.y);
    const lenOut = Math.hypot(next.x - here.x, next.y - here.y);
    const dIn = { x: (here.x - prev.x) / lenIn, y: (here.y - prev.y) / lenIn };
    const dOut = { x: (next.x - here.x) / lenOut, y: (next.y - here.y) / lenOut };
    const shareIn = i === 1 ? lenIn : lenIn / 2;
    const shareOut = i === n - 1 ? lenOut : lenOut / 2;
    const c = Math.min(r, shareIn, shareOut);
    push({ x: here.x - dIn.x * c, y: here.y - dIn.y * c });
    push({ x: here.x + dOut.x * c, y: here.y + dOut.y * c });
  }
  push(corners[n]!);
  return simplify(out);
}

/** A polyline as an SVG path (`M … L …`), coordinates rounded to 1/100 px. */
export function svgPath(points: readonly Point[]): string {
  const f = (v: number) => String(Math.round(v * 100) / 100);
  return points.map((p, i) => `${i === 0 ? "M" : "L"}${f(p.x)} ${f(p.y)}`).join(" ");
}

/**
 * The drawn trace of a wire: its SVG path and the label anchor (the middle
 * of its middle run — where the `map` chip sits). `route` is the lane
 * assignment's channels for this wire (`assignTraceLanes`); without one the
 * natural route is drawn (the in-flight connection line; spline mode never
 * calls this). The endpoints decide the route's kind — a route assigned
 * from the row model that disagrees in a tight spot yields to them and
 * draws its natural channels.
 */
export function tracePath(ends: TraceEnds, unit: number, route?: TraceRoute): [string, number, number] {
  const natural = naturalRoute(ends, unit);
  const use = route !== undefined && route.kind === natural.kind ? route : natural;
  const corners = routeCorners(ends, use, unit);
  const points = chamfer(corners, TRACE_CORNER_UNITS * unit);
  const runs = simplify(corners);
  const mid = Math.floor((runs.length - 1) / 2);
  const a = runs[mid]!;
  const b = runs[Math.min(mid + 1, runs.length - 1)]!;
  return [svgPath(points), (a.x + b.x) / 2, (a.y + b.y) / 2];
}

/** An occupied extent on a lattice line and the wire it belongs to (a wire never blocks itself). */
interface Extent {
  lo: number;
  hi: number;
  owner: string;
}

/** Occupied extents per lattice line (key = the line's lattice index). */
type Occupancy = Map<number, Extent[]>;

function isFree(occupancy: Occupancy, index: number, lo: number, hi: number, owner: string): boolean {
  const taken = occupancy.get(index);
  if (taken === undefined) return true;
  // Closed intervals: runs that merely touch would meet at their cuts.
  return taken.every((e) => e.owner === owner || hi < e.lo - EPS || lo > e.hi + EPS);
}

function occupy(occupancy: Occupancy, index: number, lo: number, hi: number, owner: string): void {
  const taken = occupancy.get(index);
  const extent = { lo: Math.min(lo, hi), hi: Math.max(lo, hi), owner };
  if (taken === undefined) occupancy.set(index, [extent]);
  else taken.push(extent);
}

interface LaneSearch {
  occupancy: Occupancy;
  /** The run's natural line (flow px). */
  natural: number;
  /** The lines the run may take, `[min, max]` (either may be infinite). */
  range: [number, number];
  /** The run's extent along its line. */
  extent: [number, number];
  /** The owning wire. */
  owner: string;
  pitch: number;
  /** Where the lattice's lines lie: `origin + k × pitch`. */
  origin: number;
  /** How far from the natural line to look, in lattice steps. */
  steps: number;
  /** A further test a line must pass (a stair's channel shape). */
  accept?: (line: number) => boolean;
}

/**
 * The lattice line for a free run: the nearest to `natural` inside `range`
 * (stepping outwards, the far side first) that passes `accept` and whose
 * occupied extents the run does not touch, within `steps` of the natural
 * line. None free: the nearest admissible line even so — the runs coincide,
 * the one case the docs allow; no admissible line at all: the natural place
 * clamped into the range. Records the run on the line it takes.
 */
function takeLane(search: LaneSearch): number {
  const { occupancy, natural, pitch, origin, steps, owner } = search;
  const [min, max] = search.range;
  const [lo, hi] = search.extent;
  const accept = search.accept ?? (() => true);
  const q = Math.round((natural - origin) / pitch);
  let nearestTaken: number | null = null;
  for (let step = 0; step <= steps; step += 1) {
    for (const k of step === 0 ? [q] : [q + step, q - step]) {
      const line = origin + k * pitch;
      if (line < min - EPS || line > max + EPS || !accept(line)) continue;
      if (isFree(occupancy, k, lo, hi, owner)) {
        occupy(occupancy, k, lo, hi, owner);
        return line;
      }
      if (nearestTaken === null) nearestTaken = k;
    }
  }
  const k = nearestTaken ?? Math.round((clamp(natural, min, max) - origin) / pitch);
  occupy(occupancy, k, lo, hi, owner);
  return nearestTaken === null ? clamp(natural, min, max) : origin + k * pitch;
}

/** The lane order: source position, target position, then the wire id — the same for any input order. */
export function compareTraceWires(a: TraceWire, b: TraceWire): number {
  return (
    a.ends.sy - b.ends.sy ||
    a.ends.sx - b.ends.sx ||
    a.ends.ty - b.ends.ty ||
    a.ends.tx - b.ends.tx ||
    (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  );
}

/**
 * Lanes for every wire at once. First every wire's pinned stubs are
 * recorded on the rows they lie on; then, in `compareTraceWires` order,
 * each free run takes the lattice line nearest its natural place that no
 * other wire's run occupies over its extent. The horizontal lattice runs
 * THROUGH the port rows — `rowOrigin` is their residue modulo the pitch
 * (`rowLatticeOrigin`), so a row itself, and a leg away from it, are
 * lattice lines; the vertical lattice is the canvas grid's (the Z's
 * midpoints lie on it). Pure in its inputs — the same wires in any order
 * give the same routes — so a re-render never moves a trace.
 */
export function assignTraceLanes(wires: readonly TraceWire[], unit: number, rowOrigin = 0): Map<string, TraceRoute> {
  const pitch = TRACE_LANE_UNITS * unit;
  const rows: Occupancy = new Map();
  const columns: Occupancy = new Map();
  const row = (y: number) => Math.round((y - rowOrigin) / pitch);
  const ordered = [...wires].sort(compareTraceWires).map((wire) => ({ wire, natural: naturalRoute(wire.ends, unit) }));

  // Pass 1 — the pinned stubs (natural extents; a laned channel moves a
  // Z's cut by at most the lanes' room).
  for (const { wire, natural } of ordered) {
    const { sx, sy, tx, ty } = wire.ends;
    if (natural.kind === "forward") {
      if (Math.abs(ty - sy) < EPS) occupy(rows, row(sy), sx, tx, wire.id);
      else {
        occupy(rows, row(sy), sx, natural.vx, wire.id);
        occupy(rows, row(ty), natural.vx, tx, wire.id);
      }
    } else {
      occupy(rows, row(sy), sx, natural.vx1, wire.id);
      occupy(rows, row(ty), natural.vx2, tx, wire.id);
    }
  }

  // Pass 2 — the free runs.
  const routes = new Map<string, TraceRoute>();
  const margin = SIDE_STEPS * pitch;
  for (const { wire, natural } of ordered) {
    const { ends } = wire;
    const { sx, sy, tx, ty } = ends;
    const owner = wire.id;
    const span: [number, number] = [Math.min(sy, ty), Math.max(sy, ty)];
    if (natural.kind === "forward") {
      // A jog or a level line has no vertical run to lane.
      if (Math.abs(ty - sy) < 2 * TRACE_MIN_LEG_UNITS * unit - EPS) {
        routes.set(owner, natural);
        continue;
      }
      const vx = takeLane({
        occupancy: columns,
        natural: natural.vx,
        range: forwardChannelRange(ends, unit),
        extent: span,
        owner,
        pitch,
        origin: 0,
        steps: TRACE_LANE_STEPS,
      });
      routes.set(owner, { kind: "forward", vx });
      continue;
    }
    if (natural.kind === "stair") {
      // The channel first — between the rows, on one of them, or a detour a
      // leg or more beyond them, nearest to the midpoint first — its extent
      // widened by the most the two verticals may still move (and the
      // whole line when it runs on a row).
      const hy = takeLane({
        occupancy: rows,
        natural: natural.hy,
        range: [-Infinity, Infinity],
        extent: [Math.max(sx, natural.vx1 - margin), Math.min(tx, natural.vx2 + margin)],
        owner,
        pitch,
        origin: rowOrigin,
        steps: TRACE_LANE_STEPS,
        accept: (line) => stairChannelOk(ends, line, unit),
      });
      const [range1, range2] = stairVerticalRanges(ends, unit);
      const vx1 =
        Math.abs(hy - sy) < EPS
          ? natural.vx1
          : takeLane({
              occupancy: columns,
              natural: natural.vx1,
              range: range1,
              extent: [Math.min(sy, hy), Math.max(sy, hy)],
              owner,
              pitch,
              origin: 0,
              steps: SIDE_STEPS,
            });
      const vx2 =
        Math.abs(hy - ty) < EPS
          ? natural.vx2
          : takeLane({
              occupancy: columns,
              natural: natural.vx2,
              range: range2,
              extent: [Math.min(hy, ty), Math.max(hy, ty)],
              owner,
              pitch,
              origin: 0,
              steps: SIDE_STEPS,
            });
      // The channel's true extent, now the verticals are placed.
      occupy(rows, row(hy), Math.abs(hy - sy) < EPS ? sx : vx1, Math.abs(hy - ty) < EPS ? tx : vx2, owner);
      routes.set(owner, { kind: "stair", vx1, hy, vx2 });
      continue;
    }
    // The back route: the horizontal channel first, widened likewise; then
    // the verticals, exactly, on the final channel.
    const hy = takeLane({
      occupancy: rows,
      natural: natural.hy,
      range: backChannelRange(ends, unit),
      extent: [natural.vx2 - margin, natural.vx1 + margin],
      owner,
      pitch,
      origin: rowOrigin,
      steps: TRACE_LANE_STEPS,
    });
    const bounds = backVerticals(ends, unit);
    const vx1 = takeLane({
      occupancy: columns,
      natural: natural.vx1,
      range: [bounds.vx1Min, Infinity],
      extent: [Math.min(sy, hy), Math.max(sy, hy)],
      owner,
      pitch,
      origin: 0,
      steps: SIDE_STEPS,
    });
    const vx2 = takeLane({
      occupancy: columns,
      natural: natural.vx2,
      range: [-Infinity, bounds.vx2Max],
      extent: [Math.min(hy, ty), Math.max(hy, ty)],
      owner,
      pitch,
      origin: 0,
      steps: SIDE_STEPS,
    });
    routes.set(owner, { kind: "back", vx1, hy, vx2 });
  }
  return routes;
}

/**
 * Where the port rows fall on the horizontal lattice: their residue modulo
 * the pitch — the row model's row centre (`1.5 × unit` for the first row)
 * plus the measured row shift. Node positions are grid cells, so every row
 * of every node shares it.
 */
export function rowLatticeOrigin(unit: number, handles: HandleGeometry): number {
  const pitch = TRACE_LANE_UNITS * unit;
  const residue = (1.5 * unit + handles.rowShift) % pitch;
  return residue < 0 ? residue + pitch : residue;
}

/** Two routes with the same kind and channels (the assignment did not move this wire). */
export function sameRoute(a: TraceRoute | undefined, b: TraceRoute | undefined): boolean {
  if (a === undefined || b === undefined) return a === b;
  if (a.kind !== b.kind) return false;
  if (a.kind === "forward" && b.kind === "forward") return a.vx === b.vx;
  if (a.kind !== "forward" && b.kind !== "forward") return a.vx1 === b.vx1 && a.hy === b.hy && a.vx2 === b.vx2;
  return false;
}

/**
 * Where React Flow puts a handle relative to the row model: `overhang` is
 * how far past the node's edge the handle's outer edge sits (the edge's
 * endpoint — a few px), `rowShift` how far the handle's centre sits from
 * the row model's row centre (a px or so, from the node's border and the
 * handle's own box). Measured from React Flow's handle bounds once the
 * nodes are mounted (`traceLanes.ts`); zero before.
 */
export interface HandleGeometry {
  overhang: number;
  rowShift: number;
}

export const UNMEASURED_HANDLES: HandleGeometry = { overhang: 0, rowShift: 0 };

/**
 * The row model of every wire's endpoints (docs/16 §Canvas conventions: a
 * header row, then one port row per unit — inputs on the left edge,
 * outputs on the right — handles centred on their rows), corrected by the
 * measured handle geometry so they are the endpoints React Flow draws the
 * edge from: the geometry the lane assignment reads, from the graph and
 * the canvas's live node positions (px; a dragged node's optimistic
 * position included). A wire whose node is not on the canvas, or whose
 * port the node does not show, has no edge to route and is left out.
 */
export function wireEnds(
  graph: GraphView,
  positions: ReadonlyMap<string, Point>,
  unit: number,
  handles: HandleGeometry = UNMEASURED_HANDLES,
): TraceWire[] {
  const nodes = new Map(graph.nodes.map((node) => [node.name, node]));
  const wires: TraceWire[] = [];
  for (const wire of graph.wires) {
    const from = nodes.get(wire.from.node);
    const to = nodes.get(wire.to.node);
    const at = positions.get(wire.from.node);
    const into = positions.get(wire.to.node);
    if (from === undefined || to === undefined || at === undefined || into === undefined) continue;
    const j = from.outputs.findIndex((o) => o.name === wire.from.port);
    const i = to.inputs.findIndex((o) => o.name === wire.to.port);
    if (i < 0 || j < 0) continue;
    wires.push({
      id: wire.id,
      ends: {
        sx: at.x + from.size[0] * unit + handles.overhang,
        sy: at.y + unit * (1.5 + j) + handles.rowShift,
        tx: into.x - handles.overhang,
        ty: into.y + unit * (1.5 + i) + handles.rowShift,
      },
    });
  }
  return wires;
}
