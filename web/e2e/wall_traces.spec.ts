/**
 * Wave 4 B2 (docs/17 §Wave 4; finding U6) on the wall — the project Ben
 * uses, and the one the B2 review found the first router failing on: in
 * trace mode `examples/wall/wall.cic` had five wires stacked on one lane of
 * its busiest three-unit gap (22 vertical runs between the `labels` column
 * and `glyphs`/`plates`/`dxf`). The same oracle as `traces.spec.ts`
 * (`traceOracle.ts`), on the wall:
 *
 *   - every wire is `M`/`L` runs only, each horizontal, vertical or 45°;
 *   - no two parallel runs of different wires coincide (the trunk out of
 *     one port excepted);
 *   - the router reports no collapsed lane (`data-trace-collapsed` = 0) and
 *     no edge drew its fallback route (`data-trace-yield` absent);
 *   - every corner cut's legs, the cuts at the wires' ends included, are
 *     between the half-unit floor and a unit.
 *
 * Evidence: a whole-page screenshot of the wall in trace mode, and one at
 * the near zoom tier over the busiest gap, in this test's output dir.
 *
 * Its own file, named to run LAST: opening the wall starts its carve on
 * the shared engine (minutes on a debug engine with two threads), and the
 * canvas is what this spec reads — it does not wait for the solve — so the
 * carve still runs while whatever spec follows would be timing its own
 * solves. After this file, nothing follows.
 */
import { expect, test } from "@playwright/test";
import config from "../playwright.config";
import { cornerLegs, overlaps, parsePath, segments, type Pt } from "./traceOracle";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "wall/wall.cic";

// Wall-scale: opening the wall starts its 1,200-part carve and its ~350 MB
// display set on the suite's shared 2-thread engine, and the per-PR smoke's
// timing specs run on that same engine — so this spec runs where the other
// wall-scale spec runs: the nightly `Playwright heavy (wall)` job, which sets
// CICADA_E2E_HEAVY=1 (locally: the same variable).
test.skip(
  !process.env.CICADA_E2E_HEAVY,
  "wall-scale spec — run with CICADA_E2E_HEAVY=1 (the nightly heavy job, or locally)",
);

test("U6 — the wall in trace mode: every run orthogonal or 45°, no two parallel runs coinciding, no collapsed lane", async ({ page }, testInfo) => {
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  // 26 nodes, 72 wires (70 drawn: two enter ports the canvas does not show).
  await expect.poll(async () => page.locator(".react-flow__node").count()).toBeGreaterThanOrEqual(26);
  await expect.poll(async () => page.locator("g.cicada-edge").count()).toBeGreaterThanOrEqual(70);
  const unit = await page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { hello: { unitPx: number } | null } } };
    const hello = w.__cicada.state().hello;
    if (hello === null) throw new Error("no hello yet");
    return hello.unitPx;
  });
  await page.evaluate(() => {
    const w = window as unknown as {
      __cicada: { state: () => { updateSettings: (patch: { wireMode: "spline" | "trace" }) => void } };
    };
    w.__cicada.state().updateSettings({ wireMode: "trace" });
  });
  const canvas = page.locator(".cicada-canvas");
  await expect(canvas).toHaveAttribute("data-trace-collapsed", /^\d+$/);
  const read = async () =>
    page.evaluate(() => {
      const out: Record<string, string> = {};
      for (const g of Array.from(document.querySelectorAll("g.cicada-edge[data-wire]"))) {
        const path = g.querySelector("path.react-flow__edge-path");
        const id = g.getAttribute("data-wire");
        if (path === null || id === null) continue;
        out[id] = path.getAttribute("d") ?? "";
      }
      return out;
    });
  await expect.poll(async () => Object.values(await read()).every((d) => !/[CQcq]/.test(d))).toBe(true);
  const drawn = await read();
  expect(Object.keys(drawn).length).toBeGreaterThanOrEqual(70);

  const paths = new Map<string, Pt[]>();
  let corners = 0;
  for (const [id, d] of Object.entries(drawn)) {
    const points = parsePath(d);
    paths.set(id, points);
    for (const leg of cornerLegs(segments(points, id))) {
      expect(leg, `${id}: corner leg`).toBeGreaterThanOrEqual(unit / 2 - 0.05);
      expect(leg, `${id}: corner leg`).toBeLessThanOrEqual(unit + 0.05);
      corners += 1;
    }
  }
  expect(corners).toBeGreaterThan(100);
  expect(overlaps(paths)).toEqual([]);
  await expect(canvas).toHaveAttribute("data-trace-collapsed", "0");
  await expect(page.locator("g.cicada-edge[data-trace-yield]")).toHaveCount(0);

  await page.screenshot({ path: testInfo.outputPath("traces-wall.png") });
  // The busiest gap up close: zoom to the near tier over the `glyphs` node.
  const glyphs = page.locator(".react-flow__node", { has: page.locator("text=glyphs") }).first();
  const box = await glyphs.boundingBox();
  if (box === null) throw new Error("no glyphs node on the canvas");
  await page.mouse.move(box.x - unit, box.y + box.height / 2);
  for (let i = 0; i < 40 && (await canvas.getAttribute("data-lod")) !== "near"; i += 1) {
    await page.mouse.wheel(0, (await canvas.getAttribute("data-lod")) === "closest" ? 190 : -190);
  }
  expect(await canvas.getAttribute("data-lod")).toBe("near");
  await page.screenshot({ path: testInfo.outputPath("traces-wall-near.png") });
  await page.evaluate(() => {
    const w = window as unknown as {
      __cicada: { state: () => { updateSettings: (patch: { wireMode: "spline" | "trace" }) => void } };
    };
    w.__cicada.state().updateSettings({ wireMode: "spline" });
  });
});
