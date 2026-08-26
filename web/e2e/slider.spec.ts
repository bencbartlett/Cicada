/**
 * Collapsed sliders (docs/17 wave 4 B4 — finding U11, Ben's words:
 * "sliders collapse to a single-unit-tall node (GH-like); refuse when
 * min/max/step are wired"). Through the real app: the node's context menu
 * collapses `size` — ONE op `collapse size`, the sidecar's `collapsed`
 * override on disk, the text untouched, the node ONE grid unit tall with
 * name, track and value on one row; the collapsed track still drags (a
 * `set_param`); the inspector's action expands it back (the override gone,
 * the sidecar file with it); Ctrl+Z walks both ways. `bound` has a wired
 * `max`: the menu item mirrors the reason as its hint, the click is refused
 * by the SERVER with a notice, and nothing is written.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file.
 */
import { expect, test, type Page } from "@playwright/test";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "slider.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);
const SIDECAR = `${FILE}.layout.json`;

// `long_named` and the even longer one are for the collapsed row's
// name-first layout (wave 5 N1): a name that fits within the room the
// track's 40 % floor leaves, and one that cannot.
const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "bound = slider(value=1.0, min=0.0, max=size)\n" +
  "driven = slider(value=size, min=0.0, max=10.0)\n" +
  "long_named = slider(value=2.0, min=0.5, max=5.0)\n" +
  "an_even_longer_slider_name_that_cannot_fit_beside_the_track = slider(value=2.0, min=0.5, max=5.0)\n";
const SLIDERS = ["size", "bound", "driven", "long_named", "an_even_longer_slider_name_that_cannot_fit_beside_the_track"];

interface DebugState {
  text: string;
  history: { can_undo: boolean; can_redo: boolean; undo_label: string | null; redo_label: string | null; depth: number };
  ops: { label: string }[];
  graph: { nodes: { name: string; size: [number, number]; collapsed?: boolean; manual: boolean }[] };
  statuses: Record<string, { state: string; message?: string }>;
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

/** Wait until `node` has SOLVED; returns `done` or `cached`. */
async function solvedState(page: Page, node: string): Promise<string> {
  let state = "";
  await expect
    .poll(async () => {
      state = (await debugState(page)).statuses[node]?.state ?? "";
      return state === "done" || state === "cached";
    })
    .toBe(true);
  return state;
}

const node = (page: Page, name: string) => page.locator(`.react-flow__node[data-id='${name}']`);
const face = (page: Page, name: string) => node(page, name).locator(".cn");

/** The node face's rendered height in grid units (`style.height / unitPx`). */
async function heightUnits(page: Page, name: string): Promise<number> {
  return face(page, name).evaluate((el) => {
    const w = window as unknown as { __cicada: { state: () => { hello: { unitPx: number } | null } } };
    const unit = w.__cicada.state().hello?.unitPx ?? 24;
    return Number.parseFloat((el as HTMLElement).style.height) / unit;
  });
}

test("a slider collapses to one grid unit from the menu and expands from the inspector — one op each, sidecar only; a wired bound is refused by the server with a notice", async ({
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
  await expect(page.locator(".react-flow__node")).toHaveCount(SLIDERS.length);
  for (const name of SLIDERS) await solvedState(page, name);
  expect((await debugState(page)).history.depth).toBe(0);
  expect(existsSync(SIDECAR), "nothing moved yet: no sidecar").toBe(false);

  // ---- the expanded slider: header + five port rows (value, min, max,
  // step and — since item 5 — scrub) + the widget row.
  expect(await heightUnits(page, "size")).toBe(7);
  await expect(face(page, "size")).not.toHaveAttribute("data-collapsed", "true");

  // ---- collapse from the node's context menu: ONE op `collapse size`.
  await node(page, "size").click({ button: "right", position: { x: 10, y: 8 } });
  const menu = page.getByTestId("context-menu");
  const collapseItem = menu.getByRole("menuitem", { name: /^collapse/ });
  await expect(collapseItem).toBeVisible();
  await expect(collapseItem, "literal bounds: no refusal to mirror").toHaveText(/one row/);
  await collapseItem.click();
  await expect(face(page, "size")).toHaveAttribute("data-collapsed", "true");
  const collapsedState = await debugState(page);
  expect(collapsedState.history).toMatchObject({ depth: 1, undo_label: "collapse size", can_redo: false });
  expect(collapsedState.ops.map((op) => op.label)).toEqual(["collapse size"]);
  expect(collapsedState.text, "sidecar only: the text is untouched").toBe(START);
  expect(readFileSync(FILE, "utf8")).toBe(START);
  const sizeView = collapsedState.graph.nodes.find((n) => n.name === "size");
  expect(sizeView?.collapsed).toBe(true);
  expect(sizeView?.size[1], "one grid unit tall in the view-model").toBe(1);
  expect(existsSync(SIDECAR), "the override is persisted at once").toBe(true);
  expect(JSON.parse(readFileSync(SIDECAR, "utf8")) as unknown).toMatchObject({
    overrides: { size: { collapsed: true } },
  });
  // The face: ONE row — name, track, value — one unit tall, no header, no input handles.
  expect(await heightUnits(page, "size")).toBe(1);
  await expect(page.getByTestId("collapsed-size")).toHaveText("size");
  await expect(node(page, "size").locator(".cn-header")).toHaveCount(0);
  await expect(node(page, "size").locator(".react-flow__handle.target")).toHaveCount(0);
  await expect(node(page, "size").locator(".react-flow__handle.source")).toHaveCount(1);
  await expect(page.getByTestId("slider-value-size")).toHaveText("2.0");
  // `bound` is untouched.
  await expect(face(page, "bound")).not.toHaveAttribute("data-collapsed", "true");

  // ---- the collapsed track is the same widget: a keyboard step commits a
  // `set_param` (End = the max).
  const range = page.getByTestId("slider-size");
  await range.focus();
  await page.keyboard.press("End");
  await expect.poll(async () => (await debugState(page)).text).toContain("size = slider(value=5.0, min=0.5, max=5.0)");
  await expect(page.getByTestId("slider-value-size")).toHaveText("5.0");
  expect((await debugState(page)).history.depth).toBe(2);
  await expect(face(page, "size"), "a param edit keeps the collapse").toHaveAttribute("data-collapsed", "true");

  // ---- expand from the inspector: ONE op `expand size`; the override
  // cleared → no overrides → no sidecar file.
  await node(page, "size").click({ position: { x: 10, y: 12 } });
  const action = page.getByTestId("action-collapse");
  await expect(action).toHaveText("expand");
  await action.click();
  await expect(face(page, "size")).not.toHaveAttribute("data-collapsed", "true");
  const expandedState = await debugState(page);
  expect(expandedState.history).toMatchObject({ depth: 3, undo_label: "expand size" });
  expect(expandedState.graph.nodes.find((n) => n.name === "size")?.size[1]).toBe(7);
  expect(await heightUnits(page, "size")).toBe(7);
  await expect.poll(() => existsSync(SIDECAR), "no override left → no file").toBe(false);
  await expect(action).toHaveText("collapse");

  // ---- Ctrl+Z walks back: the expand undone is collapsed again, then the
  // slider value, then the collapse itself (sidecar-only ops are undo steps).
  await page.locator(".react-flow__pane").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Control+z");
  await expect(face(page, "size")).toHaveAttribute("data-collapsed", "true");
  expect(await heightUnits(page, "size")).toBe(1);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await debugState(page)).text).toBe(START);
  await expect(face(page, "size"), "undoing the value edit keeps the collapse").toHaveAttribute("data-collapsed", "true");
  await page.keyboard.press("Control+z");
  await expect(face(page, "size")).not.toHaveAttribute("data-collapsed", "true");
  expect((await debugState(page)).history.depth).toBe(0);
  await expect.poll(() => existsSync(SIDECAR)).toBe(false);

  // ---- `bound` has a wired `max`, `driven` a wired `value` (the track
  // itself, so it has no widget): the menu mirrors the reason, the SERVER
  // refuses with a notice, nothing is written and no op is pushed. The
  // hint is the server's own words — the notice carries it verbatim, which
  // is what holds the client mirror (`collapseHint`) to the server's rule
  // (`collapse_refusal`).
  const notices = page.getByTestId("notices");
  for (const [name, reason] of [
    ["bound", "max is wired"],
    ["driven", "value is wired"],
  ] as const) {
    await node(page, name).click({ button: "right", position: { x: 10, y: 8 } });
    const item = menu.getByRole("menuitem", { name: /^collapse/ });
    await expect(item).toBeVisible();
    const hint = (await item.locator(".cv-menu-hint").textContent()) ?? "";
    expect(hint, `${name}: the menu hint is the mirror's reason`).toBe(reason);
    await item.click();
    await expect(notices).toContainText(
      `\`${name}\`: ${hint} — a slider collapses only while value, min, max and step are literals`,
    );
    await expect(face(page, name)).not.toHaveAttribute("data-collapsed", "true");
    const refused = await debugState(page);
    expect(refused.history.depth, `${name}: a refusal is not an op`).toBe(0);
    expect(refused.text).toBe(START);
    expect(existsSync(SIDECAR)).toBe(false);
    // The inspector's action says the same before the click.
    await node(page, name).click({ position: { x: 10, y: 8 } });
    await expect(page.getByTestId("action-collapse")).toHaveAttribute("data-blocked", reason);
    // … and so does the chevron on the face (wave 5 N1), greyed.
    await expect(page.getByTestId(`chevron-${name}`)).toHaveAttribute("data-blocked", reason);
    await expect(page.getByTestId(`chevron-${name}`)).toHaveClass(/blocked/);
  }

  // ---- wave 5 N1 (finding U17): the chevron ON the face. The expanded
  // `size` wears it centred on its bottom edge — an absolutely positioned
  // tab, so the face keeps the server's seven units — and one click is ONE
  // op `collapse size`; the collapsed row wears its twin before the output
  // handle, and one click there is `expand size`.
  await page.locator(".react-flow__pane").click({ position: { x: 5, y: 5 } });
  const chevron = page.getByTestId("chevron-size");
  await expect(chevron).toHaveClass(/expanded/);
  expect(await heightUnits(page, "size")).toBe(7);
  const faceBox = (await face(page, "size").boundingBox())!;
  const tabBox = (await chevron.boundingBox())!;
  expect(Math.abs(tabBox.x + tabBox.width / 2 - (faceBox.x + faceBox.width / 2)), "centred").toBeLessThan(2);
  expect(Math.abs(tabBox.y + tabBox.height / 2 - (faceBox.y + faceBox.height)), "on the bottom edge").toBeLessThan(6);
  await chevron.click();
  await expect(face(page, "size")).toHaveAttribute("data-collapsed", "true");
  expect((await debugState(page)).history).toMatchObject({ depth: 1, undo_label: "collapse size", can_redo: false });
  await expect(chevron).toHaveClass(/collapsed/);
  const handleBox = (await node(page, "size").locator(".react-flow__handle.source").boundingBox())!;
  const rowChevron = (await chevron.boundingBox())!;
  expect(rowChevron.x + rowChevron.width, "before the output handle").toBeLessThanOrEqual(handleBox.x + 1);

  // ---- the collapsed value is editable: double-click → the chip editor in
  // its place; Enter = ONE `set_param` through the literal rule; Esc cancels;
  // an unspellable value is a notice and no write; a single click selects.
  await page.getByTestId("slider-value-size").dblclick();
  const editor = page.getByTestId("slider-value-size-input");
  await expect(editor).toHaveValue("2.0");
  await editor.fill("3.5");
  await editor.press("Enter");
  await expect.poll(async () => (await debugState(page)).text).toContain("size = slider(value=3.5, min=0.5, max=5.0)");
  await expect(page.getByTestId("slider-value-size")).toHaveText("3.5");
  expect((await debugState(page)).history).toMatchObject({ depth: 2, undo_label: "set size.value = 3.5" });
  await expect(face(page, "size"), "a typed value keeps the collapse").toHaveAttribute("data-collapsed", "true");
  await page.getByTestId("slider-value-size").dblclick();
  await page.getByTestId("slider-value-size-input").fill("4.0");
  await page.getByTestId("slider-value-size-input").press("Escape");
  await expect(page.getByTestId("slider-value-size")).toHaveText("3.5");
  await page.getByTestId("slider-value-size").dblclick();
  await page.getByTestId("slider-value-size-input").fill("1/2");
  await page.getByTestId("slider-value-size-input").press("Enter");
  await expect(notices).toContainText('size.value: "1/2" is not a valid number — nothing written');
  const afterTyping = await debugState(page);
  expect(afterTyping.history.depth, "Esc and a refusal write nothing").toBe(2);
  expect(afterTyping.text).toContain("value=3.5");
  await page.getByTestId("slider-value-size").click();
  await expect(node(page, "size")).toHaveClass(/selected/);
  await expect(page.getByTestId("node-inspect")).toHaveAttribute("data-node", "size");
  await chevron.click();
  await expect(face(page, "size")).not.toHaveAttribute("data-collapsed", "true");
  expect((await debugState(page)).history).toMatchObject({ depth: 3, undo_label: "expand size" });

  // ---- name first: the collapsed row lays out name · track · value · tail
  // with the name never truncated until the track has shrunk to 40 % of
  // its FULL width (the width it has with no name at all), and only then
  // with an ellipsis. `long_named` fits: its box equals its scroll width
  // and the track is what the name leaves; the even longer name is cut at
  // exactly the point where the track sits at its floor — no earlier.
  for (const name of ["size", "long_named", "an_even_longer_slider_name_that_cannot_fit_beside_the_track"]) {
    await page.getByTestId(`chevron-${name}`).click();
    await expect(face(page, name)).toHaveAttribute("data-collapsed", "true");
  }
  const fits = await collapsedGeometry(page, "long_named");
  expect(fits.nameScroll, `long_named is whole: ${JSON.stringify(fits)}`).toBeLessThanOrEqual(fits.nameClient + 1);
  expect(fits.track, "the track shrank to make room").toBeLessThan(fits.full - 10);
  expect(fits.track, "… and stays at or above its 40 % floor").toBeGreaterThanOrEqual(fits.floor - 1);
  expect(Math.abs(fits.track + fits.name - fits.full), "name and track share the room").toBeLessThan(2);
  const cut = await collapsedGeometry(page, "an_even_longer_slider_name_that_cannot_fit_beside_the_track");
  expect(cut.nameScroll, `the long name is cut: ${JSON.stringify(cut)}`).toBeGreaterThan(cut.nameClient + 5);
  expect(Math.abs(cut.track - cut.floor), "… only once the track is at its floor, not before").toBeLessThan(2);
  expect(cut.track, "≥ 40 % of the full track").toBeGreaterThanOrEqual(0.4 * cut.full - 1);
  // The short name never touches the floor.
  const short = await collapsedGeometry(page, "size");
  expect(short.nameScroll).toBeLessThanOrEqual(short.nameClient + 1);
  expect(short.track).toBeGreaterThan(short.floor + 10);

  expect(errors, errors.join("\n")).toEqual([]);
});

/**
 * The collapsed row's geometry in layout px (the canvas zoom divided out):
 * the name's box and scroll width, the track's width, the FULL track width
 * (the row's content minus the value label, the tail and the three gaps —
 * what the track has with no name) and the 40 % floor of it.
 */
async function collapsedGeometry(
  page: Page,
  name: string,
): Promise<{ name: number; nameScroll: number; nameClient: number; track: number; full: number; floor: number }> {
  return face(page, name).evaluate((el) => {
    const row = el.querySelector(".cn-collapsed-row") as HTMLElement;
    const nameEl = row.querySelector(".cn-collapsed-name") as HTMLElement;
    const range = row.querySelector("input[type='range']") as HTMLElement;
    const value = row.querySelector(".cn-widget-value") as HTMLElement;
    const tail = row.querySelector(".cn-collapsed-tail") as HTMLElement;
    const zoom = row.getBoundingClientRect().width / row.offsetWidth;
    const width = (e: HTMLElement) => e.getBoundingClientRect().width / zoom;
    const style = getComputedStyle(row);
    const content = row.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight);
    const full = content - width(value) - width(tail) - 3 * Number.parseFloat(style.columnGap);
    return {
      name: width(nameEl),
      nameScroll: nameEl.scrollWidth,
      nameClient: nameEl.clientWidth,
      track: width(range),
      full,
      floor: 0.4 * full,
    };
  });
}
