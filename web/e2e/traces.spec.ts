/**
 * Wave 4 B2 (docs/17 §Wave 4; finding U6) in the real app: trace mode's
 * wires are PCB traces from our own router (`src/canvas/trace.ts`), not
 * React Flow's smooth-step path —
 *
 *   - every wire is `M`/`L` runs only, each horizontal, vertical or 45°;
 *   - every 45° cut between a horizontal and a vertical run has legs of
 *     one grid unit (`hello.unitPx`) — the corner floor;
 *   - no two parallel runs of different wires coincide, except the stub
 *     wires out of ONE port share (the trunk, pinned to the port's row);
 *   - stroke colour and width are the spline mode's, wire for wire;
 *   - a re-render (selecting a node) moves nothing;
 *   - the in-flight connection line is a trace too.
 *
 * Evidence: whole-page screenshots of `06-lists` and `07-simple-cad` in
 * trace mode land in this test's output dir under `web/test-results/`.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;

interface Pt {
  x: number;
  y: number;
}

type Seg =
  | { kind: "h" | "v"; at: number; lo: number; hi: number }
  | { kind: "d"; dx: number; dy: number };

/** `M x y L x y …` → points; anything but M/L (a bezier's `C`) throws. */
function parsePath(d: string): Pt[] {
  const tokens = d.trim().split(/\s*([A-Za-z])\s*/).filter((t) => t !== "");
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

/** Typed runs; a diagonal that is not 45° throws. */
function segments(points: readonly Pt[], id: string): Seg[] {
  const out: Seg[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1]!;
    const b = points[i]!;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    if (Math.abs(dy) < 0.02) out.push({ kind: "h", at: a.y, lo: Math.min(a.x, b.x), hi: Math.max(a.x, b.x) });
    else if (Math.abs(dx) < 0.02) out.push({ kind: "v", at: a.x, lo: Math.min(a.y, b.y), hi: Math.max(a.y, b.y) });
    else if (Math.abs(Math.abs(dx) - Math.abs(dy)) < 0.05) out.push({ kind: "d", dx, dy });
    else throw new Error(`${id}: a run that is neither orthogonal nor 45°: ${JSON.stringify([a, b])}`);
  }
  return out;
}

interface Wire {
  d: string;
  stroke: string;
  width: string;
}

/** Every drawn wire by id: its path and its stroke (the inline style React Flow's BaseEdge carries). */
async function wires(page: Page): Promise<Map<string, Wire>> {
  const record = await page.evaluate(() => {
    const out: Record<string, { d: string; stroke: string; width: string }> = {};
    for (const g of Array.from(document.querySelectorAll("g.cicada-edge[data-wire]"))) {
      const path = g.querySelector("path.react-flow__edge-path") as SVGPathElement | null;
      const id = g.getAttribute("data-wire");
      if (path === null || id === null) continue;
      out[id] = { d: path.getAttribute("d") ?? "", stroke: path.style.stroke, width: path.style.strokeWidth };
    }
    return out;
  });
  return new Map(Object.entries(record));
}

async function setWireMode(page: Page, mode: "spline" | "trace"): Promise<void> {
  await page.evaluate((m) => {
    const w = window as unknown as {
      __cicada: { state: () => { updateSettings: (patch: { wireMode: "spline" | "trace" }) => void } };
    };
    w.__cicada.state().updateSettings({ wireMode: m });
  }, mode);
}

async function unitPx(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { hello: { unitPx: number } | null } } };
    const hello = w.__cicada.state().hello;
    if (hello === null) throw new Error("no hello yet");
    return hello.unitPx;
  });
}

async function open(page: Page, pipeline: string, minNodes: number): Promise<void> {
  await page.goto(`/?token=${TOKEN}&pipeline=${pipeline}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(async () => page.locator(".react-flow__node").count()).toBeGreaterThanOrEqual(minNodes);
  await expect.poll(async () => page.locator("g.cicada-edge").count()).toBeGreaterThan(0);
}

/**
 * Pairs of parallel axis-aligned runs of DIFFERENT wires that share a line
 * and overlap in length — less the trunk: horizontal runs on the row of a
 * source port both wires leave (their paths start at the same point).
 */
function overlaps(paths: ReadonlyMap<string, Pt[]>): string[] {
  const found: string[] = [];
  const all = [...paths].flatMap(([id, points]) =>
    segments(points, id).map((s) => ({ id, s, start: points[0]! })),
  );
  for (let i = 0; i < all.length; i += 1) {
    for (let j = i + 1; j < all.length; j += 1) {
      const a = all[i]!;
      const b = all[j]!;
      if (a.id === b.id || a.s.kind === "d" || b.s.kind === "d" || a.s.kind !== b.s.kind) continue;
      if (Math.abs(a.s.at - b.s.at) > 0.02) continue;
      const lo = Math.max(a.s.lo, b.s.lo);
      const hi = Math.min(a.s.hi, b.s.hi);
      if (hi - lo <= 0.02) continue;
      const trunk =
        a.s.kind === "h" &&
        Math.abs(a.start.x - b.start.x) < 0.02 &&
        Math.abs(a.start.y - b.start.y) < 0.02 &&
        Math.abs(a.s.at - a.start.y) < 0.02;
      if (trunk) continue;
      found.push(`${a.id} ∥ ${b.id} on ${a.s.kind}=${a.s.at} over [${lo}, ${hi}]`);
    }
  }
  return found;
}

/**
 * Switch to trace mode and check every wire against the contract; returns
 * the traces. `spline` is the same wires as drawn in spline mode, for the
 * stroke comparison.
 */
async function assertTraces(page: Page, spline: ReadonlyMap<string, Wire>): Promise<Map<string, Wire>> {
  const unit = await unitPx(page);
  await setWireMode(page, "trace");
  await expect
    .poll(async () => [...(await wires(page)).values()].every((w) => !/[CQcq]/.test(w.d)))
    .toBe(true);
  const trace = await wires(page);
  expect([...trace.keys()].sort()).toEqual([...spline.keys()].sort());
  const paths = new Map<string, Pt[]>();
  let corners = 0;
  let fullCorners = 0;
  for (const [id, w] of trace) {
    const before = spline.get(id)!;
    expect(w.stroke, `${id}: stroke colour`).toBe(before.stroke);
    expect(w.width, `${id}: stroke width`).toBe(before.width);
    const points = parsePath(w.d);
    paths.set(id, points);
    const segs = segments(points, id);
    for (let i = 0; i < segs.length; i += 1) {
      const s = segs[i]!;
      if (s.kind !== "d") continue;
      const prev = segs[i - 1];
      const next = segs[i + 1];
      // A diagonal between a horizontal and a vertical run is a corner cut:
      // its legs are one unit — shrunken toward the half-unit floor only in
      // a gap too narrow for full legs and the lanes (docs/16). A jog's lone
      // diagonal, between two horizontals, is as tall as the jog.
      if (prev !== undefined && next !== undefined && prev.kind !== "d" && next.kind !== "d" && prev.kind !== next.kind) {
        expect(Math.abs(s.dx), `${id}: corner leg`).toBeGreaterThanOrEqual(unit / 2 - 0.05);
        expect(Math.abs(s.dx), `${id}: corner leg`).toBeLessThanOrEqual(unit + 0.05);
        corners += 1;
        if (Math.abs(Math.abs(s.dx) - unit) < 0.05) fullCorners += 1;
      }
    }
  }
  expect(corners, "the graph has corners to check").toBeGreaterThan(0);
  expect(fullCorners, "most corners have full one-unit legs").toBeGreaterThan(corners / 2);
  expect(overlaps(paths)).toEqual([]);
  return trace;
}

test.describe.configure({ mode: "serial" });

test("U6 — 06-lists in trace mode: orthogonal runs, one-unit 45° corners, lanes apart, the strokes of spline mode", async ({ page }, testInfo) => {
  await open(page, "06-lists.cic", 19);
  await setWireMode(page, "spline");
  const spline = await wires(page);
  expect(spline.size).toBeGreaterThan(10);
  // Spline mode really is bezier: every path has a cubic.
  for (const [id, w] of spline) expect(w.d, id).toMatch(/C/);

  const trace = await assertTraces(page, spline);

  // A re-render moves nothing: select a node (every edge re-renders with the
  // selection) and read the paths again.
  await page.locator(".react-flow__node").first().click();
  await expect(page.locator(".react-flow__node.selected")).toHaveCount(1);
  const again = await wires(page);
  for (const [id, w] of trace) expect(again.get(id)?.d, id).toBe(w.d);

  await page.screenshot({ path: testInfo.outputPath("traces-06-lists.png") });
  // The same at the near zoom tier, where the cuts and lanes are legible
  // (the heights → sorted_heights bundle is under the pane's centre).
  await zoomToNear(page);
  await page.screenshot({ path: testInfo.outputPath("traces-06-lists-near.png") });
  await setWireMode(page, "spline");
});

/** Wheel over the canvas centre until its zoom LOD tier is `near`; throws if it never gets there. */
async function zoomToNear(page: Page): Promise<void> {
  const canvas = page.locator(".cicada-canvas");
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await page.mouse.move(box.x + box.width * 0.45, box.y + box.height * 0.4);
  for (let i = 0; i < 40; i += 1) {
    const at = await canvas.getAttribute("data-lod");
    if (at === "near") return;
    await page.mouse.wheel(0, at === "closest" ? 190 : -190);
  }
  throw new Error(`the canvas never reached the near zoom tier (at ${await canvas.getAttribute("data-lod")})`);
}

test("U6 — the in-flight connection line is a trace in trace mode", async ({ page }) => {
  await open(page, "06-lists.cic", 19);
  await setWireMode(page, "trace");
  const unit = await unitPx(page);
  const zoom = await page.evaluate(() => {
    const viewport = document.querySelector(".react-flow__viewport") as HTMLElement | null;
    const match = /scale\(([\d.]+)\)/.exec(viewport?.style.transform ?? "");
    if (match === null) throw new Error("no canvas zoom");
    return Number(match[1]);
  });
  const handle = page.locator(".react-flow__node .react-flow__handle-right").first();
  const box = await handle.boundingBox();
  if (box === null) throw new Error("no source handle");
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  // Four units right and three down, in flow units: a Z (within the stair threshold, a tall enough drop for a vertical run).
  await page.mouse.move(x + 4 * unit * zoom, y + 3 * unit * zoom, { steps: 8 });
  const line = page.locator("path.cicada-connection-path");
  await expect(line).toBeVisible();
  const d = await line.getAttribute("d");
  if (d === null) throw new Error("no connection path");
  const segs = segments(parsePath(d), "connection");
  expect(segs.map((s) => s.kind)).toEqual(["h", "d", "v", "d", "h"]);
  // Back onto its own handle: the drop does nothing (no search box, no wire).
  await page.mouse.move(x, y, { steps: 4 });
  await page.mouse.up();
  await expect(page.locator(".cv-search")).toHaveCount(0);
  await setWireMode(page, "spline");
});

test("U6 — 07-simple-cad in trace mode (the denser picture)", async ({ page }, testInfo) => {
  await open(page, "07-simple-cad.cic", 40);
  await setWireMode(page, "spline");
  const spline = await wires(page);
  expect(spline.size).toBeGreaterThan(20);
  await assertTraces(page, spline);
  await page.screenshot({ path: testInfo.outputPath("traces-07-simple-cad.png") });
  await setWireMode(page, "spline");
});
