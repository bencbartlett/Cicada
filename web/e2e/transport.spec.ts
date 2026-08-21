/**
 * The transport in the app (docs/17 item 4; docs/13 §Animation transport;
 * docs/16 keyboard map): the orbit example's play bar drives the REAL
 * engine from `playwright.config.ts` (a scratch copy of `examples/`,
 * `?pipeline=07-orbit.cic` on the served project). Through the real UI —
 * the play button, Space, a pointer on the scrubber, the speed menu,
 * reset — with `/debug/state` (transport, timings, statuses, text_hash,
 * display hashes) and `window.__cicada` as the oracles:
 *
 *   - play: the frame advances, the viewport changes (a screenshot at rest
 *     and one half a second into playback differ), the generations are of
 *     kind `transport`, and the file is untouched — same `text_hash`, no
 *     op, no delta;
 *   - the second pass of the loop is pure cache playback: 0 computed, every
 *     node `cached`;
 *   - Space pauses (the transport broadcast says so); `wait=true` is the
 *     quiet oracle only while paused, so every wait follows a pause;
 *   - a scrub is a seek that paints the frame it names — the server's frame
 *     IS the thumb's, including on the frames a nominal seek used to land
 *     short of (the engine's `Playhead::at_frame`), and the display
 *     generation moves;
 *   - speed and reset; a transport-driven port is hidden on the node and in
 *     the inspector; an observer gets the bar read-only.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "07-orbit.cic";

interface TransportView {
  playing: boolean;
  speed: number;
  t_ms: number;
  frame: number;
  frames: number;
  period_ms: number;
  driven: { node: string; port: string; signal: string }[];
}

interface Timing {
  generation: number;
  kind: string;
  computed: number;
  cached: number;
  cancelled: boolean;
}

interface DebugState {
  seq: number;
  text: string;
  text_hash: string;
  history: { depth: number; can_undo: boolean };
  ops: unknown[];
  statuses: Record<string, { state: string }>;
  summary: { generation: number; running: boolean; red: number; blocked: number };
  transport: TransportView;
  timings: Timing[];
  display: Record<string, { hash: string }>;
}

async function debugState(page: Page, wait: boolean): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=${wait}`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
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

async function lastFrameGeneration(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { frames: () => { lastGeneration: number } } };
    return w.__cicada.frames().lastGeneration;
  });
}

/** The bar's data attributes — what the DOM claims, to hold against the server. */
async function bar(page: Page): Promise<{ playing: string; frame: number; frames: number; speed: number }> {
  return page.getByTestId("transport").evaluate((el) => ({
    playing: el.dataset.playing ?? "",
    frame: Number(el.dataset.frame),
    frames: Number(el.dataset.frames),
    speed: Number(el.dataset.speed),
  }));
}

test.describe.configure({ mode: "serial" });

test("play bar: play → frames advance, file untouched; second loop cached; Space pauses; scrub seeks exactly; speed, reset", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  // The bar exists because the pipeline has a time param (the orbit's `spin`).
  const transport = page.getByTestId("transport");
  await expect(transport).toBeVisible();
  await expect(page.getByTestId("tr-driven")).toHaveText("drives spin.frame");

  // ---- at rest: paused at frame 0, every node solved, the loop 120 / 4 s.
  const rest = await debugState(page, true);
  expect(rest.transport).toMatchObject({
    playing: false,
    speed: 1,
    t_ms: 0,
    frame: 0,
    frames: 120,
    period_ms: 4000,
    driven: [{ node: "spin", port: "frame", signal: "frame" }],
  });
  expect(rest.summary.red + rest.summary.blocked).toBe(0);
  expect(rest.history.depth).toBe(0);
  const textHash = rest.text_hash;
  await expect(page.getByTestId("tr-play")).toHaveAttribute("aria-label", "play");
  await expect(page.getByTestId("tr-frame")).toHaveText("0 / 120");
  await expect
    .poll(async () => lastFrameGeneration(page), { timeout: 20_000 })
    .toBeGreaterThan(0);

  // ---- the hidden port: `spin.frame` has no handle and no literal editor on
  // the node — its row shows the transport — and in the inspector it is under
  // `transport`, not `inputs`.
  const spin = page.locator(".react-flow__node[data-id='spin']");
  await expect(spin).toBeVisible();
  await expect(spin.locator(".react-flow__handle[data-handleid='frame']")).toHaveCount(0);
  await expect(spin.locator(".react-flow__handle[data-handleid='frames']")).toHaveCount(1);
  await expect(page.getByTestId("lit-spin-frame")).toHaveCount(0);
  const drivenRow = page.getByTestId("driven-spin-frame");
  await expect(drivenRow).toHaveAttribute("data-driven", "true");
  await expect(drivenRow).toHaveAttribute("data-signal", "frame");
  await spin.locator(".cn-header").click();
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "spin");
  await expect(page.getByTestId("in-frame")).toHaveCount(0);
  await expect(page.getByTestId("in-frames")).toHaveCount(1);
  await expect(page.getByTestId("node-transport")).toBeVisible();
  await expect(page.getByTestId("driven-frame")).toContainText("← transport");
  await expect(page.getByTestId("driven-frame")).toContainText("frame 0 of 120");
  // The text never names the port.
  expect(rest.text).not.toContain("frame=");

  // ---- play: the frame advances, the viewport changes, the file does not.
  const before = await viewportPng(page);
  await testInfo.attach("viewport-rest.png", { body: before, contentType: "image/png" });
  await page.getByTestId("tr-play").click();
  await expect(transport).toHaveAttribute("data-playing", "true");
  await expect(page.getByTestId("tr-play")).toHaveAttribute("aria-label", "pause");
  const first = await debugState(page, false);
  expect(first.transport.playing).toBe(true);
  await expect
    .poll(async () => (await debugState(page, false)).transport.t_ms, { timeout: 10_000 })
    .toBeGreaterThan(first.transport.t_ms + 500);
  const later = await debugState(page, false);
  expect(later.transport.frame).not.toBe(first.transport.frame);
  expect(later.timings.some((t) => t.kind === "transport"), "the generations are the transport's").toBe(true);
  expect(later.text_hash, "playback never writes the file").toBe(textHash);
  expect(later.history.depth, "playback is never an op").toBe(0);
  // The inspector's transport row follows the playhead.
  await expect(page.getByTestId("driven-frame")).not.toContainText("frame 0 of 120");
  const during = await viewportPng(page);
  await testInfo.attach("viewport-playing.png", { body: during, contentType: "image/png" });
  expect(Buffer.compare(before, during), "the viewport shows a different frame").not.toBe(0);

  // ---- the second pass of the loop is pure cache playback (docs/17 item 4
  // "done when"): wait for the playhead to cross into the second loop, then
  // for a generation that computed nothing and cached every node.
  await expect
    .poll(async () => (await debugState(page, false)).transport.t_ms, { timeout: 20_000 })
    .toBeGreaterThan(4000 + 400);
  await expect
    .poll(
      async () => {
        const state = await debugState(page, false);
        const last = state.timings.filter((t) => t.kind === "transport" && !t.cancelled).at(-1);
        return last === undefined ? null : { computed: last.computed, cached: last.cached };
      },
      { timeout: 10_000 },
    )
    .toEqual({ computed: 0, cached: 15 });

  // ---- Space pauses: the server says so, and the bar follows the broadcast.
  await page.locator(".react-flow__pane").click({ position: { x: 20, y: 20 } });
  await page.keyboard.press("Space");
  await expect(transport).toHaveAttribute("data-playing", "false");
  await expect(page.getByTestId("tr-play")).toHaveAttribute("aria-label", "play");
  // Paused: wait=true is the quiet oracle again.
  const paused = await debugState(page, true);
  expect(paused.transport.playing).toBe(false);
  expect(paused.text_hash).toBe(textHash);
  expect(paused.history.depth).toBe(0);
  expect(paused.ops).toEqual([]);
  // Every node's status says cached — the loop was warm on the second pass.
  expect(Object.values(paused.statuses).map((s) => s.state)).toEqual(Array(15).fill("cached"));
  // The bar's frame is the server's frame exactly once paused.
  await expect.poll(async () => (await bar(page)).frame).toBe(paused.transport.frame);
  await expect(page.getByTestId("tr-frame")).toHaveText(`${paused.transport.frame} / 120`);

  // ---- scrub: a pointer on the scrubber seeks; the frame the thumb names is
  // the frame the server paints, and the display generation moves.
  const generationBefore = await lastFrameGeneration(page);
  const planetBefore = paused.display["planet.out"]?.hash;
  expect(planetBefore).toBeDefined();
  const scrub = page.getByTestId("tr-scrub");
  const box = await scrub.boundingBox();
  if (box === null) throw new Error("no scrubber");
  const y = box.y + box.height / 2;
  // Land somewhere past three quarters of the loop, away from where we paused.
  const targetFraction = paused.transport.frame > 60 ? 0.2 : 0.8;
  await page.mouse.move(box.x + box.width * targetFraction, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * (targetFraction + 0.05), y, { steps: 6 });
  await page.mouse.up();
  const sought = (await bar(page)).frame;
  expect(sought).not.toBe(paused.transport.frame);
  const afterSeek = await debugState(page, true);
  expect(afterSeek.transport.frame, "the server painted the frame the thumb names").toBe(sought);
  expect(afterSeek.transport.playing).toBe(false);
  expect(afterSeek.display["planet.out"]?.hash, "the planet moved").not.toBe(planetBefore);
  await expect.poll(async () => lastFrameGeneration(page)).toBeGreaterThan(generationBefore);
  await expect(page.getByTestId("tr-frame")).toHaveText(`${sought} / 120`);

  // Keyboard: the arrows step the scrubber one frame at a time — each step a
  // seek — including onto frame 31, where the nominal playhead reads 30.
  await scrub.focus();
  const steps: number[] = [];
  for (let i = 0; i < 3; i += 1) {
    await page.keyboard.press("ArrowRight");
    steps.push((await bar(page)).frame);
  }
  expect(steps).toEqual([sought + 1, sought + 2, sought + 3].map((f) => f % 120));
  const stepped = await debugState(page, true);
  expect(stepped.transport.frame).toBe(steps[2]);
  // Directly the awkward frame: seek 31 (the thumb is set like a drag would).
  await scrub.evaluate((el) => {
    const input = el as HTMLInputElement;
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    set?.call(input, "31");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(page.getByTestId("tr-frame")).toHaveText("31 / 120");
  const at31 = await debugState(page, true);
  expect(at31.transport.frame, "a seek to 31 lands on 31, not 30").toBe(31);
  expect(at31.text_hash).toBe(textHash);

  // ---- speed: the menu sends the factor; the server's speed comes back.
  await page.getByTestId("tr-speed").selectOption("2");
  await expect(transport).toHaveAttribute("data-speed", "2");
  expect((await debugState(page, true)).transport.speed).toBe(2);
  // Space while the scrubber is focused still toggles playback (then pause again).
  await scrub.focus();
  await page.keyboard.press("Space");
  await expect(transport).toHaveAttribute("data-playing", "true");
  await page.keyboard.press("Space");
  await expect(transport).toHaveAttribute("data-playing", "false");

  // ---- reset: paused at zero, speed kept.
  await page.getByTestId("tr-reset").click();
  await expect(page.getByTestId("tr-frame")).toHaveText("0 / 120");
  const reset = await debugState(page, true);
  expect(reset.transport).toMatchObject({ playing: false, t_ms: 0, frame: 0, speed: 2 });
  expect(reset.text_hash, "nothing above touched the file").toBe(textHash);
  expect(reset.history.depth).toBe(0);
  expect(reset.summary.red + reset.summary.blocked).toBe(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("an observer sees the bar live and read-only; its controls refuse with the lease reason", async ({ browser, page }) => {
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("transport")).toBeVisible();
  await expect(page.getByTestId("tr-play")).toBeEnabled();

  const context = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const observer = await context.newPage();
  try {
    await observer.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
    const bar = observer.getByTestId("transport");
    await expect(bar).toBeVisible();
    for (const id of ["tr-play", "tr-reset", "tr-scrub", "tr-speed"]) {
      await expect(observer.getByTestId(id), id).toBeDisabled();
      await expect(observer.getByTestId(id), id).toHaveAttribute("title", /read-only observer — take the lease to drive the transport/);
    }
    // The writer seeks; the observer's bar follows the broadcast.
    await page.getByTestId("tr-scrub").evaluate((el) => {
      const input = el as HTMLInputElement;
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(input, "62");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(observer.getByTestId("tr-frame")).toHaveText("62 / 120");
    expect((await debugState(page, true)).transport.frame).toBe(62);
    // Space from the observer is refused client-side with the lease notice — no intent, nothing moves.
    await observer.locator(".react-flow__pane").click({ position: { x: 20, y: 20 } });
    await observer.keyboard.press("Space");
    await expect(observer.locator(".notice", { hasText: /read-only observer/ }).first()).toBeVisible();
    expect((await debugState(page, true)).transport.playing).toBe(false);
  } finally {
    await page.getByTestId("tr-reset").click();
    await expect(page.getByTestId("tr-frame")).toHaveText("0 / 120");
    await context.close();
  }
});
