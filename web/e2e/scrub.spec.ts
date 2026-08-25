/**
 * Scrub caching in the app (docs/17 item 5 S2; docs/12 §Speculative warming;
 * docs/13 §Scrub caching; docs/16 §Sliders). Two tests against the REAL
 * `cicada serve` from `playwright.config.ts` over a scratch copy of
 * `examples/`:
 *
 * 1. `examples/02-solids.cic` — the consumer: its cone slider `size` carries
 *    `scrub=True` (19 positions). After the open the worker warms every
 *    position while the app is idle; the buffer bar under BOTH slider
 *    widgets fills from the server's view (`/debug/state.scrub` is the
 *    oracle the DOM is held against); the toggle in the inspector's actions,
 *    the params row and the node menu reads the text's word. A pointer drag
 *    across the warm positions previews LIVE — the viewport follows while
 *    the pointer is down, every preview generation computed ZERO nodes (the
 *    generation timings), no pending chip. Turning scrub off from the
 *    inspector removes the kwarg (one op, `scrub size off`), empties the bar
 *    and the queue; Ctrl+Z brings both back.
 *
 * 2. A pipeline written into the scratch with an EXPENSIVE cone — a Python
 *    script node `burn` (a fixed CPU loop, seconds) fed by a scrub-cached
 *    slider `a` and an un-scrubbed one `b` — because the tie-in the contract
 *    names (docs/13 §Slider drags: a tick on a warm position is a pure cache
 *    read and previews live; a cold tick that would compute ≥ 1 s is
 *    withheld and shows the pending chip) cannot be shown on 02-solids,
 *    whose cone is ~40 ms and previews live warm or cold. Here the warm set
 *    fills on an OBSERVER page from the broadcast alone; dragging `a` across
 *    its warm positions follows live with zero computed nodes although every
 *    position costs seconds cold; dragging `b` onto a cold position shows
 *    `pending · N s` on both pages and solves once on release; `fine` (51
 *    positions) is greyed with the SERVER's reason and the forced intent is
 *    refused with the same words.
 */
import { expect, test, type Browser, type Locator, type Page } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;

interface Timing {
  generation: number;
  kind: string;
  elapsed_ms: number | null;
  cancelled: boolean;
  computed: number;
  cached: number;
}

interface ScrubQueue {
  node: string;
  port: string;
  positions: number;
  /** The positions as the dialect literals the warming solves (`"2.0"`), in index order. */
  values: string[];
  warmed: number[];
  warming: boolean;
  capped: boolean;
  bytes: number;
}

interface DebugState {
  text: string;
  history: { can_undo: boolean; can_redo: boolean; undo_label: string | null; redo_label: string | null; depth: number };
  ops: { label: string }[];
  statuses: Record<string, { state: string; message?: string }>;
  summary: { generation: number; running: boolean; red: number; blocked: number };
  solve: {
    last_complete_generation: number | null;
    previews_deferred: number;
    drag: { node: string; port: string | null; mode: string; deferred: number } | null;
  };
  timings: Timing[];
  scrub: { state: string; queues: ScrubQueue[] };
  graph: {
    nodes: {
      name: string;
      /** The view-model's stable node id — the viewport keys its outputs `ref:outputIndex`. */
      ref: number;
      param?: { scrub?: { on: boolean; positions: number; warmed: number[]; warming: boolean } };
    }[];
  };
}

async function debugState(page: Page, pipeline: string, wait: boolean, timeout = 60_000): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${pipeline}&wait=${wait}`, { timeout });
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

const queueOf = (state: DebugState, node: string) => state.scrub.queues.find((q) => q.node === node);

/** Every binding green — red here is the spec's environment, not the feature (the Python host for test 2). */
function expectGreen(state: DebugState, what: string): void {
  const red = Object.entries(state.statuses)
    .filter(([, s]) => s.state === "red" || s.state === "blocked")
    .map(([name, s]) => `${name}: ${s.state}${s.message ? ` — ${s.message}` : ""}`);
  expect(red, `${what} must solve green:\n${red.join("\n")}`).toEqual([]);
}

interface SceneStats {
  bounds: [number[], number[]] | null;
  /** Per drawn output, keyed `nodeRef:outputIndex` (`viewport/sceneStore.ts::outputKey`). */
  outputs: Record<string, { bounds: [number[], number[]] | null }>;
  framesReceived: number;
  lastGeneration: number;
}

async function scene(page: Page): Promise<SceneStats> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { scene: (() => unknown) | null } };
    if (w.__cicada.scene === null) throw new Error("viewport not mounted");
    return w.__cicada.scene() as SceneStats;
  });
}

/**
 * The x extent the viewport draws for one node's first output — the box's
 * width in both pipelines. The scene's UNION bounds would not do: in
 * 02-solids the ball (x up to 1.75) hides a small box, so "the viewport
 * follows the slider" is read off the box's own output.
 */
async function drawnMaxX(page: Page, ref: number): Promise<number> {
  const stats = await scene(page);
  return stats.outputs[`${ref}:0`]?.bounds?.[1]?.[0] ?? Number.NaN;
}

function refOf(state: DebugState, name: string): number {
  const view = state.graph.nodes.find((n) => n.name === name);
  if (view === undefined) throw new Error(`no node ${name} in the graph`);
  return view.ref;
}

async function storePending(page: Page): Promise<{ node: string; mode: string; value: string } | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { pending: { node: string; mode: string; value: string } | null } } };
    return w.__cicada.state().pending;
  });
}

/** The bar's data attributes — what the DOM claims, to hold against the server. */
async function barData(bar: Locator): Promise<{ positions: number; warmed: number; warming: string; current: number; capped: string | undefined }> {
  return bar.evaluate((el) => ({
    positions: Number(el.dataset.positions),
    warmed: Number(el.dataset.warmed),
    warming: el.dataset.warming ?? "",
    current: Number(el.dataset.current),
    capped: el.dataset.capped,
  }));
}

const node = (page: Page, name: string) => page.locator(`.react-flow__node[data-id='${name}']`);
/** The canvas node's bar (the params panel's twin shares the test id — scope decides). */
const canvasBar = (page: Page, name: string) => node(page, name).getByTestId(`scrub-bar-${name}`);
const paramsBar = (page: Page, name: string) => page.getByTestId("params-panel").getByTestId(`scrub-bar-${name}`);

/** Wait until `node`'s queue reports every position warm and no work left. */
async function waitWarm(page: Page, pipeline: string, nodeName: string, positions: number, timeout: number): Promise<ScrubQueue> {
  let queue: ScrubQueue | undefined;
  await expect
    .poll(
      async () => {
        queue = queueOf(await debugState(page, pipeline, false), nodeName);
        return queue !== undefined && !queue.warming && queue.warmed.length === positions;
      },
      { timeout, message: `${nodeName}: every one of ${positions} positions warm` },
    )
    .toBe(true);
  return queue!;
}

/** The preview generations since `baseline`, each required to have computed nothing. */
function previewsSince(state: DebugState, baseline: number): Timing[] {
  return state.timings.filter((t) => t.kind === "preview" && t.generation > baseline);
}

/** A slider value as the widgets spell a Number literal (`4.0`, `4.75`) — the warming spells its positions the same way. */
const literalOf = (x: number) => (Number.isInteger(x) ? x.toFixed(1) : String(x));

async function openObserver(browser: Browser, pipeline: string): Promise<{ page: Page; errors: string[] }> {
  const context = await browser.newContext({ baseURL: config.use?.baseURL, viewport: config.use?.viewport });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  await page.goto(`/?token=${TOKEN}&pipeline=${pipeline}`);
  await expect(page.getByTestId("app")).toBeVisible();
  return { page, errors };
}

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  return errors;
}

test.describe.configure({ mode: "serial", retries: 0 });

test("02-solids: the bar fills while idle on both widgets, the toggle flips the text, a drag across warm positions previews live with nothing computed", async ({
  page,
}, testInfo) => {
  test.setTimeout(4 * 60_000);
  const PIPELINE = "02-solids.cic";
  const errors = collectErrors(page);

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  const initial = await debugState(page, PIPELINE, true);
  expectGreen(initial, "02-solids");
  // The served copy is shared with the suite's other specs (one of them
  // drags `size`): the committed value is whatever the text says now; the
  // bounds, the step and the opt-in are the example's.
  const sizeLine = /size = slider\(value=([0-9.]+), min=0.5, max=5.0, step=0.25, scrub=True\)/.exec(initial.text);
  if (sizeLine === null) throw new Error(`02-solids: no scrub-cached size slider in:\n${initial.text}`);
  const committed = Number(sizeLine[1]);
  const committedNotch = Math.round((committed - 0.5) / 0.25);
  const depthBefore = initial.history.depth;

  // ---- the warming: 19 positions, the committed one a memo hit, the other
  // 18 solved at idle class; the server's queue is the oracle.
  const warm = await waitWarm(page, PIPELINE, "size", 19, 120_000);
  expect(warm.values).toHaveLength(19);
  expect(warm.values[committedNotch], "the committed value is one of the positions, spelled as the widget spells it").toBe(literalOf(committed));
  expect(warm.capped).toBe(false);
  const view = (await debugState(page, PIPELINE, false)).graph.nodes.find((n) => n.name === "size")?.param?.scrub;
  expect(view).toMatchObject({ on: true, positions: 19, warming: false });
  expect(view?.warmed).toHaveLength(19);

  // ---- the canvas bar: 19 segments, all warm, the current notch 6.
  const canvas = canvasBar(page, "size");
  await expect(canvas).toBeVisible();
  await expect.poll(async () => (await barData(canvas)).warmed).toBe(19);
  expect(await barData(canvas)).toEqual({ positions: 19, warmed: 19, warming: "false", current: committedNotch, capped: undefined });
  await expect(canvas.locator(".scrub-seg")).toHaveCount(19);
  await expect(canvas.locator(".scrub-seg.warm")).toHaveCount(19);
  await expect(canvas.locator(".scrub-seg.current")).toHaveAttribute("data-index", String(committedNotch));
  await expect(canvas).toHaveAttribute("title", /^scrub cache · 19 \/ 19 positions warm · every position is a cache read/);

  // ---- the params panel's twin and its compact toggle.
  await page.getByTestId("insp-tab-params").click();
  const panelBar = paramsBar(page, "size");
  await expect(panelBar).toBeVisible();
  expect(await barData(panelBar)).toMatchObject({ positions: 19, warmed: 19, warming: "false", current: committedNotch });
  const rowToggle = page.getByTestId("scrub-toggle-size");
  await expect(rowToggle).toHaveAttribute("aria-checked", "true");
  await expect(rowToggle).toBeEnabled();
  await expect(rowToggle).toHaveText("scrub");

  // ---- the node menu: the same state as a menu item.
  await node(page, "size").click({ button: "right", position: { x: 10, y: 8 } });
  const menu = page.getByTestId("context-menu");
  const item = menu.getByRole("menuitem", { name: /scrub-cach/ });
  await expect(item).toBeVisible();
  await expect(item).toHaveText(/^stop scrub-caching/);
  await expect(item).toBeEnabled();
  await expect(item.locator(".cv-menu-hint")).toHaveText("19 / 19 positions warm");
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // ---- the warm drag: the canvas slider snaps every tick onto the step
  // grid, so each tick is a warmed literal — a pure cache read, previewed
  // LIVE (docs/13 §Slider drags): the viewport follows while the pointer is
  // down, no generation computes a node, no pending chip.
  const before = await debugState(page, PIPELINE, true);
  const baseline = before.solve.last_complete_generation ?? 0;
  // The box is `size` wide: the viewport draws the committed value now.
  const blockRef = refOf(before, "block");
  await expect.poll(async () => drawnMaxX(page, blockRef), { timeout: 30_000 }).toBeCloseTo(committed, 6);
  const slider = page.getByTestId("slider-size");
  const box = await slider.boundingBox();
  if (box === null) throw new Error("no size slider");
  const y = box.y + box.height / 2;
  // Drag to the END of the track away from the committed value, so the
  // held value is a grid value that differs from it whatever the suite's
  // earlier specs left in the text.
  const towardMax = committed < 2.75;
  await page.mouse.move(box.x + box.width * (towardMax ? 0.3 : 0.7), y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * (towardMax ? 0.97 : 0.03), y, { steps: 12 });
  const held = Number(await slider.inputValue());
  expect(held).not.toBe(committed);
  expect(Math.round((held - 0.5) / 0.25) * 0.25 + 0.5, "the canvas snaps to the step").toBeCloseTo(held, 9);
  // The viewport follows the held value while the pointer is down.
  await expect.poll(async () => drawnMaxX(page, blockRef), { timeout: 30_000 }).toBeCloseTo(held, 6);
  const midDrag = await debugState(page, PIPELINE, true);
  const previews = previewsSince(midDrag, baseline);
  expect(previews.length, "the drag produced preview generations").toBeGreaterThan(0);
  for (const t of previews) expect(t.computed, `preview gen ${t.generation} computed nodes on a warm position`).toBe(0);
  expect(previews.every((t) => t.cached > 0)).toBe(true);
  expect(midDrag.solve.drag).toMatchObject({ node: "size", port: "value" });
  expect(midDrag.solve.drag?.deferred, "nothing withheld on warm positions").toBe(0);
  await expect(page.getByTestId("pending-size")).toHaveCount(0);
  expect(await storePending(page)).toBeNull();
  // The bar's marker followed the thumb.
  expect((await barData(canvas)).current).toBe(Math.round((held - 0.5) / 0.25));
  await page.screenshot({ path: testInfo.outputPath("warm-drag-held.png"), fullPage: false });
  await page.mouse.up();
  // The release writes the one op; the text change drops and rebuilds the
  // queue, which re-verifies every position from the memo — all hits.
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).toContain(`size = slider(value=${literalOf(held)},`);
  await waitWarm(page, PIPELINE, "size", 19, 60_000);
  const afterRelease = await debugState(page, PIPELINE, true);
  expect(afterRelease.history.depth, "the release is ONE op").toBe(depthBefore + 1);
  expect(afterRelease.text).toContain("scrub=True");

  // ---- the inspector's action turns it off: ONE op `scrub size off`, the
  // kwarg gone, the queue gone, the bars gone; Ctrl+Z brings it all back.
  await node(page, "size").click({ position: { x: 10, y: 12 } });
  await page.getByTestId("insp-tab-inspect").click();
  const action = page.getByTestId("scrub-toggle-size");
  await expect(action).toHaveText("stop scrub-caching");
  await expect(action).toHaveAttribute("aria-checked", "true");
  await action.click();
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).not.toContain("scrub=True");
  const off = await debugState(page, PIPELINE, true);
  expect(off.history).toMatchObject({ depth: depthBefore + 2, undo_label: "scrub size off", can_redo: false });
  expect(off.ops.map((op) => op.label).at(-1)).toBe("scrub size off");
  expect(off.scrub.queues).toEqual([]);
  expect(off.text).toContain(`min=0.5, max=5.0, step=0.25)`);
  await expect(canvasBar(page, "size")).toHaveCount(0);
  await expect(action).toHaveText("scrub-cache this slider");
  await expect(action).toHaveAttribute("aria-checked", "false");
  await expect(action).toBeEnabled();
  await page.getByTestId("insp-tab-params").click();
  await expect(paramsBar(page, "size")).toHaveCount(0);
  await expect(page.getByTestId("scrub-toggle-size")).toHaveAttribute("aria-checked", "false");

  await page.locator(".react-flow__pane").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).toContain("scrub=True");
  await waitWarm(page, PIPELINE, "size", 19, 60_000);
  await expect(canvasBar(page, "size")).toBeVisible();
  await expect.poll(async () => (await barData(canvasBar(page, "size"))).warmed).toBe(19);
  expect((await debugState(page, PIPELINE, false)).history).toMatchObject({
    depth: depthBefore + 1,
    can_redo: true,
    redo_label: "scrub size off",
  });

  expect(errors, errors.join("\n")).toEqual([]);
});

// The expensive cone: a Python script node that burns a fixed loop — the
// same bits every run, seconds of wall time (60 M iterations: ~2.5 s on a
// 2026 desktop with CPython 3.10, longer on a CI runner) so the cost model's
// prediction for a cold tick sits well over COMPUTE_ON_RELEASE_MS (1 s)
// everywhere. Output x + y; the loop's result is folded in at weight 0 so
// the loop is paid and the value stays a plain sum.
const BURN_SCRIPT = `# e2e scrub.spec.ts: a deterministic CPU burn — seconds per call, output x + y.
import cicada


@cicada.node(
    title="Burn",
    description="e2e fixture: a fixed CPU loop (seconds), then x + y — an expensive cone for the compute-on-release tie-in.",
)
def burn(x: "Number", y: "Number", work: "Number" = 60000000.0) -> "Number":
    acc = 0
    for i in range(int(work)):
        acc += (i * i) % 7
    return x + y + 0.0 * acc
`;

const COLD_TEXT =
  "# cicada 1\n" +
  "a = slider(value=1.0, min=1.0, max=3.0, step=1.0, scrub=True)\n" +
  "b = slider(value=1.0, min=1.0, max=3.0, step=1.0)\n" +
  "fine = slider(value=0.5, min=0.0, max=1.0, step=0.02)\n" +
  "w = burn(x=a, y=b)\n" +
  "span = construct_domain(start=0.0, end=w)\n" +
  "block = box(x=span, y=span, z=span)\n";

test("an expensive cone: warm positions preview live with nothing computed, a cold un-scrubbed slider shows the pending chip, observers see the bar fill, an ineligible slider is greyed with the server's reason", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(8 * 60_000);
  const PIPELINE = "scrub-cold/cold.cic";
  const dir = join(meta.scratch, "examples", "scrub-cold");
  mkdirSync(join(dir, "scripts"), { recursive: true });
  writeFileSync(join(dir, "scripts", "burn.py"), BURN_SCRIPT);
  writeFileSync(join(dir, "cold.cic"), COLD_TEXT);
  const errors = collectErrors(page);

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  // The observer joins at once, so the warm set reaches it as BROADCASTS
  // (`scrub_progress`), not in its snapshot.
  const observer = await openObserver(browser, PIPELINE);
  await observer.page.getByTestId("insp-tab-params").click();
  const observerBar = paramsBar(observer.page, "a");
  await expect(observerBar).toBeVisible();
  await expect(observer.page.getByTestId("scrub-toggle-a"), "an observer cannot toggle").toBeDisabled();

  // ---- the cold open: the burn computes once (seconds).
  const initial = await debugState(page, PIPELINE, true, 180_000);
  expectGreen(initial, "the burn pipeline (is Python 3 on PATH?)");
  expect(["done", "cached"]).toContain(initial.statuses["w"]?.state);

  // ---- the warming: positions 1.0 / 2.0 / 3.0 of `a`; the committed one a
  // hit, the other two solved at idle — two more burns. The observer's bar
  // fills from the broadcast alone.
  const warm = await waitWarm(page, PIPELINE, "a", 3, 240_000);
  expect(warm.values).toEqual(["1.0", "2.0", "3.0"]);
  await expect.poll(async () => (await barData(observerBar)).warmed, { timeout: 30_000 }).toBe(3);
  expect(await barData(observerBar)).toMatchObject({ positions: 3, warmed: 3, warming: "false", current: 0 });
  await expect(observerBar.locator(".scrub-seg.warm")).toHaveCount(3);
  await expect.poll(async () => (await barData(canvasBar(page, "a"))).warmed).toBe(3);
  const warmed = await debugState(page, PIPELINE, true);
  const idleGenerations = warmed.timings.filter((t) => t.kind === "hypothetical");
  expect(idleGenerations.length, "two idle-class solves warmed the two cold positions").toBeGreaterThanOrEqual(2);

  // ---- `fine` (0…1 by 0.02 = 51 positions) cannot scrub-cache: the menu
  // item and the params toggle are greyed with the SERVER's reason, and the
  // intent forced past them is refused with the same words — no op.
  await node(page, "fine").click({ button: "right", position: { x: 10, y: 8 } });
  const menu = page.getByTestId("context-menu");
  const fineItem = menu.getByRole("menuitem", { name: /scrub-cach/ });
  await expect(fineItem).toBeVisible();
  await expect(fineItem).toHaveText(/^scrub-cache this slider/);
  await expect(fineItem).toBeDisabled();
  await expect(fineItem.locator(".cv-menu-hint")).toHaveText("too many positions (51 > 32)");
  await page.keyboard.press("Escape");
  await page.getByTestId("insp-tab-params").click();
  const fineToggle = page.getByTestId("scrub-toggle-fine");
  await expect(fineToggle).toBeDisabled();
  await expect(fineToggle).toHaveAttribute("data-blocked", "too many positions (51 > 32)");
  await expect(paramsBar(page, "fine")).toHaveCount(0);
  const depthBefore = (await debugState(page, PIPELINE, false)).history.depth;
  await page.evaluate(() => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send({ type: "set_scrub", payload: { node: "fine", on: true } });
  });
  await expect(page.getByTestId("notices")).toContainText("`fine`: too many positions (51 > 32)");
  expect((await debugState(page, PIPELINE, true)).history.depth, "a refusal is not an op").toBe(depthBefore);
  // `b` is eligible and off: live toggle, no bar.
  const bToggle = page.getByTestId("scrub-toggle-b");
  await expect(bToggle).toBeEnabled();
  await expect(bToggle).toHaveAttribute("aria-checked", "false");
  await expect(paramsBar(page, "b")).toHaveCount(0);

  // ---- the warm drag on `a`: every tick is a warmed position of a cone
  // that costs seconds cold — and previews LIVE, nothing computed.
  const before = await debugState(page, PIPELINE, true);
  const baseline = before.solve.last_complete_generation ?? 0;
  const sliderA = page.getByTestId("slider-a");
  const boxA = await sliderA.boundingBox();
  if (boxA === null) throw new Error("no slider a");
  const yA = boxA.y + boxA.height / 2;
  await page.mouse.move(boxA.x + boxA.width * 0.1, yA);
  await page.mouse.down();
  await page.mouse.move(boxA.x + boxA.width * 0.95, yA, { steps: 10 });
  expect(await sliderA.inputValue()).toBe("3");
  // The box is `a + b` wide: 3 + 1 = 4 while the pointer is held.
  const blockRef = refOf(before, "block");
  await expect.poll(async () => drawnMaxX(page, blockRef), { timeout: 30_000 }).toBeCloseTo(4, 6);
  const heldA = await debugState(page, PIPELINE, true);
  const warmPreviews = previewsSince(heldA, baseline);
  expect(warmPreviews.length).toBeGreaterThan(0);
  for (const t of warmPreviews) expect(t.computed, `preview gen ${t.generation} computed on a warm position of a seconds-cold cone`).toBe(0);
  expect(heldA.solve.drag?.deferred).toBe(0);
  await expect(page.getByTestId("pending-a")).toHaveCount(0);
  expect((await barData(canvasBar(page, "a"))).current).toBe(2);
  await page.mouse.up();
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).toContain("a = slider(value=3.0,");
  // The text change rebuilt `a`'s queue around b = 1: every position a hit.
  await waitWarm(page, PIPELINE, "a", 3, 60_000);

  // ---- the cold drag on `b` (not scrub-cached): the first cold tick is
  // predicted at the burn's cost (seconds ≥ 1 s) and WITHHELD — the pending
  // chip on the writer's canvas widget and the observer's params row, no
  // preview generation computes, the viewport stands; the release solves
  // once.
  const settled = await debugState(page, PIPELINE, true);
  const baselineB = settled.solve.last_complete_generation ?? 0;
  const deferredBefore = settled.solve.previews_deferred;
  const sliderB = page.getByTestId("slider-b");
  const boxB = await sliderB.boundingBox();
  if (boxB === null) throw new Error("no slider b");
  const yB = boxB.y + boxB.height / 2;
  await page.mouse.move(boxB.x + boxB.width * 0.1, yB);
  await page.mouse.down();
  await page.mouse.move(boxB.x + boxB.width * 0.95, yB, { steps: 10 });
  expect(await sliderB.inputValue()).toBe("3");
  const chip = page.getByTestId("pending-b");
  await expect(chip).toBeVisible({ timeout: 30_000 });
  await expect(chip).toHaveText(/^pending · ~?\d+(\.\d+)? (ms|s)$/);
  expect(await storePending(page)).toMatchObject({ node: "b", mode: "compute_on_release", value: "3.0" });
  await expect(observer.page.getByTestId("param-pending-b"), "the observer hears the policy").toBeVisible({ timeout: 30_000 });
  const heldB = await debugState(page, PIPELINE, false);
  expect(heldB.solve.drag).toMatchObject({ node: "b", port: "value", mode: "compute_on_release" });
  expect(heldB.solve.previews_deferred).toBeGreaterThan(deferredBefore);
  for (const t of previewsSince(heldB, baselineB)) expect(t.computed, `preview gen ${t.generation} computed mid-drag on a cold position`).toBe(0);
  expect(await drawnMaxX(page, blockRef), "the viewport waits for the release").toBeCloseTo(4, 6);
  await page.screenshot({ path: testInfo.outputPath("cold-drag-held.png"), fullPage: false });
  await page.mouse.up();
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).toContain("b = slider(value=3.0,");
  await expect(chip).toHaveCount(0, { timeout: 30_000 });
  await expect(observer.page.getByTestId("param-pending-b")).toHaveCount(0, { timeout: 30_000 });
  // The release's solve: the burn computes once; the box is 3 + 3 = 6.
  const released = await debugState(page, PIPELINE, true, 180_000);
  expectGreen(released, "the release");
  const structural = released.timings.filter((t) => t.kind === "structural" && t.generation > baselineB);
  expect(structural.length, "one structural generation for the release").toBe(1);
  expect(structural[0]!.computed).toBeGreaterThanOrEqual(1);
  await expect.poll(async () => drawnMaxX(page, blockRef), { timeout: 60_000 }).toBeCloseTo(6, 6);

  // ---- tidy: turn `a`'s scrub off from the params row (the rebuilt queue
  // around b = 3 would otherwise keep burning at idle under the next spec).
  await page.getByTestId("scrub-toggle-a").click();
  await expect.poll(async () => (await debugState(page, PIPELINE, false)).text).toContain("a = slider(value=3.0, min=1.0, max=3.0, step=1.0)");
  await expect.poll(async () => (await debugState(page, PIPELINE, true, 180_000)).scrub.queues).toEqual([]);
  await expect(paramsBar(page, "a")).toHaveCount(0);
  await expect(observerBar).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
  expect(observer.errors, observer.errors.join("\n")).toEqual([]);
  await observer.page.context().close();
});
