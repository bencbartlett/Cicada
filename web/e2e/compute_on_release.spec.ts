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
 * suite by design — the wall carves in seconds on a release engine, in
 * tens of seconds to minutes on a debug one, and the release pays a second
 * carve — hence its own timeout. The nightly job runs it on the RELEASE
 * engine: this is a timing spec, and on a debug engine the 2026-08-22..24
 * nightlies (98–114 s per carve on the runner) turned its 15 s waits into
 * coin flips — red three nights with no engine change. The drag goes
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
 * announced again), write nothing and solve nothing. The observer's
 * `preview_policy` latency while its 368 MB display set streams is
 * MEASURED here (logged, attached, bounded by a sanity net) — the socket's
 * ordering is the server tests' and the wire probe's to assert (docs/13
 * §Two lanes, one socket).
 *
 * Two oracles, kept apart (2026-08-24): the ORDER of intents and answers
 * is read off this page's own WebSocket frames (`wire` — tapped, stamped,
 * attached, printed on failure), where it is exact; the PAGE — DOM, store
 * — is waited on under one generous bound (`PAGE_BOUND_MS`), because at
 * wall scale it redraws ~13 M triangles in software GL on every state
 * change and on a starved runner its main thread is away for seconds.
 */
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import config from "../playwright.config";
import { paramValueText } from "../src/state/literals";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "wall/wall.cic";
// The initial wall solve plus the release's carve, with two threads, on a
// loaded CI runner — sized for a debug engine (98–114 s per carve on the
// 2026-08-22..24 runners) so a local debug run still fits.
const SPEC_TIMEOUT_MS = 10 * 60_000;
const SOLVE_TIMEOUT_MS = 5 * 60_000;
// The observer's `preview_policy` while its display set is still streaming
// (docs/13 §Two lanes, one socket). The socket puts the text behind at most
// one frame and the server no longer holds its lock or the joiner's
// `hello` for the restream's build — both pinned deterministically by the
// server's tests and measured on the wire by `tools/measure/lanes.mjs`.
// What the PAGE then pays is its own: the browser takes the whole restream
// into its message queue faster than it handles the frames (the wall: 368
// MB, five frames of 27–94 MB; headless Chromium renders the ~13 M
// triangles in software, seconds per large frame), so a text sent once the
// server has written its frames waits behind every frame the page has not
// handled yet — the page cannot be the socket's oracle, and this spec
// MEASURES the observer's hint (logged + attached) and bounds it only by
// this sanity net. The number is a DIAGNOSTIC of the page, never a
// before/after of the lanes: it is set by where the page's frame handling
// stands at the grab — uncontrolled here (the observer's setup takes about
// as long as a debug engine's ~3 s restream) — and a one-queue engine whose
// observer had already handled every frame posts the BETTER number (192 ms
// with 26 of 26 frames in at the grab, reproduced 2026-08-21, against
// 7.3 s for the lanes with 23 in). The lanes' evidence is the wire probe.
// Reaching the status cadence here is the display plane's follow-up
// (docs/17): frame handling off the main thread.
const OBSERVER_POLICY_BOUND_MS = 60_000;
// Every wait on the WRITER page at wall scale is bounded the same way: the
// page renders the wall's ~13 M triangles in software GL on every state
// change, so under CPU starvation its main thread is away for seconds at a
// time — the 2026-08-24 Nightly's `expect.poll` on the store timed out with
// no received value at all, which is what a `page.evaluate` that never got
// a turn in 15 s looks like, and pinned to 4 cores locally the writer's
// own hint took 6 s. The contract those waits used to carry is read off
// the wire below (`wire`), where the order is exact; the page gets a sanity
// net, never a clock.
const PAGE_BOUND_MS = 60_000;

// The wire as each page saw it: every text frame out (`param_preview`,
// `set_param`, `end_drag`) and in (`preview_policy`, `delta`, `drag_ended`,
// `status`, …), stamped on arrival. The order of those frames IS the
// contract under test (docs/13 §Slider drags), and when this spec fails on
// the nightly runner it is the one thing worth reading — attached on every
// run and printed into the log on failure, because the runner's artifacts
// may not upload (the account's storage quota) while its log always does.
const wire: string[] = [];
function tapWire(page: Page, who: string): void {
  page.on("websocket", (socket) => {
    const note = (direction: string, payload: string | Buffer) => {
      if (typeof payload !== "string") return; // a binary display frame
      let summary = payload.slice(0, 160);
      try {
        const parsed = JSON.parse(payload) as { type?: string; seq?: number; payload?: Record<string, unknown> };
        const p = parsed.payload ?? {};
        const source = p.source as { label?: string } | undefined;
        summary = `${parsed.type ?? "?"}${parsed.seq === undefined ? "" : `#${parsed.seq}`} ${JSON.stringify({
          node: p.node,
          port: p.port,
          value: p.value ?? p.pending_value,
          mode: p.mode,
          label: source?.label,
          kind: p.kind,
        })}`;
      } catch {
        // not JSON: keep the raw head
      }
      wire.push(`${new Date().toISOString()} ${who} ${direction} ${summary}`);
    };
    socket.on("framesent", (frame) => note("→", frame.payload));
    socket.on("framereceived", (frame) => note("←", frame.payload));
  });
}
// eslint-disable-next-line no-empty-pattern -- Playwright's fixture signature
test.afterEach(async ({}, testInfo) => {
  // A file in the test's output dir (kept for passed runs too), not a body
  // attachment — the list reporter keeps those in memory only.
  const path = testInfo.outputPath("wire.log");
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, wire.join("\n"));
  await testInfo.attach("wire.log", { path, contentType: "text/plain" });
  if (testInfo.status !== testInfo.expectedStatus) {
    console.log(`[compute-on-release] wire, last 80 of ${wire.length} frames:\n${wire.slice(-80).join("\n")}`);
  }
  wire.length = 0;
});

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
  /** The display table: every output whose frames a joining client receives. */
  display: Record<string, { hash: string; generation: number; stats: { bytes: number } }>;
}

/** The bytes of frames a fresh socket receives: the display table's total. */
function restreamBytes(state: DebugState): number {
  return Object.values(state.display).reduce((sum, d) => sum + d.stats.bytes, 0);
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

// Never retried, on CI either (the suite's default there is one retry): the
// spec WRITES the served project (the release commits `deboss`) and reads
// the server's cold-start counters as preconditions (`previews_deferred` is
// 0 only before the first drag), and Playwright restarts no server between
// attempts — a retry on the same session cannot pass and only hides the
// first attempt's failure behind "Expected: 0, Received: 17" (the
// 2026-08-22..24 nightlies).
test.describe.configure({ mode: "serial", retries: 0 });

// Wall-scale: the 1,200-part carve solves for seconds (release) to minutes
// (debug) on a CI runner and its display set (~350 MB of frames) streams to
// every page over the one socket — the per-PR smoke is not the place for
// it. The nightly `Playwright heavy (wall)` job sets CICADA_E2E_HEAVY=1 and
// runs the release engine.
test.skip(
  !process.env.CICADA_E2E_HEAVY,
  "wall-scale spec — run with CICADA_E2E_HEAVY=1 (the nightly heavy job, or locally)",
);

/** The page's frame counters (`window.__cicada.frames()`). */
async function observerFrames(page: Page): Promise<{ received: number; bytes: number }> {
  return page.evaluate(() => {
    const handle = (window as unknown as { __cicada?: { frames?: () => { received: number; bytes: number } } })
      .__cicada;
    return handle?.frames ? { received: handle.frames().received, bytes: handle.frames().bytes } : { received: -1, bytes: -1 };
  });
}

/**
 * Wait until the page has stopped receiving display frames for `quietMs`:
 * a fresh page (writer or observer) receives the WHOLE display set, which
 * for the wall takes tens of seconds on a loaded machine. The first half's
 * preconditions are "the wall is solved AND its frames are in" — not "the
 * machine is fast".
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
  browser,
}, testInfo) => {
  test.setTimeout(SPEC_TIMEOUT_MS);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  tapWire(page, "writer");
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
  await expect(hint).toBeVisible({ timeout: PAGE_BOUND_MS });
  await expect(hint).toHaveText(/^pending · ~?\d+(\.\d+)? (ms|s)$/);
  const pending = await storePending(page);
  expect(pending).not.toBeNull();
  expect(pending).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release" });
  expect(pending!.estimateMs, "the bar is 1 s — the wall's deboss cone is far over it").toBeGreaterThanOrEqual(1000);
  await expect(page.getByTestId("number-deboss")).toHaveClass(/pending/, { timeout: PAGE_BOUND_MS });
  const shown = Number(await page.getByTestId("number-deboss").inputValue());
  expect(shown).toBeGreaterThan(1.0);
  await expect(row).toHaveClass(/pending/, { timeout: PAGE_BOUND_MS });
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
  await expect(hint).toHaveCount(0, { timeout: PAGE_BOUND_MS });
  await expect.poll(async () => storePending(page), { timeout: PAGE_BOUND_MS }).toBeNull();
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
  // The same contract read off the wire, where order is exact (the page's
  // clock above is a sanity net): the release's `set_param` is the last
  // thing the writer sent for this drag — the widget cancels its queued
  // tick on commit, so no `param_preview` follows it — its `delta` came
  // back, and no `preview_policy` followed that delta (the write ended the
  // drag server-side; a tick after it would be a fresh drag with a badge
  // nothing takes down — the failure shape this guards).
  const writeAt = wire.findIndex((line) => line.includes("writer → set_param") && line.includes('"node":"deboss"'));
  expect(writeAt, "the release sent set_param").toBeGreaterThanOrEqual(0);
  const afterWrite = wire.slice(writeAt + 1);
  expect(afterWrite.filter((line) => line.includes("writer → param_preview")), "no tick after the write").toEqual([]);
  const deltaAt = afterWrite.findIndex((line) => line.includes("writer ← delta") && line.includes("set deboss.value"));
  expect(deltaAt, `the write's delta arrived:\n${afterWrite.join("\n")}`).toBeGreaterThanOrEqual(0);
  expect(
    afterWrite.slice(deltaAt + 1).filter((line) => line.includes("writer ← preview_policy")),
    "no policy after the write's delta",
  ).toEqual([]);

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
  // The observer gets its OWN browser context: a second page in the
  // writer's context can share its renderer process, and the observer's
  // frame handling (368 MB decoded in software GL, seconds per large frame)
  // would then stall the writer's main thread — the writer's hint below is
  // the product's contract, so its clock must not be the observer's. A real
  // observer is another browser anyway.
  const observerContext = await browser.newContext({
    baseURL: config.use?.baseURL,
    viewport: config.use?.viewport,
  });
  const observer = await observerContext.newPage();
  const observerErrors: string[] = [];
  observer.on("pageerror", (error) => observerErrors.push(`pageerror: ${error.message}`));
  tapWire(observer, "observer");
  await observer.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(observer.getByTestId("app")).toBeVisible();
  // The observer, too, receives the whole display set — and the drag below
  // starts WHILE it streams: the control plane has its own lane (docs/13
  // §Two lanes, one socket), so the snapshot is in already (the app is
  // visible above before the first large frame could have been decoded)
  // and the `preview_policy` does not wait for the frames on the socket.
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
  // What the observer's restream amounts to, read from the server's display
  // table BEFORE the drags: the socket-order guard below compares against
  // this, not against the observer's final count — a release that wrote a
  // new value would broadcast frames after the text and let a one-queue
  // engine pass a "frames at hint < frames at the end" guard.
  const observerRestreamBytes = restreamBytes(settled);
  expect(observerRestreamBytes, "the wall's display set is on the server").toBeGreaterThan(100e6);

  // Grab the thumb where it sits and pull it onto cold values.
  const observerFramesAtGrab = await observerFrames(observer);
  await page.mouse.move(xCommitted, y2);
  const tGrab = Date.now();
  await page.mouse.down();
  await dragTo(page, slider, xFor(box2, 1.1, min, max), y2, "1.1");
  await expect(hint).toBeVisible({ timeout: PAGE_BOUND_MS });
  const writerHintMs = Date.now() - tGrab;
  // Where the writer's wait went, from the wire's own stamps: the first
  // tick out after the grab and the first `preview_policy` back separate
  // the server's share (tick → policy on the socket) from the page's (the
  // rest of `writerHintMs`: the drag's own event handling and the render).
  const grabIso = new Date(tGrab).toISOString();
  const sinceGrab = (what: string) => wire.filter((line) => line > grabIso && line.includes(`writer ${what}`));
  const stampOf = (line: string | undefined) => (line === undefined ? NaN : Date.parse(line.slice(0, 24)));
  const ticksSent = sinceGrab("→ param_preview");
  const firstPolicyAt = stampOf(sinceGrab("← preview_policy")[0]);
  const writerTickToPolicyMs = firstPolicyAt - stampOf(ticksSent[0]);
  const writerFirstTickMs = stampOf(ticksSent[0]) - tGrab;
  const pending2 = await storePending(page);
  expect(pending2).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release", value: "1.1" });
  // The canvas twin renders the same entry (hidden by LOD, present in the
  // DOM): the chip and the value label.
  await expect(page.getByTestId("pending-deboss")).toHaveCount(1, { timeout: PAGE_BOUND_MS });
  await expect(page.getByTestId("pending-deboss")).toHaveText(await hint.innerText(), { timeout: PAGE_BOUND_MS });
  await expect(page.getByTestId("slider-value-deboss")).toHaveText("1.1", { timeout: PAGE_BOUND_MS });
  // The observer hears the broadcast: the hint, the class, the entry —
  // while its display set is still streaming. Before the lanes (one
  // channel per client) the wall's ~350 MB of frames stood between the
  // observer and this text on the SOCKET whenever the tick reached the
  // server while they were still queued there. Now the text leaves the
  // server behind at most the one frame in flight; what is left is the
  // page's own queue (see OBSERVER_POLICY_BOUND_MS), measured here as a
  // diagnostic — the number says where the page's frame handling stood at
  // the grab, not which engine served it.
  const observerHint = observer.getByTestId("param-pending-deboss");
  await expect(observerHint).toBeVisible({ timeout: OBSERVER_POLICY_BOUND_MS });
  const observerHintMs = Date.now() - tGrab;
  const observerFramesAtHint = await observerFrames(observer);
  await expect(observer.getByTestId("param-deboss")).toHaveClass(/pending/, { timeout: PAGE_BOUND_MS });
  expect(await storePending(observer)).toMatchObject({ node: "deboss", port: "value", mode: "compute_on_release" });
  console.log(
    `[compute-on-release] observer preview_policy: writer hint ${writerHintMs} ms ` +
      `(first tick ${writerFirstTickMs} ms after the grab, tick → policy on the wire ${writerTickToPolicyMs} ms, ${ticksSent.length} ticks sent), ` +
      `observer hint ${observerHintMs} ms after the grab · ` +
      `observer frames at grab ${observerFramesAtGrab.received} (${(observerFramesAtGrab.bytes / 1e6).toFixed(0)} MB), ` +
      `at hint ${observerFramesAtHint.received} (${(observerFramesAtHint.bytes / 1e6).toFixed(0)} MB) ` +
      `of a ${(observerRestreamBytes / 1e6).toFixed(0)} MB restream`,
  );
  await testInfo.attach("observer-preview-policy.json", {
    body: JSON.stringify(
      {
        writer_hint_ms: writerHintMs,
        writer_first_tick_ms: writerFirstTickMs,
        writer_tick_to_policy_ms: writerTickToPolicyMs,
        writer_ticks_sent: ticksSent.length,
        observer_hint_ms: observerHintMs,
        observer_frames_at_grab: observerFramesAtGrab,
        observer_frames_at_hint: observerFramesAtHint,
        observer_restream_bytes: observerRestreamBytes,
        observer_hint_landed_mid_restream: observerFramesAtHint.bytes < observerRestreamBytes,
      },
      null,
      2,
    ),
    contentType: "application/json",
  });

  // Back onto the committed value and release: no `change` fires in
  // Chrome for a drag that returns to its start.
  await dragTo(page, slider, xCommitted, y2, committed);
  await page.mouse.up();
  await expect(hint).toHaveCount(0, { timeout: PAGE_BOUND_MS });
  await expect.poll(async () => storePending(page), { timeout: PAGE_BOUND_MS }).toBeNull();
  await expect(page.getByTestId("number-deboss")).toHaveValue(committed, { timeout: PAGE_BOUND_MS });
  await expect(page.getByTestId("number-deboss")).not.toHaveClass(/pending/, { timeout: PAGE_BOUND_MS });
  await expect(page.getByTestId("pending-deboss")).toHaveCount(0, { timeout: PAGE_BOUND_MS });
  // The label shows the value as the dialect writes it (`2.0`, never a bare
  // `2` — that would parse as an Integer literal); the inputs hold the number.
  await expect(page.getByTestId("slider-value-deboss")).toHaveText(paramValueText("slider", released), {
    timeout: PAGE_BOUND_MS,
  });
  // … and the observer's badge goes down with it (`drag_ended`), its
  // number back on the committed value.
  await expect(observerHint).toHaveCount(0, { timeout: PAGE_BOUND_MS });
  await expect.poll(async () => storePending(observer), { timeout: PAGE_BOUND_MS }).toBeNull();
  await expect(observer.getByTestId("number-deboss")).toHaveValue(committed, { timeout: PAGE_BOUND_MS });
  await expect(observer.getByTestId("param-deboss")).not.toHaveClass(/pending/, { timeout: PAGE_BOUND_MS });

  // The re-grab: a fresh drag, announced again — whether inside the gap
  // (the release ended the server's drag) or after it (the gap rule).
  await page.mouse.down();
  await dragTo(page, slider, xFor(box2, 1.2, min, max), y2, "1.2");
  await expect(hint, "the re-grab is announced").toBeVisible({ timeout: PAGE_BOUND_MS });
  await expect(observerHint).toBeVisible({ timeout: PAGE_BOUND_MS });
  await dragTo(page, slider, xCommitted, y2, committed);
  await page.mouse.up();
  await expect(hint).toHaveCount(0, { timeout: PAGE_BOUND_MS });
  await expect(observerHint).toHaveCount(0, { timeout: PAGE_BOUND_MS });

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
  // Off the wire: both no-write releases went out as `end_drag`, nothing
  // was written after the first, and the observer heard an end for each.
  const endDragAt = wire.findIndex((line) => line.includes("writer → end_drag"));
  expect(endDragAt, "the no-write release is the end_drag intent").toBeGreaterThanOrEqual(0);
  expect(wire.slice(endDragAt + 1).filter((line) => line.includes("writer → set_param")), "nothing written").toEqual([]);
  expect(wire.filter((line) => line.includes("writer → end_drag")).length, "one end_drag per no-write release").toBe(2);
  expect(
    wire.filter((line) => line.includes("observer ← drag_ended")).length,
    "the observer hears the end of every announced drag",
  ).toBeGreaterThanOrEqual(2);
  console.log(
    `[compute-on-release] no-write releases: ${after.solve.previews_deferred - settled.solve.previews_deferred} more ticks withheld · ` +
      `drag ${after.solve.drag === null ? "ended" : "STANDING"} · text ${debossValue(after.text)} · observer cleared`,
  );
  // Where the text landed among the observer's frames — a measurement, not
  // a guard. Message events fire in wire order, so "frame bytes handled
  // before the hint" is where the text sat on the wire. It discriminates
  // the one-queue engine from the lanes ONLY when the tick reaches the
  // server while frames are still queued there: with one queue the text
  // always lands after the last frame; with the lanes it lands behind the
  // frame in flight — but once the server has written the restream (a
  // debug engine encodes the wall's in ~3 s, streaming as it goes) the
  // page's own queue holds whatever it has not handled and the text is
  // legitimately last on the wire. The restream's size is the server's
  // display table as read before the drags (`observerRestreamBytes`), so
  // nothing a later release broadcasts widens the bar.
  const landedMidRestream = observerFramesAtHint.bytes < observerRestreamBytes;
  testInfo.annotations.push({
    type: "note",
    description:
      `observer preview_policy landed ${landedMidRestream ? "mid-restream" : "after its whole restream"} ` +
      `(${(observerFramesAtHint.bytes / 1e6).toFixed(0)} of ${(observerRestreamBytes / 1e6).toFixed(0)} MB in; ` +
      `${observerFramesAtGrab.received} frames in at the grab)`,
  });
  // Let the observer's restream finish before the page goes — a frame
  // decoding error mid-stream would otherwise close with the page unseen.
  await waitForFramesToSettle(observer, 2_000, 180_000);
  const observerFinal = await observerFrames(observer);
  expect(observerFinal.bytes, "the observer received its whole display set").toBeGreaterThanOrEqual(observerRestreamBytes);
  await observerContext.close();

  expect(errors, errors.join("\n")).toEqual([]);
  expect(observerErrors, observerErrors.join("\n")).toEqual([]);
});
