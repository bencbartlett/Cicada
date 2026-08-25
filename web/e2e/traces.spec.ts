/**
 * Wave 4 B2 (docs/17 §Wave 4; finding U6) in the real app: trace mode's
 * wires are PCB traces from our own router (`src/canvas/trace.ts`), not
 * React Flow's smooth-step path —
 *
 *   - every wire is `M`/`L` runs only, each horizontal, vertical or 45°;
 *   - every 45° cut between a horizontal and a vertical run — the cuts at a
 *     wire's ends included — has legs of one grid unit (`hello.unitPx`),
 *     shrunken no further than the half-unit floor in a tight gap;
 *   - no two parallel runs of different wires coincide, except the stub
 *     wires out of ONE port share (the trunk, pinned to the port's row);
 *   - the router reports no collapsed lane (`data-trace-collapsed` = 0) and
 *     no edge drew its fallback route (`data-trace-yield` absent);
 *   - stroke colour and width are the spline mode's, wire for wire;
 *   - a re-render (selecting a node) moves nothing;
 *   - the in-flight connection line is a trace too.
 *
 * The oracle is `traceOracle.ts`, shared with the router's unit test and
 * with `wall_traces.spec.ts` (the wall, in its own file so it runs last).
 * Evidence: whole-page screenshots of `06-lists` and `07-simple-cad` in
 * trace mode land in this test's output dir under `web/test-results/`.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";
import { cornerLegs, overlaps, parsePath, segments, type Pt } from "./traceOracle";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;

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
    // Every corner cut's legs are one unit — shrunken toward the half-unit
    // floor only in a gap too narrow for full legs and the lanes (docs/16);
    // the cuts at a wire's very ends (a stub consumed whole) included.
    for (const leg of cornerLegs(segments(points, id))) {
      expect(leg, `${id}: corner leg`).toBeGreaterThanOrEqual(unit / 2 - 0.05);
      expect(leg, `${id}: corner leg`).toBeLessThanOrEqual(unit + 0.05);
      corners += 1;
      if (Math.abs(leg - unit) < 0.05) fullCorners += 1;
    }
  }
  expect(corners, "the graph has corners to check").toBeGreaterThan(0);
  expect(fullCorners, "most corners have full one-unit legs").toBeGreaterThan(corners / 2);
  expect(overlaps(paths)).toEqual([]);
  // The router's two fallbacks are marked, never silent — and never taken here.
  await expect(page.locator(".cicada-canvas")).toHaveAttribute("data-trace-collapsed", "0");
  await expect(page.locator("g.cicada-edge[data-trace-yield]")).toHaveCount(0);
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
  // Spline mode carries no trace counters.
  await expect(page.locator(".cicada-canvas")).not.toHaveAttribute("data-trace-collapsed", /.*/);

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
  // Five units right and five down, in flow units: a Z — inside the staircase
  // threshold (TRACE_STAIRCASE_UNITS = 6) with a drop that clears two
  // one-unit corner cuts AND a vertical leg whatever the fit zoom. Three
  // units down used to pass on the 19-node 06-lists; at the 31-node layout's
  // smaller zoom (catalog C2b's pegboard, 2026-08-24) it reached the router
  // as 1.75 units and the route was, correctly, one diagonal.
  await page.mouse.move(x + 5 * unit * zoom, y + 5 * unit * zoom, { steps: 8 });
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
