/**
 * The menu bar (docs/16 §Application layout; docs/17 wave 5 M1, finding
 * U27) against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file (placing writes it):
 * Maths opens a panel with ≥ 2 columns — exactly the server's `subgroups`
 * row for the category, in its order; hovering another tab switches; an
 * outside click, Esc and a re-click close; a click on a node places it at
 * the view's centre cell (± 1), computed here from the DOM independently of
 * the store's `canvasCenter`, and closes the panel; the settings menu no
 * longer offers a ribbon to collapse.
 */
import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "menu.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

const START = "# cicada 1\nsize = slider(value=2.0, min=0.5, max=5.0)\n";

interface DebugState {
  text: string;
  graph: { nodes: { name: string; func?: string; cell: [number, number]; manual: boolean }[] };
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

/** The server's sub-group table row for `category` (format-3 `catalog.subgroups`). */
async function subgroupsOf(page: Page, category: string): Promise<string[]> {
  const response = await page.request.get(`/api/catalog?pipeline=${PIPELINE}`, { headers: { "X-Cicada-Token": TOKEN } });
  expect(response.ok(), await response.text()).toBeTruthy();
  const catalog = (await response.json()) as { format: number; subgroups: { category: string; subgroups: string[] }[] };
  expect(catalog.format).toBe(3);
  const row = catalog.subgroups.find((r) => r.category === category);
  if (row === undefined) throw new Error(`no subgroups row for ${category}`);
  return row.subgroups;
}

/** The columns the open panel shows, top to bottom = left to right. */
async function columnsShown(page: Page): Promise<string[]> {
  return page.getByTestId("menu-panel").locator("[data-testid^='menu-col-']").evaluateAll((cols) =>
    cols.map((col) => col.getAttribute("aria-label") ?? ""),
  );
}

/**
 * The grid cell under the centre of the canvas view, from the DOM alone:
 * the canvas's rect centre through the inverse of React Flow's viewport
 * transform (`translate(tx, ty) scale(z)`), divided by the grid unit — the
 * store's `canvasCenter` is what the menu bar SENDS; this is what the user
 * SEES, so the two must agree for the placement to be where they look.
 */
async function centreCellFromDom(page: Page): Promise<[number, number]> {
  return page.evaluate(() => {
    const canvas = document.querySelector(".cicada-canvas");
    const viewport = document.querySelector<HTMLElement>(".react-flow__viewport");
    if (canvas === null || viewport === null) throw new Error("no canvas");
    const rect = canvas.getBoundingClientRect();
    const m = /translate\(([-\d.e]+)px,\s*([-\d.e]+)px\)\s*scale\(([-\d.e]+)\)/.exec(viewport.style.transform);
    if (m === null) throw new Error(`no viewport transform: ${viewport.style.transform}`);
    const [tx, ty, zoom] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const fx = (rect.width / 2 - tx) / zoom;
    const fy = (rect.height / 2 - ty) / zoom;
    const unit = (window as unknown as { __cicada: { state: () => { hello: { unitPx: number } | null } } }).__cicada.state().hello?.unitPx;
    if (unit === undefined) throw new Error("no hello");
    return [Math.round(fx / unit), Math.round(fy / unit)];
  });
}

test("the menu bar: Maths opens its sub-group columns, hover switches, outside click / Esc / re-click close, a click places at the view's centre", async ({
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

  // ---- tabs alone: nothing below the top bar is persistent.
  const bar = page.getByTestId("menubar");
  await expect(bar).toBeVisible();
  const maths = page.getByTestId("menu-tab-Maths");
  await expect(maths).toBeVisible(); // the catalog is in
  await expect(page.getByTestId("menu-panel")).toHaveCount(0);
  await expect(bar.getByText(/collapse/)).toHaveCount(0);
  expect(await bar.boundingBox().then((b) => b?.height ?? 0), "one row of tabs").toBeLessThan(40);

  // ---- Maths opens: ≥ 2 columns, exactly the server's row for the category, in its order.
  await maths.click();
  const panel = page.getByTestId("menu-panel");
  await expect(panel).toBeVisible();
  await expect(maths).toHaveAttribute("aria-expanded", "true");
  const mathsRow = await subgroupsOf(page, "Maths & logic");
  expect(mathsRow.length).toBeGreaterThanOrEqual(2);
  await expect.poll(() => columnsShown(page)).toEqual(mathsRow);
  await expect(page.getByTestId("menu-col-Operators").getByTestId("menu-node-add")).toBeVisible();
  // The panel lies inside the window, under the bar.
  const panelBox = await panel.boundingBox();
  const barBox = await bar.boundingBox();
  if (panelBox === null || barBox === null) throw new Error("no boxes");
  const width = await page.evaluate(() => window.innerWidth);
  expect(panelBox.x).toBeGreaterThanOrEqual(0);
  expect(panelBox.x + panelBox.width).toBeLessThanOrEqual(width);
  expect(panelBox.y).toBeGreaterThanOrEqual(barBox.y + barBox.height - 2);

  // ---- hovering another tab switches the panel to it.
  await page.getByTestId("menu-tab-Vector").hover();
  await expect(panel).toHaveAttribute("aria-label", "Point · Vector · Plane");
  await expect.poll(() => columnsShown(page)).toEqual(await subgroupsOf(page, "Point · Vector · Plane"));
  await expect(maths).toHaveAttribute("aria-expanded", "false");

  // ---- an outside click closes (on empty canvas, away from the one node).
  const pane = page.locator(".react-flow__pane");
  const paneBox = await pane.boundingBox();
  if (paneBox === null) throw new Error("no canvas pane");
  await pane.click({ position: { x: paneBox.width * 0.9, y: paneBox.height * 0.9 } });
  await expect(panel).toHaveCount(0);

  // ---- Esc closes.
  await maths.click();
  await expect(panel).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);

  // ---- a re-click of the open tab closes.
  await maths.click();
  await expect(panel).toBeVisible();
  await maths.click();
  await expect(panel).toHaveCount(0);
  await expect(maths).toHaveAttribute("aria-expanded", "false");

  // ---- a click on a node places it at the centre of the view and closes the panel.
  const centre = await centreCellFromDom(page);
  const reported = await page.evaluate(
    () => (window as unknown as { __cicada: { state: () => { canvasCenter: [number, number] | null } } }).__cicada.state().canvasCenter,
  );
  expect(reported, "the canvas reported its centre cell after the first fit").not.toBeNull();
  await maths.click();
  await page.getByTestId("menu-node-add").click();
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".react-flow__node")).toHaveCount(2);
  const state = await debugState(page);
  const placed = state.graph.nodes.find((n) => n.func === "add");
  expect(placed, state.text).toBeDefined();
  expect(placed!.manual, "a placement at a cell is a manual cell").toBe(true);
  expect(Math.abs(placed!.cell[0] - centre[0]), `x: placed ${placed!.cell}, centre ${centre}`).toBeLessThanOrEqual(1);
  expect(Math.abs(placed!.cell[1] - centre[1]), `y: placed ${placed!.cell}, centre ${centre}`).toBeLessThanOrEqual(1);
  expect(placed!.cell).toEqual(reported);
  expect(state.text).toContain("add(");

  // ---- the settings menu offers no ribbon to collapse any more.
  await page.getByTestId("tb-settings").click();
  await expect(page.getByRole("dialog", { name: "settings" })).toBeVisible();
  await expect(page.getByText("ribbon collapsed")).toHaveCount(0);
  await page.keyboard.press("Escape");

  expect(errors).toEqual([]);
});
