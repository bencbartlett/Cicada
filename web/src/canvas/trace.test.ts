/**
 * The trace router (docs/16 §Canvas conventions; wave 4 B2, finding U6):
 * the corner cut's one-unit legs (and their half-unit floor in a tight
 * gap), the three route shapes, the ¼-unit lanes, the deterministic
 * assignment, and no two parallel runs coinciding on a fan-in of five.
 */
import { describe, expect, it } from "vitest";
import type { GraphView, NodeView } from "../protocol/messages";
import {
  TRACE_CORNER_UNITS,
  TRACE_LANE_UNITS,
  TRACE_MIN_LEG_UNITS,
  TRACE_STAIRCASE_UNITS,
  assignTraceLanes,
  chamfer,
  compareTraceWires,
  isForward,
  naturalRoute,
  routeCorners,
  rowLatticeOrigin,
  sameRoute,
  svgPath,
  tracePath,
  wireEnds,
  type Point,
  type TraceEnds,
  type TraceRoute,
  type TraceWire,
} from "./trace";

const U = 24;
const R = TRACE_CORNER_UNITS * U;
const PITCH = TRACE_LANE_UNITS * U;

/** Endpoints in grid units → px. */
const ends = (sx: number, sy: number, tx: number, ty: number): TraceEnds => ({
  sx: sx * U,
  sy: sy * U,
  tx: tx * U,
  ty: ty * U,
});

/** `M x y L x y …` → points. */
function parsePath(d: string): Point[] {
  return d
    .split(/\s*[ML]\s*/)
    .filter((s) => s !== "")
    .map((pair) => {
      const [x, y] = pair.trim().split(/\s+/).map(Number);
      return { x: x!, y: y! };
    });
}

type Segment =
  | { kind: "h" | "v"; at: number; lo: number; hi: number }
  | { kind: "d"; dx: number; dy: number };

function segments(points: readonly Point[]): Segment[] {
  const out: Segment[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1]!;
    const b = points[i]!;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    if (Math.abs(dy) < 1e-6) out.push({ kind: "h", at: a.y, lo: Math.min(a.x, b.x), hi: Math.max(a.x, b.x) });
    else if (Math.abs(dx) < 1e-6) out.push({ kind: "v", at: a.x, lo: Math.min(a.y, b.y), hi: Math.max(a.y, b.y) });
    else {
      // Every other segment is a 45° diagonal — nothing else is allowed.
      expect(Math.abs(dx), `diagonal ${JSON.stringify([a, b])}`).toBeCloseTo(Math.abs(dy), 6);
      out.push({ kind: "d", dx, dy });
    }
  }
  return out;
}

const kinds = (points: readonly Point[]) => segments(points).map((s) => s.kind).join("");

/**
 * Pairs of parallel axis-aligned runs from DIFFERENT wires that share a
 * line and overlap in length — less the trunk out of a shared source port
 * (both paths start at the same point and the run is on that row).
 */
function overlaps(paths: ReadonlyMap<string, Point[]>): string[] {
  const found: string[] = [];
  const all = [...paths].flatMap(([id, points]) => segments(points).map((s) => ({ id, s, start: points[0]! })));
  for (let i = 0; i < all.length; i += 1) {
    for (let j = i + 1; j < all.length; j += 1) {
      const a = all[i]!;
      const b = all[j]!;
      if (a.id === b.id || a.s.kind === "d" || b.s.kind === "d" || a.s.kind !== b.s.kind) continue;
      if (Math.abs(a.s.at - b.s.at) > 1e-6) continue;
      const lo = Math.max(a.s.lo, b.s.lo);
      const hi = Math.min(a.s.hi, b.s.hi);
      if (hi - lo <= 1e-6) continue;
      const trunk =
        a.s.kind === "h" && Math.abs(a.start.x - b.start.x) < 1e-6 && Math.abs(a.start.y - b.start.y) < 1e-6 && Math.abs(a.s.at - a.start.y) < 1e-6;
      if (trunk) continue;
      found.push(`${a.id} ∥ ${b.id} on ${a.s.kind}=${a.s.at} over [${lo}, ${hi}]`);
    }
  }
  return found;
}

/** The legs of every corner cut (a diagonal between a horizontal and a vertical run). */
function cornerLegs(points: readonly Point[]): number[] {
  const segs = segments(points);
  const legs: number[] = [];
  for (let i = 1; i < segs.length - 1; i += 1) {
    const s = segs[i]!;
    const prev = segs[i - 1]!;
    const next = segs[i + 1]!;
    if (s.kind === "d" && prev.kind !== "d" && next.kind !== "d" && prev.kind !== next.kind) legs.push(Math.abs(s.dx));
  }
  return legs;
}

const drawn = (e: TraceEnds, route?: TraceRoute) => parsePath(tracePath(e, U, route)[0]);

describe("the Z — a forward target within the stair threshold", () => {
  it("a level target to the right is a straight line", () => {
    const e = ends(0, 0, 5, 0);
    expect(isForward(e, U)).toBe(true);
    expect(tracePath(e, U)[0]).toBe("M0 0 L120 0");
  });

  it("is stub · 45° cut · vertical channel · 45° cut · stub, the channel halfway", () => {
    const e = ends(0, 0, 6, 6);
    expect(naturalRoute(e, U)).toEqual({ kind: "forward", vx: 3 * U });
    const points = drawn(e);
    expect(points).toEqual([
      { x: 0, y: 0 },
      { x: 2 * U, y: 0 },
      { x: 3 * U, y: U },
      { x: 3 * U, y: 5 * U },
      { x: 4 * U, y: 6 * U },
      { x: 6 * U, y: 6 * U },
    ]);
    expect(kinds(points)).toBe("hdvdh");
  });

  it("the corner cut's legs are one unit — even when the stubs are consumed whole", () => {
    for (const p of [drawn(ends(0, 0, 6, 6)), drawn(ends(0, 6, 6, 0))]) expect(cornerLegs(p)).toEqual([R, R]);
    // Exactly two legs apart: the stubs ARE the legs; one diagonal spans the two cuts.
    expect(drawn(ends(0, 0, 2, 6))).toEqual([
      { x: 0, y: 0 },
      { x: U, y: U },
      { x: U, y: 5 * U },
      { x: 2 * U, y: 6 * U },
    ]);
  });

  it("a jog shorter than two legs is one 45° diagonal between the stubs", () => {
    const e = ends(0, 0, 6, 0.5);
    expect(isForward(e, U)).toBe(true);
    const points = drawn(e);
    expect(kinds(points)).toBe("hdh");
    expect(points).toEqual([
      { x: 0, y: 0 },
      { x: 2.75 * U, y: 0 },
      { x: 3.25 * U, y: 0.5 * U },
      { x: 6 * U, y: 0.5 * U },
    ]);
  });

  it("in a gap between one and two legs wide the legs shrink to the gap's half, never below the floor", () => {
    const e = ends(0, 0, 1.5, 6);
    expect(isForward(e, U)).toBe(true);
    const legs = cornerLegs(drawn(e));
    expect(legs).toHaveLength(0); // the two cuts merge into one diagonal: no straight vertical between them? no — check the shape
    const points = drawn(e);
    expect(kinds(points)).toBe("dvd");
    expect(Math.abs(points[1]!.x - points[0]!.x)).toBe(0.75 * U);
    expect(0.75).toBeGreaterThanOrEqual(TRACE_MIN_LEG_UNITS);
    expect(isForward(ends(0, 0, 0.9, 6), U)).toBe(false);
  });
});

describe("the stair — a forward target beyond the threshold", () => {
  it("is escape · down to the channel between the rows · the long run · down · stub", () => {
    const e = ends(0, 0, 10, 6);
    expect(10).toBeGreaterThan(TRACE_STAIRCASE_UNITS);
    expect(naturalRoute(e, U)).toEqual({ kind: "stair", vx1: 1.5 * U, hy: 3 * U, vx2: 8.5 * U });
    const points = drawn(e);
    expect(kinds(points)).toBe("hdvdhdvdh");
    expect(cornerLegs(points)).toEqual([R, R, R, R]);
    expect(points[0]).toEqual({ x: 0, y: 0 });
    expect(points[points.length - 1]).toEqual({ x: 10 * U, y: 6 * U });
    // The long run sits on the channel, a leg short of each vertical.
    expect(points[4]).toEqual({ x: 2.5 * U, y: 3 * U });
    expect(points[5]).toEqual({ x: 7.5 * U, y: 3 * U });
  });

  it("a level target is a straight line", () => {
    expect(tracePath(ends(0, 0, 10, 0), U)[0]).toBe("M0 0 L240 0");
  });

  it("two units of drop make the verticals single diagonals; one unit is the smallest vertical", () => {
    const two = drawn(ends(0, 0, 10, 2));
    expect(kinds(two)).toBe("hdhdh");
    // A channel a quarter unit off a row would be a kink: it is pushed to a leg away, or onto the row.
    const corners = routeCorners(ends(0, 0, 10, 6), { kind: "stair", vx1: 1.5 * U, hy: 0.25 * U, vx2: 8.5 * U }, U);
    expect(corners[2]!.y).toBe(0);
    const legAway = routeCorners(ends(0, 0, 10, 6), { kind: "stair", vx1: 1.5 * U, hy: 0.75 * U, vx2: 8.5 * U }, U);
    expect(legAway[2]!.y).toBe(U);
  });

  it("the label anchor is the middle of the middle run", () => {
    expect(tracePath(ends(0, 0, 6, 6), U).slice(1)).toEqual([3 * U, 3 * U]);
    expect(tracePath(ends(0, 0, 6, 0), U).slice(1)).toEqual([3 * U, 0]);
    expect(tracePath(ends(0, 0, 10, 6), U).slice(1)).toEqual([5 * U, 3 * U]);
    expect(tracePath(ends(10, 0, 0, 0), U).slice(1)).toEqual([5 * U, 3 * U]);
  });
});

describe("the back route — a target to the left", () => {
  it("goes out, down beside the source, left along a channel three legs below both rows, up, in", () => {
    const level = ends(10, 0, 0, 0);
    expect(isForward(level, U)).toBe(false);
    expect(naturalRoute(level, U)).toEqual({ kind: "back", vx1: 11.5 * U, hy: 3 * U, vx2: -1.5 * U });
    const points = drawn(level);
    expect(kinds(points)).toBe("hdvdhdvdh");
    expect(cornerLegs(points)).toEqual([R, R, R, R]);
    expect(points[0]).toEqual({ x: 10 * U, y: 0 });
    expect(points[points.length - 1]).toEqual({ x: 0, y: 0 });
    // Rows six legs apart or more: the channel runs between them.
    expect(naturalRoute(ends(10, 0, 0, 8), U)).toEqual({ kind: "back", vx1: 11.5 * U, hy: 4 * U, vx2: -1.5 * U });
    expect(kinds(drawn(ends(10, 0, 0, 8)))).toBe("hdvdhdvdh");
    // Too close for two legs with a tall jog: back, not a forward squeeze.
    expect(isForward(ends(0, 0, 0.5, 6), U)).toBe(false);
    expect(isForward(ends(0, 0, 0.4, 0.5), U)).toBe(false);
  });

  it("a U-turn's runs are at least three legs, so its two same-sense cuts never meet at 90°", () => {
    // A target only just too close for a forward route: the channel run
    // between the verticals is widened to three legs (1.5 each side of the
    // midpoint) — with the natural escape it would be shorter.
    const near = ends(0, 0, 0.9, 6);
    const route = naturalRoute(near, U) as { kind: "back"; vx1: number; vx2: number; hy: number };
    expect(route.kind).toBe("back");
    expect(route.vx1 - route.vx2).toBeCloseTo(3 * R, 9);
    for (const e of [near, ends(10, 0, 0, 0), ends(10, 0, 0, 8), ends(5, 2, 4, 2.5)]) {
      // Consecutive diagonals never appear: every 45° cut is followed by a straight run (or the end).
      expect(kinds(drawn(e)), JSON.stringify(e)).not.toMatch(/dd/);
    }
  });
});

describe("tracePath with an assigned route", () => {
  it("clamps an assigned channel into what its own endpoints allow, and yields to them on the route's kind", () => {
    const e = ends(0, 0, 6, 6);
    // A lane far to the right: the target's stub keeps a (shrunken) leg.
    const corners = routeCorners(e, { kind: "forward", vx: 20 * U }, U);
    expect(corners[1]).toEqual({ x: 5.5 * U, y: 0 });
    // A route of the other kind (the model and the handles disagreed): the natural one.
    expect(tracePath(e, U, { kind: "back", vx1: 0, hy: 0, vx2: 0 })[0]).toBe(tracePath(e, U)[0]);
    expect(tracePath(e, U, { kind: "stair", vx1: 0, hy: 0, vx2: 0 })[0]).toBe(tracePath(e, U)[0]);
  });

  it("chamfer merges collinear points and leaves a line alone", () => {
    expect(chamfer([{ x: 0, y: 0 }, { x: 10, y: 0 }], 5)).toEqual([
      { x: 0, y: 0 },
      { x: 10, y: 0 },
    ]);
    expect(chamfer([{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 0 }, { x: 20, y: 0 }], 5)).toEqual([
      { x: 0, y: 0 },
      { x: 20, y: 0 },
    ]);
    expect(svgPath([{ x: 1.234, y: 2 }, { x: 3, y: 4.005 }])).toBe("M1.23 2 L3 4.01");
  });
});

/** Five sources in a column, five inputs of one node ten units right (stairs). */
const fanIn: TraceWire[] = [0, 1, 2, 3, 4].map((i) => ({ id: `w${i}`, ends: ends(0, i, 10, 10 + i) }));
/** The same, four units right (Zs). */
const fanInZ: TraceWire[] = [0, 1, 2, 3, 4].map((i) => ({ id: `z${i}`, ends: ends(0, i, 4, 10 + i) }));

const paths = (wires: readonly TraceWire[], routes: ReadonlyMap<string, TraceRoute>) =>
  new Map(wires.map((w) => [w.id, drawn(w.ends, routes.get(w.id))]));

describe("lanes", () => {
  it("five wires into one node (stairs): every free run on its own ¼-unit lattice line, no two parallel runs coinciding", () => {
    const routes = assignTraceLanes(fanIn, U);
    expect(routes.size).toBe(5);
    const got = fanIn.map((w) => routes.get(w.id) as { kind: "stair"; vx1: number; hy: number; vx2: number });
    for (const r of got) expect(r.kind).toBe("stair");
    // The channels are the wires' own mid-rows — distinct already; the escapes beside each node part into lanes.
    expect(got.map((r) => r.hy)).toEqual([5, 6, 7, 8, 9].map((v) => v * U));
    const left = got.map((r) => r.vx1).sort((a, b) => a - b);
    const right = got.map((r) => r.vx2).sort((a, b) => a - b);
    expect(left).toEqual([1, 1.25, 1.5, 1.75, 2].map((v) => v * U));
    expect(right).toEqual([8, 8.25, 8.5, 8.75, 9].map((v) => v * U));
    for (const x of [...left, ...right]) expect(Math.abs(x / PITCH - Math.round(x / PITCH))).toBeLessThan(1e-9);
    expect(overlaps(paths(fanIn, routes))).toEqual([]);
  });

  it("five wires into one node (Zs): distinct vertical channels ¼ unit apart around the midpoint, full legs, no overlap", () => {
    const routes = assignTraceLanes(fanInZ, U);
    const xs = fanInZ.map((w) => (routes.get(w.id) as { vx: number }).vx).sort((a, b) => a - b);
    expect(xs).toEqual([1.5, 1.75, 2, 2.25, 2.5].map((v) => v * U));
    const drawnZ = paths(fanInZ, routes);
    for (const [, p] of drawnZ) expect(cornerLegs(p)).toEqual([R, R]);
    expect(overlaps(drawnZ)).toEqual([]);
  });

  it("is the same for any input order, and across re-runs", () => {
    for (const set of [fanIn, fanInZ]) {
      const a = assignTraceLanes(set, U);
      const b = assignTraceLanes([...set].reverse(), U);
      const c = assignTraceLanes([set[2]!, set[4]!, set[0]!, set[3]!, set[1]!], U);
      for (const w of set) {
        expect(sameRoute(a.get(w.id), b.get(w.id))).toBe(true);
        expect(sameRoute(a.get(w.id), c.get(w.id))).toBe(true);
      }
      expect([...assignTraceLanes(set, U)]).toEqual([...a]);
    }
  });

  it("orders by source position, then target position, then the id", () => {
    const w = (id: string, sy: number, sx: number, ty: number, tx: number): TraceWire => ({ id, ends: ends(sx, sy, tx, ty) });
    expect(compareTraceWires(w("b", 0, 0, 5, 5), w("a", 1, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 1, 5, 5), w("a", 0, 0, 5, 5))).toBeGreaterThan(0);
    expect(compareTraceWires(w("b", 0, 0, 4, 5), w("a", 0, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 0, 5, 4), w("a", 0, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 0, 5, 5), w("a", 0, 0, 5, 5))).toBeGreaterThan(0);
    // The first in the order keeps the natural channel.
    expect(assignTraceLanes(fanInZ, U).get("z0")).toEqual({ kind: "forward", vx: 2 * U });
  });

  it("a fan-out from one port to a column of targets parts into lanes; the stubs out of the port are the trunk", () => {
    const fanOut: TraceWire[] = [0, 1, 2].map((i) => ({ id: `o${i}`, ends: ends(0, 0, 4, 10 + 3 * i) }));
    const routes = assignTraceLanes(fanOut, U);
    const xs = fanOut.map((w) => (routes.get(w.id) as { vx: number }).vx).sort((a, b) => a - b);
    expect(xs).toEqual([1.75, 2, 2.25].map((v) => v * U));
    // The only shared runs are the stubs on the source's row (pinned to the port) — the checker exempts the trunk.
    expect(overlaps(paths(fanOut, routes))).toEqual([]);
  });

  it("runs that do not overlap in length share a lattice line", () => {
    const wires: TraceWire[] = [
      { id: "top", ends: ends(0, 0, 4, 4) },
      { id: "bottom", ends: ends(0, 20, 4, 24) },
    ];
    const routes = assignTraceLanes(wires, U);
    expect(routes.get("top")).toEqual({ kind: "forward", vx: 2 * U });
    expect(routes.get("bottom")).toEqual({ kind: "forward", vx: 2 * U });
  });

  it("a jog or a level Z holds no lane — there is no free run to lane", () => {
    const wires: TraceWire[] = [
      { id: "flat", ends: ends(0, 0, 4, 0) },
      { id: "jog", ends: ends(0, 1, 4, 1.5) },
      { id: "tall", ends: ends(0, 0, 4, 6) },
    ];
    const routes = assignTraceLanes(wires, U);
    expect(routes.get("flat")).toEqual({ kind: "forward", vx: 2 * U });
    expect(routes.get("jog")).toEqual({ kind: "forward", vx: 2 * U });
    expect(routes.get("tall")).toEqual({ kind: "forward", vx: 2 * U });
  });

  it("in a two-unit gap the lanes win and the legs shrink toward the half-unit floor — never coinciding", () => {
    const wires: TraceWire[] = [
      { id: "a", ends: ends(0, 0, 2, 6) },
      { id: "b", ends: ends(0, 1, 2, 7) },
      { id: "c", ends: ends(0, 2, 2, 8) },
    ];
    const routes = assignTraceLanes(wires, U);
    expect(routes.get("a")).toEqual({ kind: "forward", vx: U });
    expect(routes.get("b")).toEqual({ kind: "forward", vx: 1.25 * U });
    expect(routes.get("c")).toEqual({ kind: "forward", vx: 0.75 * U });
    const got = paths(wires, routes);
    expect(overlaps(got)).toEqual([]);
    for (const [id, p] of got) {
      // The first cut's legs: a full unit where the stub allows, else the stub (≥ the floor).
      const cut = segments(p).find((s) => s.kind === "d") as { dx: number } | undefined;
      expect(cut, id).toBeDefined();
      expect(Math.abs(cut!.dx), id).toBeGreaterThanOrEqual(TRACE_MIN_LEG_UNITS * U - 1e-9);
      expect(Math.abs(cut!.dx), id).toBeLessThanOrEqual(R + 1e-9);
    }
    expect(segments(got.get("c")!).find((s) => s.kind === "d")).toMatchObject({ dx: 0.75 * U });
    // Only when even the shrunken legs leave no lane room do runs coincide — the documented exception.
    const many: TraceWire[] = [0, 1, 2, 3, 4, 5].map((i) => ({ id: `m${i}`, ends: ends(0, i, 2, 10 + i) }));
    const xs = many.map((w) => (assignTraceLanes(many, U).get(w.id) as { vx: number }).vx);
    expect(new Set(xs).size).toBe(5);
  });

  it("a stair's long run avoids a row another wire's stub occupies; a level long wire detours a full leg only when its row is taken", () => {
    // `through` runs level along row 0 from x=0 to x=12; `short` is a Z on row 0 between x=5 and x=8 — its
    // stub is pinned there, so `through` leaves the row: a detour one leg away, as a stair.
    const wires: TraceWire[] = [
      { id: "short", ends: ends(5, 0, 8, 0) },
      { id: "through", ends: ends(0, 0, 12, 0) },
    ];
    const routes = assignTraceLanes(wires, U);
    expect(routes.get("short")).toEqual({ kind: "forward", vx: 6.5 * U });
    const through = routes.get("through") as { kind: "stair"; hy: number };
    expect(through.kind).toBe("stair");
    expect(Math.abs(through.hy)).toBe(U);
    const got = paths(wires, routes);
    expect(kinds(got.get("through")!)).toBe("hdhdh");
    expect(overlaps(got)).toEqual([]);
    // Alone, the level long wire is a straight line.
    expect(assignTraceLanes([wires[1]!], U).get("through")).toEqual({ kind: "stair", vx1: 1.5 * U, hy: 0, vx2: 10.5 * U });
  });

  it("rows off the grid's lattice (the measured handle shift) are lattice lines of their own: a detour lands exactly a leg away", () => {
    // The real handles sit a px off the row model (rowShift −1 in the app):
    // rows at 35, 59, … — not multiples of the 6 px pitch. The horizontal
    // lattice is anchored on them (`rowLatticeOrigin`), so a stair's
    // channel can be its own row, or a leg away, and the search never
    // falls back onto the source row through everything.
    const shift = -1;
    const off = (e: TraceEnds): TraceEnds => ({ ...e, sy: e.sy + shift, ty: e.ty + shift });
    const origin = rowLatticeOrigin(U, { overhang: 3.5, rowShift: shift });
    expect(origin).toBe(5);
    const wires: TraceWire[] = [
      { id: "short", ends: off(ends(5, 0, 8, 0)) },
      { id: "through", ends: off(ends(0, 0, 12, 0)) },
      { id: "down", ends: off(ends(0, 0, 12, 1)) },
      { id: "blocker", ends: off(ends(6, 1, 9, 1)) },
    ];
    const routes = assignTraceLanes(wires, U, origin);
    const through = routes.get("through") as { kind: "stair"; hy: number };
    expect(through.kind).toBe("stair");
    expect(Math.abs(through.hy - shift)).toBe(U);
    // One row down, both rows taken by other wires' stubs (the rows are the
    // only admissible channels one unit apart): the wire detours a leg
    // beyond one of them — never the inadmissible midpoint, never a row
    // another wire's run already holds.
    const down = routes.get("down") as { kind: "stair"; hy: number };
    expect([shift - U, U + shift + U]).toContain(down.hy);
    const got = paths(wires, routes);
    expect(overlaps(got)).toEqual([]);
    for (const w of wires) expect(kinds(got.get(w.id)!), w.id).not.toMatch(/dd/);
    // Without the anchor the same wires would have no admissible lattice line at all.
    expect(rowLatticeOrigin(U, { overhang: 0, rowShift: 0 })).toBe(0);
  });

  it("two stairs between the same rows take different channels", () => {
    const wires: TraceWire[] = [
      { id: "s1", ends: ends(0, 0, 10, 6) },
      { id: "s2", ends: ends(0, 0, 10, 6.5) },
    ];
    const routes = assignTraceLanes(wires, U);
    const r1 = routes.get("s1") as { hy: number };
    const r2 = routes.get("s2") as { hy: number };
    expect(r1.hy).toBe(3 * U);
    expect(r2.hy).toBe(3.25 * U);
    expect(overlaps(paths(wires, routes))).toEqual([]);
  });

  it("two back routes between the same rows take different horizontal channels", () => {
    const wires: TraceWire[] = [
      { id: "b1", ends: ends(10, 0, 0, 0) },
      { id: "b2", ends: ends(10, 1, 0, 1) },
    ];
    const routes = assignTraceLanes(wires, U);
    const r1 = routes.get("b1") as { kind: "back"; hy: number; vx1: number; vx2: number };
    const r2 = routes.get("b2") as { kind: "back"; hy: number; vx1: number; vx2: number };
    expect(r1.kind).toBe("back");
    expect(r2.kind).toBe("back");
    // b1 first (row 0): its natural channel three legs below its rows; b2's natural (4 units) is free already.
    expect(r1.hy).toBe(3 * U);
    expect(r2.hy).toBe(4 * U);
    expect(r1.vx1).not.toBe(r2.vx1);
    expect(r1.vx2).not.toBe(r2.vx2);
    expect(overlaps(paths(wires, routes))).toEqual([]);
  });

  it("every drawn segment of a laned route is horizontal, vertical or 45°, and consecutive cuts never meet", () => {
    const wires: TraceWire[] = [
      ...fanIn,
      ...fanInZ,
      { id: "back", ends: ends(10, 3, 0, 12) },
      { id: "jog", ends: ends(0, 7, 4, 7.25) },
      { id: "level", ends: ends(0, 20, 12, 20) },
    ];
    const routes = assignTraceLanes(wires, U);
    for (const w of wires) expect(kinds(drawn(w.ends, routes.get(w.id))), w.id).not.toMatch(/dd/);
  });
});

describe("wireEnds — the row model", () => {
  const node = (name: string, cell: [number, number], size: [number, number], inputs: string[], outputs: string[]): NodeView =>
    ({
      ref: 0,
      name,
      targets: [],
      line: 0,
      text: "",
      kind: "call",
      title: name,
      category: "",
      inputs: inputs.map((n) => ({ name: n, type: "Number", base: "Number", list_depth: 0, optional: false, required: true, lift: 0 })),
      outputs: outputs.map((n) => ({ name: n, type: "Number", base: "Number", list_depth: 0, optional: false, displayable: false })),
      diagnostics: [],
      effectful: false,
      preview: false,
      cell,
      size,
      manual: false,
    }) as unknown as NodeView;

  const graph: GraphView = {
    nodes: [node("a", [0, 0], [6, 3], [], ["out", "rest"]), node("b", [10, 2], [6, 4], ["x", "y", "z"], ["out"])],
    wires: [
      { id: "a.out→b.y", from: { node: "a", port: "out" }, to: { node: "b", port: "y" }, lift: 0, depth: 0, red: false },
      { id: "a.rest→b.z", from: { node: "a", port: "rest" }, to: { node: "b", port: "z" }, lift: 0, depth: 0, red: false },
      { id: "gone", from: { node: "ghost", port: "out" }, to: { node: "b", port: "x" }, lift: 0, depth: 0, red: true },
      { id: "noport", from: { node: "a", port: "nope" }, to: { node: "b", port: "x" }, lift: 0, depth: 0, red: true },
    ],
    diagnostics: [],
  };

  it("puts a handle on the node's edge at its row's centre: the header, then one row per port", () => {
    const positions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 10 * U, y: 2 * U }],
    ]);
    const wires = wireEnds(graph, positions, U);
    expect(wires.map((w) => w.id)).toEqual(["a.out→b.y", "a.rest→b.z"]);
    // a's first output: right edge (6 units), row 0 → y = 1.5 units; b's `y` is its second input → 2 + 2.5 units.
    expect(wires[0]!.ends).toEqual({ sx: 6 * U, sy: 1.5 * U, tx: 10 * U, ty: 4.5 * U });
    expect(wires[1]!.ends).toEqual({ sx: 6 * U, sy: 2.5 * U, tx: 10 * U, ty: 5.5 * U });
  });

  it("applies the measured handle geometry: the overhang past each edge, the row shift", () => {
    const positions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 10 * U, y: 2 * U }],
    ]);
    const [wire] = wireEnds(graph, positions, U, { overhang: 4.5, rowShift: 1 });
    expect(wire!.ends).toEqual({ sx: 6 * U + 4.5, sy: 1.5 * U + 1, tx: 10 * U - 4.5, ty: 4.5 * U + 1 });
  });

  it("reads the canvas's live positions, not the cells (a dragged node's wires follow it)", () => {
    const positions = new Map([
      ["a", { x: 3, y: 7 }],
      ["b", { x: 10 * U, y: 2 * U }],
    ]);
    expect(wireEnds(graph, positions, U)[0]!.ends.sx).toBe(3 + 6 * U);
    expect(wireEnds(graph, positions, U)[0]!.ends.sy).toBe(7 + 1.5 * U);
    // A node React Flow has not placed yet has no handle to route to.
    expect(wireEnds(graph, new Map([["a", { x: 0, y: 0 }]]), U)).toEqual([]);
  });
});
