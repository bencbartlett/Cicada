/**
 * The `choice` param — Grasshopper's Value List as a dropdown (catalog C2b,
 * docs/10 §3, docs/16 §Canvas conventions). Through the real app: the node's
 * select on the canvas lists the text's options with the value selected;
 * picking another writes ONE `set_param` — the text reads
 * `value="exact"`, one op (`Ctrl+Z` walks it back), the node stays green
 * and its downstream `text_tag` follows; the params panel's twin shows the
 * same value and picks too; a value the text carries that is not among the
 * options (`slow`) paints the node red and the select keeps it, marked
 * "(not an option)" — never a silent swap to the first option.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file (picking writes it).
 */
import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "choice.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

const START =
  "# cicada 1\n" +
  'mode = choice(value="fast", options=["fast", "exact", "draft"])\n' +
  "at = construct_point(x=0.0, y=0.0, z=2.0)\n" +
  "frame = xy_plane(origin=at)\n" +
  "label = text_tag(location=frame, text=mode, size=1.0)\n";

interface DebugState {
  text: string;
  statuses: Record<string, { state: string; message?: string }>;
  history: { depth: number; undo_label: string | null };
  graph: {
    nodes: {
      name: string;
      param?: { kind: string; value: unknown; options?: string[] };
      excluded?: { status: string; reason: string };
    }[];
  };
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

/** Wait until the file reads `line` on one of its lines. */
async function expectLine(page: Page, line: string): Promise<void> {
  await expect.poll(async () => (await debugState(page)).text.split("\n")).toContain(line);
}

/** Wait until `node` has SOLVED (`done` or `cached`). */
async function expectSolved(page: Page, node: string): Promise<void> {
  await expect
    .poll(async () => {
      const state = (await debugState(page)).statuses[node]?.state ?? "";
      return state === "done" || state === "cached";
    })
    .toBe(true);
}

test("the choice dropdown: options from the text, one set_param per pick, undoable, a stray value kept and marked", async ({
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
  await expect(page.locator(".react-flow__node")).toHaveCount(4);

  // The view-model carries the widget: kind `choice`, the options in the
  // text's order, the value as written.
  const before = await debugState(page);
  const param = before.graph.nodes.find((n) => n.name === "mode")?.param;
  expect(param).toEqual({ kind: "choice", port: "value", value: "fast", options: ["fast", "exact", "draft"] });
  await expectSolved(page, "label");
  const depth0 = before.history.depth;

  // The canvas select: the options, the value selected.
  const select = page.getByTestId("choice-mode");
  await expect(select).toBeVisible();
  await expect(select).toHaveValue("fast");
  expect(await select.locator("option").allTextContents()).toEqual(["fast", "exact", "draft"]);

  // Pick `exact`: ONE set_param — the text's one literal rewritten, one op,
  // the node green and the tag downstream following.
  await select.selectOption("exact");
  await expectLine(page, 'mode = choice(value="exact", options=["fast", "exact", "draft"])');
  await expectSolved(page, "label");
  let state = await debugState(page);
  expect(state.history.depth).toBe(depth0 + 1);
  expect(state.history.undo_label).toBe('set mode.value = "exact"');
  expect(state.graph.nodes.find((n) => n.name === "mode")?.excluded).toBeUndefined();
  await expect(select).toHaveValue("exact");

  // The params panel's twin shows the same value, and picks too.
  await page.getByTestId("insp-tab-params").click();
  const twin = page.getByTestId("widget-mode");
  await expect(twin).toHaveValue("exact");
  await twin.selectOption("draft");
  await expectLine(page, 'mode = choice(value="draft", options=["fast", "exact", "draft"])');
  await expect(select).toHaveValue("draft");
  await expect(twin).toHaveValue("draft");
  expect((await debugState(page)).history.depth).toBe(depth0 + 2);

  // Ctrl+Z walks one pick back.
  await page.locator(".react-flow__pane").click({ position: { x: 10, y: 10 } });
  await page.keyboard.press("Control+z");
  await expectLine(page, 'mode = choice(value="exact", options=["fast", "exact", "draft"])');
  await expect(select).toHaveValue("exact");

  // A stray value written in the text: the node is red with the node's own
  // reason, and the select keeps the value, marked — no silent swap.
  writeFileSync(
    FILE,
    START.replace('value="fast"', 'value="slow"'),
  );
  await expectLine(page, 'mode = choice(value="slow", options=["fast", "exact", "draft"])');
  await expect(select).toHaveValue("slow");
  await expect(select).toHaveAttribute("data-stray", "true");
  expect(await select.locator("option").first().textContent()).toBe("slow (not an option)");
  await expect
    .poll(async () => (await debugState(page)).statuses.mode?.state)
    .toBe("red");
  state = await debugState(page);
  expect(state.statuses.mode?.message).toContain('value "slow" is not one of the options');
  // Picking a real option heals it.
  await select.selectOption("fast");
  await expectLine(page, 'mode = choice(value="fast", options=["fast", "exact", "draft"])');
  await expectSolved(page, "label");
  await expect(select).not.toHaveAttribute("data-stray", "true");

  expect(errors).toEqual([]);
});
