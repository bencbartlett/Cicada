/**
 * File → Open / Recent / Close and the landing picker (docs/17 wave 4 O2;
 * docs/16 §Application layout) against the REAL `cicada serve` from
 * `playwright.config.ts` — a scratch copy of `examples/` is the served
 * root, `02-solids.cic` its default pipeline:
 *
 *   - a page with `?token=` and no `?pipeline=` IS the picker: the scratch
 *     `examples/` tree lists — directories first, then the `.cic` files —
 *     read from `GET /api/files` and never from `/api/project`; the
 *     keyboard walks into `wall/` (breadcrumb, listing) and Backspace back up;
 *   - a double-click on `02-solids.cic` opens it in place: the URL gains
 *     `pipeline=`, the canvas shows its graph, the viewport its geometry;
 *   - File → Open… → the dialog over the same listing; arrows + Enter on
 *     `06-lists.cic` switches the pipeline: the URL, the top bar, the graph
 *     (the node count `/debug/state` reports for it) and the text are the
 *     other file's; the first session's socket is gone (0 clients);
 *   - File → Recent holds both, most recent first (and `localStorage`
 *     does); Back returns to the previous file, Forward again; a Recent
 *     entry opens its file;
 *   - File → Close shows the picker (no `pipeline=` in the URL, no socket),
 *     the picker's Recent lists what was opened, and Back from the picker
 *     reopens the file.
 */
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const SOLIDS = "02-solids.cic";
const LISTS = "06-lists.cic";

interface DebugState {
  text: string;
  graph: { nodes: unknown[] };
  lease: { writer: number | null; clients: [number, string][] };
}

async function debugState(page: Page, pipeline: string): Promise<DebugState> {
  const response = await page.request.get(
    `/debug/state?token=${TOKEN}&pipeline=${encodeURIComponent(pipeline)}&wait=true`,
  );
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  pipeline: string;
  role: string;
  connection: string;
  text: string;
  hello: { clientId: number; pipeline: string } | null;
}

async function store(page: Page): Promise<StoreView | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada?: { state: () => StoreView } };
    if (w.__cicada === undefined) return null;
    const s = w.__cicada.state();
    return { pipeline: s.pipeline, role: s.role, connection: s.connection, text: s.text, hello: s.hello };
  });
}

async function triangles(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada?: { scene: (() => { outputs: Record<string, { triangles: number }> }) | null } };
    const scene = w.__cicada?.scene;
    if (scene === null || scene === undefined) return 0;
    return Object.values(scene().outputs).reduce((n, o) => n + o.triangles, 0);
  });
}

const pipelineParam = (page: Page) => new URL(page.url()).searchParams.get("pipeline");

test.describe.configure({ mode: "serial" });

test("the picker lists the root; Open, Recent, Close and Back switch the pipeline in place", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  // The picker must read `/api/files` and never `/api/project` (docs/17 O1:
  // over a home root the walk is seconds and lists what the picker must
  // not show).
  const projectReads: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/api/project") projectReads.push(request.url());
  });

  // ---- the landing IS the picker.
  await page.goto(`/?token=${TOKEN}`);
  await expect(page.getByTestId("landing")).toBeVisible();
  await expect(page.getByTestId("app")).toHaveCount(0);
  const list = page.getByTestId("files-list");
  await expect(page.getByTestId(`files-entry-${SOLIDS}`)).toBeVisible();
  await expect(page.getByTestId(`files-entry-${LISTS}`)).toBeVisible();
  await expect(page.getByTestId("files-entry-wall")).toHaveAttribute("data-kind", "dir");
  await expect(page.getByTestId("files-crumb-root")).toHaveText("examples");
  const kinds = await list.locator("[role=option]").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-kind")));
  const firstPipeline = kinds.indexOf("pipeline");
  expect(firstPipeline, "the scratch examples/ has pipelines").toBeGreaterThan(0);
  expect(kinds.slice(0, firstPipeline).every((kind) => kind === "dir"), `directories first: ${kinds.join(",")}`).toBe(true);
  expect(kinds.slice(firstPipeline).every((kind) => kind === "pipeline"), `then pipelines: ${kinds.join(",")}`).toBe(true);
  await expect(list, "the list has the keyboard").toBeFocused();

  // ---- keyboard: into wall/ (breadcrumb + listing), Backspace back up.
  await page.getByTestId("files-entry-wall").click();
  await expect(page.getByTestId("files-entry-wall")).toHaveAttribute("aria-selected", "true");
  await list.press("Enter");
  await expect(page.getByTestId("files-crumb-wall")).toBeVisible();
  await expect(page.getByTestId("files-entry-wall.cic")).toBeVisible();
  await expect(page.getByTestId("file-browser")).toHaveAttribute("data-dir", "wall");
  await list.press("Backspace");
  await expect(page.getByTestId("files-crumb-wall")).toHaveCount(0);
  await expect(page.getByTestId(`files-entry-${SOLIDS}`)).toBeVisible();

  // ---- double-click opens 02-solids in place.
  await page.getByTestId(`files-entry-${SOLIDS}`).dblclick();
  await expect(page.getByTestId("app")).toBeVisible();
  expect(pipelineParam(page)).toBe(SOLIDS);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(SOLIDS);
  const solids = await debugState(page, SOLIDS);
  await expect(page.locator(".react-flow__node")).toHaveCount(solids.graph.nodes.length);
  await expect.poll(() => triangles(page), { timeout: 20_000 }).toBeGreaterThan(500);
  await expect.poll(async () => (await store(page))?.role).toBe("writer");
  const solidsClient = (await store(page))?.hello?.clientId;
  expect(solidsClient).toBeDefined();
  expect((await debugState(page, SOLIDS)).lease.writer).toBe(solidsClient);

  // ---- File → Open… → keyboard to 06-lists.cic → Enter.
  await page.getByTestId("tb-file").click();
  await expect(page.getByTestId("file-menu")).toBeVisible();
  await page.getByTestId("file-open").click();
  const dialog = page.getByTestId("open-dialog");
  await expect(dialog).toBeVisible();
  const dialogList = dialog.getByTestId("files-list");
  await expect(dialog.getByTestId(`files-entry-${LISTS}`)).toBeVisible();
  await expect(dialogList).toBeFocused();
  for (let step = 0; step < 40; step += 1) {
    if ((await dialog.getByTestId(`files-entry-${LISTS}`).getAttribute("aria-selected")) === "true") break;
    await dialogList.press("ArrowDown");
  }
  await expect(dialog.getByTestId(`files-entry-${LISTS}`)).toHaveAttribute("aria-selected", "true");
  await dialogList.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => pipelineParam(page)).toBe(LISTS);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(LISTS);
  const lists = await debugState(page, LISTS);
  await expect(page.locator(".react-flow__node")).toHaveCount(lists.graph.nodes.length);
  const listsSource = readFileSync(join(meta.scratch, "examples", LISTS), "utf8");
  const firstBinding = listsSource.split("\n").find((line) => /^[a-z_][a-z0-9_]* = /u.test(line));
  expect(firstBinding, "06-lists.cic has a binding").toBeDefined();
  await expect.poll(async () => (await store(page))?.text).toContain(firstBinding!);
  await expect.poll(async () => (await store(page))?.hello?.pipeline).toBe(LISTS);
  await expect.poll(async () => (await store(page))?.role).toBe("writer");
  // The first session's socket is gone — the switch closed it.
  await expect.poll(async () => (await debugState(page, SOLIDS)).lease.clients.length).toBe(0);
  expect((await debugState(page, LISTS)).lease.clients.length).toBe(1);

  // ---- Recent: both, most recent first — in the menu and in localStorage.
  await page.getByTestId("tb-file").click();
  const menu = page.getByTestId("file-menu");
  await expect(menu).toBeVisible();
  const recents = await menu.locator("[data-testid^='file-recent-']").evaluateAll((rows) => rows.map((row) => row.textContent));
  expect(recents).toEqual([LISTS, SOLIDS]);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("cicada.recent.v1") ?? "null"))).toEqual([LISTS, SOLIDS]);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // ---- Back returns to the previous file; Forward to the next; Recent opens one.
  await page.goBack();
  await expect.poll(() => pipelineParam(page)).toBe(SOLIDS);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(SOLIDS);
  await expect(page.locator(".react-flow__node")).toHaveCount(solids.graph.nodes.length);
  await expect.poll(async () => (await store(page))?.hello?.pipeline).toBe(SOLIDS);
  await page.goForward();
  await expect.poll(() => pipelineParam(page)).toBe(LISTS);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(LISTS);
  await page.getByTestId("tb-file").click();
  await page.getByTestId(`file-recent-${SOLIDS}`).click();
  await expect.poll(() => pipelineParam(page)).toBe(SOLIDS);
  await expect(page.getByTestId("tb-pipeline")).toHaveText(SOLIDS);
  await expect.poll(async () => (await store(page))?.hello?.pipeline).toBe(SOLIDS);

  // ---- Close: the picker, no pipeline in the URL, no socket; Back reopens.
  await page.getByTestId("tb-file").click();
  await page.getByTestId("file-close").click();
  await expect(page.getByTestId("landing")).toBeVisible();
  await expect(page.getByTestId("app")).toHaveCount(0);
  expect(new URL(page.url()).searchParams.has("pipeline")).toBe(false);
  expect(new URL(page.url()).searchParams.get("token")).toBe(TOKEN);
  await expect.poll(async () => (await debugState(page, SOLIDS)).lease.clients.length).toBe(0);
  await expect(page.getByTestId(`landing-recent-${SOLIDS}`)).toBeVisible();
  await expect(page.getByTestId(`landing-recent-${LISTS}`)).toBeVisible();
  await page.goBack();
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(() => pipelineParam(page)).toBe(SOLIDS);
  await expect.poll(async () => (await store(page))?.hello?.pipeline).toBe(SOLIDS);

  expect(projectReads, "the picker never reads /api/project").toEqual([]);
  expect(errors, errors.join("\n")).toEqual([]);
});
