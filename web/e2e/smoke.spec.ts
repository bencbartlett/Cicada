/**
 * The stage-5 smoke (doc 15 DoD): serve → place → wire → drag → screenshot
 * asserts geometry changed — headless, through the real UI where the UI
 * exists (search-to-place, wire drag, slider drag), with `/debug/state`
 * and `window.__cicada` as the oracles. The server is started by
 * `playwright.config.ts` over a scratch copy of `examples/`.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

interface DebugState {
  seq: number;
  text: string;
  graph: { nodes: { name: string; cell: [number, number]; manual: boolean; preview: boolean }[]; wires: unknown[] };
  statuses: Record<string, { state: string; message?: string }>;
  summary: { generation: number; running: boolean; red: number; blocked: number };
  display: Record<string, { hash: string; stats: { triangles: number; bounds: [number[], number[]] | null } }>;
}

async function debugState(page: Page, wait = true): Promise<DebugState> {
  const response = await page.request.get(
    `/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=${wait}`,
  );
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface SceneStats {
  bounds: [number[], number[]] | null;
  outputs: Record<string, { triangles: number }>;
  drawCalls: number;
  framesReceived: number;
}

async function scene(page: Page): Promise<SceneStats> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { scene: (() => unknown) | null } };
    if (w.__cicada.scene === null) throw new Error("viewport not mounted");
    return w.__cicada.scene() as SceneStats;
  });
}

async function viewportPng(page: Page): Promise<Buffer> {
  const base64 = await page.evaluate(async () => {
    const w = window as unknown as { __cicada: { screenshot: () => Promise<Blob> } };
    const blob = await w.__cicada.screenshot();
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  });
  return Buffer.from(base64, "base64");
}

test.describe.configure({ mode: "serial" });

test("serve → load → place → wire → drag → screenshot asserts geometry changed", async ({
  page,
}, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();

  // ---- load: the canvas shows every binding, the viewport draws the solids.
  // 10 = the bindings of examples/02-solids.cic (the served scratch copy);
  // a change to that example updates this count in the same commit.
  const initial = await debugState(page);
  expect(initial.graph.nodes.length).toBe(10);
  await expect(page.locator(".react-flow__node")).toHaveCount(10);
  await expect
    .poll(async () => (await scene(page)).framesReceived, { timeout: 20_000 })
    .toBeGreaterThan(0);
  await expect
    .poll(async () => Object.values((await scene(page)).outputs).reduce((n, o) => n + o.triangles, 0))
    .toBeGreaterThan(500);
  const before = await viewportPng(page);
  await testInfo.attach("viewport-before.png", { body: before, contentType: "image/png" });
  const boundsBefore = (await scene(page)).bounds;
  expect(boundsBefore).not.toBeNull();

  // ---- place: search-to-place (double-click the canvas, type, Enter).
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await pane.dblclick({ position: { x: box.width * 0.55, y: box.height * 0.8 } });
  const search = page.getByTestId("search-input");
  await expect(search).toBeVisible();
  await search.fill("sphere");
  await search.press("Enter");
  await expect(page.locator(".react-flow__node[data-id='sphere_1']")).toBeVisible();
  const placed = await debugState(page);
  expect(placed.text).toContain("sphere_1 = sphere()");
  expect(placed.statuses["sphere_1"]?.state).toBe("red"); // radius unwired: honest red

  // ---- wire: drag size.out → sphere_1.radius through the handles.
  const source = page.locator(
    ".react-flow__node[data-id='size'] .react-flow__handle.source",
  );
  const target = page.locator(
    ".react-flow__node[data-id='sphere_1'] .react-flow__handle.target[data-handleid='radius']",
  );
  await source.hover();
  await page.mouse.down();
  const t = await target.boundingBox();
  if (t === null) throw new Error("no radius handle");
  await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2, { steps: 12 });
  await page.mouse.up();
  await expect
    .poll(async () => (await debugState(page)).text)
    .toContain("sphere_1 = sphere(radius=size)");
  // Poll for the solve, don't read once: `/debug/state?wait=true` settles a
  // fresh node to "done" within a generation, but a project-watcher reload
  // (the roundtrip spec shares this server's watched dir) can momentarily
  // re-seed it to "queued", so a single read right after the text lands is
  // racy under CI load.
  await expect
    .poll(async () => {
      const state = (await debugState(page)).statuses["sphere_1"]?.state;
      // "cached" (a memo hit on a re-solve) is as solved as "done".
      return state === "done" || state === "cached";
    })
    .toBe(true);
  const wired = await debugState(page);
  expect(wired.display["sphere_1.out"]?.stats.triangles ?? 0).toBeGreaterThan(0);

  // ---- drag: the size slider on the canvas (previews stream, release commits).
  const slider = page.getByTestId("slider-size");
  await expect(slider).toBeVisible();
  const s = await slider.boundingBox();
  if (s === null) throw new Error("no slider");
  await page.mouse.move(s.x + s.width * 0.3, s.y + s.height / 2);
  await page.mouse.down();
  await page.mouse.move(s.x + s.width * 0.9, s.y + s.height / 2, { steps: 15 });
  await page.mouse.up();
  await expect
    .poll(async () => (await debugState(page)).text)
    .not.toContain("size = slider(value=2.0,");
  const dragged = await debugState(page);
  const newValue = /size = slider\(value=([0-9.]+),/.exec(dragged.text)?.[1];
  expect(Number(newValue)).toBeGreaterThan(2.0);
  expect(dragged.display["block.out"]?.stats.bounds?.[1][0]).toBeCloseTo(Number(newValue), 6);

  // ---- screenshot asserts geometry changed: viewport bounds and pixels.
  await expect
    .poll(async () => (await scene(page)).bounds?.[1][0] ?? 0)
    .toBeGreaterThan(boundsBefore![1][0]!);
  const after = await viewportPng(page);
  await testInfo.attach("viewport-after.png", { body: after, contentType: "image/png" });
  expect(after.equals(before)).toBeFalsy();
  await page.screenshot({ path: testInfo.outputPath("page-after.png"), fullPage: false });

  // ---- backward picking: click the geometry → the node lights up on the canvas.
  const viewport = page.getByTestId("viewport-pane");
  const vb = await viewport.boundingBox();
  if (vb === null) throw new Error("no viewport");
  await page.mouse.click(vb.x + vb.width / 2, vb.y + vb.height / 2);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const w = window as unknown as {
          __cicada: { state: () => { selection: { nodes: string[]; element: unknown } } };
        };
        return w.__cicada.state().selection.nodes.length;
      }),
    )
    .toBeGreaterThan(0);

  expect(errors, errors.join("\n")).toEqual([]);
});
