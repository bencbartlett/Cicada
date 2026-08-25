/**
 * Wave 4 B1 (docs/17 §Wave 4; findings U4, U5, U7) in the real app:
 *
 *   - the grid tokens are the docs/16 §Theme values (halfway to `--bg`)
 *     in both themes, as the page computes them;
 *   - the gimbal is drawn in the viewport's upper-left — its three hues
 *     are in that corner of the WebGL canvas's own pixels, and nowhere
 *     else along the top edge — and it follows the camera: an orbit turns
 *     the axis directions `scene().gimbal` reports (the GIMBAL camera's
 *     pose, `Gimbal.directions()`) AND moves the red pixels' centroid;
 *   - the output value summaries show on every tier that shows the face
 *     (`near`, from zoom 0.35) and not on the title-only `far` tier — two
 *     visible states, nothing in between (U7, then U18);
 *   - the port handles sit at the same place on the node at `far` as at
 *     `near`, so the wires still meet them (U19).
 *
 * Evidence files (the whole page, dark and light, and the viewport's own
 * PNG) land in this test's output dir under `web/test-results/`.
 */
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { writeFileSync } from "node:fs";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

/** docs/16 §Theme (2026-08-24). */
const GRID_TOKENS = {
  dark: { "--grid": "#1b1e23", "--grid-strong": "#202329", "--bg": "#15171b" },
  light: { "--grid": "#eef0f4", "--grid-strong": "#e9ebf0", "--bg": "#f4f5f7" },
} as const;

/** `gimbal.ts`: GIMBAL_MARGIN_PX + GIMBAL_SIZE_PX, in CSS px from the canvas's top-left. */
const GIMBAL = { left: 6, top: 56, size: 72 } as const;

interface SceneStats {
  framesReceived: number;
  renders: number;
  gimbal: { x: number[]; y: number[]; z: number[] };
}

async function scene(page: Page): Promise<SceneStats> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { scene: (() => unknown) | null } };
    if (w.__cicada.scene === null) throw new Error("viewport not mounted");
    return w.__cicada.scene() as SceneStats;
  });
}

async function computedTokens(page: Page, names: readonly string[]): Promise<Record<string, string>> {
  return page.evaluate((wanted) => {
    const style = getComputedStyle(document.documentElement);
    return Object.fromEntries(wanted.map((name) => [name, style.getPropertyValue(name).trim()]));
  }, names);
}

async function setTheme(page: Page, theme: "dark" | "light"): Promise<void> {
  await page.evaluate((t) => {
    const w = window as unknown as {
      __cicada: { state: () => { updateSettings: (patch: { theme: "dark" | "light" }) => void } };
    };
    w.__cicada.state().updateSettings({ theme: t });
  }, theme);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

interface HueCounts {
  red: number;
  green: number;
  blue: number;
  total: number;
}

/**
 * Render the viewport to PNG (`__cicada.screenshot()`, the `/debug/screenshot`
 * path) and count strongly red / green / blue pixels inside a CSS-px
 * rectangle of it — the axis hues. The PNG is at the drawing buffer's
 * resolution (CSS px × devicePixelRatio). Returns the counts, the centroid
 * of the strongly-red pixels (the X bar and its disc; CSS px from the
 * rectangle's top-left, `null` when there are none) and the PNG.
 */
async function axisHuesIn(
  page: Page,
  rect: { left: number; top: number; width: number; height: number },
): Promise<{ counts: HueCounts; redCentroid: [number, number] | null; png: Buffer }> {
  const result = await page.evaluate(async (r) => {
    const w = window as unknown as { __cicada: { screenshot: () => Promise<Blob> } };
    const blob = await w.__cicada.screenshot();
    const bitmap = await createImageBitmap(blob);
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d");
    if (context === null) throw new Error("no 2D context");
    context.drawImage(bitmap, 0, 0);
    const dpr = window.devicePixelRatio || 1;
    const x = Math.round(r.left * dpr);
    const y = Math.round(r.top * dpr);
    const width = Math.min(Math.round(r.width * dpr), canvas.width - x);
    const height = Math.min(Math.round(r.height * dpr), canvas.height - y);
    const data = context.getImageData(x, y, width, height).data;
    const counts = { red: 0, green: 0, blue: 0, total: width * height };
    let redX = 0;
    let redY = 0;
    // "Strongly X": the channel leads the other two by a wide margin — the
    // grey ground and the solids' muted node colors never qualify.
    for (let i = 0, p = 0; i < data.length; i += 4, p += 1) {
      const [rr, gg, bb] = [data[i]!, data[i + 1]!, data[i + 2]!];
      if (rr > gg + 60 && rr > bb + 60) {
        counts.red += 1;
        redX += p % width;
        redY += Math.floor(p / width);
      } else if (gg > rr + 50 && gg > bb + 50) counts.green += 1;
      else if (bb > rr + 50 && bb > gg + 50) counts.blue += 1;
    }
    const redCentroid: [number, number] | null =
      counts.red === 0 ? null : [redX / counts.red / dpr, redY / counts.red / dpr];
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return { counts, redCentroid, png: btoa(binary) };
  }, rect);
  return { counts: result.counts, redCentroid: result.redCentroid, png: Buffer.from(result.png, "base64") };
}

async function open(page: Page): Promise<void> {
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  // 02-solids has 12 bindings; the suite's earlier specs (smoke) may have
  // placed more into the shared scratch copy — the count is not ours to pin.
  await expect.poll(async () => page.locator(".react-flow__node").count()).toBeGreaterThanOrEqual(12);
  await expect.poll(async () => (await scene(page)).framesReceived, { timeout: 20_000 }).toBeGreaterThan(0);
  await expect.poll(async () => (await scene(page)).renders).toBeGreaterThan(0);
}

/** A whole-page screenshot into the test's output dir (`web/test-results/…`). */
async function evidence(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  await page.screenshot({ path: testInfo.outputPath(name), fullPage: false });
}

/**
 * The viewport's own PNG as an evidence FILE (the list reporter keeps
 * body-only attachments in memory; a file survives the run).
 */
async function viewportEvidence(testInfo: TestInfo, name: string, png: Buffer): Promise<void> {
  const path = testInfo.outputPath(name);
  writeFileSync(path, png);
  await testInfo.attach(name, { path, contentType: "image/png" });
}

test.describe.configure({ mode: "serial" });

test("U4 — the grid tokens sit halfway to the background in both themes", async ({ page }, testInfo) => {
  await open(page);
  for (const theme of ["dark", "light"] as const) {
    await setTheme(page, theme);
    const tokens = await computedTokens(page, Object.keys(GRID_TOKENS[theme]));
    expect(tokens, theme).toEqual(GRID_TOKENS[theme]);
    await evidence(page, testInfo, `grid-${theme}.png`);
  }
  await setTheme(page, "dark");
});

test("U5 — the gimbal is drawn in the viewport's upper-left and follows the camera", async ({ page }, testInfo) => {
  await open(page);
  const viewport = page.getByTestId("viewport-canvas");
  const box = await viewport.boundingBox();
  if (box === null) throw new Error("no viewport canvas");
  expect(box.width).toBeGreaterThan(GIMBAL.left + GIMBAL.size + 50);

  // The default pose looks at the origin from above with Z up: Z reads
  // straight up on screen (no sideways lean), X and Y are not degenerate.
  const before = (await scene(page)).gimbal;
  expect(before.z[0]).toBeCloseTo(0, 6);
  expect(before.z[1]).toBeGreaterThan(0.7);
  expect(Math.hypot(before.x[0]!, before.x[1]!)).toBeGreaterThan(0.3);
  expect(Math.hypot(before.y[0]!, before.y[1]!)).toBeGreaterThan(0.3);

  // The hues are in the gimbal's square …
  const square = { left: GIMBAL.left, top: GIMBAL.top, width: GIMBAL.size, height: GIMBAL.size };
  const inside = await axisHuesIn(page, square);
  await viewportEvidence(testInfo, "viewport-gimbal.png", inside.png);
  expect(inside.counts.red, JSON.stringify(inside.counts)).toBeGreaterThan(20);
  expect(inside.counts.green, JSON.stringify(inside.counts)).toBeGreaterThan(20);
  expect(inside.counts.blue, JSON.stringify(inside.counts)).toBeGreaterThan(20);
  // … and not in the same-sized square to its right along the top edge,
  // which holds only the ground and the scene (the solids wear muted
  // node colors; the ground triad at the origin sits far from the top). A
  // triad drawn without the scissor — at the full viewport's scale — would
  // put hundreds there; a handful tolerates another GL's anti-aliasing.
  const beside = { left: GIMBAL.left + GIMBAL.size + 20, top: GIMBAL.top, width: GIMBAL.size, height: GIMBAL.size };
  const outside = await axisHuesIn(page, beside);
  expect(outside.counts.red + outside.counts.green + outside.counts.blue, JSON.stringify(outside.counts)).toBeLessThan(5);
  await evidence(page, testInfo, "gimbal-dark.png");

  // Orbit (Rhino preset: right button) a quarter of the width to the right:
  // the camera swings about Z, the X/Y directions turn, Z stays up. Two
  // oracles, both of the GIMBAL (review finding 2026-08-24 — the directions
  // used to come from the main camera, which turns whether or not the
  // gimbal does, and nothing asserted the drawn pixels moved: a gimbal
  // frozen at its first pose stayed green): `scene().gimbal` is the pose of
  // the gimbal's own camera, and the red centroid is the drawing.
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(cx + box.width / 4, cy, { steps: 10 });
  await page.mouse.up({ button: "right" });
  await expect
    .poll(async () => {
      const after = (await scene(page)).gimbal;
      return Math.hypot(after.x[0]! - before.x[0]!, after.x[1]! - before.x[1]!);
    })
    .toBeGreaterThan(0.2);
  const after = (await scene(page)).gimbal;
  expect(after.z[1]).toBeGreaterThan(0.5);
  for (const axis of [after.x, after.y, after.z]) expect(Math.hypot(...axis)).toBeCloseTo(1, 6);
  // The drawing followed: the hues are still in the square, and the red
  // (X) pixels MOVED — the centroid of the X bar + disc shifts with the
  // axis. The X direction's screen component changed by > 0.2 (above) and
  // the red mass sits ~24 CSS px out from the triad's centre, so the
  // centroid moves by > ~5 CSS px; 4 is the bound, DPR-independent (the
  // centroid is in CSS px).
  const turned = await axisHuesIn(page, square);
  expect(turned.counts.red).toBeGreaterThan(20);
  expect(turned.counts.green).toBeGreaterThan(20);
  expect(turned.counts.blue).toBeGreaterThan(20);
  if (inside.redCentroid === null || turned.redCentroid === null) throw new Error("no red pixels in the gimbal square");
  const moved = Math.hypot(
    turned.redCentroid[0] - inside.redCentroid[0],
    turned.redCentroid[1] - inside.redCentroid[1],
  );
  expect(moved, `red centroid ${JSON.stringify(inside.redCentroid)} → ${JSON.stringify(turned.redCentroid)}`).toBeGreaterThan(4);
  // The numbers behind the two oracles, as evidence beside the PNGs.
  writeFileSync(
    testInfo.outputPath("gimbal-follow.json"),
    JSON.stringify({ directions: { before, after }, redCentroidCssPx: { before: inside.redCentroid, after: turned.redCentroid, moved } }, null, 2),
  );
  await viewportEvidence(testInfo, "viewport-gimbal-orbited.png", turned.png);

  // The light theme recolors it (darker hues on a light ground) and keeps it in place.
  await setTheme(page, "light");
  await expect.poll(async () => (await axisHuesIn(page, square)).counts.blue).toBeGreaterThan(20);
  const light = await axisHuesIn(page, square);
  expect(light.counts.red).toBeGreaterThan(20);
  expect(light.counts.green).toBeGreaterThan(20);
  await evidence(page, testInfo, "gimbal-light.png");
  await setTheme(page, "dark");
});

const TIER_ORDER = ["far", "near", "closest"] as const;
type Tier = (typeof TIER_ORDER)[number];

/** Wheel over the canvas until its zoom LOD tier (`data-lod`) is `tier`; throws if it never gets there. */
async function zoomToTier(page: Page, tier: Tier): Promise<void> {
  const canvas = page.locator(".cicada-canvas");
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 60; i += 1) {
    const at = (await canvas.getAttribute("data-lod")) as Tier | null;
    if (at === tier) return;
    // Small steps (×1.3 per notch) so the near band (0.35 – 1.6) is never
    // jumped over.
    const below = at !== null && TIER_ORDER.indexOf(at) < TIER_ORDER.indexOf(tier);
    await page.mouse.wheel(0, below ? -190 : 190);
  }
  throw new Error(`the canvas never reached the ${tier} zoom tier (at ${await canvas.getAttribute("data-lod")})`);
}

/** The canvas zoom factor, read off React Flow's viewport transform. */
async function canvasZoom(page: Page): Promise<number> {
  const zoom = await page.evaluate(() => {
    const viewport = document.querySelector(".react-flow__viewport") as HTMLElement | null;
    const match = /scale\(([\d.]+)\)/.exec(viewport?.style.transform ?? "");
    return match === null ? null : Number(match[1]);
  });
  if (zoom === null) throw new Error("no canvas zoom");
  return zoom;
}

/**
 * Every port handle's centre in NODE units (canvas px at zoom 1, relative
 * to its node's top-left), keyed by `node.port` — the places the wires are
 * drawn to. Read at one tier and compared at another.
 */
async function handlePlaces(page: Page): Promise<Record<string, [number, number]>> {
  const zoom = await canvasZoom(page);
  return page.evaluate((z) => {
    const out: Record<string, [number, number]> = {};
    for (const node of Array.from(document.querySelectorAll(".cn[data-node]")) as HTMLElement[]) {
      const box = node.getBoundingClientRect();
      for (const handle of Array.from(node.querySelectorAll(".react-flow__handle")) as HTMLElement[]) {
        const h = handle.getBoundingClientRect();
        const key = `${node.dataset.node}.${handle.dataset.handleid ?? handle.dataset.port ?? "?"}.${handle.classList.contains("source") ? "out" : "in"}`;
        out[key] = [
          Math.round(((h.left + h.width / 2 - box.left) / z) * 10) / 10,
          Math.round(((h.top + h.height / 2 - box.top) / z) * 10) / 10,
        ];
      }
    }
    return out;
  }, zoom);
}

test("U7 · U18 · U19 — the face shows its values from the first full tier; the title-only tier keeps the handles in place", async ({
  page,
}, testInfo) => {
  await open(page);
  // Every binding solved (the summaries are fetched for solved nodes only).
  const state = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(state.ok()).toBeTruthy();

  // The full face, with its values, at the near tier …
  await zoomToTier(page, "near");
  const values = page.locator(".cn-port-value");
  await expect.poll(async () => values.count()).toBeGreaterThan(0);
  // The summaries arrive (an `inspect` per visible solved node): at least
  // one shows a value, not the `—` placeholder.
  await expect.poll(async () => (await values.allTextContents()).filter((t) => t !== "—").length).toBeGreaterThan(0);
  const nearZoom = await canvasZoom(page);
  expect(nearZoom, "the canvas zoom is inside the near band").toBeGreaterThanOrEqual(0.35);
  expect(nearZoom).toBeLessThan(1.6);
  await expect(page.locator(".cn-port-label").first()).toBeVisible();
  await evidence(page, testInfo, "values-at-near.png");
  const atNear = await handlePlaces(page);
  expect(Object.keys(atNear).length).toBeGreaterThan(4);

  // … and the title alone at far: no values, no labels — and every handle
  // exactly where it was, in node units, so the wires still meet the dots.
  await zoomToTier(page, "far");
  await expect(page.locator(".cn-port-value")).toHaveCount(0);
  await expect(page.locator(".cn-port-label").first()).toBeHidden();
  await evidence(page, testInfo, "title-only-at-far.png");
  const atFar = await handlePlaces(page);
  expect(Object.keys(atFar).sort()).toEqual(Object.keys(atNear).sort());
  for (const [key, [x, y]] of Object.entries(atNear)) {
    const [fx, fy] = atFar[key]!;
    expect(Math.abs(fx - x), `${key} x moved between tiers`).toBeLessThanOrEqual(1);
    expect(Math.abs(fy - y), `${key} y moved between tiers`).toBeLessThanOrEqual(1);
  }

  // Back to the face: the values return at once (nothing in between).
  await zoomToTier(page, "near");
  await expect.poll(async () => values.count()).toBeGreaterThan(0);
});
