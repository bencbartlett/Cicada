/**
 * The transport in the app (docs/17 item 4; docs/13 §Animation transport;
 * docs/16 keyboard map): the orbit example's play bar drives the REAL
 * engine from `playwright.config.ts` (a scratch copy of `examples/`,
 * `?pipeline=08-orbit.cic` on the served project). Through the real UI —
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
 *   - a refused control (a seek beyond the loop, a speed above 64×, sent on
 *     the socket — the bar offers neither) is answered by the error toast
 *     alone: nothing is broadcast, the server's view and the bar stand; a
 *     `set_param` into the driven port is refused by the server with the
 *     reason and the text stands;
 *   - speed and reset; a transport-driven port is hidden on the node and in
 *     the inspector; an observer gets the bar read-only;
 *   - (a second pipeline, written into the scratch project) two cycles and
 *     a clock: every inspector row shows the value ITS port is fed, on its
 *     own loop; a wire a human wrote into `cycle.frame` is drawn, named as
 *     the headless source, and removable from the canvas; a `connect` into
 *     a transport-driven port is refused by the server.
 */
import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "08-orbit.cic";

interface TransportView {
  playing: boolean;
  speed: number;
  t_ms: number;
  frame: number;
  frames: number;
  period_ms: number;
  driven: { node: string; port: string; signal: string; loop?: { frames: number; period_ms: number } }[];
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
  graph: { wires: { id: string; from: { node: string; port: string }; to: { node: string; port: string } }[] };
  notices?: unknown;
}

async function debugState(page: Page, wait: boolean, pipeline: string = PIPELINE): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${pipeline}&wait=${wait}`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

async function send(page: Page, message: unknown): Promise<void> {
  await page.evaluate((msg) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(msg);
  }, message);
}

/** Set the scrubber like a drag would (React reads the `input` event) and wait for the server to paint the frame. */
async function seekTo(page: Page, frame: number, pipeline: string = PIPELINE): Promise<DebugState> {
  await page.getByTestId("tr-scrub").evaluate((el, f) => {
    const input = el as HTMLInputElement;
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    set?.call(input, String(f));
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, frame);
  const state = await debugState(page, true, pipeline);
  expect(state.transport.frame).toBe(frame);
  return state;
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

  // ---- a refused control broadcasts nothing: the error toast is the whole
  // answer and the bar keeps the last view. The bar offers neither a seek
  // beyond the loop nor a speed above the server's 64× bound; a script on
  // the same socket can send both.
  await send(page, { type: "transport_seek", payload: { frame: 500 } });
  await expect(page.locator(".notice", { hasText: "frame 500 is outside the loop (frames 0..120)" }).first()).toBeVisible();
  await send(page, { type: "transport_speed", payload: { factor: 1e300 } });
  await expect(page.locator(".notice", { hasText: "speed must be at most 64×, got 1e300" }).first()).toBeVisible();
  const refused = await debugState(page, true);
  expect(refused.transport, "a refused control changes nothing").toMatchObject({ playing: false, frame: 31, speed: 1 });
  expect(await bar(page), "the bar shows the view that stands").toMatchObject({ playing: "false", frame: 31, speed: 1 });

  // ---- the server owns the driven-port rule: a `set_param` into
  // `spin.frame` — no editor reaches it in the UI, any script on the socket
  // can — is refused with the reason, and the text stands.
  await send(page, { type: "set_param", payload: { node: "spin", port: "frame", value: "5" } });
  await expect(
    page
      .locator(".notice", {
        hasText: /`spin`: `frame` is driven by the transport — the session fills it with the loop frame whatever the text says, so a param edit cannot set it in the app/,
      })
      .first(),
  ).toBeVisible();
  const untouched = await debugState(page, true);
  expect(untouched.text_hash, "the refused edit never reached the file").toBe(textHash);
  expect(untouched.history.depth).toBe(0);
  expect(untouched.text).not.toContain("frame=");

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

// Two cycles and a clock, one of the cycles hand-wired. The scrubber is the
// PRIMARY loop's (`slow`, the longest period); `fast` loops inside it four
// times; `tick` is seconds. Written into the scratch project (the suite's
// copy of `examples/`) and opened by name — the orbit stays as it is.
const LOOPS = "transport-loops.cic";
const LOOPS_TEXT =
  "# cicada 1\n" +
  "n = 7\n" +
  "slow = cycle(period=8.0, frames=40)\n" +
  "fast = cycle(period=2.0, frames=60, frame=n)\n" +
  "tick = clock(speed=1.0)\n" +
  "a = slow + fast\n" +
  "b = a + tick\n";

test("two cycles and a clock: each inspector row shows its own loop's frame; a hand-wired `frame` is drawn, named and removable; the server refuses wiring into the port", async ({
  page,
}) => {
  writeFileSync(join(meta.scratch, "examples", LOOPS), LOOPS_TEXT);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${LOOPS}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.getByTestId("transport")).toBeVisible();
  await expect(page.getByTestId("tr-driven")).toHaveText("drives slow.frame +2");

  // ---- at rest: the primary loop is `slow`; every driven frame port
  // carries ITS OWN loop; the clock carries none.
  const rest = await debugState(page, true, LOOPS);
  expect(rest.summary.red + rest.summary.blocked, "the hand-wired `frame` is not a red").toBe(0);
  expect(rest.transport).toMatchObject({ playing: false, frame: 0, frames: 40, period_ms: 8000 });
  // The driven list is in lowering order; keyed so the order is not asserted.
  const drivenByNode = Object.fromEntries(rest.transport.driven.map((d) => [d.node, d]));
  expect(drivenByNode).toEqual({
    slow: { node: "slow", port: "frame", signal: "frame", loop: { frames: 40, period_ms: 8000 } },
    fast: { node: "fast", port: "frame", signal: "frame", loop: { frames: 60, period_ms: 2000 } },
    tick: { node: "tick", port: "t", signal: "time" },
  });
  const textHash = rest.text_hash;
  await expect(page.getByTestId("tr-frame")).toHaveText("0 / 40");

  // ---- the hand-wired port: the text's wire `n.out → fast.frame` is in the
  // graph AND drawn on the canvas — the driven row keeps a handle for it (not
  // connectable); the unwired `slow.frame` has none.
  const wire = rest.graph.wires.find((w) => w.to.node === "fast" && w.to.port === "frame");
  expect(wire, JSON.stringify(rest.graph.wires)).toBeDefined();
  expect(wire!.from).toEqual({ node: "n", port: "out" });
  const edge = page.locator(`.react-flow__edge[data-id='${wire!.id}']`);
  await expect(edge).toHaveCount(1);
  await expect(page.locator(".react-flow__edge")).toHaveCount(rest.graph.wires.length);
  const fastHandle = page.locator(".react-flow__node[data-id='fast'] .react-flow__handle[data-handleid='frame']");
  await expect(fastHandle).toHaveCount(1);
  await expect(fastHandle).not.toHaveClass(/connectable/);
  await expect(page.locator(".react-flow__node[data-id='slow'] .react-flow__handle[data-handleid='frame']")).toHaveCount(0);
  const fastRow = page.getByTestId("driven-fast-frame");
  await expect(fastRow).toHaveAttribute("data-wired", "n.out");
  await expect(fastRow).toHaveAttribute("data-driven", "true");
  await expect(fastRow).toHaveAttribute("title", /The text wires `frame=n` — the headless source/);
  await expect(fastRow).not.toHaveAttribute("title", /not wired/);

  // ---- seek the primary loop to frame 10 (2 s): `slow` is at frame 10 of
  // 40, `fast` has come round to frame 0 of 60, `tick` reads 2.00 s — each
  // inspector row on its own loop, and the frame the lowering injected.
  const at10 = await seekTo(page, 10, LOOPS);
  expect(at10.transport.t_ms).toBe(2000);
  await expect(page.getByTestId("tr-frame")).toHaveText("10 / 40");
  await page.locator(".react-flow__node[data-id='fast'] .cn-header").click();
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "fast");
  await expect(page.getByTestId("in-frame")).toHaveCount(0);
  const fastInsp = page.getByTestId("driven-frame");
  await expect(fastInsp).toContainText("frame 0 of 60");
  await expect(fastInsp).not.toContainText("of 40");
  await expect(fastInsp).toHaveAttribute("data-wired", "n.out");
  await expect(page.getByTestId("driven-frame-wired")).toContainText("headless ← n.out");
  await expect(fastInsp).not.toContainText("not wired");
  // `fast.out` is 0 / 60 = 0 — the value of the frame the row names.
  await expect(page.getByTestId("out-out").locator(".samples li").first()).toHaveText("0");
  await page.locator(".react-flow__node[data-id='slow'] .cn-header").click();
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "slow");
  await expect(page.getByTestId("driven-frame")).toContainText("frame 10 of 40");
  await page.locator(".react-flow__node[data-id='tick'] .cn-header").click();
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "tick");
  await expect(page.getByTestId("in-t")).toHaveCount(0);
  await expect(page.getByTestId("driven-t")).toContainText("2.00 s");
  await expect(page.getByTestId("driven-t")).toHaveAttribute("data-signal", "time");
  // A seek is never an edit.
  expect(at10.text_hash).toBe(textHash);
  expect(at10.history.depth).toBe(0);

  // ---- the server owns the rule: `connect` into a transport-driven port is
  // refused (the canvas cannot even offer it — no handle to drop on), and
  // the text never moves.
  await send(page, {
    type: "connect",
    payload: { from: { node: "n", port: "out" }, to: { node: "slow", port: "frame" }, lift: false },
  });
  await expect(page.locator(".notice", { hasText: /`slow`: `frame` is driven by the transport/ }).first()).toBeVisible();
  const refused = await debugState(page, true, LOOPS);
  expect(refused.text_hash).toBe(textHash);
  expect(refused.history.depth).toBe(0);
  expect(refused.graph.wires.some((w) => w.to.node === "slow" && w.to.port === "frame")).toBe(false);

  // ---- the hand-wired wire is removable from the canvas: drag its target
  // anchor off every handle and release — the one kwarg goes, the edge and
  // the handle with it, and the inspector's row drops the headless note.
  await page.locator(".react-flow__pane").click({ position: { x: 8, y: 8 } });
  const anchor = await edge.locator(".react-flow__edgeupdater-target").boundingBox();
  if (anchor === null) throw new Error("no target anchor on the n→fast.frame wire");
  const pane = await page.locator(".react-flow__pane").boundingBox();
  if (pane === null) throw new Error("no pane");
  await page.mouse.move(anchor.x + anchor.width / 2, anchor.y + anchor.height / 2);
  await page.mouse.down();
  await page.mouse.move(pane.x + pane.width - 40, pane.y + pane.height - 40, { steps: 10 });
  await page.mouse.up();
  await expect.poll(async () => (await debugState(page, true, LOOPS)).text).toContain("fast = cycle(period=2.0, frames=60)\n");
  const unwired = await debugState(page, true, LOOPS);
  expect(unwired.text).not.toContain("frame=");
  expect(unwired.history.depth).toBe(1);
  expect(unwired.graph.wires.some((w) => w.to.node === "fast" && w.to.port === "frame")).toBe(false);
  expect(unwired.summary.red + unwired.summary.blocked).toBe(0);
  await expect(edge).toHaveCount(0);
  await expect(fastHandle).toHaveCount(0);
  await expect(fastRow).not.toHaveAttribute("data-wired", /.+/);
  await expect(fastRow).toHaveAttribute("data-driven", "true");
  await page.locator(".react-flow__node[data-id='fast'] .cn-header").click();
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "fast");
  await expect(page.getByTestId("driven-frame")).toContainText("frame 0 of 60");
  await expect(page.getByTestId("driven-frame")).not.toContainText("headless");
  // Still paused where the seek left it — the edit kept the playhead.
  expect(unwired.transport).toMatchObject({ playing: false, frame: 10, t_ms: 2000 });

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
