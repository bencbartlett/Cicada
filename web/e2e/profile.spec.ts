/**
 * The profiler (docs/17 wave 5 P1; docs/13 §The profiler; docs/16
 * §Inspector contents) against the REAL `cicada serve` from
 * `playwright.config.ts`, on `02-solids.cic`:
 *
 *   - the `profile` button opens the Profile tab; the view names the
 *     session's last complete generation (`/debug/state.profile` is the
 *     oracle), the ring has at least two arcs, the client's own phases are
 *     measured (frames decoded, a first paint), and the table lists every
 *     node of the pipeline with the state the server reports;
 *   - a second generation (a slider change) leaves the nodes the change did
 *     not reach as memo hits: their rows read `cached · last …`, and the
 *     display rows are exactly the outputs that pass drew;
 *   - the table sorts on a header click and filters on the box; Esc closes
 *     the tab; an observer page reads the profile too.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

interface ProfileNode {
  name: string;
  state: string;
  nanos?: number;
  last_nanos?: number;
}

interface DebugState {
  text: string;
  graph: { nodes: { name: string }[] };
  summary: { generation: number; running: boolean };
  solve: { last_complete_generation: number | null };
  profile: {
    generation: number;
    kind: string;
    phases: { solve_ms: number; tessellate_ms: number; encode_ms: number; bytes: number };
    nodes: ProfileNode[];
    display: { node: string; output: string; tier?: string; solids: number }[];
  } | null;
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

function send(page: Page, message: unknown): Promise<void> {
  return page.evaluate((m) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(m);
  }, message);
}

async function role(page: Page): Promise<string> {
  return page.evaluate(() => (window as unknown as { __cicada: { state: () => { role: string } } }).__cicada.state().role);
}

const rowNames = (page: Page) => page.getByTestId("profile-node-row").evaluateAll((rows) => rows.map((r) => r.getAttribute("data-node")));
const rowStates = (page: Page) =>
  page.getByTestId("profile-node-row").evaluateAll((rows) => Object.fromEntries(rows.map((r) => [r.getAttribute("data-node"), r.getAttribute("data-state")])));

/** Wait until the panel shows `generation`'s profile (the read is asynchronous). */
async function shows(page: Page, generation: number): Promise<void> {
  await expect(page.getByTestId("profile-view")).toHaveAttribute("data-generation", String(generation));
}

test.describe.configure({ mode: "serial" });

test("the profiler: the ring, every node with its state and cost, cached rows after a second generation, sort, filter, Esc, an observer", async ({ page, context }, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(() => role(page)).toBe("writer");
  // (No `display` segment is required: a page joining a session at rest
  // hears of no pass until the next one — docs/17 L5-7.)
  await expect(page.getByTestId("tb-solve-text")).toHaveText(/gen \d+ · solve .+/, { timeout: 60_000 });

  // ---- open: the view is the server's last complete generation.
  await page.getByTestId("tb-profile").click();
  await expect(page.getByTestId("insp-tab-profile")).toHaveAttribute("aria-selected", "true");
  const first = await debugState(page);
  expect(first.profile, "the server keeps a profile once a generation completed").not.toBeNull();
  const generation = first.solve.last_complete_generation!;
  expect(first.profile!.generation).toBe(generation);
  await shows(page, generation);
  await expect(page.getByTestId("profile-title")).toHaveText(`gen ${generation} · ${first.profile!.kind}`);

  // The ring: at least two arcs (a node's compute and the tessellation at the least).
  const arcs = page.getByTestId("profile-arc");
  expect(await arcs.count()).toBeGreaterThanOrEqual(2);
  const phases = await arcs.evaluateAll((paths) => paths.map((p) => p.getAttribute("data-phase")));
  expect(phases.some((p) => p?.startsWith("node:")), `a node arc among ${phases.join(", ")}`).toBe(true);
  const legend = page.getByTestId("profile-legend");
  await expect(legend).toContainText("tessellation");

  // The client's own phases: this page decoded the generation's frames (the
  // join's restream carries them at that generation). Its first paint is
  // asserted after the pass this page itself watches, below.
  await expect(page.getByTestId("profile-decode")).toHaveText(/\d+ frames?$/);
  await expect(page.getByTestId("profile-phases")).toContainText(`solve`);

  // The table lists EVERY node of the pipeline — the `#off` export chain
  // included — with the state the server reports for each.
  const names = await rowNames(page);
  for (const node of first.graph.nodes) {
    expect(names, `\`${node.name}\` is a row`).toContain(node.name);
  }
  const states = await rowStates(page);
  for (const node of first.profile!.nodes) {
    expect(states[node.name], `${node.name}'s state`).toBe(node.state);
  }
  // The display rows are what the pass drew, as the server lists them.
  const drawn = await page.getByTestId("profile-display-row").evaluateAll((rows) => rows.map((r) => r.getAttribute("data-output")));
  expect(drawn.sort()).toEqual(first.profile!.display.map((d) => `${d.node}.${d.output}`).sort());
  await page.screenshot({ path: testInfo.outputPath("profile-open.png") });

  // ---- a second generation: the nodes the change did not reach are memo hits.
  await send(page, { type: "set_param", payload: { node: "size", port: "value", value: "2.5" } });
  await expect.poll(async () => (await debugState(page)).text).toContain("size = slider(value=2.5");
  const second = await debugState(page);
  const next = second.solve.last_complete_generation!;
  expect(next).toBeGreaterThan(generation);
  await shows(page, next);
  const after = await rowStates(page);
  for (const node of second.profile!.nodes) {
    expect(after[node.name], `${node.name}'s state after the change`).toBe(node.state);
  }
  // `ball` does not depend on `size`: a cache hit with its last compute.
  expect(after["ball"]).toBe("cached");
  const ballRow = page.locator('[data-testid="profile-node-row"][data-node="ball"]');
  await expect(ballRow).toContainText(/last \S+ (ms|s)/);
  const cachedServer = second.profile!.nodes.find((n) => n.name === "ball")!;
  expect(cachedServer.last_nanos, "the memo entry's recorded cost").toBeGreaterThan(0);
  expect(cachedServer.nanos, "not this generation's").toBeUndefined();
  // (`block` depends on `size` and may still be a memo hit: `size` is
  // scrub-cached in 02-solids and the warmer pre-solves its positions —
  // the loop above holds every row to the server's word either way.)
  const drawnAfter = await page.getByTestId("profile-display-row").evaluateAll((rows) => rows.map((r) => r.getAttribute("data-output")));
  expect(drawnAfter.sort()).toEqual(second.profile!.display.map((d) => `${d.node}.${d.output}`).sort());
  expect(drawnAfter.length).toBeGreaterThan(0);
  // This page watched the pass: its own phases are measured — the frames
  // decoded and uploaded, the socket's share, the first paint.
  await expect(page.getByTestId("profile-decode")).toHaveText(/\d+ frames?$/);
  await expect(page.getByTestId("profile-upload")).toHaveText(/(ms|s)$/);
  // The socket is a residual: a time WITH the rate the bytes make of it, or
  // `—` (with the reason in the hover) when nothing remains of this client's
  // wall once the server's phases are out — never `0.00 ms` (L3-P1-3).
  const socket = page.getByTestId("profile-socket");
  await expect(socket).toHaveText(/^(—|[\d.]+ (ms|s) · [\d.]+ [KMG]?B at [\d.]+ [KM]B\/s)$/);
  if ((await socket.textContent()) === "—") await expect(socket).toHaveAttribute("title", /not measurable/);
  await expect(page.getByTestId("profile-first-paint")).toHaveText(/(ms|s)$/);
  await page.screenshot({ path: testInfo.outputPath("profile-second-generation.png") });

  // ---- nothing but the name column ever clips (L5-2): at the reference
  // 1400 × 900 every numeric cell and every header — a cached row's `last
  // 2.1 ms`, `156.3 KB`, `100.0 %`, `elements` — is as wide as its string;
  // the name column gives way and keeps its hover.
  const clipped = await page.locator(".prof-table th, .prof-table td").evaluateAll((cells) =>
    cells
      .filter((cell) => cell.scrollWidth > cell.clientWidth)
      .map((cell) => `${(cell as HTMLElement).cellIndex}:${cell.textContent?.trim() ?? ""}`),
  );
  expect(clipped.filter((c) => !c.startsWith("0:")), `only the name column may ellipsise; clipped: ${clipped.join(" | ")}`).toEqual([]);
  const untitledNames = await page.locator(".prof-table td:first-child").evaluateAll((cells) =>
    cells.filter((cell) => cell.scrollWidth > cell.clientWidth && cell.querySelector("[title]") === null && !cell.hasAttribute("title")).length,
  );
  expect(untitledNames, "an ellipsised name has its full text in a hover").toBe(0);
  const inspector = page.getByTestId("inspector");
  if ((await inspector.count()) > 0) {
    const box = (await inspector.boundingBox())!;
    const tables = await page.locator(".prof-table").evaluateAll((ts) => ts.map((t) => t.getBoundingClientRect().right));
    for (const right of tables) expect(right, "the table stays inside the panel").toBeLessThanOrEqual(box.x + box.width + 0.5);
  }

  // ---- sort and filter.
  await page.getByTestId("profile-sort-name").click();
  const byName = await rowNames(page);
  expect(byName).toEqual([...byName].sort((a, b) => a!.localeCompare(b!)));
  await page.getByTestId("profile-sort-name").click();
  expect((await rowNames(page))[0]).toBe(byName[byName.length - 1]);
  await page.getByTestId("profile-filter").fill("ball");
  expect(await rowNames(page)).toEqual(["ball"]);
  await page.getByTestId("profile-filter").fill("");
  expect((await rowNames(page)).length).toBe(names.length);

  // ---- Esc closes the tab (the filter box is a text field: blur it first).
  await page.getByTestId("profile-title").click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("insp-tab-inspect")).toHaveAttribute("aria-selected", "true");

  // ---- an observer reads it too.
  const observer = await context.newPage();
  await observer.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(observer.getByTestId("app")).toBeVisible();
  await expect.poll(() => role(observer)).toBe("observer");
  await observer.getByTestId("tb-profile").click();
  await shows(observer, next);
  expect((await rowNames(observer)).length).toBe(names.length);
  await observer.close();

  expect(errors, errors.join("\n")).toEqual([]);
});
