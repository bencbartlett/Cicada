/**
 * Undo/redo in the app (docs/17 item 1 WP-P, docs/13 §Undo/redo, the
 * DECISIONS.md undo row): delete a node, `Ctrl+Z`, the node and its wire
 * are back, the text panel shows the restored line, and the restored
 * outputs are CACHE HITS — undo never recomputes. Also the keyboard-map
 * rows that changed: Backspace does NOT delete (`Del` only), Ctrl+Shift+Z
 * and Ctrl+Y redo, the toolbar buttons redo/undo with the op labels as
 * tooltips, and a multi-select delete is ONE op (a `batch`) that one
 * `Ctrl+Z` reverts whole.
 *
 * Runs against the REAL `cicada serve` started by `playwright.config.ts`
 * over a SCRATCH copy of `examples/`; this test writes its OWN pipeline
 * there so it never collides with the smoke's 02-solids or the round-trip
 * spec. Oracles: `/debug/state?wait=true` (history, ops, statuses, text),
 * the served file on disk, `window.__cicada.state()`, and the DOM.
 */
import { expect, test, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "undo.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

// The exact starting text — a slider, a domain, a box — with a trailing
// newline (the writer preserves it).
const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "span = construct_domain(start=0.0, end=size)\n" +
  "block = box(x=span, y=span, z=span)\n";
// After search-to-place `sphere` (auto-named `sphere_1`) and wiring
// `size` → `sphere_1.radius`:
const WITH_SPHERE = START + "sphere_1 = sphere(radius=size)\n";
const WIRE_ID = "size.out->sphere_1.radius";
const WIRE_LABEL = "wire size.out → sphere_1.radius";

interface HistoryView {
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  depth: number;
}

interface DebugState {
  seq: number;
  text: string;
  history: HistoryView;
  ops: { id: number; label: string; actor: { kind: string }; at: number }[];
  graph: {
    nodes: { name: string; cell: [number, number]; manual: boolean }[];
    wires: { id: string }[];
  };
  statuses: Record<string, { state: string; message?: string }>;
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  seq: number;
  lastDeltaLabel: string;
  history: HistoryView;
  selection: { nodes: string[] };
  graph: { nodes: { name: string }[]; wires: { id: string }[] };
}

async function store(page: Page): Promise<StoreView> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => StoreView } };
    const s = w.__cicada.state();
    return {
      seq: s.seq,
      lastDeltaLabel: s.lastDeltaLabel,
      history: s.history,
      selection: { nodes: s.selection.nodes },
      graph: {
        nodes: s.graph.nodes.map((n) => ({ name: n.name })),
        wires: s.graph.wires.map((wire) => ({ id: wire.id })),
      },
    };
  });
}

async function send(page: Page, message: unknown): Promise<void> {
  await page.evaluate((msg) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(msg);
  }, message);
}

/** Wait until every output of `node` has SOLVED; returns the state word (`done` or `cached`). */
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

const sphere = (page: Page) => page.locator(".react-flow__node[data-id='sphere_1']");
const wireEdge = (page: Page) => page.locator(`.react-flow__edge[data-id='${WIRE_ID}']`);

test.describe.configure({ mode: "serial" });

test("delete → Ctrl+Z restores the node, its wire and the text as a cache hit; Backspace does not delete; redo by button and key; a multi-delete is one undo step", async ({
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

  // A fresh session: nothing to undo, the buttons say so and are disabled.
  const undoButton = page.getByTestId("tb-undo");
  const redoButton = page.getByTestId("tb-redo");
  await expect(undoButton).toBeDisabled();
  await expect(redoButton).toBeDisabled();
  await expect(undoButton).toHaveAttribute("title", /nothing to undo/);
  expect((await debugState(page)).history).toEqual({
    can_undo: false,
    can_redo: false,
    undo_label: null,
    redo_label: null,
    depth: 0,
  });

  // ---- place via search-to-place (double-click the canvas, type, Enter) —
  // the same path a user takes; the click position gives the node a manual
  // cell in the sidecar, which the undo below must restore too.
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await pane.dblclick({ position: { x: box.width * 0.6, y: box.height * 0.75 } });
  const search = page.getByTestId("search-input");
  await expect(search).toBeVisible();
  await search.fill("sphere");
  await search.press("Enter");
  await expect(sphere(page)).toBeVisible();

  // ---- wire size → sphere_1.radius through the same op pipeline the UI uses.
  await send(page, {
    type: "connect",
    payload: { from: { node: "size", port: "out" }, to: { node: "sphere_1", port: "radius" }, lift: false },
  });
  await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(WITH_SPHERE);
  await expect(wireEdge(page)).toHaveCount(1);
  await solvedState(page, "sphere_1");

  const placed = await debugState(page);
  expect(placed.text).toBe(WITH_SPHERE);
  expect(placed.history).toEqual({
    can_undo: true,
    can_redo: false,
    undo_label: WIRE_LABEL,
    redo_label: null,
    depth: 2,
  });
  expect(placed.ops.map((op) => op.label)).toEqual(["place sphere", WIRE_LABEL]);
  expect(placed.ops.every((op) => op.actor.kind === "human")).toBe(true);
  const placedNode = placed.graph.nodes.find((n) => n.name === "sphere_1");
  if (placedNode === undefined) throw new Error("sphere_1 not in the graph");
  expect(placedNode.manual, "search-to-place at a click position gives a manual cell").toBe(true);
  const placedCell = placedNode.cell;
  await expect(undoButton).toBeEnabled();
  await expect(undoButton).toHaveAttribute("title", `undo: ${WIRE_LABEL} (Ctrl+Z)`);
  await expect(redoButton).toBeDisabled();

  // ---- select the node, Backspace: NOTHING happens (docs/16: Del only).
  await sphere(page).click({ position: { x: 10, y: 8 } });
  await expect.poll(async () => (await store(page)).selection.nodes).toEqual(["sphere_1"]);
  await page.keyboard.press("Backspace");
  // The socket is ordered: a `move_node` to the node's CURRENT cell is
  // answered with a delta but is not an op (docs/13 no-op rule), so its
  // delta arriving proves anything Backspace might have sent was processed
  // before it — and the node, the text and the op log are untouched.
  await send(page, { type: "move_node", payload: { node: "sphere_1", cell: placedCell } });
  await expect.poll(async () => (await store(page)).lastDeltaLabel).toBe("move sphere_1");
  await expect(sphere(page)).toHaveCount(1);
  const afterBackspace = await debugState(page);
  expect(afterBackspace.text, "Backspace must not delete").toBe(WITH_SPHERE);
  expect(afterBackspace.history.depth, "a no-op move pushes no op").toBe(2);
  expect(afterBackspace.ops.map((op) => op.label)).toEqual(["place sphere", WIRE_LABEL]);
  expect((await store(page)).selection.nodes, "the selection survived Backspace").toEqual(["sphere_1"]);

  // ---- Del deletes: node and wire gone, the text is back to the start.
  await page.keyboard.press("Delete");
  await expect(sphere(page)).toHaveCount(0);
  await expect(wireEdge(page)).toHaveCount(0);
  await expect.poll(async () => (await debugState(page)).text).toBe(START);
  const deleted = await debugState(page);
  expect(deleted.history).toEqual({
    can_undo: true,
    can_redo: false,
    undo_label: "delete sphere_1",
    redo_label: null,
    depth: 3,
  });
  expect(deleted.graph.wires.map((w) => w.id)).not.toContain(WIRE_ID);
  expect(deleted.statuses["sphere_1"], "a deleted binding has no status").toBeUndefined();
  await expect(undoButton).toHaveAttribute("title", "undo: delete sphere_1 (Ctrl+Z)");

  // ---- Ctrl+Z: the node AND its wire are back; the text panel shows the
  // restored line; the restored output is a cache hit, not a recompute.
  await page.keyboard.press("Control+z");
  await expect(sphere(page)).toHaveCount(1);
  await expect(wireEdge(page)).toHaveCount(1);
  await expect.poll(async () => (await store(page)).lastDeltaLabel).toBe("undo: delete sphere_1");
  await expect(page.getByTestId("sb-generation")).toContainText("undo: delete sphere_1");

  await page.getByTestId("insp-tab-text").click();
  const restoredLine = page.locator(".text-line[data-node='sphere_1']");
  await expect(restoredLine).toHaveCount(1);
  await expect(restoredLine).toContainText("sphere_1 = sphere(radius=size)");

  expect(await solvedState(page, "sphere_1"), "undo never recomputes: the restored cone is a memo hit").toBe(
    "cached",
  );
  const undone = await debugState(page);
  expect(undone.text).toBe(WITH_SPHERE);
  expect(readFileSync(FILE, "utf8"), "the file on disk is the restored text").toBe(WITH_SPHERE);
  expect(undone.history).toEqual({
    can_undo: true,
    can_redo: true,
    undo_label: WIRE_LABEL,
    redo_label: "delete sphere_1",
    depth: 2,
  });
  expect(undone.graph.wires.map((w) => w.id)).toContain(WIRE_ID);
  const restoredNode = undone.graph.nodes.find((n) => n.name === "sphere_1");
  expect(restoredNode?.manual, "the sidecar cell came back with the op's snapshot").toBe(true);
  expect(restoredNode?.cell).toEqual(placedCell);
  await expect(undoButton).toHaveAttribute("title", `undo: ${WIRE_LABEL} (Ctrl+Z)`);
  await expect(redoButton).toBeEnabled();
  await expect(redoButton).toHaveAttribute("title", "redo: delete sphere_1 (Ctrl+Shift+Z / Ctrl+Y)");

  // ---- redo by the toolbar button: deleted again; Ctrl+Z: back; Ctrl+Y:
  // deleted again; Ctrl+Shift+Z with nothing to redo is an info notice,
  // not an error; Ctrl+Z: back.
  await redoButton.click();
  await expect(sphere(page)).toHaveCount(0);
  await expect.poll(async () => (await store(page)).lastDeltaLabel).toBe("redo: delete sphere_1");
  await expect(redoButton).toBeDisabled();
  await page.keyboard.press("Control+z");
  await expect(sphere(page)).toHaveCount(1);
  await page.keyboard.press("Control+y");
  await expect(sphere(page)).toHaveCount(0);
  await expect.poll(async () => (await debugState(page)).history.can_redo).toBe(false);
  await page.keyboard.press("Control+Shift+z");
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const w = window as unknown as {
          __cicada: { state: () => { notices: { level: string; message: string }[] } };
        };
        return w.__cicada.state().notices.at(-1) ?? null;
      }),
    )
    .toMatchObject({ level: "info", message: expect.stringMatching(/^nothing to redo/) });
  await page.keyboard.press("Control+z");
  await expect(sphere(page)).toHaveCount(1);
  await expect(wireEdge(page)).toHaveCount(1);
  expect(await solvedState(page, "sphere_1")).toBe("cached");
  expect((await debugState(page)).text).toBe(WITH_SPHERE);

  // ---- a multi-select delete is ONE op: Ctrl+A, Del → `delete 4 nodes`;
  // one Ctrl+Z brings all four back, every one a cache hit.
  await pane.click({ position: { x: 8, y: 8 } }); // focus the canvas, clear selection
  await page.keyboard.press("Control+a");
  await expect.poll(async () => (await store(page)).selection.nodes.length).toBe(4);
  await page.keyboard.press("Delete");
  await expect(page.locator(".react-flow__node")).toHaveCount(0);
  await expect.poll(async () => (await debugState(page)).text).toBe("# cicada 1\n");
  const allGone = await debugState(page);
  expect(allGone.history.undo_label, "four deletes went as one batch").toBe("delete 4 nodes");
  expect(allGone.history.depth).toBe(3);
  expect(allGone.ops.map((op) => op.label)).toEqual(["place sphere", WIRE_LABEL, "delete 4 nodes"]);

  await page.keyboard.press("Control+z");
  await expect(page.locator(".react-flow__node")).toHaveCount(4);
  await expect(wireEdge(page)).toHaveCount(1);
  await expect.poll(async () => (await debugState(page)).text).toBe(WITH_SPHERE);
  for (const node of ["size", "span", "block", "sphere_1"]) {
    expect(await solvedState(page, node), `${node} after undoing the batch`).toBe("cached");
  }
  const restoredAll = await debugState(page);
  expect(restoredAll.history).toEqual({
    can_undo: true,
    can_redo: true,
    undo_label: WIRE_LABEL,
    redo_label: "delete 4 nodes",
    depth: 2,
  });
  expect(readFileSync(FILE, "utf8")).toBe(WITH_SPHERE);

  expect(errors, errors.join("\n")).toEqual([]);
});
