/**
 * `#off` in the app (docs/17 item 1's rider; docs/10 §1, docs/16 keyboard
 * map row `D`, DECISIONS.md node-disable row): `D` on a selected node
 * prefixes its statement with `#off ` — the node ghosts on the canvas WITH
 * its ports and wiring, downstream goes red for the precise reason, the
 * params panel locks its widget; `D` again re-enables it as a pure cache
 * hit (undo never recomputes, and neither does re-enabling). The context
 * menu offers enable/disable, a multi-select `D` is ONE op, and Ctrl+Z
 * walks both directions.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file. Oracles:
 * `/debug/state?wait=true`, the served file on disk, the store, the DOM.
 */
import { expect, test, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "disable.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "span = construct_domain(start=0.0, end=size)\n" +
  "block = box(x=span, y=span, z=span)\n";
const SPAN_OFF =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "#off span = construct_domain(start=0.0, end=size)\n" +
  "block = box(x=span, y=span, z=span)\n";
const ALL_OFF =
  "# cicada 1\n" +
  "#off size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "#off span = construct_domain(start=0.0, end=size)\n" +
  "#off block = box(x=span, y=span, z=span)\n";

interface DebugState {
  text: string;
  history: { can_undo: boolean; can_redo: boolean; undo_label: string | null; redo_label: string | null; depth: number };
  ops: { label: string }[];
  graph: {
    nodes: {
      name: string;
      kind: string;
      func?: string;
      inputs: { name: string; wired?: { node: string; port: string }; literal?: string }[];
      outputs: { name: string }[];
      excluded?: { status: string; reason: string };
      diagnostics: { message: string }[];
    }[];
    wires: { id: string; red: boolean; reason?: string }[];
  };
  statuses: Record<string, { state: string; message?: string }>;
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

async function lastDeltaLabel(page: Page): Promise<string> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => { lastDeltaLabel: string } } };
    return w.__cicada.state().lastDeltaLabel;
  });
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

test("D ghosts a node with its ports and wiring, D again re-enables it as a cache hit; the menu, a multi-select and Ctrl+Z agree", async ({
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
  await expect(page.locator(".react-flow__node")).toHaveCount(3);
  for (const name of ["size", "span", "block"]) await solvedState(page, name);
  expect((await debugState(page)).history.depth).toBe(0);

  // ---- D on `span`: ONE op `disable span`; the text gains exactly the
  // prefix; the ghost keeps its ports, literal and incoming wire; `block`
  // is red because `span` is disabled (named — never unknown-name).
  await node(page, "span").click({ position: { x: 10, y: 8 } });
  await page.keyboard.press("d");
  await expect.poll(async () => (await debugState(page)).text).toBe(SPAN_OFF);
  expect(readFileSync(FILE, "utf8"), "persisted at once").toBe(SPAN_OFF);
  const off = await debugState(page);
  expect(off.history).toMatchObject({ depth: 1, undo_label: "disable span", can_redo: false });
  expect(off.ops.map((op) => op.label)).toEqual(["disable span"]);
  const ghost = off.graph.nodes.find((n) => n.name === "span");
  if (ghost === undefined) throw new Error("span left the graph");
  expect(ghost.kind).toBe("disabled");
  expect(ghost.func).toBe("construct_domain");
  expect(ghost.inputs.map((i) => i.name), "ports intact").toEqual(["start", "end"]);
  expect(ghost.inputs[0]?.literal).toBe("0.0");
  expect(ghost.inputs[1]?.wired).toEqual({ node: "size", port: "out" });
  expect(ghost.outputs.map((o) => o.name)).toEqual(["out"]);
  expect(ghost.excluded).toEqual({ status: "red", reason: "disabled (`#off`)" });
  expect(off.graph.wires.map((w) => w.id).sort()).toEqual(
    ["size.out->span.end", "span.out->block.x", "span.out->block.y", "span.out->block.z"].sort(),
  );
  const downstream = off.graph.wires.find((w) => w.id === "span.out->block.x");
  expect(downstream?.red, "the wire out of a ghost is red").toBe(true);
  expect(downstream?.reason).toMatch(/`span` is disabled/);
  const block = off.graph.nodes.find((n) => n.name === "block");
  expect(block?.diagnostics.map((d) => d.message).join("\n")).toMatch(/`span` is disabled \(`#off`\)/);
  expect(off.statuses["block"]?.state).toBe("red");
  expect(off.statuses["span"]?.state).toBe("red");
  expect(off.statuses["size"]?.state, "upstream untouched").toBe("cached");

  // The DOM: the same node box, dimmed, with its handles; the wire into it
  // is drawn ghosted, the wires out of it red; the eye is gone.
  const ghostBox = node(page, "span").locator(".cn.cn-disabled");
  await expect(ghostBox).toHaveCount(1);
  await expect(ghostBox).toHaveAttribute("data-state", "off");
  await expect(node(page, "span").locator(".react-flow__handle.target[data-handleid='end']")).toHaveCount(1);
  await expect(node(page, "span").locator(".react-flow__handle.source[data-handleid='out']")).toHaveCount(1);
  await expect(node(page, "span").locator(".cn-func")).toHaveText("disabled (#off)");
  await expect(page.getByTestId("state-span")).toHaveText("off");
  await expect(page.locator(".react-flow__edge[data-id='size.out->span.end'] .cicada-edge.ghost")).toHaveCount(1);
  await expect(page.locator(".react-flow__edge[data-id='span.out->block.x'] .cicada-edge.red")).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath("span-disabled.png") });
  // The inspector (the node is selected): the action says `enable`, the
  // literal is shown read-only (no in-place editor on a ghost), preview is
  // locked; the canvas offers no inline editor either.
  await page.getByTestId("insp-tab-inspect").click();
  await expect(page.getByTestId("action-disable")).toHaveText("enable");
  await expect(page.getByTestId("action-preview")).toBeDisabled();
  await expect(page.getByTestId("insp-lit-span-start")).toHaveCount(0);
  await expect(page.getByTestId("lit-span-start")).toHaveCount(0);
  await expect(node(page, "span").locator(".cn-literal")).toHaveText("0.0");

  // The context menu says `enable` on a ghost (and `disable` on a live node).
  await node(page, "span").click({ button: "right", position: { x: 10, y: 8 } });
  const menu = page.getByTestId("context-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: /^enable/ })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await node(page, "block").click({ button: "right", position: { x: 10, y: 8 } });
  await expect(menu.getByRole("menuitem", { name: /^disable/ })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // ---- D again on `span`: `enable span`, the text is byte-identical to the
  // start, and NOTHING recomputes — both outputs are memo hits.
  await node(page, "span").click({ position: { x: 10, y: 8 } });
  await page.keyboard.press("d");
  await expect.poll(async () => (await debugState(page)).text).toBe(START);
  await expect.poll(async () => lastDeltaLabel(page)).toBe("enable span");
  expect(await solvedState(page, "span"), "re-enabling is a cache hit").toBe("cached");
  expect(await solvedState(page, "block"), "downstream too").toBe("cached");
  const on = await debugState(page);
  expect(on.history).toMatchObject({ depth: 2, undo_label: "enable span" });
  expect(on.graph.nodes.find((n) => n.name === "span")?.kind).toBe("call");
  await expect(node(page, "span").locator(".cn.cn-disabled")).toHaveCount(0);
  await expect(page.getByTestId("action-disable")).toHaveText("disable");
  await expect(page.getByTestId("insp-lit-span-start")).toHaveCount(1);

  // ---- Ctrl+Z walks back through both: ghost again, then live again.
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await debugState(page)).text).toBe(SPAN_OFF);
  await expect.poll(async () => lastDeltaLabel(page)).toBe("undo: enable span");
  await expect(node(page, "span").locator(".cn.cn-disabled")).toHaveCount(1);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await debugState(page)).text).toBe(START);
  await expect.poll(async () => lastDeltaLabel(page)).toBe("undo: disable span");
  expect(await solvedState(page, "block")).toBe("cached");
  expect((await debugState(page)).history).toMatchObject({ depth: 0, can_redo: true });

  // ---- a multi-select D is ONE op (`disable 3 nodes`): every line prefixed,
  // the params panel locks the slider's row; one Ctrl+Z brings all back.
  await page.locator(".react-flow__pane").click({ position: { x: 8, y: 8 } });
  await page.keyboard.press("Control+a");
  await page.keyboard.press("d");
  await expect.poll(async () => (await debugState(page)).text).toBe(ALL_OFF);
  const allOff = await debugState(page);
  expect(allOff.history).toMatchObject({ depth: 1, undo_label: "disable 3 nodes", can_redo: false });
  expect(allOff.ops.map((op) => op.label)).toEqual(["disable 3 nodes"]);
  expect(allOff.graph.nodes.every((n) => n.kind === "disabled")).toBe(true);
  await expect(page.locator(".react-flow__node .cn.cn-disabled")).toHaveCount(3);
  await expect(page.getByTestId("eye-block"), "a ghost has no preview eye").toHaveCount(0);
  await page.getByTestId("insp-tab-params").click();
  const sizeRow = page.getByTestId("param-size");
  await expect(sizeRow).toHaveClass(/param-off/);
  await expect(page.getByTestId("widget-size")).toBeDisabled();
  await expect(page.getByTestId("slider-size")).toBeDisabled();

  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await debugState(page)).text).toBe(START);
  await expect.poll(async () => lastDeltaLabel(page)).toBe("undo: disable 3 nodes");
  for (const name of ["size", "span", "block"]) {
    expect(await solvedState(page, name), `${name} after undoing the batch`).toBe("cached");
  }
  await expect(page.locator(".react-flow__node .cn.cn-disabled")).toHaveCount(0);
  await expect(page.getByTestId("eye-block")).toHaveCount(1);
  await expect(sizeRow).not.toHaveClass(/param-off/);
  await expect(page.getByTestId("widget-size")).toBeEnabled();
  expect(readFileSync(FILE, "utf8")).toBe(START);

  expect(errors, errors.join("\n")).toEqual([]);
});
