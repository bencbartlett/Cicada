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
 * reach its handle) and are recorded first, at their natural length; then
 * the horizontal channels — the stair's and the back route's — are placed
 * in the wire order (source position, target position, then the wire id),
 * each on the row line nearest its natural place that no other wire's run
 * occupies; then the VERTICAL runs — the Z's channel, the stair's and the
 * back route's two — are solved a column at a time (the runs that can
 * meet) by a depth-first search in top-down order over each run's lines,
 * nearest its natural place first, under two constraints: no two runs
 * that overlap on one line, and no stub — at the length its line gives
 * it, as drawn — running into another wire's run on its row (a channel, a
 * level line, the stub of a port that shares the row). The natural-first
 * greedy is the search's first branch; it backtracks only where that
 * fails, so the picture is the natural one wherever there is room, and
 * the wall's busiest three-unit gap (22 verticals, 8 deep, on seven lines
 * a Z may take) comes out with nothing coinciding. Every order is a pure
 * function of the inputs, so the picture never depends on render order.
 * Two wires out of one port share their stub until they part — a trunk,
 * as on a board. Where a gap is too narrow for full legs AND lanes, the
 * legs shrink toward `TRACE_MIN_LEG_UNITS` before any two runs are let
 * coincide (Ben's finding U6 ranks them: the radius "~1 unit", the overlap
 * "never"). Only a column with no overlap-free assignment at all falls
 * back to the greedy — the nearest free line, else the line with the
 * fewest runs over the extent, so coincidences spread rather than stack —
 * and the router measures its own drawing: every wire whose run coincides
 * with another's is reported in `TraceLanes.collapsed` (the canvas shows
 * the count as `data-trace-collapsed`; never silent). The router knows no
 * obstacles: a run may cross a node that lies in its way.
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
 * How far from the natural line a Z's channel, or a horizontal channel, may
 * be laned, in lattice steps (16 units).
 */
export const TRACE_LANE_STEPS = 64;

/**
 * A stair's or back route's verticals may move at most this many steps (2
 * units) — the bound its horizontal channel's extent is widened by when
 * the channel is placed, before the verticals are.
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
 * The drawn trace of a wire: its SVG path, the label anchor (the middle of
 * its middle run — where the `map` chip sits), and whether the assigned
 * route was set aside. `route` is the lane assignment's channels for this
 * wire (`assignTraceLanes`); without one the natural route is drawn (the
 * in-flight connection line; spline mode never calls this). The endpoints
 * decide the route's kind — a route assigned from the row model that
 * disagrees in a tight spot yields to them and draws its natural channels,
 * and the last element says so (the edge marks itself `data-trace-yield`,
 * so the fallback is never silent).
 */
export function tracePath(ends: TraceEnds, unit: number, route?: TraceRoute): [string, number, number, boolean] {
  const natural = naturalRoute(ends, unit);
  const yielded = route !== undefined && route.kind !== natural.kind;
  const use = route !== undefined && !yielded ? route : natural;
  const corners = routeCorners(ends, use, unit);
  const points = chamfer(corners, TRACE_CORNER_UNITS * unit);
  const runs = simplify(corners);
  const mid = Math.floor((runs.length - 1) / 2);
  const a = runs[mid]!;
  const b = runs[Math.min(mid + 1, runs.length - 1)]!;
  return [svgPath(points), (a.x + b.x) / 2, (a.y + b.y) / 2, yielded];
}

/** An occupied extent on a lattice line and the wire it belongs to (a wire never blocks itself). */
interface Extent {
  lo: number;
  hi: number;
  owner: string;
  /** A stub whose inner end a vertical run still decides: not a fixed obstacle, a constraint between runs. */
  floating?: boolean;
}

/** Occupied extents per lattice line (key = the line's lattice index). */
type Occupancy = Map<number, Extent[]>;

/** Whether two closed intervals meet — runs that merely touch would meet at their cuts. */
function meets(aLo: number, aHi: number, bLo: number, bHi: number): boolean {
  return !(aHi < bLo - EPS || aLo > bHi + EPS);
}

/** The fixed extents of OTHER wires a run over `[lo, hi]` would meet on a line. */
function blockers(occupancy: Occupancy, index: number, lo: number, hi: number, owner: string): Extent[] {
  const taken = occupancy.get(index);
  if (taken === undefined) return [];
  return taken.filter((e) => e.owner !== owner && e.floating !== true && meets(lo, hi, e.lo, e.hi));
}

function occupy(occupancy: Occupancy, index: number, lo: number, hi: number, owner: string): Extent {
  const taken = occupancy.get(index);
  const extent: Extent = { lo: Math.min(lo, hi), hi: Math.max(lo, hi), owner };
  if (taken === undefined) occupancy.set(index, [extent]);
  else taken.push(extent);
  return extent;
}

/** Where a run may go: its natural line, the admissible range, and how far to look. */
interface LaneSearch {
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

interface Candidate {
  k: number;
  line: number;
}

/** The admissible lattice lines within `steps` of the natural line, nearest first (the far side first at each step). */
function admissibleLines(search: LaneSearch): Candidate[] {
  const { natural, pitch, origin, steps } = search;
  const [min, max] = search.range;
  const accept = search.accept ?? (() => true);
  const q = Math.round((natural - origin) / pitch);
  const out: Candidate[] = [];
  for (let step = 0; step <= steps; step += 1) {
    for (const k of step === 0 ? [q] : [q + step, q - step]) {
      const line = origin + k * pitch;
      if (line < min - EPS || line > max + EPS || !accept(line)) continue;
      out.push({ k, line });
    }
  }
  return out;
}

/**
 * The row line for a horizontal channel: the nearest to `natural` inside
 * `range` (stepping outwards, the far side first) that passes `accept` and
 * whose occupied extents the run does not touch, within `steps` of the
 * natural line. None free: the admissible line with the FEWEST runs over
 * this extent (the nearest such), so coincidences spread instead of
 * stacking, and `collapsed` says so. No admissible line at all: the natural
 * place clamped into the range, `collapsed` if that line already carries a
 * run over the extent. Records the run on the line it takes.
 */
function takeLane(occupancy: Occupancy, search: LaneSearch): { line: number; collapsed: boolean } {
  const [lo, hi] = search.extent;
  const candidates = admissibleLines(search);
  let lightest: { k: number; line: number; runs: number } | null = null;
  for (const c of candidates) {
    const runs = blockers(occupancy, c.k, lo, hi, search.owner).length;
    if (runs === 0) {
      occupy(occupancy, c.k, lo, hi, search.owner);
      return { line: c.line, collapsed: false };
    }
    if (lightest === null || runs < lightest.runs) lightest = { ...c, runs };
  }
  if (lightest !== null) {
    occupy(occupancy, lightest.k, lo, hi, search.owner);
    return { line: lightest.line, collapsed: true };
  }
  const line = clamp(search.natural, search.range[0], search.range[1]);
  const k = Math.round((line - search.origin) / search.pitch);
  const collapsed = blockers(occupancy, k, lo, hi, search.owner).length > 0;
  occupy(occupancy, k, lo, hi, search.owner);
  return { line, collapsed };
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

/** The lane assignment: every wire's route, and the wires a run of which had to share a line. */
export interface TraceLanes {
  routes: Map<string, TraceRoute>;
  /**
   * The ids of the wires a run of which coincides with another wire's run
   * — on its column line, or along the stub its line decides — because no
   * assignment of its column kept every run apart within the search's
   * budget (in lane order, each wire once). Empty on every committed
   * example.
   */
  collapsed: string[];
}

/**
 * How many candidate placements the search over one column's runs may try
 * before it gives the column up to the greedy fallback (and reports what
 * coincides). The wall's busiest column — 22 runs — settles in under a
 * hundred; the budget is for a column no assignment can satisfy.
 */
const LANE_SEARCH_BUDGET = 4000;

/** A wire being laned: its natural route and the channels placed so far. */
interface Plan {
  wire: TraceWire;
  /** The wire's place in `compareTraceWires` order — the last key of every other order. */
  order: number;
  natural: TraceRoute;
  /** The horizontal channel (stair, back), once placed. */
  hy: number;
  vx: number;
  vx1: number;
  vx2: number;
  /** The wire's pinned stubs as recorded on their rows, at their natural length (`null` for a level line — one run, never moved). */
  sourceStub: Extent | null;
  targetStub: Extent | null;
}

/** A pinned stub whose inner end a vertical run decides. */
interface Stub {
  /** The row's lattice index. */
  row: number;
  /** The handle's x — the stub's fixed end. */
  handle: number;
  /** Out of a source port (`[handle, x]`; wires out of ONE port share it — the trunk) or into a target port (`[x, handle]`). */
  side: "source" | "target";
}

/** One vertical run waiting for its line. */
interface VerticalRun {
  plan: Plan;
  which: "vx" | "vx1" | "vx2";
  natural: number;
  range: [number, number];
  extent: [number, number];
  /** The lines the run may take, nearest its natural place first. */
  domain: Candidate[];
  /** The stubs this run's line decides: a Z's two, a stair's or back route's one. */
  stubs: Stub[];
}

const ORDER_OF_WHICH = { vx: 0, vx1: 0, vx2: 1 };

/**
 * The stub's DRAWN extent once its run takes `x`: `chamfer` cuts the corner
 * at the stub's inner end by the shorter of a leg, the stub, and half the
 * vertical beside it (`vertical` = the run's length), so two stubs facing
 * each other on one row may come a leg or two closer than their corners
 * before their drawn runs meet.
 */
function drawnStub(stub: Stub, x: number, vertical: number, unit: number): [number, number] {
  const cut = Math.min(TRACE_CORNER_UNITS * unit, Math.abs(x - stub.handle), vertical / 2);
  const inner = x - Math.sign(x - stub.handle) * cut;
  return [Math.min(stub.handle, inner), Math.max(stub.handle, inner)];
}

/** Whether two intervals overlap over a positive length (runs that merely touch end to end do not coincide). */
function overlap(aLo: number, aHi: number, bLo: number, bHi: number): boolean {
  return Math.min(aHi, bHi) - Math.max(aLo, bLo) > EPS;
}

/**
 * Lanes for every wire at once. First every wire's pinned stubs are
 * recorded on the rows they lie on, at their natural length; then, in
 * `compareTraceWires` order, the stairs' and back routes' horizontal
 * channels take the row lines nearest their natural places that no other
 * wire's run occupies over their extents; then the vertical runs. Those
 * are solved a COLUMN at a time — a column being the runs that can meet
 * (overlapping extents, or stubs on a shared row that can run into each
 * other) — by a depth-first search in top-down order over each run's
 * lines, nearest its natural place first, under two constraints: no two
 * runs that overlap share a line, and no stub a line decides runs into
 * another wire's run on its row (a channel, a level line, or the stub of
 * a port sharing the row — the trunk out of one port excepted). The
 * greedy, natural-first placement is the search's first branch; it
 * backtracks only where that fails, so the picture is the natural one
 * wherever there is room. A column with no overlap-free assignment within
 * `LANE_SEARCH_BUDGET` falls back to the greedy placement — the nearest
 * free line, else the line with the fewest runs over the extent — and
 * every coincidence is reported in `collapsed`. The horizontal lattice
 * runs THROUGH the port rows — `rowOrigin` is their residue modulo the
 * pitch (`rowLatticeOrigin`), so a row itself, and a leg away from it, are
 * lattice lines; the vertical lattice is the canvas grid's (the Z's
 * midpoints lie on it). Pure in its inputs — the same wires in any order
 * give the same routes — so a re-render never moves a trace.
 */
export function assignTraceLanes(wires: readonly TraceWire[], unit: number, rowOrigin = 0): TraceLanes {
  const pitch = TRACE_LANE_UNITS * unit;
  const rows: Occupancy = new Map();
  const row = (y: number) => Math.round((y - rowOrigin) / pitch);
  const margin = SIDE_STEPS * pitch;
  const plans: Plan[] = [...wires].sort(compareTraceWires).map((wire, order) => {
    const natural = naturalRoute(wire.ends, unit);
    const stubs = { sourceStub: null, targetStub: null };
    return natural.kind === "forward"
      ? { wire, order, natural, hy: NaN, vx: natural.vx, vx1: NaN, vx2: NaN, ...stubs }
      : { wire, order, natural, hy: natural.hy, vx: NaN, vx1: natural.vx1, vx2: natural.vx2, ...stubs };
  });
  const planOf = new Map(plans.map((plan) => [plan.wire.id, plan]));

  // Pass 1 — the pinned stubs, at their natural length.
  for (const plan of plans) {
    const { wire, natural } = plan;
    const { sx, sy, tx, ty } = wire.ends;
    if (natural.kind === "forward") {
      if (Math.abs(ty - sy) < EPS) {
        occupy(rows, row(sy), sx, tx, wire.id);
        continue;
      }
      plan.sourceStub = occupy(rows, row(sy), sx, natural.vx, wire.id);
      plan.targetStub = occupy(rows, row(ty), natural.vx, tx, wire.id);
    } else {
      plan.sourceStub = occupy(rows, row(sy), sx, natural.vx1, wire.id);
      plan.targetStub = occupy(rows, row(ty), natural.vx2, tx, wire.id);
    }
  }

  // Pass 2 — the horizontal channels, in wire order.
  for (const plan of plans) {
    const { wire, natural } = plan;
    const { ends } = wire;
    const { sx, tx } = ends;
    if (natural.kind === "forward") continue;
    if (natural.kind === "stair") {
      // Between the rows, on one of them, or a detour a leg or more beyond
      // them, nearest to the midpoint first — its extent widened by the
      // most the two verticals may still move (and the whole line when it
      // runs on a row).
      const took = takeLane(rows, {
        natural: natural.hy,
        range: [-Infinity, Infinity],
        extent: [Math.max(sx, natural.vx1 - margin), Math.min(tx, natural.vx2 + margin)],
        owner: wire.id,
        pitch,
        origin: rowOrigin,
        steps: TRACE_LANE_STEPS,
        accept: (line) => stairChannelOk(ends, line, unit),
      });
      plan.hy = took.line;
      continue;
    }
    const took = takeLane(rows, {
      natural: natural.hy,
      range: backChannelRange(ends, unit),
      extent: [natural.vx2 - margin, natural.vx1 + margin],
      owner: wire.id,
      pitch,
      origin: rowOrigin,
      steps: TRACE_LANE_STEPS,
    });
    plan.hy = took.line;
  }

  // Pass 3 — the vertical runs: gather them, with their lines and the
  // stubs they decide.
  const between = (a: number, b: number): [number, number] => [Math.min(a, b), Math.max(a, b)];
  const runs: VerticalRun[] = [];
  const gather = (plan: Plan, which: VerticalRun["which"], natural: number, range: [number, number], extent: [number, number], steps: number) => {
    const { sx, sy, tx, ty } = plan.wire.ends;
    const domain = admissibleLines({ natural, range, extent, owner: plan.wire.id, pitch, origin: 0, steps });
    const stubs: Stub[] = [];
    if (which !== "vx2" && plan.sourceStub !== null) {
      plan.sourceStub.floating = true;
      stubs.push({ row: row(sy), handle: sx, side: "source" });
    }
    if (which !== "vx1" && plan.targetStub !== null) {
      plan.targetStub.floating = true;
      stubs.push({ row: row(ty), handle: tx, side: "target" });
    }
    runs.push({ plan, which, natural, range, extent, domain, stubs });
  };
  for (const plan of plans) {
    const { wire, natural, hy } = plan;
    const { ends } = wire;
    const { sy, ty } = ends;
    if (natural.kind === "forward") {
      // A jog or a level line has no vertical run to lane.
      if (Math.abs(ty - sy) < 2 * TRACE_MIN_LEG_UNITS * unit - EPS) continue;
      gather(plan, "vx", natural.vx, forwardChannelRange(ends, unit), between(sy, ty), TRACE_LANE_STEPS);
      continue;
    }
    if (natural.kind === "stair") {
      // A channel on a row leaves that side without a vertical.
      const [range1, range2] = stairVerticalRanges(ends, unit);
      if (Math.abs(hy - sy) >= EPS) gather(plan, "vx1", natural.vx1, range1, between(sy, hy), SIDE_STEPS);
      if (Math.abs(hy - ty) >= EPS) gather(plan, "vx2", natural.vx2, range2, between(hy, ty), SIDE_STEPS);
      continue;
    }
    const bounds = backVerticals(ends, unit);
    gather(plan, "vx1", natural.vx1, [bounds.vx1Min, Infinity], between(sy, hy), SIDE_STEPS);
    gather(plan, "vx2", natural.vx2, [-Infinity, bounds.vx2Max], between(hy, ty), SIDE_STEPS);
  }
  runs.sort(
    (a, b) =>
      a.extent[0] - b.extent[0] ||
      a.extent[1] - b.extent[1] ||
      a.plan.order - b.plan.order ||
      ORDER_OF_WHICH[a.which] - ORDER_OF_WHICH[b.which],
  );

  // The trunk: wires out of one port share their source stub.
  const trunkMates = (a: Plan, b: Plan): boolean =>
    Math.abs(a.wire.ends.sx - b.wire.ends.sx) < EPS && Math.abs(a.wire.ends.sy - b.wire.ends.sy) < EPS;
  const vertical = (run: VerticalRun) => run.extent[1] - run.extent[0];
  // Whether a stub at `x`, as drawn, runs into a FIXED run on its row (a
  // channel, a level line, a stub no run decides).
  const stubBlocked = (run: VerticalRun, stub: Stub, x: number): boolean => {
    const [lo, hi] = drawnStub(stub, x, vertical(run), unit);
    return blockers(rows, stub.row, lo, hi, run.plan.wire.id).some((e) => {
      const other = planOf.get(e.owner);
      return overlap(lo, hi, e.lo, e.hi) && !(stub.side === "source" && other !== undefined && trunkMates(run.plan, other));
    });
  };
  // Whether two runs at the given lines meet: the same column line over
  // extents that touch, or stubs on a shared row whose drawn runs overlap.
  const conflict = (a: VerticalRun, ca: Candidate, b: VerticalRun, cb: Candidate): boolean => {
    if (ca.k === cb.k && meets(a.extent[0], a.extent[1], b.extent[0], b.extent[1])) return true;
    for (const sa of a.stubs) {
      for (const sb of b.stubs) {
        if (sa.row !== sb.row) continue;
        if (sa.side === "source" && sb.side === "source" && trunkMates(a.plan, b.plan)) continue;
        const [alo, ahi] = drawnStub(sa, ca.line, vertical(a), unit);
        const [blo, bhi] = drawnStub(sb, cb.line, vertical(b), unit);
        if (overlap(alo, ahi, blo, bhi)) return true;
      }
    }
    return false;
  };
  // Each run's lines, those that keep its stubs clear of the fixed runs
  // first; a run none of whose lines does keeps them all (it coincides
  // wherever it goes — the measure below will say so).
  for (const run of runs) {
    const clear = run.domain.filter((c) => run.stubs.every((stub) => !stubBlocked(run, stub, c.line)));
    if (clear.length > 0) run.domain = clear;
  }

  // Columns: the runs that can meet — on a line both may take over
  // overlapping extents, or by stubs on a shared row, by the widest they
  // can reach.
  const parent = runs.map((_, i) => i);
  const find = (i: number): number => (parent[i] === i ? i : (parent[i] = find(parent[i]!)));
  const hull = (run: VerticalRun, stub: Stub): [number, number] => {
    const lines = run.domain.map((c) => c.line);
    return [Math.min(stub.handle, ...lines), Math.max(stub.handle, ...lines)];
  };
  const lineSets = runs.map((run) => new Set(run.domain.map((c) => c.k)));
  for (let i = 0; i < runs.length; i += 1) {
    for (let j = i + 1; j < runs.length; j += 1) {
      const a = runs[i]!;
      const b = runs[j]!;
      const shareLine = [...lineSets[i]!].some((k) => lineSets[j]!.has(k));
      let linked = shareLine && meets(a.extent[0], a.extent[1], b.extent[0], b.extent[1]);
      for (const sa of a.stubs) {
        if (linked) break;
        for (const sb of b.stubs) {
          if (sa.row !== sb.row) continue;
          if (sa.side === "source" && sb.side === "source" && trunkMates(a.plan, b.plan)) continue;
          const [alo, ahi] = hull(a, sa);
          const [blo, bhi] = hull(b, sb);
          if (meets(alo, ahi, blo, bhi)) {
            linked = true;
            break;
          }
        }
      }
      if (linked) parent[find(i)] = find(j);
    }
  }
  const columns = new Map<number, VerticalRun[]>();
  runs.forEach((run, i) => {
    const root = find(i);
    const column = columns.get(root);
    if (column === undefined) columns.set(root, [run]);
    else column.push(run);
  });

  // Each column: the search, else the greedy fallback.
  const placeAll = (column: readonly VerticalRun[], chosen: readonly Candidate[]) => {
    column.forEach((run, i) => {
      run.plan[run.which] = chosen[i]!.line;
    });
  };
  for (const column of columns.values()) {
    const chosen: Candidate[] = [];
    const fits = (i: number, c: Candidate): boolean => {
      const run = column[i]!;
      for (let j = 0; j < i; j += 1) if (conflict(run, c, column[j]!, chosen[j]!)) return false;
      return true;
    };
    // Depth-first with forward checking: each run keeps the candidates no
    // assignment so far rules out; an assignment that leaves a later run
    // none is undone at once (the trail), so a dead end is seen at the
    // first run it dooms, not at that run.
    let budget = LANE_SEARCH_BUDGET;
    const alive = column.map((run) => run.domain.map(() => true));
    const aliveCount = column.map((run) => run.domain.length);
    const trail: [number, number][] = [];
    const undo = (mark: number) => {
      while (trail.length > mark) {
        const [j, d] = trail.pop()!;
        alive[j]![d] = true;
        aliveCount[j] = aliveCount[j]! + 1;
      }
    };
    const assign = (i: number, c: Candidate): boolean => {
      for (let j = i + 1; j < column.length; j += 1) {
        const other = column[j]!;
        other.domain.forEach((d, di) => {
          if (alive[j]![di] && conflict(other, d, column[i]!, c)) {
            alive[j]![di] = false;
            aliveCount[j] = aliveCount[j]! - 1;
            trail.push([j, di]);
          }
        });
        if (aliveCount[j] === 0) return false;
      }
      return true;
    };
    const search = (i: number): boolean => {
      if (i === column.length) return true;
      const run = column[i]!;
      for (let ci = 0; ci < run.domain.length; ci += 1) {
        if (!alive[i]![ci]) continue;
        if (budget <= 0) return false;
        budget -= 1;
        const c = run.domain[ci]!;
        const mark = trail.length;
        chosen[i] = c;
        if (assign(i, c) && search(i + 1)) return true;
        undo(mark);
      }
      return false;
    };
    if (search(0)) {
      placeAll(column, chosen);
      continue;
    }
    // The fallback: top-down, the nearest line that fits; else the nearest
    // whose column line is free (its stub coincides); else the line with the
    // fewest runs over the extent (the column coincides) — reported.
    chosen.length = 0;
    column.forEach((run, i) => {
      if (run.domain.length === 0) {
        // No lattice line in the range at all (a gap too narrow for one
        // between the shrunken legs): the natural place, clamped.
        const line = clamp(run.natural, run.range[0], run.range[1]);
        chosen[i] = { k: Math.round(line / pitch), line };
        return;
      }
      const fitting = run.domain.find((c) => fits(i, c));
      if (fitting !== undefined) {
        chosen[i] = fitting;
        return;
      }
      const load = (c: Candidate) =>
        column.slice(0, i).filter((other, j) => chosen[j]!.k === c.k && meets(run.extent[0], run.extent[1], other.extent[0], other.extent[1])).length;
      const free = run.domain.find((c) => load(c) === 0);
      chosen[i] = free ?? run.domain.reduce((best, c) => (load(c) < load(best) ? c : best), run.domain[0]!);
    });
    placeAll(column, chosen);
  }

  const routes = new Map<string, TraceRoute>();
  for (const plan of plans) {
    const { natural, hy, vx, vx1, vx2 } = plan;
    if (natural.kind === "forward") routes.set(plan.wire.id, { kind: "forward", vx });
    else routes.set(plan.wire.id, { kind: natural.kind, vx1, hy, vx2 });
  }
  return { routes, collapsed: coincidences(plans, routes, unit) };
}

/** A drawn axis-aligned run of a wire, for the measure of coincidences. */
interface DrawnRun {
  owner: string;
  /** The wire's first point — wires out of one port start at the same point. */
  start: Point;
  kind: "h" | "v";
  at: number;
  lo: number;
  hi: number;
}

/**
 * The wires whose drawn runs coincide with another wire's — parallel, on
 * one line, overlapping over a positive length — less the trunk: the runs
 * along the row of a source port both wires leave. Measured on the routes
 * as drawn (`routeCorners` + `chamfer`, what `tracePath` draws), so the
 * report is what the picture shows, not what the model feared.
 */
function coincidences(plans: readonly Plan[], routes: ReadonlyMap<string, TraceRoute>, unit: number): string[] {
  const byLine = new Map<string, DrawnRun[]>();
  for (const plan of plans) {
    const { ends } = plan.wire;
    const route = routes.get(plan.wire.id);
    if (route === undefined) continue;
    const points = chamfer(routeCorners(ends, route, unit), TRACE_CORNER_UNITS * unit);
    const start = points[0];
    if (start === undefined) continue;
    for (let i = 1; i < points.length; i += 1) {
      const a = points[i - 1]!;
      const b = points[i]!;
      let run: DrawnRun | null = null;
      if (Math.abs(b.y - a.y) < EPS) run = { owner: plan.wire.id, start, kind: "h", at: a.y, lo: Math.min(a.x, b.x), hi: Math.max(a.x, b.x) };
      else if (Math.abs(b.x - a.x) < EPS) run = { owner: plan.wire.id, start, kind: "v", at: a.x, lo: Math.min(a.y, b.y), hi: Math.max(a.y, b.y) };
      if (run === null) continue;
      const key = `${run.kind}${Math.round(run.at * 100)}`;
      const line = byLine.get(key);
      if (line === undefined) byLine.set(key, [run]);
      else line.push(run);
    }
  }
  const collapsed: string[] = [];
  const report = (id: string) => {
    if (!collapsed.includes(id)) collapsed.push(id);
  };
  for (const line of byLine.values()) {
    if (line.length < 2) continue;
    line.sort((a, b) => a.lo - b.lo);
    for (let i = 0; i < line.length; i += 1) {
      for (let j = i + 1; j < line.length && line[j]!.lo < line[i]!.hi - EPS; j += 1) {
        const a = line[i]!;
        const b = line[j]!;
        if (a.owner === b.owner) continue;
        const trunk = a.kind === "h" && same(a.start, b.start) && Math.abs(a.at - a.start.y) < EPS;
        if (trunk) continue;
        report(a.owner);
        report(b.owner);
      }
    }
  }
  // In lane order, like every other list the assignment hands out.
  const order = new Map(plans.map((plan) => [plan.wire.id, plan.order]));
  return collapsed.sort((a, b) => order.get(a)! - order.get(b)!);
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
