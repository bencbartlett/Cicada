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

const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "bound = slider(value=1.0, min=0.0, max=size)\n" +
  "driven = slider(value=size, min=0.0, max=10.0)\n";

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
  await expect(page.locator(".react-flow__node")).toHaveCount(3);
  for (const name of ["size", "bound", "driven"]) await solvedState(page, name);
  expect((await debugState(page)).history.depth).toBe(0);
  expect(existsSync(SIDECAR), "nothing moved yet: no sidecar").toBe(false);

  // ---- the expanded slider: header + four port rows + the widget row.
  expect(await heightUnits(page, "size")).toBe(6);
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
  expect(expandedState.graph.nodes.find((n) => n.name === "size")?.size[1]).toBe(6);
  expect(await heightUnits(page, "size")).toBe(6);
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
  }

  expect(errors, errors.join("\n")).toEqual([]);
});
