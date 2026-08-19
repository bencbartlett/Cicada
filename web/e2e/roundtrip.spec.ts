/**
 * Canvas round-trip (doc 15 DoD, measurement criterion 4): the two
 * directions of the code-is-truth contract, measured.
 *
 *   A. Canvas → text: place + wire through the same op pipeline the UI uses
 *      (intents on `window.__cicada.send`), then read the served `.cic` from
 *      disk and assert it is BYTE-EXACT the writer's expected text (docs/10
 *      minimal-edit writer: append `name = func()`, rewrite one kwarg).
 *   B. Text → canvas: edit the `.cic` on disk and measure, from the write to
 *      the canvas showing the new node (the DOM node AND the store graph),
 *      the elapsed ms. 5 trials, each asserted < 500 ms; all five reported.
 *
 * Runs against the REAL `cicada serve` started by `playwright.config.ts`
 * over a SCRATCH copy of `examples/` (the app writes the files, so this must
 * never touch the repo's examples). This test writes its OWN pipeline into
 * that scratch project so it never collides with the smoke's 02-solids.
 */
import { expect, test, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "roundtrip.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

// The exact starting text — a slider, a domain, a box — with a trailing
// newline (the writer preserves it).
const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "span = construct_domain(start=0.0, end=size)\n" +
  "block = box(x=span, y=span, z=span)\n";

// After placing `sphere` (auto-named `sphere_1`, appended after `block`)
// and wiring `size` → `sphere_1.radius` (bare reference: `size` is a param):
const EXPECTED_AFTER_PLACE_WIRE = START + "sphere_1 = sphere(radius=size)\n";

interface StoreNode {
  name: string;
}
interface StoreGraph {
  nodes: StoreNode[];
}

async function graphNames(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { graph: StoreGraph } } };
    return w.__cicada.state().graph.nodes.map((n) => n.name);
  });
}

async function send(page: Page, message: unknown): Promise<void> {
  await page.evaluate((msg) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(msg);
  }, message);
}

test.describe.configure({ mode: "serial" });

test("canvas ↔ text round-trip: byte-exact writer output, and file edits reach the canvas < 500 ms", async ({
  page,
}, testInfo) => {
  writeFileSync(FILE, START);

  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  // Three bindings (`# cicada 1` is the version pragma, not a node).
  await expect(page.locator(".react-flow__node")).toHaveCount(3);

  // ---- A. Canvas → text: place + wire via intents, assert byte-exact file.
  await send(page, { type: "place_node", payload: { func: "sphere" } });
  await expect(page.locator(".react-flow__node[data-id='sphere_1']")).toBeVisible();
  await send(page, {
    type: "connect",
    payload: { from: { node: "size", port: "out" }, to: { node: "sphere_1", port: "radius" }, lift: false },
  });
  await expect
    .poll(async () => readFileSync(FILE, "utf8"))
    .toBe(EXPECTED_AFTER_PLACE_WIRE);
  const afterWire = readFileSync(FILE, "utf8");
  expect(afterWire, "place + wire is byte-exact the writer's expected text").toBe(
    EXPECTED_AFTER_PLACE_WIRE,
  );
  // The wired node solves green (radius now fed) — the round trip is live, not just textual.
  await expect
    .poll(async () => {
      const r = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
      const s = (await r.json()) as { statuses: Record<string, { state: string }> };
      return s.statuses["sphere_1"]?.state;
    })
    .toBe("done");

  // ---- B. Text → canvas: edit the file on disk, measure write → canvas.
  const trials: { trial: number; node: string; elapsed_ms: number }[] = [];
  let current = readFileSync(FILE, "utf8");
  for (let i = 1; i <= 5; i += 1) {
    const node = `edit_${i}`;
    const line = `${node} = unit_x(factor=${i}.0)\n`;
    const next = current + line;
    const before = new Set(await graphNames(page));
    const t0 = Date.now();
    writeFileSync(FILE, next);
    current = next;
    // The watcher (80 ms coalesce) reloads and broadcasts a snapshot; the
    // store mirrors it. Wait for BOTH the store graph and the DOM node.
    await expect
      .poll(async () => (await graphNames(page)).includes(node), { timeout: 5_000, intervals: [10] })
      .toBe(true);
    await expect(page.locator(`.react-flow__node[data-id='${node}']`)).toBeVisible();
    const elapsed = Date.now() - t0;
    expect(before.has(node), "the node was genuinely new this trial").toBe(false);
    trials.push({ trial: i, node, elapsed_ms: elapsed });
  }

  await testInfo.attach("roundtrip-trials.json", {
    body: JSON.stringify({ file_edit_to_canvas_ms: trials }, null, 2),
    contentType: "application/json",
  });
  const summary = trials.map((t) => `#${t.trial} ${t.elapsed_ms} ms`).join(", ");
  console.log(`[roundtrip] file edit → canvas, 5 trials: ${summary}`);
  for (const t of trials) {
    expect(t.elapsed_ms, `trial ${t.trial} (${t.node}) must reach the canvas < 500 ms`).toBeLessThan(500);
  }

  expect(errors, errors.join("\n")).toEqual([]);
});
