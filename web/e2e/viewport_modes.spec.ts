/**
 * The viewport modes (docs/16 §Viewport conventions; docs/17 wave 5 V1,
 * finding U16) against the REAL `cicada serve` from `playwright.config.ts`
 * over a scratch copy of `examples/`, on 02-solids:
 *
 *   - floating: the control's `floating` takes the splitter away and the
 *     canvas takes the whole work area; the viewport is a panel over it
 *     holding the SAME canvas element (no remount — a marker set on it
 *     before survives); its title strip drags it, its corner resizes it,
 *     the rect lands in the per-user settings and survives a reload; back
 *     to split restores the panes.
 *   - window: on this Chromium (a secure-context loopback URL exposes
 *     `documentPictureInPicture` headless) the viewport's element MOVES
 *     into the picture-in-picture window — the same element, the main
 *     window keeps only the placeholder, the scene still follows a write —
 *     and the placeholder brings it back; the PiP window closing on its
 *     own returns to split.
 *   - window without the API (stubbed out): the wave-4 observer pop-out
 *     opens with a notice saying so, and the mode stays put.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

interface SceneStats {
  bounds: [number[], number[]] | null;
  outputs: Record<string, { triangles: number }>;
  framesReceived: number;
  renders?: number;
}

async function scene(page: Page): Promise<SceneStats | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada?: { scene: (() => SceneStats) | null } };
    const read = w.__cicada?.scene;
    return read === null || read === undefined ? null : read();
  });
}

const triangles = (stats: SceneStats | null) => (stats === null ? 0 : Object.values(stats.outputs).reduce((n, o) => n + o.triangles, 0));

interface StoredSettings {
  viewportMode: string;
  floatingViewport: { x: number; y: number; width: number; height: number } | null;
}

async function storedSettings(page: Page): Promise<StoredSettings> {
  return page.evaluate(() => JSON.parse(localStorage.getItem("cicada.settings.v1") ?? "{}") as StoredSettings);
}

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

async function box(page: Page, testId: string): Promise<Box> {
  const b = await page.getByTestId(testId).boundingBox();
  if (b === null) throw new Error(`${testId} has no box`);
  return b;
}

async function debugText(page: Page): Promise<string> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return ((await response.json()) as { text: string }).text;
}

/** Load the app as the writer with geometry drawn, and mark the viewport's canvas element so a remount would be seen. */
async function loadDrawn(page: Page): Promise<void> {
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(async () => triangles(await scene(page)), { timeout: 20_000 }).toBeGreaterThan(500);
  await page.getByTestId("viewport-canvas").evaluate((el) => {
    (el as HTMLElement).dataset.marker = "same-canvas";
  });
}

test.describe.configure({ mode: "serial" });

test("floating: the panel over the canvas — the same scene, drag + resize persisted across a reload, back to split", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  await loadDrawn(page);

  // ---- split: two panes and a splitter; the control says so.
  await expect(page.locator(".splitter")).toHaveCount(1);
  await expect(page.getByTestId("viewport-mode-split")).toHaveAttribute("aria-checked", "true");
  const work = await box(page, "canvas-pane");
  const workArea = await page.locator(".app-work").boundingBox();
  expect(workArea).not.toBeNull();
  expect(work.height).toBeLessThan(workArea!.height - 100);

  // ---- floating: the canvas takes the whole work area, the panel floats over it, the canvas element is the same.
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-mode-floating")).toHaveAttribute("aria-checked", "true");
  await expect(page.locator(".splitter")).toHaveCount(0);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.getByTestId("viewport-float-title")).toBeVisible();
  await expect(page.getByTestId("viewport-float-corner")).toBeVisible();
  expect((await box(page, "canvas-pane")).height).toBeGreaterThan(workArea!.height - 2);
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  expect(triangles(await scene(page))).toBeGreaterThan(500);
  // The default panel: 40 % of the area each way, in the lower-right corner.
  const initial = await box(page, "viewport-pane");
  expect(initial.width).toBeCloseTo(Math.round(workArea!.width * 0.4), 0);
  expect(initial.height).toBeCloseTo(Math.round(workArea!.height * 0.4), 0);
  expect(initial.x + initial.width).toBeCloseTo(workArea!.x + workArea!.width - 12, 0);
  expect(initial.y + initial.height).toBeCloseTo(workArea!.y + workArea!.height - 12, 0);
  // The panel is inside the work area, so a pane test of the canvas still finds the canvas under it.
  expect(initial.x).toBeGreaterThanOrEqual(workArea!.x);

  // ---- drag the title strip by (-160, -120).
  const title = await box(page, "viewport-float-title");
  await page.mouse.move(title.x + 30, title.y + title.height / 2);
  await page.mouse.down();
  await page.mouse.move(title.x + 30 - 80, title.y + title.height / 2 - 60, { steps: 4 });
  await page.mouse.move(title.x + 30 - 160, title.y + title.height / 2 - 120, { steps: 4 });
  await page.mouse.up();
  const moved = await box(page, "viewport-pane");
  expect(moved.x).toBeCloseTo(initial.x - 160, 0);
  expect(moved.y).toBeCloseTo(initial.y - 120, 0);
  expect(moved.width).toBeCloseTo(initial.width, 0);
  expect(moved.height).toBeCloseTo(initial.height, 0);

  // ---- resize from the corner by (-60, -40): the place stays.
  const corner = await box(page, "viewport-float-corner");
  await page.mouse.move(corner.x + corner.width / 2, corner.y + corner.height / 2);
  await page.mouse.down();
  await page.mouse.move(corner.x + corner.width / 2 - 60, corner.y + corner.height / 2 - 40, { steps: 6 });
  await page.mouse.up();
  const resized = await box(page, "viewport-pane");
  expect(resized.x).toBeCloseTo(moved.x, 0);
  expect(resized.y).toBeCloseTo(moved.y, 0);
  expect(resized.width).toBeCloseTo(moved.width - 60, 0);
  expect(resized.height).toBeCloseTo(moved.height - 40, 0);
  // The scene followed the panel: the canvas is the panel's size, less the strip.
  const canvasBox = await box(page, "viewport-canvas");
  expect(canvasBox.width).toBeCloseTo(resized.width - 2, 0);
  expect(canvasBox.height).toBeCloseTo(resized.height - 2 - 22, 0);

  // ---- persisted: the settings hold the rect relative to the work area.
  const stored = await storedSettings(page);
  expect(stored.viewportMode).toBe("floating");
  expect(stored.floatingViewport).not.toBeNull();
  expect(stored.floatingViewport!.x).toBeCloseTo(resized.x - workArea!.x - 0, 0);
  expect(stored.floatingViewport!.y).toBeCloseTo(resized.y - workArea!.y - 0, 0);
  expect(stored.floatingViewport!.width).toBeCloseTo(resized.width, 0);
  expect(stored.floatingViewport!.height).toBeCloseTo(resized.height, 0);

  // ---- a reload keeps the mode and the rect.
  await page.reload();
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.locator(".splitter")).toHaveCount(0);
  const reloaded = await box(page, "viewport-pane");
  expect(reloaded.x).toBeCloseTo(resized.x, 0);
  expect(reloaded.y).toBeCloseTo(resized.y, 0);
  expect(reloaded.width).toBeCloseTo(resized.width, 0);
  expect(reloaded.height).toBeCloseTo(resized.height, 0);
  await expect.poll(async () => triangles(await scene(page)), { timeout: 20_000 }).toBeGreaterThan(500);

  // ---- back to split from the settings menu's control: the panes and the splitter return.
  await page.getByTestId("tb-settings").click();
  await expect(page.getByTestId("settings-viewport-mode-floating")).toHaveAttribute("aria-checked", "true");
  await page.getByTestId("settings-viewport-mode-split").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.locator(".splitter")).toHaveCount(1);
  expect((await storedSettings(page)).viewportMode).toBe("split");
  expect(errors, errors.join("\n")).toEqual([]);
});

test("window: the viewport's element moves into the picture-in-picture window and comes back — by the placeholder, and when that window closes", async ({
  page,
  context,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  await loadDrawn(page);
  const hasApi = await page.evaluate(() => "documentPictureInPicture" in window);
  expect(hasApi, "this Chromium exposes documentPictureInPicture on the loopback origin").toBe(true);
  const rendersBefore = (await scene(page))?.renders ?? 0;

  // ---- the control's `window`: a PiP window appears as a page; the element is in it.
  const [pip] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  await expect(page.getByTestId("viewport-placeholder")).toBeVisible();
  await expect(page.getByTestId("viewport-placeholder")).toHaveText("the viewport is in its own window — click to bring it back");
  await expect(page.getByTestId("viewport")).toHaveCount(0);
  await expect(page.locator(".splitter")).toHaveCount(0);
  await expect(pip.getByTestId("viewport")).toBeVisible();
  await expect(pip.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(pip.getByTestId("viewport-mode-window")).toHaveAttribute("aria-checked", "true");
  // The app's stylesheet and theme came along: the host fills the window, on the theme's ground.
  const styled = await pip.getByTestId("viewport").evaluate((el) => {
    const style = getComputedStyle(el);
    return { position: style.position, theme: el.ownerDocument.documentElement.dataset.theme, width: el.clientWidth, inner: el.ownerDocument.defaultView?.innerWidth };
  });
  expect(styled.position).toBe("absolute");
  expect(styled.theme).toBe("dark");
  expect(styled.width).toBe(styled.inner);
  expect(await pip.title()).toBe(`${PIPELINE} — viewport · Cicada`);
  expect((await storedSettings(page)).viewportMode).toBe("window");

  // ---- the same scene: a write moves the geometry the PiP shows (the scene
  // object is the main window's). The value written differs from the file's
  // current one — another spec on this scratch copy may have written 3.5.
  const boundsBefore = (await scene(page))?.bounds;
  expect(boundsBefore).not.toBeNull();
  const current = /size = slider\(value=([\d.]+)/.exec(await debugText(page));
  expect(current, "02-solids has its size slider").not.toBeNull();
  const next = Number(current![1]) < 4 ? "4.5" : "2.5";
  await page.evaluate((value) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send({ type: "set_param", payload: { node: "size", port: "value", value } });
  }, next);
  await expect.poll(async () => await debugText(page)).toContain(`size = slider(value=${next}`);
  await expect.poll(async () => (await scene(page))?.bounds?.[1][0] ?? 0).not.toBeCloseTo(boundsBefore![1][0]!, 3);
  await expect.poll(async () => (await scene(page))?.renders ?? 0).toBeGreaterThan(rendersBefore);

  // ---- the placeholder brings it back: the element in the main document again, split, the PiP page closed.
  await page.getByTestId("viewport-placeholder").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(page.locator(".splitter")).toHaveCount(1);
  await expect.poll(() => pip.isClosed()).toBe(true);
  expect(triangles(await scene(page))).toBeGreaterThan(500);

  // ---- again, from floating this time; the PiP window closing on its own returns to SPLIT (the contract).
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  const [pip2] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  await expect(pip2.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await pip2.close();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
  expect((await storedSettings(page)).viewportMode).toBe("split");
  expect(errors, errors.join("\n")).toEqual([]);
});

test("window without the API: the observer pop-out opens with a notice, and the mode stays", async ({ page, context }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "documentPictureInPicture", { value: undefined, configurable: true });
  });
  await loadDrawn(page);
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");

  const [popup] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await popup.waitForLoadState();
  const url = new URL(popup.url());
  expect(url.searchParams.get("view")).toBe("viewport");
  expect(url.searchParams.get("pipeline")).toBe(PIPELINE);
  expect(await popup.evaluate(() => window.name)).toBe("cicada-viewport");
  await expect(popup.getByTestId("viewport-only")).toBeVisible();
  await expect(popup.getByTestId("viewport-modes")).toHaveCount(0);

  // The main window: the notice says why, the mode did not change, the viewport is still here.
  const notice = page.getByTestId("notices").locator(".notice.warning");
  await expect(notice).toHaveCount(1);
  await expect(notice).toContainText("no picture-in-picture window");
  await expect(notice).toContainText("read-only window instead");
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.getByTestId("viewport-mode-floating")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("viewport-mode-window")).toHaveAttribute("aria-checked", "false");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  expect((await storedSettings(page)).viewportMode).toBe("floating");
  await popup.close();
});
