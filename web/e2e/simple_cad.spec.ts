/**
 * The B-rep consumer (docs/17 item 3 WP-D): `examples/07-simple-cad.cic`
 * — a bracket built from OCCT-backed Solids — opens in the app, every
 * binding solves, the Solids DISPLAY (mesh frames from the session's
 * tessellation cache reach the viewport) and their bounds are the
 * bracket's. Oracles: `/debug/state` (statuses, per-output display
 * stats, the Solid cache counters) and `window.__cicada.scene()`.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "07-simple-cad.cic";

interface DebugState {
  statuses: Record<string, { state: string; message?: string }>;
  summary: {
    generation: number;
    running: boolean;
    red: number;
    blocked: number;
  };
  display: Record<
    string,
    {
      hash: string;
      stats: {
        triangles: number;
        bounds: [number[], number[]] | null;
        solids?: number;
        errors?: string[];
      };
    }
  >;
  display_cache: {
    entries: number;
    bytes: number;
    hits: number;
    misses: number;
  };
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(
    `/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`,
  );
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface SceneStats {
  bounds: [number[], number[]] | null;
  outputs: Record<string, { triangles: number }>;
  framesReceived: number;
}

async function scene(page: Page): Promise<SceneStats> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __cicada: { scene: (() => unknown) | null };
    };
    if (w.__cicada.scene === null) throw new Error("viewport not mounted");
    return w.__cicada.scene() as SceneStats;
  });
}

test("07-simple-cad: the bracket's Solids solve, display and bound the part", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();

  // Every binding is green (the exporter is skipped, never red).
  const state = await debugState(page);
  expect(state.summary.red, JSON.stringify(state.statuses)).toBe(0);
  expect(state.summary.blocked).toBe(0);
  for (const name of [
    "plate",
    "boss",
    "rib",
    "blank",
    "bracket",
    "hull",
    "outline",
    "material",
  ]) {
    expect(state.statuses[name]?.state, name).toBe("done");
  }

  // The bracket is a Solid drawn through the tessellation cache: its
  // display stats count a solid, carry triangles, and bound the part —
  // 80 × 50 in plan, thickness 8 + boss 20 = 28 high at the defaults.
  const bracket = state.display["bracket.out"];
  if (bracket === undefined)
    throw new Error(
      `no display for bracket.out: ${Object.keys(state.display).join(", ")}`,
    );
  expect(bracket.stats.solids).toBe(1);
  expect(bracket.stats.errors ?? []).toEqual([]);
  expect(bracket.stats.triangles).toBeGreaterThan(100);
  const [min, max] = bracket.stats.bounds ?? [[], []];
  expect(min[0]).toBeCloseTo(0, 5);
  expect(min[1]).toBeCloseTo(0, 5);
  expect(min[2]).toBeCloseTo(0, 5);
  expect(max[0]).toBeCloseTo(80, 5);
  expect(max[1]).toBeCloseTo(50, 5);
  expect(max[2]).toBeCloseTo(28, 5);
  // The bounding box node agrees with the display bounds.
  const hull = state.display["hull.out"];
  if (hull === undefined) throw new Error("no display for hull.out");
  expect(hull.stats.bounds).toEqual(bracket.stats.bounds);
  // The cache did the work: entries for the displayed Solids, no errors.
  expect(state.display_cache.entries).toBeGreaterThan(0);
  expect(state.display_cache.misses).toBeGreaterThan(0);

  // And the frames reached the viewport: triangles on screen, the scene's
  // bounds holding the bracket.
  await expect
    .poll(async () => (await scene(page)).framesReceived, { timeout: 20_000 })
    .toBeGreaterThan(0);
  await expect
    .poll(async () =>
      Object.values((await scene(page)).outputs).reduce(
        (n, o) => n + o.triangles,
        0,
      ),
    )
    .toBeGreaterThan(100);
  const view = await scene(page);
  expect(view.bounds).not.toBeNull();
  const [sceneMin, sceneMax] = view.bounds ?? [[], []];
  expect(sceneMin[0]).toBeLessThanOrEqual(0 + 1e-6);
  expect(sceneMax[0]).toBeGreaterThanOrEqual(80 - 1e-6);
  expect(sceneMax[2]).toBeGreaterThanOrEqual(28 - 1e-6);

  expect(errors, errors.join("\n")).toEqual([]);
});
