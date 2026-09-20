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
 *     window keeps only the placeholder, the scene still follows a write,
 *     the toolbar that moved with it still works (clicked IN the PiP page:
 *     display modes, frame all, the mode control landing on the chosen
 *     mode) — the placeholder brings it back; the PiP window closing on
 *     its own returns to split.
 *   - window without the API (stubbed out): the wave-4 observer pop-out
 *     opens with a notice saying so, and the mode stays put.
 *
 * Runner assumptions, named: the headless shell exposes the API on the
 * loopback origin (the window test SKIPS without it, the file goes on); and
 * the shell's PiP window does NOT follow its opener's unload — a reload or a
 * cross-pipeline navigation leaves it open holding the dead page's canvas,
 * where Edge and Chrome close it — so the "opener leaving" arm of docs/16 is
 * verified under a real channel, not here; what IS asserted here is what
 * holds in both: a reload in window mode loads as split, with no
 * placeholder and the viewport in its pane (`window` never rests).
 */
import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";
/** The file the window test opens while in window mode (File → Open keeps the PiP window, its title follows). */
const CURVES = "01-curves.cic";

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

/** The store's display mode (the main window's; the PiP page has no store of its own). */
async function displayMode(page: Page): Promise<string> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { settings: { displayMode: string } } } };
    return w.__cicada.state().settings.displayMode;
  });
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

const rounded = (b: Box): Box => ({ x: Math.round(b.x), y: Math.round(b.y), width: Math.round(b.width), height: Math.round(b.height) });

/** The work area's children in DOM order, by test id (the splitter by its class). */
async function workOrder(page: Page): Promise<string[]> {
  return page.locator(".app-work > *").evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.testid ?? el.className));
}

/** `inner` lies within `outer` (to half a pixel). */
function inside(inner: Box, outer: Box): boolean {
  const eps = 0.5;
  return (
    inner.x >= outer.x - eps &&
    inner.y >= outer.y - eps &&
    inner.x + inner.width <= outer.x + outer.width + eps &&
    inner.y + inner.height <= outer.y + outer.height + eps
  );
}

async function debugText(page: Page): Promise<string> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return ((await response.json()) as { text: string }).text;
}

/**
 * Wrap `ResizeObserver` in every document of the context so a page can say
 * how many LIVE observers of ITS realm watch the element with a test id —
 * the observer the scene watches its container with must belong to the
 * window the container is laid out by (docs/16): one from the main window
 * never fires for an element in the PiP document (a test that waits for the
 * canvas to follow cannot tell — the runner's trace screencast drives the
 * main document's rendering and lets a main-realm observer fire late).
 */
async function countResizeObservers(context: BrowserContext): Promise<void> {
  await context.addInitScript(() => {
    const Native = window.ResizeObserver;
    const targets = new Map<ResizeObserver, Set<Element>>();
    class Counting extends Native {
      override observe(target: Element, options?: ResizeObserverOptions): void {
        let set = targets.get(this);
        if (set === undefined) {
          set = new Set();
          targets.set(this, set);
        }
        set.add(target);
        super.observe(target, options);
      }
      override unobserve(target: Element): void {
        targets.get(this)?.delete(target);
        super.unobserve(target);
      }
      override disconnect(): void {
        targets.delete(this);
        super.disconnect();
      }
    }
    window.ResizeObserver = Counting;
    (window as unknown as { __resizeObserved: (testId: string) => number }).__resizeObserved = (testId) => {
      let n = 0;
      for (const set of targets.values()) for (const el of set) if ((el as HTMLElement).dataset?.testid === testId) n += 1;
      return n;
    };
  });
}

/** How many live `ResizeObserver`s of THIS page's realm watch the element with `testId`. */
async function resizeObserversOf(page: Page, testId: string): Promise<number> {
  return page.evaluate((id) => (window as unknown as { __resizeObserved: (testId: string) => number }).__resizeObserved(id), testId);
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

  // ---- swap: the work area's children are KEYED, so the swap reorders the
  // viewport's wrapper and remounts nothing — the marked canvas element and
  // its scene survive (review finding F3: without the keys a swap remounted
  // the viewport and its WebGL context, and nothing went red).
  expect(await workOrder(page)).toEqual(["canvas-pane", "splitter", "viewport-pane"]);
  await page.getByTestId("tb-settings").click();
  await page.getByTestId("settings-swap").check();
  await expect.poll(() => workOrder(page)).toEqual(["viewport-pane", "splitter", "canvas-pane"]);
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  expect(triangles(await scene(page))).toBeGreaterThan(500);
  await page.getByTestId("settings-swap").uncheck();
  await expect.poll(() => workOrder(page)).toEqual(["canvas-pane", "splitter", "viewport-pane"]);
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("settings-swap")).toHaveCount(0);

  // ---- floating: the canvas takes the whole work area, the panel floats over it, the canvas element is the same.
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-mode-floating")).toHaveAttribute("aria-checked", "true");
  await expect(page.locator(".splitter")).toHaveCount(0);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.getByTestId("viewport-float-title")).toBeVisible();
  await expect(page.getByTestId("viewport-float-corner")).toBeVisible();
  // The placeholder is the window mode's alone: the panel IS the viewport here.
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
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

  // ---- the window shrinking pulls the panel back inside the work area (the
  // frame's ResizeObserver re-clamps at every measured size — docs/16); the
  // stored rect is untouched, so growing the window again restores the place.
  await page.setViewportSize({ width: 900, height: 600 });
  await expect
    .poll(async () => {
      const pane = rounded(await box(page, "viewport-pane"));
      const area = (await page.locator(".app-work").boundingBox())!;
      return { inside: inside(pane, area), width: pane.width, height: pane.height };
    })
    .toEqual({ inside: true, width: Math.round(resized.width), height: Math.round(resized.height) });
  expect(resized.width).toBeGreaterThanOrEqual(240);
  expect(resized.height).toBeGreaterThanOrEqual(160);
  await page.setViewportSize({ width: 1400, height: 900 });
  await expect.poll(async () => rounded(await box(page, "viewport-pane"))).toEqual(rounded(resized));

  // ---- a reload keeps the mode and the rect.
  await page.reload();
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.locator(".splitter")).toHaveCount(0);
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
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
  await countResizeObservers(context);
  await loadDrawn(page);
  expect(await resizeObserversOf(page, "viewport"), "the scene watches its container from the main window").toBe(1);
  // A runner assumption, named: the headless shell exposes the API on the
  // loopback origin (a secure context). Without it this ONE test is skipped
  // — the fallback test below covers that browser — rather than the file
  // failing (the tests after it in this serial file still run).
  const hasApi = await page.evaluate(() => "documentPictureInPicture" in window);
  test.skip(!hasApi, "this Chromium exposes no documentPictureInPicture on the loopback origin — the window mode falls back to the pop-out here (tested below)");
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

  // ---- the scene is re-homed to the PiP window: its resize observer is THAT
  // window's — the PiP realm holds the one live observer of the host, the
  // main realm none — so the canvas (its CSS size and its drawing buffer)
  // follows the PiP window's size when it changes after the move. A
  // main-document observer never fires for an element in another document
  // (review finding F2); the realm is asserted because the runner's trace
  // screencast can make even that one fire late.
  expect(await resizeObserversOf(pip, "viewport"), "the PiP realm's observer watches the host").toBe(1);
  expect(await resizeObserversOf(page, "viewport"), "no main-realm observer watches the moved host").toBe(0);
  const pipInner = () => pip.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight, dpr: Math.min(window.devicePixelRatio || 1, 2) }));
  const canvasSize = () =>
    pip.getByTestId("viewport-canvas").evaluate((el) => {
      const c = el as HTMLCanvasElement;
      return { client: { width: c.clientWidth, height: c.clientHeight }, buffer: { width: c.width, height: c.height } };
    });
  const expectedCanvas = (inner: { width: number; height: number; dpr: number }) => ({
    client: { width: inner.width, height: inner.height },
    buffer: { width: Math.floor(inner.width * inner.dpr), height: Math.floor(inner.height * inner.dpr) },
  });
  const openedAt = await pipInner();
  await expect.poll(canvasSize).toEqual(expectedCanvas(openedAt));
  const larger = { width: openedAt.width + 200, height: openedAt.height + 100 };
  await pip.setViewportSize(larger);
  await expect.poll(pipInner, { message: "the shell honours setViewportSize on the PiP page" }).toMatchObject(larger);
  await expect.poll(canvasSize).toEqual(expectedCanvas({ ...larger, dpr: openedAt.dpr }));

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

  // ---- the toolbar that moved with the viewport WORKS in the PiP page: its
  // handlers are React's, reached through the portal's listeners on the host
  // (a click in the PiP document never bubbles to the main document's root —
  // review finding 2026-09-20, the toolbar was dead there).
  expect(await displayMode(page)).toBe("shaded_edges");
  await pip.getByTitle("wireframe").click();
  await expect.poll(() => displayMode(page)).toBe("wireframe");
  await expect(pip.getByTitle("wireframe")).toHaveClass("active");
  await pip.getByTitle("shaded + edges").click();
  await expect.poll(() => displayMode(page)).toBe("shaded_edges");
  const rendersBeforeFrame = (await scene(page))?.renders ?? 0;
  await pip.getByTestId("viewport-frame-all").click();
  await expect.poll(async () => (await scene(page))?.renders ?? 0).toBeGreaterThan(rendersBeforeFrame);
  // The mode control in the PiP: leaving by it lands on the CHOSEN mode (floating, not split), the element home first, the window closed.
  await pip.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(page.getByTestId("viewport-float-title")).toBeVisible();
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
  await expect.poll(() => pip.isClosed()).toBe(true);
  expect((await storedSettings(page)).viewportMode).toBe("floating");
  expect(triangles(await scene(page))).toBeGreaterThan(500);

  // ---- again, from floating: the placeholder brings it back — the element in the main document again, split, the PiP page closed.
  const [pip2] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  await expect(pip2.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await page.getByTestId("viewport-placeholder").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(page.locator(".splitter")).toHaveCount(1);
  await expect.poll(() => pip2.isClosed()).toBe(true);
  expect(triangles(await scene(page))).toBeGreaterThan(500);
  // Home: the scene watches its container from the main window again.
  expect(await resizeObserversOf(page, "viewport")).toBe(1);

  // ---- once more, from floating; the PiP window closing on its own returns to SPLIT (the contract), not to floating.
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "floating");
  const [pip3] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  await expect(pip3.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await pip3.close();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
  expect((await storedSettings(page)).viewportMode).toBe("split");

  // ---- File → Open while in window mode: the viewport stays mounted, so the
  // window stays and the same scene follows the next pipeline — and the
  // window's title names it (review finding: it kept naming the old file).
  const [pip4] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  expect(await pip4.title()).toBe(`${PIPELINE} — viewport · Cicada`);
  await page.getByTestId("tb-file").click();
  await page.getByTestId("file-open").click();
  const dialog = page.getByTestId("open-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId(`files-entry-${CURVES}`).dblclick();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(CURVES);
  await expect.poll(() => pip4.title()).toBe(`${CURVES} — viewport · Cicada`);
  expect(pip4.isClosed()).toBe(false);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  await expect(pip4.getByTestId("viewport-canvas")).toHaveAttribute("data-marker", "same-canvas");
  await page.getByTestId("viewport-placeholder").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect.poll(() => pip4.isClosed()).toBe(true);

  // ---- a reload in window mode: `window` never rests — the stored mode
  // loads as split, no placeholder, the viewport in its pane. (The shell's
  // PiP page may outlive the reload, unlike a real browser's: closed here
  // when it does, so the context ends clean.)
  const [pip5] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-mode-window").click()]);
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "window");
  expect((await storedSettings(page)).viewportMode).toBe("window");
  await page.reload();
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
  await expect(page.getByTestId("viewport-placeholder")).toHaveCount(0);
  await expect(page.getByTestId("viewport")).toBeVisible();
  await expect(page.locator(".splitter")).toHaveCount(1);
  // The stored `window` is normalised at load (`viewportModeFrom`); the app's
  // settings say split (the stored copy is rewritten on the next write).
  expect(await page.evaluate(() => (window as unknown as { __cicada: { state: () => { settings: { viewportMode: string } } } }).__cicada.state().settings.viewportMode)).toBe("split");
  if (!pip5.isClosed()) await pip5.close();
  expect(errors, errors.join("\n")).toEqual([]);
});

test("floating at the minimum: the toolbar wraps, every control — the mode control included — stays inside the panel", async ({ page }) => {
  await loadDrawn(page);
  await page.getByTestId("viewport-mode-floating").click();
  await expect(page.getByTestId("viewport-float-corner")).toBeVisible();
  // The corner dragged far up-left: the panel stops at 240 × 160 (the contract's minimum).
  const corner = await box(page, "viewport-float-corner");
  await page.mouse.move(corner.x + corner.width / 2, corner.y + corner.height / 2);
  await page.mouse.down();
  await page.mouse.move(corner.x - 1000, corner.y - 1000, { steps: 8 });
  await page.mouse.up();
  const panel = await box(page, "viewport-pane");
  expect(rounded(panel).width).toBe(240);
  expect(rounded(panel).height).toBe(160);
  // A layout invariant, not a font fact (Linux draws the same text wider):
  // every toolbar button and the mode control lie inside the panel, and the
  // mode control is usable — the panel's own way out of floating.
  const buttons = await page.locator(".viewport-toolbar button").evaluateAll((els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      return { text: el.textContent ?? "", x: r.x, y: r.y, width: r.width, height: r.height };
    }),
  );
  expect(buttons.length).toBeGreaterThanOrEqual(7);
  for (const button of buttons) expect(inside(button, panel), `${button.text} inside the panel`).toBe(true);
  expect(inside(await box(page, "viewport-modes"), panel)).toBe(true);
  await page.getByTestId("viewport-mode-split").click();
  await expect(page.getByTestId("viewport-pane")).toHaveAttribute("data-mode", "split");
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

  // A second click: the fixed window name re-targets the open pop-out — no
  // second page, and the warning is not stacked again.
  const pagesBefore = context.pages().length;
  await page.getByTestId("viewport-mode-window").click();
  await expect(page.getByTestId("viewport-mode-floating")).toHaveAttribute("aria-checked", "true");
  await expect(notice).toHaveCount(1);
  expect(context.pages().length).toBe(pagesBefore);
  expect(popup.isClosed()).toBe(false);
  await popup.close();
});
