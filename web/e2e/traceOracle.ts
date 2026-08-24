/**
 * The trace oracle (docs/16 §Canvas conventions, trace mode; wave 4 B2,
 * finding U6), shared by the router's unit test (`src/canvas/trace.test.ts`)
 * and the Playwright specs (`traces.spec.ts`, `wall_traces.spec.ts`) so the
 * two judge a drawn wire by ONE rule: parse an `M`/`L` path, type its runs
 * — horizontal, vertical, 45° diagonal, nothing else — list the pairs of
 * parallel runs of different wires that coincide (the trunk out of one port
 * excepted), and measure every corner cut's legs, the cuts at a wire's very
 * ends included (a stub consumed whole in a tight gap: the shortest legs
 * there are, where the half-unit floor binds). Pure, no test framework:
 * it throws on a path that is not a trace.
 */

export interface Pt {
  x: number;
  y: number;
}

export type Seg = { kind: "h" | "v"; at: number; lo: number; hi: number } | { kind: "d"; dx: number; dy: number };

/** Coordinates within this many px are the same: a drawn path is rounded to 1/100 px. */
export const TOL = 0.02;

/** `M x y L x y …` → points; anything but M/L (a bezier's `C`) throws. */
export function parsePath(d: string): Pt[] {
  const tokens = d
    .trim()
    .split(/\s*([A-Za-z])\s*/)
    .filter((t) => t !== "");
  const points: Pt[] = [];
  for (let i = 0; i < tokens.length; i += 2) {
    const command = tokens[i]!;
    if (command !== "M" && command !== "L") throw new Error(`not a trace: command ${command} in ${d}`);
    const [x, y] = (tokens[i + 1] ?? "").trim().split(/[\s,]+/).map(Number);
    if (x === undefined || y === undefined || Number.isNaN(x) || Number.isNaN(y)) throw new Error(`bad pair in ${d}`);
    points.push({ x, y });
  }
  return points;
}

/** Typed runs; a run that is neither orthogonal nor 45° throws. */
export function segments(points: readonly Pt[], id = "trace"): Seg[] {
  const out: Seg[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1]!;
    const b = points[i]!;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    if (Math.abs(dy) < TOL) out.push({ kind: "h", at: a.y, lo: Math.min(a.x, b.x), hi: Math.max(a.x, b.x) });
    else if (Math.abs(dx) < TOL) out.push({ kind: "v", at: a.x, lo: Math.min(a.y, b.y), hi: Math.max(a.y, b.y) });
    else if (Math.abs(Math.abs(dx) - Math.abs(dy)) < 2.5 * TOL) out.push({ kind: "d", dx, dy });
    else throw new Error(`${id}: a run that is neither orthogonal nor 45°: ${JSON.stringify([a, b])}`);
  }
  return out;
}

/** The run kinds as a string — `hdvdh` is a Z. */
export function kinds(points: readonly Pt[], id = "trace"): string {
  return segments(points, id)
    .map((s) => s.kind)
    .join("");
}

/**
 * Pairs of parallel axis-aligned runs of DIFFERENT wires that share a line
 * and overlap in length — less the trunk: horizontal runs on the row of a
 * source port both wires leave (their paths start at the same point).
 */
export function overlaps(paths: ReadonlyMap<string, readonly Pt[]>): string[] {
  const found: string[] = [];
  const all = [...paths].flatMap(([id, points]) => segments(points, id).map((s) => ({ id, s, start: points[0]! })));
  for (let i = 0; i < all.length; i += 1) {
    for (let j = i + 1; j < all.length; j += 1) {
      const a = all[i]!;
      const b = all[j]!;
      if (a.id === b.id || a.s.kind === "d" || b.s.kind === "d" || a.s.kind !== b.s.kind) continue;
      if (Math.abs(a.s.at - b.s.at) > TOL) continue;
      const lo = Math.max(a.s.lo, b.s.lo);
      const hi = Math.min(a.s.hi, b.s.hi);
      if (hi - lo <= TOL) continue;
      const trunk =
        a.s.kind === "h" &&
        Math.abs(a.start.x - b.start.x) < TOL &&
        Math.abs(a.start.y - b.start.y) < TOL &&
        Math.abs(a.s.at - a.start.y) < TOL;
      if (trunk) continue;
      found.push(`${a.id} ∥ ${b.id} on ${a.s.kind}=${a.s.at} over [${lo}, ${hi}]`);
    }
  }
  return found;
}

/**
 * The legs of every corner cut — a diagonal between a horizontal and a
 * vertical run, OR a diagonal at either end of the path beside one
 * orthogonal run (the stub was consumed whole: the cut starts at the
 * handle). A jog's lone diagonal between two horizontals, and a wire that
 * is one diagonal, are not corners.
 */
export function cornerLegs(segs: readonly Seg[]): number[] {
  const legs: number[] = [];
  for (let i = 0; i < segs.length; i += 1) {
    const s = segs[i]!;
    if (s.kind !== "d") continue;
    const prev = segs[i - 1];
    const next = segs[i + 1];
    const orthogonal = (t: Seg | undefined) => t !== undefined && t.kind !== "d";
    const corner =
      prev !== undefined && next !== undefined
        ? orthogonal(prev) && orthogonal(next) && prev.kind !== next.kind
        : orthogonal(prev) || orthogonal(next);
    if (corner) legs.push(Math.abs(s.dx));
  }
  return legs;
}
