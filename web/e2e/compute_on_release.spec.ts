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
 * twin of the slider is in the DOM — hidden by LOD at the wall's zoom, its
 * text is still the oracle) and the oracles are the DOM,
 * `window.__cicada.state()` and `/debug/state` (`solve.drag`,
 * `solve.previews_deferred`, the generation timings).
 *
 * The second half (review findings, 2026-08-20) is the release that writes
 * nothing: a pointer drag away and BACK to the committed value — Chrome
 * fires no `change` for it — with an observer page open. The release must
 * take both pages' badges down (`end_drag` → `drag_ended`), end the
 * server's drag at once (a re-grab inside the 300 ms gap is a fresh drag,
 * announced again), write nothing and solve nothing.
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

/** The pointer x for `value` on a range input's box (thumb-width aware; `dragTo` corrects by reading back). */
function xFor(box: { x: number; width: number }, value: number, min: number, max: number): number {
  const thumb = 16;
  return box.x + thumb / 2 + ((value - min) / (max - min)) * (box.width - thumb);
}

/**
 * With the pointer held down, move along the track until the range reads
 * exactly `target` — a pixel estimate can land one step off, and the test
 * needs the release to be ON the committed value, not near it.
 */
async function dragTo(page: Page, slider: ReturnType<Page["getByTestId"]>, x: number, y: number, target: string) {
  await page.mouse.move(x, y, { steps: 8 });
  for (let i = 0; i < 60 && (await slider.inputValue()) !== target; i += 1) {
    const shown = Number(await slider.inputValue());
    x += Number(target) > shown ? 1 : -1;
    await page.mouse.move(x, y);
  }
  expect(await slider.inputValue(), `the thumb must sit on ${target}`).toBe(target);
}

test.describe.configure({ mode: "serial" });

// Wall-scale: the 1,200-part carve solves for minutes on a 2-vCPU runner and
// its display set (~350 MB of frames) streams to every page over the one
// socket — the per-PR smoke is not the place for it. The nightly
// `Playwright heavy (wall)` job sets CICADA_E2E_HEAVY=1.
test.skip(
  !process.env.CICADA_E2E_HEAVY,
  "wall-scale spec — run with CICADA_E2E_HEAVY=1 (the nightly heavy job, or locally)",
);

/**
 * Wait until the page has stopped receiving display frames for `quietMs`:
 * a fresh page (writer or observer) receives the WHOLE display set on the
 * socket it shares with the control plane, and text frames queue behind it
 * (docs/17 §Follow-ups). The spec's preconditions are "the wall is solved
 * AND its frames are in" — not "the machine is fast".
 */
async function waitForFramesToSettle(page: Page, quietMs: number, timeoutMs: number): Promise<void> {
  const started = Date.now();
  let last = -1;
  let quietSince = Date.now();
  for (;;) {
    const counters = await page.evaluate(() => {
      const handle = (window as unknown as { __cicada?: { frames?: () => { received: number } } }).__cicada;
      return handle?.frames ? handle.frames().received : -1;
    });
    if (counters !== last) {
      last = counters;
      quietSince = Date.now();
    } else if (Date.now() - quietSince >= quietMs) {
      return;
    }
    if (Date.now() - started > timeoutMs) throw new Error(`display frames still arriving after ${timeoutMs} ms (received ${counters})`);
    await page.waitForTimeout(250);
  }
}

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

  // The display set must be IN before the drag (see waitForFramesToSettle).
  await waitForFramesToSettle(page, 2_000, 180_000);

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

  // ================================================================
  // The release that writes nothing (review findings 2026-08-20): a drag
  // away and back to the committed value, with an observer watching.
  // ================================================================
  const observer = await page.context().newPage();
  const observerErrors: string[] = [];
  observer.on("pageerror", (error) => observerErrors.push(`pageerror: ${error.message}`));
  await observer.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(observer.getByTestId("app")).toBeVisible();
  // The observer, too, first receives the whole display set.
  await waitForFramesToSettle(observer, 2_000, 180_000);
  await observer.getByTestId("insp-tab-params").click();
  await expect(observer.getByTestId("param-deboss")).toBeVisible();
  await expect(observer.getByTestId("widget-deboss"), "the second page observes").toBeDisabled();
  expect(await storePending(observer)).toBeNull();
  const committed = String(released);
  const min = Number(await slider.getAttribute("min"));
  const max = Number(await slider.getAttribute("max"));
  const box2 = await slider.boundingBox();
  if (box2 === null) throw new Error("no deboss slider");
  const y2 = box2.y + box2.height / 2;
  const xCommitted = xFor(box2, released, min, max);
  const baseline2 = settled.solve.last_complete_generation ?? 0;
  const structuralBefore = settled.timings.filter((t) => t.kind === "structural").length;

  // Grab the thumb where it sits and pull it onto cold values.
  await page.mouse.move(xCommitted, y2);
  await page.mouse.down();
  await dragTo(page, slider, xFor(box2, 1.1, min, max), y2, "1.1");
  await expect(hint).toBeVisible();
  const pending2 = await storePending(page);
  expect(pending2).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release", value: "1.1" });
  // The canvas twin renders the same entry (hidden by LOD, present in the
  // DOM): the chip and the value label.
  await expect(page.getByTestId("pending-deboss")).toHaveCount(1);
  await expect(page.getByTestId("pending-deboss")).toHaveText(await hint.innerText());
  await expect(page.getByTestId("slider-value-deboss")).toHaveText("1.1");
  // The observer hears the broadcast: the hint, the class, the entry.
  // A freshly joined observer first receives the whole display set on the
  // SAME socket (the wall: ~350 MB of binary frames, measured 2026-08-20),
  // and text frames queue behind it — on a loaded machine `preview_policy`
  // reached the observer ~26 s after the drag. That head-of-line blocking
  // is a protocol work item (docs/17 §Follow-ups); this assertion waits for
  // delivery, it does not assert latency.
  const observerHint = observer.getByTestId("param-pending-deboss");
  await expect(observerHint).toBeVisible({ timeout: 120_000 });
  await expect(observer.getByTestId("param-deboss")).toHaveClass(/pending/);
  expect(await storePending(observer)).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release" });

  // Back onto the committed value and release: no `change` fires in
  // Chrome for a drag that returns to its start.
  await dragTo(page, slider, xCommitted, y2, committed);
  await page.mouse.up();
  await expect(hint).toHaveCount(0);
  await expect.poll(async () => storePending(page)).toBeNull();
  await expect(page.getByTestId("number-deboss")).toHaveValue(committed);
  await expect(page.getByTestId("number-deboss")).not.toHaveClass(/pending/);
  await expect(page.getByTestId("pending-deboss")).toHaveCount(0);
  await expect(page.getByTestId("slider-value-deboss")).toHaveText(committed);
  // … and the observer's badge goes down with it (`drag_ended`), its
  // number back on the committed value.
  await expect(observerHint).toHaveCount(0);
  await expect.poll(async () => storePending(observer)).toBeNull();
  await expect(observer.getByTestId("number-deboss")).toHaveValue(committed);
  await expect(observer.getByTestId("param-deboss")).not.toHaveClass(/pending/);

  // The re-grab: a fresh drag, announced again — whether inside the gap
  // (the release ended the server's drag) or after it (the gap rule).
  await page.mouse.down();
  await dragTo(page, slider, xFor(box2, 1.2, min, max), y2, "1.2");
  await expect(hint, "the re-grab is announced").toBeVisible();
  await expect(observerHint).toBeVisible();
  await dragTo(page, slider, xCommitted, y2, committed);
  await page.mouse.up();
  await expect(hint).toHaveCount(0);
  await expect(observerHint).toHaveCount(0);

  // Nothing was written, nothing solved: the text is as released, the
  // server's drag is gone, no structural generation ran, and every preview
  // generation since (a warm tick painting as a cache read) computed nothing.
  const after = await debugState(page, true);
  expect(debossValue(after.text)).toBe(released);
  expect(after.solve.drag, "end_drag ended the server's drag").toBeNull();
  expect(after.timings.filter((t) => t.kind === "structural")).toHaveLength(structuralBefore);
  for (const t of after.timings.filter((t) => t.kind === "preview" && t.generation > baseline2)) {
    expect(t.computed, `preview gen ${t.generation} computed nodes during the no-write drags`).toBe(0);
  }
  expect(after.solve.previews_deferred).toBeGreaterThan(settled.solve.previews_deferred);
  console.log(
    `[compute-on-release] no-write releases: ${after.solve.previews_deferred - settled.solve.previews_deferred} more ticks withheld · ` +
      `drag ${after.solve.drag === null ? "ended" : "STANDING"} · text ${debossValue(after.text)} · observer cleared`,
  );
  await observer.close();

  expect(errors, errors.join("\n")).toEqual([]);
  expect(observerErrors, observerErrors.join("\n")).toEqual([]);
});
