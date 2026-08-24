/**
 * The trace router (docs/16 §Canvas conventions; wave 4 B2, finding U6):
 * the corner cut's one-unit legs (and their half-unit floor in a tight
 * gap), the three route shapes, the ¼-unit lanes, the deterministic
 * assignment, no two parallel runs coinciding on a fan-in of five — and,
 * since the B2 review, on the wall itself (`fixtures/wallTraceWires.ts`):
 * the top-down order of the vertical runs, the blocker-moving repair of a
 * saturated column, the spread-and-reported collapse when even that fails,
 * the stubs reserved over the room their lane may take, and the two
 * fallbacks surfacing (`TraceLanes.collapsed`, `tracePath`'s yield flag).
 * The oracle — runs, overlaps, corner legs — is `e2e/traceOracle.ts`, the
 * one the Playwright specs judge the real app by.
 */
import { describe, expect, it } from "vitest";
import { cornerLegs, kinds, overlaps, parsePath, segments, type Pt } from "../../e2e/traceOracle";
import type { GraphView, NodeView } from "../protocol/messages";
import { WALL_ROW_ORIGIN, WALL_UNIT, WALL_WIRES } from "./fixtures/wallTraceWires";
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

const drawn = (e: TraceEnds, route?: TraceRoute): Pt[] => parsePath(tracePath(e, U, route)[0]);
const legs = (points: readonly Pt[]): number[] => cornerLegs(segments(points));

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
    for (const p of [drawn(ends(0, 0, 6, 6)), drawn(ends(0, 6, 6, 0))]) expect(legs(p)).toEqual([R, R]);
    // Exactly two legs apart: the stubs ARE the legs; one diagonal spans the two cuts.
    const tight = drawn(ends(0, 0, 2, 6));
    expect(tight).toEqual([
      { x: 0, y: 0 },
      { x: U, y: U },
      { x: U, y: 5 * U },
      { x: 2 * U, y: 6 * U },
    ]);
    // The cuts at the wire's very ends are corners too (the review's third
    // finding: the first router's oracles never measured them).
    expect(kinds(tight)).toBe("dvd");
    expect(legs(tight)).toEqual([R, R]);
  });

  it("a jog shorter than two legs is one 45° diagonal between the stubs — not a corner", () => {
    const e = ends(0, 0, 6, 0.5);
    expect(isForward(e, U)).toBe(true);
    const points = drawn(e);
    expect(kinds(points)).toBe("hdh");
    expect(legs(points)).toEqual([]);
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
    const points = drawn(e);
    expect(kinds(points)).toBe("dvd");
    expect(legs(points)).toEqual([0.75 * U, 0.75 * U]);
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
    expect(legs(points)).toEqual([R, R, R, R]);
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
    expect(tracePath(ends(0, 0, 6, 6), U).slice(1, 3)).toEqual([3 * U, 3 * U]);
    expect(tracePath(ends(0, 0, 6, 0), U).slice(1, 3)).toEqual([3 * U, 0]);
    expect(tracePath(ends(0, 0, 10, 6), U).slice(1, 3)).toEqual([5 * U, 3 * U]);
    expect(tracePath(ends(10, 0, 0, 0), U).slice(1, 3)).toEqual([5 * U, 3 * U]);
  });
});

describe("the back route — a target to the left", () => {
  it("goes out, down beside the source, left along a channel three legs below both rows, up, in", () => {
    const level = ends(10, 0, 0, 0);
    expect(isForward(level, U)).toBe(false);
    expect(naturalRoute(level, U)).toEqual({ kind: "back", vx1: 11.5 * U, hy: 3 * U, vx2: -1.5 * U });
    const points = drawn(level);
    expect(kinds(points)).toBe("hdvdhdvdh");
    expect(legs(points)).toEqual([R, R, R, R]);
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
  it("clamps an assigned channel into what its own endpoints allow, and yields to them on the route's kind — saying so", () => {
    const e = ends(0, 0, 6, 6);
    // A lane far to the right: the target's stub keeps a (shrunken) leg.
    const corners = routeCorners(e, { kind: "forward", vx: 20 * U }, U);
    expect(corners[1]).toEqual({ x: 5.5 * U, y: 0 });
    expect(tracePath(e, U, { kind: "forward", vx: 20 * U })[3]).toBe(false);
    expect(tracePath(e, U)[3]).toBe(false);
    // A route of the other kind (the model and the handles disagreed): the
    // natural one, and the yield flag raised — the edge marks itself.
    const back = tracePath(e, U, { kind: "back", vx1: 0, hy: 0, vx2: 0 });
    expect(back[0]).toBe(tracePath(e, U)[0]);
    expect(back[3]).toBe(true);
    const stair = tracePath(e, U, { kind: "stair", vx1: 0, hy: 0, vx2: 0 });
    expect(stair[0]).toBe(tracePath(e, U)[0]);
    expect(stair[3]).toBe(true);
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

const lanes = (wires: readonly TraceWire[], rowOrigin = 0) => assignTraceLanes(wires, U, rowOrigin);
const paths = (wires: readonly TraceWire[], routes: ReadonlyMap<string, TraceRoute>) =>
  new Map(wires.map((w) => [w.id, drawn(w.ends, routes.get(w.id))]));

describe("lanes", () => {
  it("five wires into one node (stairs): every free run on its own ¼-unit lattice line, no two parallel runs coinciding", () => {
    const { routes, collapsed } = lanes(fanIn);
    expect(routes.size).toBe(5);
    expect(collapsed).toEqual([]);
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
    const { routes } = lanes(fanInZ);
    const xs = fanInZ.map((w) => (routes.get(w.id) as { vx: number }).vx).sort((a, b) => a - b);
    expect(xs).toEqual([1.5, 1.75, 2, 2.25, 2.5].map((v) => v * U));
    const drawnZ = paths(fanInZ, routes);
    for (const [, p] of drawnZ) expect(legs(p)).toEqual([R, R]);
    expect(overlaps(drawnZ)).toEqual([]);
  });

  it("is the same for any input order, and across re-runs", () => {
    for (const set of [fanIn, fanInZ, WALL_WIRES]) {
      const origin = set === WALL_WIRES ? WALL_ROW_ORIGIN : 0;
      const a = lanes(set, origin);
      const b = lanes([...set].reverse(), origin);
      const shuffled = [...set].sort((x, y) => (x.id.length - y.id.length) || (x.id < y.id ? 1 : -1));
      const c = lanes(shuffled, origin);
      for (const w of set) {
        expect(sameRoute(a.routes.get(w.id), b.routes.get(w.id)), w.id).toBe(true);
        expect(sameRoute(a.routes.get(w.id), c.routes.get(w.id)), w.id).toBe(true);
      }
      expect([...lanes(set, origin).routes]).toEqual([...a.routes]);
      expect(b.collapsed).toEqual(a.collapsed);
    }
  });

  it("orders the wires by source position, then target position, then the id", () => {
    const w = (id: string, sy: number, sx: number, ty: number, tx: number): TraceWire => ({ id, ends: ends(sx, sy, tx, ty) });
    expect(compareTraceWires(w("b", 0, 0, 5, 5), w("a", 1, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 1, 5, 5), w("a", 0, 0, 5, 5))).toBeGreaterThan(0);
    expect(compareTraceWires(w("b", 0, 0, 4, 5), w("a", 0, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 0, 5, 4), w("a", 0, 0, 5, 5))).toBeLessThan(0);
    expect(compareTraceWires(w("b", 0, 0, 5, 5), w("a", 0, 0, 5, 5))).toBeGreaterThan(0);
    // The topmost run keeps the natural channel.
    expect(lanes(fanInZ).routes.get("z0")).toEqual({ kind: "forward", vx: 2 * U });
  });

  it("places the vertical runs top-down — by the run's top end, not the wire's source row", () => {
    // `down` leaves the upper row; `up` leaves the lower row but its run
    // reaches higher. The source-row order would give `down` the natural
    // line; the runs' order gives it to `up`, whose top comes first.
    const wires: TraceWire[] = [
      { id: "down", ends: ends(0, 0, 3, 10) },
      { id: "up", ends: ends(0, 8, 3, -2) },
    ];
    const { routes, collapsed } = lanes(wires);
    expect(collapsed).toEqual([]);
    expect(routes.get("up")).toEqual({ kind: "forward", vx: 1.5 * U });
    expect(routes.get("down")).toEqual({ kind: "forward", vx: 1.75 * U });
    expect(overlaps(paths(wires, routes))).toEqual([]);
  });

  it("a fan-out from one port to a column of targets parts into lanes; the stubs out of the port are the trunk", () => {
    const fanOut: TraceWire[] = [0, 1, 2].map((i) => ({ id: `o${i}`, ends: ends(0, 0, 4, 10 + 3 * i) }));
    const { routes } = lanes(fanOut);
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
    const { routes } = lanes(wires);
    expect(routes.get("top")).toEqual({ kind: "forward", vx: 2 * U });
    expect(routes.get("bottom")).toEqual({ kind: "forward", vx: 2 * U });
  });

  it("a jog or a level Z holds no lane — there is no free run to lane", () => {
    const wires: TraceWire[] = [
      { id: "flat", ends: ends(0, 0, 4, 0) },
      { id: "jog", ends: ends(0, 1, 4, 1.5) },
      { id: "tall", ends: ends(0, 0, 4, 6) },
    ];
    const { routes } = lanes(wires);
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
    const { routes, collapsed } = lanes(wires);
    expect(collapsed).toEqual([]);
    expect(routes.get("a")).toEqual({ kind: "forward", vx: U });
    expect(routes.get("b")).toEqual({ kind: "forward", vx: 1.25 * U });
    expect(routes.get("c")).toEqual({ kind: "forward", vx: 0.75 * U });
    const got = paths(wires, routes);
    expect(overlaps(got)).toEqual([]);
    for (const [id, p] of got) {
      // The first cut's legs: a full unit where the stub allows, else the stub (≥ the floor).
      for (const leg of legs(p)) {
        expect(leg, id).toBeGreaterThanOrEqual(TRACE_MIN_LEG_UNITS * U - 1e-9);
        expect(leg, id).toBeLessThanOrEqual(R + 1e-9);
      }
    }
    // `c` is laned to 0.75 u: its source stub is consumed whole (a ¾-unit leg), its target stub keeps a full leg.
    expect(legs(got.get("c")!)).toEqual([0.75 * U, R]);
    // Six runs deep on five lines: the sixth shares a line, and the
    // assignment names both wires of the pair (measured on the drawing).
    const many: TraceWire[] = [0, 1, 2, 3, 4, 5].map((i) => ({ id: `m${i}`, ends: ends(0, i, 2, 10 + i) }));
    const full = lanes(many);
    const xs = many.map((w) => (full.routes.get(w.id) as { vx: number }).vx);
    expect(new Set(xs).size).toBe(5);
    expect(full.collapsed).toEqual(["m0", "m5"]);
  });

  it("a stair's long run avoids a row another wire's stub occupies; a level long wire detours a full leg only when its row is taken", () => {
    // `through` runs level along row 0 from x=0 to x=12; `short` is a Z on row 0 between x=5 and x=8 — its
    // stub is pinned there, so `through` leaves the row: a detour one leg away, as a stair.
    const wires: TraceWire[] = [
      { id: "short", ends: ends(5, 0, 8, 0) },
      { id: "through", ends: ends(0, 0, 12, 0) },
    ];
    const { routes } = lanes(wires);
    expect(routes.get("short")).toEqual({ kind: "forward", vx: 6.5 * U });
    const through = routes.get("through") as { kind: "stair"; hy: number };
    expect(through.kind).toBe("stair");
    expect(Math.abs(through.hy)).toBe(U);
    const got = paths(wires, routes);
    expect(kinds(got.get("through")!)).toBe("hdhdh");
    expect(overlaps(got)).toEqual([]);
    // Alone, the level long wire is a straight line.
    expect(lanes([wires[1]!]).routes.get("through")).toEqual({ kind: "stair", vx1: 1.5 * U, hy: 0, vx2: 10.5 * U });
  });

  it("a stub that grows with its lane never runs into a channel placed on its row — the vertical takes a line that keeps it short", () => {
    // The review's second finding: a column of Zs into one node laned the
    // row-0 Z far right of its natural midpoint (the eighteenth of eighteen
    // crossing runs, to x = 126 — 2¼ units right of the midpoint, 72), so
    // its drawn stub ran on to x = 102, past the extent the first router
    // had recorded; a level stair along row 0 out of a node ending at
    // x = 74.4 had seen the row free there and run on it, the two
    // coinciding over [74.4, 102]. A vertical now takes no line whose stub
    // would run into a fixed run on its row: the Z takes the free line
    // that keeps its stub short of the stair, and the stair keeps its
    // straight line.
    const zs: TraceWire[] = [];
    for (let i = -17; i <= 0; i += 1) zs.push({ id: `z${i}`, ends: ends(0, i, 6, 30 + i) });
    const wires: TraceWire[] = [...zs, { id: "level", ends: ends(3.1, 0, 20, 0) }];
    const { routes, collapsed } = lanes(wires);
    expect(collapsed).toEqual([]);
    const z0 = routes.get("z0") as { kind: "forward"; vx: number };
    expect(z0.vx + R).toBeLessThanOrEqual(3.1 * U);
    const level = routes.get("level") as { kind: "stair"; vx1: number; hy: number; vx2: number };
    expect(level.kind).toBe("stair");
    expect(level.hy).toBe(0);
    expect(level.vx1).toBeCloseTo(3.1 * U + 1.5 * U, 9);
    expect(level.vx2).toBe(18.5 * U);
    expect(overlaps(paths(wires, routes))).toEqual([]);
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
    const { routes } = lanes(wires, origin);
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
    const { routes } = lanes(wires);
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
    const { routes } = lanes(wires);
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
    const { routes } = lanes(wires);
    for (const w of wires) expect(kinds(drawn(w.ends, routes.get(w.id))), w.id).not.toMatch(/dd/);
  });
});

/**
 * The wall's gap, as the handles make it: a column's right edge at x = 0
 * and the next column's left edge three units on, the handles 3.5 px past
 * each. A Z between them may take seven lattice lines (`[sx + ½u, tx − ½u]`
 * holds 18 … 54); a stair's vertical into the right column eleven (every
 * line up to `tx − u`, the gap's left edge included), one out of the left
 * column likewise to the right.
 */
const OVERHANG = 3.5;
const gapZ = (id: string, sy: number, ty: number): TraceWire => ({ id, ends: { sx: OVERHANG, sy: sy * U, tx: 3 * U - OVERHANG, ty: ty * U } });
const gapStairIn = (id: string, sy: number, ty: number): TraceWire => ({ id, ends: { sx: -500, sy: sy * U, tx: 3 * U - OVERHANG, ty: ty * U } });
const Z_LINES = [18, 24, 30, 36, 42, 48, 54];

describe("a saturated column", () => {
  it("moves a run with lines to spare — a stair's vertical — out of a Z's way, so a column holds more runs than a Z has lines", () => {
    // Six Zs, all crossing each other, take six of the seven Z lines; the
    // stair's vertical, natural at the seventh's neighbour, would take the
    // seventh, leaving the last Z — eight runs cross its top — no line. The
    // search backtracks: the stair goes out to a line the Z cannot use
    // (12 px off the left column); no two runs coincide, nothing is
    // reported.
    const zs = [0, 1, 2, 3, 4, 5].map((i) => gapZ(`z${i}`, 10 + i, 30 + i));
    const stair = gapStairIn("stair", 0, 25);
    const victim = gapZ("z6", 16, 36);
    const wires = [...zs, stair, victim];
    const { routes, collapsed } = lanes(wires);
    expect(collapsed).toEqual([]);
    const xs = [...zs, victim].map((w) => (routes.get(w.id) as { vx: number }).vx).sort((a, b) => a - b);
    expect(xs).toEqual(Z_LINES);
    const s = routes.get("stair") as { kind: "stair"; vx2: number };
    expect(s.kind).toBe("stair");
    expect(s.vx2).toBe(12);
    expect(overlaps(paths(wires, routes))).toEqual([]);
    // Without the victim the stair sits where it naturally falls: among the Z lines.
    const alone = lanes([...zs, stair]);
    expect(alone.collapsed).toEqual([]);
    expect((alone.routes.get("stair") as { vx2: number }).vx2).toBe(24);
  });

  it("with no assignment that keeps every run apart, the losers take the lightest lines — coincidences spread, never stack — and are reported", () => {
    // Eight Zs crossing each other on seven lines: the eighth shares the
    // nearest line with one run; a ninth shares the NEXT line, not the same.
    const eight = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => gapZ(`z${i}`, 10 + i, 30 + i));
    const { routes, collapsed } = lanes(eight);
    // Both wires of the coinciding pair are named, in lane order.
    expect(collapsed).toEqual(["z0", "z7"]);
    const vx = (id: string) => (routes.get(id) as { vx: number }).vx;
    expect(vx("z7")).toBe(36);
    expect(vx("z0")).toBe(36);
    expect(overlaps(paths(eight, routes))).toHaveLength(1);
    const nine = [...eight, gapZ("z8", 18, 38)];
    const more = lanes(nine);
    expect(more.collapsed).toEqual(["z0", "z1", "z7", "z8"]);
    const vx9 = (id: string) => (more.routes.get(id) as { vx: number }).vx;
    expect(vx9("z8")).toBe(42);
    expect(vx9("z8")).not.toBe(vx9("z7"));
    // Each line carries at most two runs: the overlaps are the two shared pairs, nothing stacked deeper.
    expect(overlaps(paths(nine, more.routes))).toHaveLength(2);
  });
});

describe("the wall", () => {
  it("in trace mode has no two parallel runs coinciding, no collapsed lane, every cut within the legs' bounds", () => {
    const { routes, collapsed } = assignTraceLanes(WALL_WIRES, WALL_UNIT, WALL_ROW_ORIGIN);
    expect(routes.size).toBe(WALL_WIRES.length);
    expect(collapsed).toEqual([]);
    const got = new Map(WALL_WIRES.map((w) => [w.id, parsePath(tracePath(w.ends, WALL_UNIT, routes.get(w.id))[0])]));
    expect(overlaps(got)).toEqual([]);
    let corners = 0;
    for (const [id, points] of got) {
      expect(kinds(points), id).not.toMatch(/dd/);
      for (const leg of cornerLegs(segments(points, id))) {
        expect(leg, id).toBeGreaterThanOrEqual(TRACE_MIN_LEG_UNITS * WALL_UNIT - 1e-9);
        expect(leg, id).toBeLessThanOrEqual(TRACE_CORNER_UNITS * WALL_UNIT + 1e-9);
        corners += 1;
      }
    }
    expect(corners).toBeGreaterThan(100);
    // The fixture keeps its teeth: the busiest three-unit gap — between the
    // `labels` column (right edge x = 2424) and `glyphs`/`plates`/`dxf`
    // (left edge 2496) — carries over twenty vertical runs, eight deep.
    const verticals = [...got].flatMap(([, points]) =>
      segments(points).flatMap((s) => (s.kind === "v" && s.at >= 2400 && s.at <= 2520 ? [s] : [])),
    );
    expect(verticals.length).toBeGreaterThanOrEqual(20);
    const events = verticals.flatMap((v) => [[v.lo, 1], [v.hi, -1]] as [number, number][]).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    let depth = 0;
    let deepest = 0;
    for (const [, step] of events) {
      depth += step;
      deepest = Math.max(deepest, depth);
    }
    expect(deepest).toBeGreaterThanOrEqual(8);
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
