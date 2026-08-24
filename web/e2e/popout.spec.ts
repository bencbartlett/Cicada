/**
 * The pop-out viewport (docs/17 wave 4 O3; docs/16 §Viewport conventions;
 * docs/13 — the join hint) against the REAL `cicada serve` from
 * `playwright.config.ts`: the viewport's button opens the same URL with
 * `view=viewport` in a window named `cicada-viewport`; that page renders the
 * viewport alone (no canvas, top bar, ribbon, no pop-out button of its own)
 * and joins as a DECLARED observer — the main window keeps the lease, the
 * pop-out shows the same geometry, follows the main window's writes live
 * (its text and its scene move), stays read-only throughout, and its
 * `take_lease` is refused with the reason.
 */
import { expect, test, type Page } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

interface DebugState {
  text: string;
  lease: { writer: number | null; clients: [number, string][] };
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  role: string;
  connection: string;
  text: string;
  hello: { clientId: number; role: string; pipeline: string } | null;
  lastError: { kind: string; message: string } | null;
}

async function store(page: Page): Promise<StoreView | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada?: { state: () => StoreView } };
    if (w.__cicada === undefined) return null;
    const s = w.__cicada.state();
    return { role: s.role, connection: s.connection, text: s.text, hello: s.hello, lastError: s.lastError };
  });
}

interface SceneStats {
  bounds: [number[], number[]] | null;
  outputs: Record<string, { triangles: number }>;
  framesReceived: number;
}

async function scene(page: Page): Promise<SceneStats | null> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada?: { scene: (() => SceneStats) | null } };
    const read = w.__cicada?.scene;
    return read === null || read === undefined ? null : read();
  });
}

const triangles = (stats: SceneStats | null) => (stats === null ? 0 : Object.values(stats.outputs).reduce((n, o) => n + o.triangles, 0));

test("the pop-out shows the geometry as a declared observer while the main window keeps writing", async ({ page, context }) => {
  const errors: string[] = [];
  const watch = (target: Page, name: string) => {
    target.on("pageerror", (error) => errors.push(`${name} pageerror: ${error.message}`));
    target.on("console", (message) => {
      if (message.type() === "error") errors.push(`${name} console: ${message.text()}`);
    });
  };
  watch(page, "main");

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(async () => (await store(page))?.role).toBe("writer");
  await expect.poll(async () => triangles(await scene(page)), { timeout: 20_000 }).toBeGreaterThan(500);
  const mainId = (await store(page))?.hello?.clientId;
  expect(mainId).toBeDefined();

  // ---- the button opens <same URL>&view=viewport as the named window.
  const [popup] = await Promise.all([context.waitForEvent("page"), page.getByTestId("viewport-popout").click()]);
  watch(popup, "popout");
  await popup.waitForLoadState();
  const url = new URL(popup.url());
  expect(url.searchParams.get("view")).toBe("viewport");
  expect(url.searchParams.get("pipeline")).toBe(PIPELINE);
  expect(url.searchParams.get("token")).toBe(TOKEN);
  expect(url.pathname).toBe(new URL(page.url()).pathname);
  expect(await popup.evaluate(() => window.name)).toBe("cicada-viewport");

  // ---- the viewport alone: no canvas, top bar, ribbon, inspector; no pop-out button of its own.
  await expect(popup.getByTestId("viewport-only")).toBeVisible();
  await expect(popup.getByTestId("viewport")).toBeVisible();
  await expect(popup.getByTestId("app")).toHaveCount(0);
  await expect(popup.locator(".react-flow")).toHaveCount(0);
  await expect(popup.getByTestId("topbar")).toHaveCount(0);
  await expect(popup.getByTestId("viewport-popout")).toHaveCount(0);
  await expect(popup.getByTestId("viewport-only-pipeline")).toHaveText(PIPELINE);
  await expect(popup.getByTestId("viewport-only-role")).toHaveText("read-only observer");
  await expect(popup).toHaveTitle(`${PIPELINE} — viewport · Cicada`);

  // ---- joined as a declared observer; the main window keeps the lease.
  await expect.poll(async () => (await store(popup))?.hello?.role).toBe("observer");
  await expect.poll(async () => (await store(popup))?.role).toBe("observer");
  const popoutId = (await store(popup))?.hello?.clientId;
  expect(popoutId).not.toBe(mainId);
  let state = await debugState(page);
  expect(state.lease.writer).toBe(mainId);
  expect(state.lease.clients.map(([id]) => id).sort()).toEqual([mainId, popoutId].sort());
  expect((await store(page))?.role, "the main window is still the writer").toBe("writer");

  // ---- the same display set: geometry in the pop-out.
  await expect.poll(async () => (await scene(popup))?.framesReceived ?? 0, { timeout: 20_000 }).toBeGreaterThan(0);
  await expect.poll(async () => triangles(await scene(popup))).toBeGreaterThan(500);
  const boundsBefore = (await scene(popup))?.bounds;
  expect(boundsBefore).not.toBeNull();

  // ---- the main window writes (the same op pipeline the slider uses); the pop-out follows, read-only.
  await page.evaluate(() => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send({ type: "set_param", payload: { node: "size", port: "value", value: "3.5" } });
  });
  await expect.poll(async () => (await debugState(page)).text).toContain("size = slider(value=3.5");
  await expect.poll(async () => (await store(popup))?.text).toContain("size = slider(value=3.5");
  await expect.poll(async () => (await scene(popup))?.bounds?.[1][0] ?? 0).toBeGreaterThan(boundsBefore![1][0]!);
  expect((await store(popup))?.role).toBe("observer");
  expect((await store(page))?.role).toBe("writer");
  state = await debugState(page);
  expect(state.lease.writer).toBe(mainId);

  // ---- it can never take the lease: refused with the reason, the writer unchanged.
  await popup.evaluate(() => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send({ type: "take_lease", payload: {} });
  });
  await expect.poll(async () => (await store(popup))?.lastError?.kind).toBe("lease");
  expect((await store(popup))?.lastError?.message).toMatch(/declared observer/);
  expect((await debugState(page)).lease.writer).toBe(mainId);
  expect((await store(popup))?.role).toBe("observer");
  // And a write from it is refused too (the observer rule, unchanged).
  await popup.evaluate(() => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send({ type: "set_param", payload: { node: "size", port: "value", value: "1.0" } });
  });
  await expect.poll(async () => (await store(popup))?.lastError?.message).toMatch(/read-only observer/);
  expect((await debugState(page)).text).toContain("size = slider(value=3.5");

  await popup.close();
  await expect.poll(async () => (await debugState(page)).lease.clients.length).toBe(1);
  expect((await debugState(page)).lease.writer).toBe(mainId);
  expect(errors, errors.join("\n")).toEqual([]);
});
