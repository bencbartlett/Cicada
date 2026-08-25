/**
 * Typed literals on unconnected inputs (docs/17 wave 4 B3 — finding U9,
 * Ben's words: "no way to type a primitive input (e.g. `construct_domain`'s
 * `end = 40.0`) on a placed node; any keyboard-typeable unconnected input
 * should be directly editable"). Through the real app: place
 * `construct_domain` from search, double-click its `end` chip, type `40`,
 * Enter — the text reads `end=40.0`; type `start` from the inspector's
 * chip — the kwarg lands BEFORE `end` (spec order) and the node is green.
 * Esc cancels. A Boolean port (`shift_list.wrap`, the catalog's default
 * shown as `True`) toggles to `wrap=False`; an Integer port writes a bare
 * `3`; a Text port writes `text="hello"`; a wired port shows no chip and a
 * slider's own `value` port belongs to its widget. Review closure: `1/2`
 * typed into a Number chip is a warning notice and NOT a write (the field
 * is text, so the rule sees the slash a number input would have dropped);
 * Enter over `start=0` writes no spelling-only `0.0`; a literal inside
 * `each(…)` is rewritten inside it.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/`, on its own pipeline file (typing writes it).
 */
import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "literals.cic";
const FILE = join(meta.scratch, "examples", PIPELINE);

// A slider and a node wired from it: `span.end` is wired (no chip),
// `span.start` is a Number literal spelled as an integer (the checker
// accepts it; Enter over it must not re-spell it), `size.value` is the
// slider widget's. `lifted.start` is a literal inside `each(…)` — its chip
// edits the inner token and the lift stays (the node is red: a lift over
// a scalar — that is the text's business, not the chip's).
const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "span = construct_domain(start=0, end=size)\n" +
  "lifted = construct_domain(start=each(1.0), end=3.0)\n";

interface DebugState {
  text: string;
  statuses: Record<string, { state: string; message?: string }>;
  graph: { nodes: { name: string; func?: string; excluded?: { status: string; reason: string } }[] };
  history: { depth: number; undo_label: string | null };
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

/**
 * Search-to-place `func` at a fraction of the pane — the top strip, above
 * the auto-laid-out nodes, so the double-click lands on the PANE (one on a
 * node selects it instead); returns once its node is on the canvas.
 */
async function place(page: Page, func: string, at: { x: number; y: number }): Promise<string> {
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await pane.dblclick({ position: { x: box.width * at.x, y: box.height * at.y } });
  const search = page.getByTestId("search-input");
  await expect(search).toBeVisible();
  await search.fill(func);
  await expect(page.getByTestId("search-item").first()).toHaveAttribute("data-func", func);
  await search.press("Enter");
  const name = `${func}_1`;
  await expect(node(page, name)).toBeVisible();
  return name;
}

test("a placed node's unconnected inputs are typed into: chips on the canvas and in the inspector, one set_param per Enter, Esc cancels", async ({
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

  // ---- The starting file: a wired port has no chip; a literal one has a
  // chip reading the text AS WRITTEN; the slider's own `value` port is its
  // widget's.
  await expect(page.getByTestId("lit-span-end")).toHaveCount(0);
  await expect(page.getByTestId("lit-span-start")).toHaveText("0");
  await expect(page.getByTestId("lit-span-start")).toHaveAttribute("data-state", "literal");
  await expect(page.getByTestId("lit-size-value")).toHaveCount(0);
  await expect(page.getByTestId("lit-size-min")).toHaveText("0.5");
  const depth0 = (await debugState(page)).history.depth;

  // Enter over the untouched `0` writes nothing — not even the `0.0` the
  // rule would spell: the same number is no edit (review minor 2).
  await page.getByTestId("lit-span-start").dblclick();
  await expect(page.getByTestId("lit-span-start-input")).toBeFocused();
  await expect(page.getByTestId("lit-span-start-input")).toHaveValue("0");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("lit-span-start-input")).toHaveCount(0);
  await expectLine(page, "span = construct_domain(start=0, end=size)");
  expect((await debugState(page)).history.depth).toBe(depth0);

  // A literal inside `each(…)`: the chip shows the inner token beside the
  // lift badge, and a commit rewrites THAT token — the lift stays (review
  // minor 3; gestures fixture `set_param_lifted`).
  const liftedStart = page.getByTestId("lit-lifted-start");
  await expect(liftedStart).toHaveText("1.0");
  await expect(node(page, "lifted").locator(".cn-lift")).toHaveText("map");
  await liftedStart.dblclick();
  await expect(page.getByTestId("lit-lifted-start-input")).toBeFocused();
  await page.keyboard.type("2");
  await page.keyboard.press("Enter");
  await expectLine(page, "lifted = construct_domain(start=each(2.0), end=3.0)");
  await expect(liftedStart).toHaveText("2.0");
  await expect(node(page, "lifted").locator(".cn-lift")).toHaveText("map");
  expect((await debugState(page)).history.depth).toBe(depth0 + 1);

  // ---- U9 verbatim: place construct_domain. Both required ports are empty
  // slots and the node is red (nothing typed yet).
  const domain = await place(page, "construct_domain", { x: 0.02, y: 0.08 });
  await expectLine(page, `${domain} = construct_domain()`);
  const end = page.getByTestId(`lit-${domain}-end`);
  await expect(end).toHaveAttribute("data-state", "unset");
  await expect(end).toHaveText("…");
  await expect(page.getByTestId(`lit-${domain}-start`)).toHaveAttribute("data-state", "unset");
  await expect.poll(async () => (await debugState(page)).statuses[domain]?.state).toBe("red");

  // Double-click `end`, type 40, Enter: the text reads `end=40.0`.
  await end.dblclick();
  const endInput = page.getByTestId(`lit-${domain}-end-input`);
  await expect(endInput).toBeFocused();
  await page.keyboard.type("40");
  await page.keyboard.press("Enter");
  await expectLine(page, `${domain} = construct_domain(end=40.0)`);
  await expect(endInput).toHaveCount(0);
  await expect(end).toHaveText("40.0");
  await expect(end).toHaveAttribute("data-state", "literal");
  // Still red: `start` is required and untyped.
  expect((await debugState(page)).statuses[domain]?.state).toBe("red");

  // `start` from the INSPECTOR's chip (a click opens it): the kwarg lands
  // before `end` — spec order, not typing order — and the node is green.
  await node(page, domain).click({ position: { x: 10, y: 8 } });
  await page.getByTestId("insp-tab-inspect").click();
  const startChip = page.getByTestId(`insp-lit-${domain}-start`);
  await expect(startChip).toHaveAttribute("data-state", "unset");
  await startChip.click();
  const startInput = page.getByTestId(`insp-lit-${domain}-start-input`);
  await expect(startInput).toBeFocused();
  await page.keyboard.type("0");
  await page.keyboard.press("Enter");
  await expectLine(page, `${domain} = construct_domain(start=0.0, end=40.0)`);
  expect(await solvedState(page, domain)).toBe("done");
  await expect(page.getByTestId(`state-${domain}`)).not.toHaveText(/red|idle/);
  await expect(startChip).toHaveText("0.0");
  await expect(page.getByTestId(`insp-lit-${domain}-end`)).toHaveText("40.0");
  await page.screenshot({ path: testInfo.outputPath("construct-domain-typed.png") });

  // Esc cancels: type 99 into `end`, Escape — the chip and the text stand.
  await end.dblclick();
  await expect(page.getByTestId(`lit-${domain}-end-input`)).toBeFocused();
  await page.keyboard.type("99");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId(`lit-${domain}-end-input`)).toHaveCount(0);
  await expect(end).toHaveText("40.0");
  expect((await debugState(page)).text).toContain(`${domain} = construct_domain(start=0.0, end=40.0)`);
  // Enter on the unchanged value writes nothing: the lifted edit, the
  // place, `end`, `start` — four ops, no fifth.
  await end.dblclick();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId(`lit-${domain}-end-input`)).toHaveCount(0);
  expect((await debugState(page)).history.depth).toBe(depth0 + 4);

  // `1/2` into a Number chip: the field is TEXT, so the slash reaches the
  // rule — a warning notice, the chip and the file stand, no op (review
  // major: a number input would have dropped the slash and written `12.0`).
  await end.dblclick();
  const half = page.getByTestId(`lit-${domain}-end-input`);
  await expect(half).toBeFocused();
  await expect(half).toHaveAttribute("type", "text");
  await page.keyboard.type("1/2");
  await expect(half).toHaveValue("1/2");
  await page.keyboard.press("Enter");
  await expect(half).toHaveCount(0);
  await expect(page.getByTestId("notices")).toContainText(
    `${domain}.end: "1/2" is not a valid number — nothing written`,
  );
  await expect(end).toHaveText("40.0");
  expect((await debugState(page)).text).toContain(`${domain} = construct_domain(start=0.0, end=40.0)`);
  expect((await debugState(page)).history.depth).toBe(depth0 + 4);

  // ---- A Boolean port: `shift_list.wrap` defaults to `true` in the catalog
  // and the chip says `True` (the dialect's spelling) greyed; the editor is
  // a checkbox; Space toggles, Enter writes `wrap=False`. An Integer port
  // writes a bare `3`, placed before `wrap` in the text.
  const shifted = await place(page, "shift_list", { x: 0.34, y: 0.08 });
  await expectLine(page, `${shifted} = shift_list()`);
  const wrap = page.getByTestId(`lit-${shifted}-wrap`);
  await expect(wrap).toHaveAttribute("data-state", "default");
  await expect(wrap).toHaveText("True");
  await wrap.dblclick();
  const wrapInput = page.getByTestId(`lit-${shifted}-wrap-input`);
  await expect(wrapInput).toBeFocused();
  await expect(wrapInput).toBeChecked();
  await page.keyboard.press("Space");
  await expect(wrapInput).not.toBeChecked();
  await page.keyboard.press("Enter");
  await expectLine(page, `${shifted} = shift_list(wrap=False)`);
  await expect(wrap).toHaveText("False");
  await expect(wrap).toHaveAttribute("data-state", "literal");
  await page.getByTestId(`lit-${shifted}-offset`).dblclick();
  await expect(page.getByTestId(`lit-${shifted}-offset-input`)).toBeFocused();
  await page.keyboard.type("3");
  await page.keyboard.press("Enter");
  await expectLine(page, `${shifted} = shift_list(offset=3, wrap=False)`);

  // ---- A Text port: `text_outlines.text` is required Text; `font` carries
  // the catalog default quoted. Type `hello` and a size: the node solves.
  const outlines = await place(page, "text_outlines", { x: 0.66, y: 0.08 });
  await expectLine(page, `${outlines} = text_outlines()`);
  await expect(page.getByTestId(`lit-${outlines}-font`)).toHaveText('"DejaVu Sans Bold"');
  await expect(page.getByTestId(`lit-${outlines}-font`)).toHaveAttribute("data-state", "default");
  await expect(page.getByTestId(`lit-${outlines}-plane`), "a Plane takes no literal").toHaveCount(0);
  await page.getByTestId(`lit-${outlines}-text`).dblclick();
  await expect(page.getByTestId(`lit-${outlines}-text-input`)).toBeFocused();
  await page.keyboard.type("hello");
  await page.keyboard.press("Enter");
  await expectLine(page, `${outlines} = text_outlines(text="hello")`);
  await expect(page.getByTestId(`lit-${outlines}-text`)).toHaveText('"hello"');
  await page.getByTestId(`lit-${outlines}-size`).dblclick();
  await page.keyboard.type("2");
  await page.keyboard.press("Enter");
  await expectLine(page, `${outlines} = text_outlines(text="hello", size=2.0)`);
  expect(await solvedState(page, outlines)).toBe("done");
  await page.screenshot({ path: testInfo.outputPath("literal-chips.png") });

  // ---- Ctrl+Z walks the typed values back one at a time: each was one op.
  await page.keyboard.press("Control+z");
  await expectLine(page, `${outlines} = text_outlines(text="hello")`);

  expect(errors).toEqual([]);
});
