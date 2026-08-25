/**
 * Grasshopper names in search-to-place and port docs on hover (docs/14
 * §node file format: `gh` is "fed to search-to-place"; docs/17 Track C web
 * lane). A migrant types the GH component name — `Series`, `Merge`,
 * `Addition`, `Pick'n'Choose` — and the node that replaces it is the FIRST
 * result, with the GH name as a hint on the row when it differs from the
 * title; Enter places it. Port hovers carry `name: type — doc`, the output
 * doc coming from the catalog (the view-model has none for outputs). Both
 * entry points are driven: the double-click box and the box a wire dropped
 * on empty canvas opens (probe-filtered, placing also wires).
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file (placing writes it).
 */
import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "search.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

const START = "# cicada 1\nsize = slider(value=2.0, min=0.5, max=5.0)\n";

interface DebugState {
  text: string;
  graph: { nodes: { name: string; func?: string }[] };
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

/** The funcs of the search rows, top to bottom. */
async function resultFuncs(page: Page): Promise<string[]> {
  return page.getByTestId("search-item").evaluateAll((items) =>
    items.map((item) => item.getAttribute("data-func") ?? ""),
  );
}

test("a Grasshopper name finds the node that replaces it first, the row says which GH name matched, and port hovers carry the docs", async ({
  page,
}) => {
  writeFileSync(FILE, START);

  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.locator(".react-flow__node")).toHaveCount(1);

  // ---- open search-to-place on empty canvas.
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await pane.dblclick({ position: { x: box.width * 0.6, y: box.height * 0.7 } });
  const search = page.getByTestId("search-input");
  await expect(search).toBeVisible();
  // The catalog must be in before the rows mean anything.
  await expect(page.getByTestId("search-item").first()).toBeVisible();

  // ---- `Merge` is nobody's name or title: only concat's GH name matches,
  // and the row says so.
  await search.fill("Merge");
  await expect.poll(() => resultFuncs(page)).toEqual(["concat"]);
  const concatRow = page.getByTestId("search-item").first();
  await expect(concatRow.getByTestId("search-gh")).toHaveText("GH Merge");

  // ---- `Addition` is add's GH name AND a substring of mass_addition's
  // name, title and GH name: the exact GH hit ranks first.
  await search.fill("Addition");
  await expect.poll(() => resultFuncs(page)).toEqual(["add", "mass_addition"]);

  // ---- Punctuation in a GH name is matched verbatim, case-insensitively.
  await search.fill("pick'n'choose");
  await expect.poll(() => resultFuncs(page)).toEqual(["pick"]);
  await expect(page.getByTestId("search-item").first().getByTestId("search-gh")).toHaveText("GH Pick'n'Choose");

  // ---- `Series` names the node, its title and its GH component: first,
  // and no hint (the GH name says nothing the title does not). Enter places it.
  await search.fill("Series");
  await expect.poll(async () => (await resultFuncs(page))[0]).toBe("series");
  await expect(page.getByTestId("search-item").first().getByTestId("search-gh")).toHaveCount(0);
  await search.press("Enter");
  await expect(page.locator(".react-flow__node[data-id='series_1']")).toBeVisible();
  const placed = await debugState(page);
  expect(placed.text).toContain("series_1 = series(");
  expect(placed.graph.nodes.find((n) => n.name === "series_1")?.func).toBe("series");

  // ---- Port hovers: `name: type — doc`. The input doc rides on the
  // view-model; the output doc (a bare `out`'s `# Returns` line) comes from
  // the catalog entry of the node's func.
  const seriesNode = page.locator(".react-flow__node[data-id='series_1']");
  await expect(seriesNode.locator(".cn-port.cn-in").filter({ hasText: "count" })).toHaveAttribute(
    "title",
    "count: Integer — Number of values.",
  );
  // From the near tier up (the default zoom) the row's hover also carries
  // the value line under the doc — exactly the text the row's
  // `.cn-port-value` shows (docs/16 LOD table, B1); below near it is the
  // doc alone. Both read in ONE evaluate, so the pair is from one render.
  const SERIES_OUT_DOC = "out: [Number] — `count` numbers, `start` first, each `step` after the previous.";
  const hover = await seriesNode.locator(".cn-port.cn-out:not(.cn-empty)").evaluate((el) => ({
    title: el.getAttribute("title"),
    value: el.querySelector(".cn-port-value")?.textContent ?? null,
  }));
  expect(hover.title).toBe(hover.value === null ? SERIES_OUT_DOC : `${SERIES_OUT_DOC}\n${hover.value}`);
  await expect(page.locator(".react-flow__node[data-id='size'] .cn-port.cn-out:not(.cn-empty)")).toHaveAttribute(
    "title",
    /^out: Number — The current value, within `min\.\.=max`\./,
  );

  // The inspector's port rows say the same.
  await seriesNode.click({ position: { x: 10, y: 8 } });
  await expect(page.getByTestId("out-out").locator("span[title]:has(.port-name)")).toHaveAttribute(
    "title",
    "out: [Number] — `count` numbers, `start` first, each `step` after the previous.",
  );
  await expect(page.getByTestId("in-count").locator("span[title]:has(.port-name)")).toHaveAttribute(
    "title",
    "count: Integer — Number of values.",
  );

  // ---- The other entry point: dropping a wire on empty canvas opens the
  // SAME search, filtered by the server's probe to funcs with a port that
  // accepts the wire — and the GH name still finds the node. `series_1.out`
  // is `[Number]`; `Merge` names only concat's GH component, whose `a` and
  // `b` take the list as is (no `· map` chip). Enter places AND wires it.
  await seriesNode.locator(".react-flow__handle.source").hover();
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.85, box.y + box.height * 0.3, { steps: 12 });
  await page.mouse.up();
  const wireSearch = page.getByTestId("search-input");
  await expect(wireSearch).toBeVisible();
  await expect(wireSearch).toHaveAttribute("placeholder", "nodes accepting series_1.out…");
  await wireSearch.fill("Merge");
  await expect.poll(() => resultFuncs(page)).toEqual(["concat"]);
  const wireRow = page.getByTestId("search-item").first();
  await expect(wireRow.getByTestId("search-gh")).toHaveText("GH Merge");
  await expect(wireRow.locator(".cv-search-port")).toHaveText(["a", "b"]);
  await wireSearch.press("Enter");
  await expect(page.getByTestId("search-box")).toHaveCount(0);
  await expect(page.locator(".react-flow__node[data-id='concat_1']")).toBeVisible();
  await expect.poll(async () => (await debugState(page)).text).toContain("concat_1 = concat(a=series_1)");

  expect(errors, errors.join("\n")).toEqual([]);
});

// ---- Grasshopper's slider shortcut (wave 4 B4, finding U10: "slider
// shortcuts like GH's `1<20` and `0.0<0.5<1.0`"). Its own pipeline file:
// the test above writes START into `search.cic` and the session holds it.
const SLIDER_PIPELINE = "search_slider.cic";
const SLIDER_FILE = join(meta.scratch, "examples", SLIDER_PIPELINE);

interface SliderDebugState extends DebugState {
  history: { depth: number; undo_label: string | null };
  statuses: Record<string, { state: string; message?: string }>;
}

async function sliderDebugState(page: Page): Promise<SliderDebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${SLIDER_PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as SliderDebugState;
}

test("`1<20` and `0.0<0.5<1.0` preview and place a slider as ONE op (the typed precision is the step, whole numbers the integer slider); an impossible range is a notice, not a node", async ({
  page,
}) => {
  writeFileSync(SLIDER_FILE, START);

  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${SLIDER_PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.locator(".react-flow__node")).toHaveCount(1);

  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  // Each placement lands on the cell double-clicked, so every search opens
  // on a different empty spot (a double-click on a placed node is not the
  // pane's).
  const openSearch = async (at: { x: number; y: number }) => {
    await pane.dblclick({ position: { x: box.width * at.x, y: box.height * at.y } });
    const search = page.getByTestId("search-input");
    await expect(search).toBeVisible();
    return search;
  };

  // ---- `1<20`: the row previews the integer slider (step 1); no catalog
  // rows compete with it. Enter places `slider_1` with the four literals in
  // port order — ONE op, labelled by the server as a placement.
  let search = await openSearch({ x: 0.6, y: 0.7 });
  await search.fill("1<20");
  const row = page.getByTestId("search-slider");
  await expect(row).toBeVisible();
  await expect(row).toHaveAttribute("data-valid", "true");
  await expect(row).toContainText("integer slider 1.0 … 20.0");
  await expect(row).toContainText("value 1.0 · step 1.0");
  await expect(page.getByTestId("search-item")).toHaveCount(0);
  await search.press("Enter");
  await expect(page.getByTestId("search-box")).toHaveCount(0);
  await expect(page.locator(".react-flow__node[data-id='slider_1']")).toBeVisible();
  let state = await sliderDebugState(page);
  expect(state.text.split("\n")).toContain("slider_1 = slider(value=1.0, min=1.0, max=20.0, step=1.0)");
  expect(state.history).toMatchObject({ depth: 1, undo_label: "place slider" });
  // The placed slider is green, with the step the widget snaps by.
  await expect.poll(async () => (await sliderDebugState(page)).statuses["slider_1"]?.state).toMatch(/^(done|cached)$/);
  await expect(page.getByTestId("slider-slider_1")).toHaveAttribute("step", "1");
  await expect(page.getByTestId("slider-slider_1")).toHaveAttribute("min", "1");
  await expect(page.getByTestId("slider-slider_1")).toHaveAttribute("max", "20");

  // ---- ONE op: a single Ctrl+Z removes the whole slider, literals and all.
  await pane.click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Control+z");
  await expect(page.locator(".react-flow__node[data-id='slider_1']")).toHaveCount(0);
  await expect.poll(async () => (await sliderDebugState(page)).text).toBe(START);
  expect((await sliderDebugState(page)).history.depth).toBe(0);

  // ---- `0.0<0.5<1.0`: min A, value B, max C; one decimal typed → step 0.1.
  search = await openSearch({ x: 0.6, y: 0.7 });
  await search.fill("0.0<0.5<1.0");
  await expect(row).toContainText("slider 0.0 … 1.0");
  await expect(row).toContainText("value 0.5 · step 0.1");
  await search.press("Enter");
  await expect.poll(async () => (await sliderDebugState(page)).text.split("\n")).toContain(
    "slider_1 = slider(value=0.5, min=0.0, max=1.0, step=0.1)",
  );

  // ---- negatives, whitespace, and the most decimals typed: `-1.5<0<1.5`.
  search = await openSearch({ x: 0.3, y: 0.75 });
  await search.fill(" -1.5 < 0 < 1.5 ");
  await expect(row).toContainText("slider -1.5 … 1.5");
  await search.press("Enter");
  await expect.poll(async () => (await sliderDebugState(page)).text.split("\n")).toContain(
    "slider_2 = slider(value=0.0, min=-1.5, max=1.5, step=0.1)",
  );
  state = await sliderDebugState(page);
  expect(state.history.depth).toBe(2);
  await expect.poll(async () => (await sliderDebugState(page)).statuses["slider_2"]?.state).toMatch(/^(done|cached)$/);

  // ---- an impossible range: the row says why in place of the preview,
  // Enter is a notice and the box stays open; nothing is placed.
  search = await openSearch({ x: 0.85, y: 0.25 });
  await search.fill("5<2");
  await expect(row).toHaveAttribute("data-valid", "false");
  await expect(row).toContainText("min 5 must be below max 2");
  await search.press("Enter");
  await expect(page.getByTestId("notices")).toContainText("slider shortcut: min 5 must be below max 2");
  await expect(page.getByTestId("search-box")).toBeVisible();
  await search.fill("0<7<5");
  await expect(row).toContainText("value 7 is outside 0 … 5");
  // A partial shortcut is an ordinary (empty) search again.
  await search.fill("1<");
  await expect(row).toHaveCount(0);
  await expect(page.locator(".cv-search-empty")).toContainText('no node matches "1<"');
  await search.press("Escape");
  await expect(page.getByTestId("search-box")).toHaveCount(0);
  state = await sliderDebugState(page);
  expect(state.history.depth, "refusals place nothing").toBe(2);
  expect(state.graph.nodes.map((n) => n.name).sort()).toEqual(["size", "slider_1", "slider_2"]);

  expect(errors, errors.join("\n")).toEqual([]);
});
