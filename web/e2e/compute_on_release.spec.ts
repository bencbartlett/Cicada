/**
 * Compute-on-release in the app (docs/17 item 3b; docs/13 §Slider drags;
 * DECISIONS.md row 39): a drag on the wall's `deboss` — the full-pipeline
 * slider whose cone (labels → glyph solids → the 1,200-part carve) the
 * cost model predicts at several seconds — shows the pending value and an
 * honest estimate WHILE the pointer is down, paints no preview that would
 * compute, and solves exactly once, on release.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/` with the fresh cache the suite uses: the
 * wall is opened cold here (`examples/wall/wall.cic` inside the scratch
 * project), so the initial solve IS the cost evidence the prediction reads
 * (every node in the cone computed once). This is the slow spec of the
 * suite by design — a debug engine carves the wall in tens of seconds, and
 * the release pays a second carve — hence its own timeout. The drag goes
 * through the params panel's range input (a real pointer drag; the canvas
 * twin of the slider is read from the store) and the oracles are the DOM,
 * `window.__cicada.state()` and `/debug/state` (`solve.drag`,
 * `solve.previews_deferred`, the generation timings).
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "wall/wall.cic";
// The initial wall solve plus the release's carve, on a debug engine with
// two threads, on a loaded CI runner.
const SPEC_TIMEOUT_MS = 10 * 60_000;
const SOLVE_TIMEOUT_MS = 5 * 60_000;

interface Timing {
  generation: number;
  kind: string;
  elapsed_ms: number | null;
  cancelled: boolean;
  computed: number;
  cached: number;
}

interface DebugState {
  text: string;
  statuses: Record<string, { state: string; message?: string }>;
  summary: { generation: number; running: boolean; red: number; blocked: number };
  solve: {
    busy: boolean;
    last_complete_generation: number | null;
    previews_deferred: number;
    drag: { node: string; port: string | null; mode: string; deferred: number } | null;
  };
  timings: Timing[];
}

interface PendingParam {
  node: string;
  port: string | null;
  mode: string;
  value: string;
  estimateMs: number;
  rough: boolean;
  seq: number;
}

async function debugState(page: Page, wait: boolean): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=${wait}`, {
    // `wait=true` blocks until the in-flight generation is done — the carve.
    timeout: wait ? SOLVE_TIMEOUT_MS : 30_000,
  });
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

async function storePending(page: Page): Promise<PendingParam | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { pending: PendingParam | null } } };
    return w.__cicada.state().pending;
  });
}

function debossValue(text: string): number {
  const match = /deboss = slider\(value=([0-9.]+),/.exec(text);
  if (match === null) throw new Error(`no deboss slider in the text:\n${text}`);
  return Number(match[1]);
}

test.describe.configure({ mode: "serial" });

test("a deboss drag shows `pending · N s` while held, paints no computing preview, and solves once on release", async ({
  page,
}, testInfo) => {
  test.setTimeout(SPEC_TIMEOUT_MS);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();

  // ---- the cold open: every node of the cone computes once — the cost
  // evidence. Red here means the script host is missing (the wall's
  // Python nodes), which would leave nothing to predict from.
  const t0 = Date.now();
  const initial = await debugState(page, true);
  const openMs = Date.now() - t0;
  const red = Object.entries(initial.statuses)
    .filter(([, s]) => s.state === "red" || s.state === "blocked")
    .map(([name, s]) => `${name}: ${s.state}${s.message ? ` — ${s.message}` : ""}`);
  expect(red, `the wall must solve green before the drag (is Python 3 on PATH?):\n${red.join("\n")}`).toEqual([]);
  expect(["done", "cached"]).toContain(initial.statuses["carved"]?.state);
  expect(initial.solve.previews_deferred).toBe(0);
  expect(initial.solve.drag).toBeNull();
  expect(debossValue(initial.text)).toBe(1.0);
  const baselineGeneration = initial.solve.last_complete_generation ?? 0;
  expect(baselineGeneration).toBeGreaterThan(0);
  expect(await storePending(page), "nothing pending before any drag").toBeNull();

  // ---- the drag: the params panel's range input for `deboss`, a real
  // pointer drag from 30 % to 90 % of the track, held down at the end.
  await page.getByTestId("insp-tab-params").click();
  const row = page.getByTestId("param-deboss");
  await expect(row).toBeVisible();
  await expect(page.getByTestId("param-pending-deboss")).toHaveCount(0);
  const slider = page.getByTestId("widget-deboss");
  await expect(slider).toBeVisible();
  await expect(slider).toBeEnabled();
  const box = await slider.boundingBox();
  if (box === null) throw new Error("no deboss slider");
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * 0.3, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.9, y, { steps: 12 });

  // While the pointer is held: the hint is up, the store has the verdict,
  // the thumb and the number follow the drag (not the committed 1.0).
  const hint = page.getByTestId("param-pending-deboss");
  await expect(hint).toBeVisible();
  await expect(hint).toHaveText(/^pending · ~?\d+(\.\d+)? (ms|s)$/);
  const pending = await storePending(page);
  expect(pending).not.toBeNull();
  expect(pending).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release" });
  expect(pending!.estimateMs, "the bar is 1 s — the wall's deboss cone is far over it").toBeGreaterThanOrEqual(1000);
  await expect(page.getByTestId("number-deboss")).toHaveClass(/pending/);
  const shown = Number(await page.getByTestId("number-deboss").inputValue());
  expect(shown).toBeGreaterThan(1.0);
  await expect(row).toHaveClass(/pending/);
  // The canvas twin reads the same store entry; the value it would show
  // is the pending one (its chip is hidden by LOD when the wall's canvas
  // is zoomed out, so the DOM check is the panel's).
  const midDrag = await debugState(page, false);
  expect(midDrag.solve.drag).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release" });
  expect(midDrag.solve.previews_deferred).toBeGreaterThan(0);
  // No preview that would compute ran: every preview generation since the
  // open (a tick landing on the warm committed value may paint as a pure
  // cache read) computed nothing, and no structural generation ran yet.
  const previewsMidDrag = midDrag.timings.filter((t) => t.kind === "preview" && t.generation > baselineGeneration);
  for (const t of previewsMidDrag) expect(t.computed, `preview gen ${t.generation} computed nodes mid-drag`).toBe(0);
  expect(midDrag.timings.filter((t) => t.kind === "structural" && t.generation > baselineGeneration)).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("mid-drag.png"), fullPage: false });

  // ---- release: the ONE real set_param → its delta clears the hint; the
  // value solves once.
  const t1 = Date.now();
  await page.mouse.up();
  await expect(hint).toHaveCount(0);
  await expect.poll(async () => storePending(page)).toBeNull();
  await expect.poll(async () => debossValue((await debugState(page, false)).text)).toBeGreaterThan(1.0);
  const settled = await debugState(page, true);
  const releaseMs = Date.now() - t1;
  const released = debossValue(settled.text);
  expect(released).toBe(shown);
  expect(settled.solve.drag, "the write ended the drag").toBeNull();
  const structural = settled.timings.filter((t) => t.kind === "structural" && t.generation > baselineGeneration);
  expect(structural, `exactly one generation for the release:\n${JSON.stringify(structural, null, 2)}`).toHaveLength(1);
  expect(structural[0]!.cancelled).toBe(false);
  expect(structural[0]!.computed).toBeGreaterThan(0);
  const previews = settled.timings.filter((t) => t.kind === "preview" && t.generation > baselineGeneration);
  for (const t of previews) expect(t.computed, `preview gen ${t.generation} computed nodes`).toBe(0);
  expect(["done", "cached"]).toContain(settled.statuses["carved"]?.state);
  expect(settled.summary.red).toBe(0);

  await testInfo.attach("compute-on-release.json", {
    body: JSON.stringify(
      {
        open_ms: openMs,
        release_to_settled_ms: releaseMs,
        policy: pending,
        previews_deferred: settled.solve.previews_deferred,
        preview_generations: previews,
        release_generation: structural[0],
        released_value: released,
      },
      null,
      2,
    ),
    contentType: "application/json",
  });
  console.log(
    `[compute-on-release] open ${openMs} ms · policy estimate ${pending!.rough ? "~" : ""}${pending!.estimateMs} ms · ` +
      `${settled.solve.previews_deferred} ticks withheld · ${previews.length} cache-read previews · ` +
      `release ${released} solved once in ${structural[0]!.elapsed_ms?.toFixed(0)} ms (settled after ${releaseMs} ms)`,
  );

  expect(errors, errors.join("\n")).toEqual([]);
});
