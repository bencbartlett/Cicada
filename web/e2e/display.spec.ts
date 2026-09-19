/**
 * The display edge, bounded and visible (docs/17 wave 5 D1; docs/12
 * §Display; docs/13 §The display edge; docs/16 §Status and progress
 * language) against the REAL `cicada serve` from `playwright.config.ts`,
 * on a pipeline written into the scratch with enough B-rep spheres to
 * cross the triangle budget — 140 spheres of radius 0.4 on a ring: 8,000
 * fine-tier triangles each (the angular deflection governs), 1.1 M in all
 * against the 1 M budget, 121 k at the preview tier.
 *
 *   - the output is drawn at PREVIEW although the generation asked for
 *     fine, its `stats.budget` says which tier was asked and which drawn,
 *     and the frames the viewport holds are the preview count;
 *   - the chip reads `gen N · solve T · display T` with a non-zero display
 *     time, the viewport's indicator reads `painted 1 output · … in T ms`;
 *   - a slider change shows the spinner: the chip says `painting…`, the
 *     indicator `painting 1 output…` — the pass is seconds long on the
 *     debug engine — then both settle again;
 *   - a structural edit that leaves the spheres alone re-sends nothing
 *     for them (the `already displayed` rule asks with the tier the budget
 *     chooses);
 *   - the caches indicator reads the session's `caches` view (`cache … /
 *     1G · N meshes`), its click opens the breakdown, and the bar
 *     still fits the window (the gear is reachable, nothing scrolls
 *     sideways);
 *   - the settings menu's display-cache select resizes the session's cache
 *     live (the indicator and `/debug/state` follow), the choice is kept
 *     per user, and the WRITER re-applies it on connect (a preference
 *     planted in `localStorage` is applied by the next page's `hello`).
 *
 * The notice (`over_budget` / `thrash`) needs a cache below the intent's
 * 64 MiB floor to bite on 140 preview spheres (3 MB) and is covered by the
 * session's unit tests; here the indicator is asserted quiet.
 */
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "display/heavy.cic";

const HEAVY = `# cicada 1
# wave 5 D1 evidence: 140 B-rep spheres on a ring — 1.1 M triangles at the
# fine tier (over the 1 M per-output budget), 121 k at the preview tier.
radius = slider(value=6.0, min=2.0, max=10.0)
knob = slider(value=1.0, min=0.0, max=2.0)
turn = construct_domain(start=0.0, end=6.283185307179586)
angles = range(domain=turn, steps=140)
east = unit_x(factor=1.0)
up = unit_z(factor=1.0)
arm = amplitude(vector=east, length=radius)
spokes = rotate_vector(vector=arm, angle=each(angles), axis=up)
centre = construct_point(x=0.0, y=0.0, z=0.0)
posts_with_return = move(geometry=centre, motion=each(spokes))
posts, post_sources = cull_duplicates(points=posts_with_return, tolerance=0.000001)
post_frames = xy_plane(origin=each(posts))
balls = sphere(plane=each(post_frames), radius=0.4)
`;

interface Budget {
  limit: number;
  requested: string;
  drawn: string;
  triangles: number;
  over_budget?: boolean;
}

interface DebugState {
  text: string;
  summary: { generation: number; running: boolean };
  display: Record<string, { generation: number; stats: { triangles: number; solids?: number; tier?: string; budget?: Budget } }>;
  display_cache: { entries: number; budget: number; refusals: number; bytes: number };
  caches: {
    display: { entries: number; bytes: number; budget: number; refusals: number; working_set: number; over_budget: boolean; thrash: boolean };
    memo: { bytes: number; entries: number };
  };
  timings: { generation: number; kind: string; tessellate_ms: number; encode_ms: number; frame_bytes: number }[];
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  role: string;
  display: { generation: number; phase: string; outputs: number; frames: number; bytes: number; tessellateMs: number; encodeMs: number; paintedMs: number | null } | null;
  caches: DebugState["caches"] | null;
  displayCacheMib: number | null;
}

async function store(page: Page): Promise<StoreView> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __cicada: { state: () => { role: string; display: StoreView["display"]; caches: StoreView["caches"]; settings: { displayCacheMib: number | null } } };
    };
    const s = w.__cicada.state();
    return { role: s.role, display: s.display, caches: s.caches, displayCacheMib: s.settings.displayCacheMib };
  });
}

interface SceneStats {
  outputs: Record<string, { triangles: number; generation: number }>;
  framesReceived: number;
}

async function scene(page: Page): Promise<SceneStats> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { scene: (() => unknown) | null } };
    if (w.__cicada.scene === null) throw new Error("viewport not mounted");
    return w.__cicada.scene() as SceneStats;
  });
}

function send(page: Page, message: unknown): Promise<void> {
  return page.evaluate((m) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(m);
  }, message);
}

/** `display 5.62 s` / `display 734 ms` → milliseconds. */
function displayMs(chip: string): number {
  const match = /display ([\d.]+) (ms|s)/.exec(chip);
  if (match === null) throw new Error(`no display time in ${JSON.stringify(chip)}`);
  const value = Number(match[1]);
  return match[2] === "s" ? value * 1000 : value;
}

test.describe.configure({ mode: "serial" });

test("a heavy output is drawn at preview, the chip and the viewport show the display edge, the caches indicator reads the cache", async ({ page }, testInfo) => {
  const dir = join(meta.scratch, "examples", "display");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "heavy.cic"), HEAVY);

  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });

  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(async () => (await store(page)).role).toBe("writer");

  // ---- the first paint: the budget drops the spheres to preview and says so.
  const chip = page.getByTestId("tb-solve-text");
  await expect(chip, "the pass is seconds long on the debug engine").toHaveText(/gen \d+ · solve .+ · display .+/, { timeout: 80_000 });
  const first = await debugState(page);
  const displayed = first.display["balls.out"];
  expect(displayed, `balls.out is displayed: ${Object.keys(first.display).join(", ")}`).toBeDefined();
  const balls = displayed!;
  expect(balls.stats.tier).toBe("preview");
  expect(balls.stats.solids).toBe(140);
  const budget = balls.stats.budget!;
  expect(budget.limit).toBe(1_000_000);
  expect(budget.requested).toBe("fine");
  expect(budget.drawn).toBe("preview");
  expect(budget.over_budget, "121 k preview triangles fit the budget").toBeUndefined();
  expect(budget.triangles).toBeGreaterThan(100_000);
  expect(budget.triangles).toBeLessThan(1_000_000);
  expect(balls.stats.triangles).toBe(budget.triangles);
  // The fine tally stopped at the budget: fewer fine misses than spheres.
  expect(first.display_cache.entries).toBeLessThan(2 * 140);
  expect(first.display_cache.entries).toBeGreaterThanOrEqual(140);
  // The viewport holds the preview count, not 1.1 M triangles.
  await expect.poll(async () => Object.values((await scene(page)).outputs).reduce((n, o) => n + o.triangles, 0)).toBe(budget.triangles);

  // ---- the chip: a non-zero display time, the counts in the hover; the viewport's indicator: painted.
  const chipText = await chip.textContent();
  expect(displayMs(chipText ?? "")).toBeGreaterThan(0);
  await expect(page.getByTestId("tb-solve")).toHaveAttribute("data-phase", "idle");
  await expect(page.getByTestId("tb-solve")).toHaveAttribute("title", /computed .* cached/);
  await expect(page.getByTestId("tb-solve")).toHaveAttribute("title", /tessellation .* · encode .*/);
  // Every displayable output of the first paint (the spheres, the posts, the
  // centre, the ring's points), not the spheres alone.
  const indicator = page.getByTestId("viewport-display");
  await expect(indicator).toHaveAttribute("data-phase", "painted");
  await expect(indicator).toHaveText(/painted \d+ outputs · .+ in .+/);
  const pass = (await store(page)).display!;
  expect(pass.generation).toBe(first.summary.generation);
  expect(pass.outputs).toBeGreaterThanOrEqual(1);
  expect(pass.outputs).toBe(Object.keys(first.display).length);
  expect(pass.frames).toBeGreaterThan(0);
  expect(pass.tessellateMs).toBeGreaterThan(0);
  expect(pass.paintedMs).not.toBeNull();
  const timing = first.timings.find((t) => t.generation === first.summary.generation)!;
  expect(timing.tessellate_ms).toBeCloseTo(pass.tessellateMs, 3);
  expect(timing.encode_ms).toBeCloseTo(pass.encodeMs, 3);
  expect(timing.frame_bytes).toBe(pass.bytes);

  // ---- the caches indicator reads the session's view.
  const caches = page.getByTestId("tb-caches");
  await expect(caches).toHaveAttribute("data-warn", "false");
  const cachesText = (await page.getByTestId("tb-caches-text").textContent()) ?? "";
  expect(cachesText).toMatch(/^cache \S+ \/ 1G · [\d,]+ meshes$/);
  const meshes = Number((/· ([\d,]+) meshes/.exec(cachesText)?.[1] ?? "NaN").replace(/,/g, ""));
  expect(meshes).toBe(first.caches.display.entries - first.caches.display.refusals);
  expect(first.caches.display.budget).toBe(1024 * 1024 * 1024);
  expect(first.caches.display.over_budget).toBe(false);
  expect(first.caches.display.thrash).toBe(false);
  expect(first.caches.memo.bytes).toBeGreaterThan(0);
  await caches.click();
  await expect(page.getByTestId("tb-caches-detail")).toContainText("display cache:");
  await expect(page.getByTestId("tb-caches-detail")).toContainText("memo store:");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("tb-caches-detail")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("display-first-paint.png") });

  // ---- a structural edit that leaves the spheres alone re-sends nothing for them.
  const framesBefore = (await scene(page)).framesReceived;
  await send(page, { type: "set_param", payload: { node: "knob", port: "value", value: "1.5" } });
  await expect.poll(async () => (await debugState(page)).text).toContain("knob = slider(value=1.5");
  const unchanged = await debugState(page);
  expect(unchanged.display["balls.out"]!.generation, "the spheres kept their frame").toBe(balls.generation);
  expect(unchanged.display_cache.entries, "nothing was tessellated").toBe(first.display_cache.entries);
  await expect(chip).toHaveText(new RegExp(`gen ${unchanged.summary.generation} · solve .+ · display .+`));
  expect((await scene(page)).framesReceived, "no frame for the unchanged output").toBe(framesBefore);
  await expect(indicator).toHaveText(/painted 0 outputs · 0 B in .+/);

  // ---- a slider change redraws the spheres: the spinner shows, then settles.
  await send(page, { type: "set_param", payload: { node: "radius", port: "value", value: "7.0" } });
  await expect(page.getByTestId("tb-solve"), "the chip paints").toHaveAttribute("data-phase", /solving|painting/, { timeout: 15_000 });
  await expect(indicator).toHaveAttribute("data-phase", "painting", { timeout: 30_000 });
  await expect(indicator).toHaveText(/painting \d+ outputs?…/);
  await expect(page.getByTestId("tb-solve-spinner")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("display-painting.png") });
  await expect(chip).toHaveText(/gen \d+ · painting…/);
  await expect(indicator).toHaveAttribute("data-phase", "painted", { timeout: 80_000 });
  await expect(chip).toHaveText(/gen \d+ · solve .+ · display .+/);
  const moved = await debugState(page);
  const redrawn = moved.display["balls.out"]!;
  expect(redrawn.generation).toBeGreaterThan(balls.generation);
  expect(redrawn.stats.budget!.drawn).toBe("preview");
  await expect(indicator).toHaveText(/painted \d+ outputs? · .+ in .+/);

  // ---- the bar fits the window with both chips carrying their idle text:
  // the gear is inside the viewport and the app does not scroll sideways
  // (at 1400 px the caches chip pushed it off-screen — review finding
  // 2026-08-25). Playwright would scroll the gear into view to click it, so
  // the geometry is asserted before the click.
  const fit = await page.evaluate(() => {
    const gear = document.querySelector('[data-testid="tb-settings"]')!.getBoundingClientRect();
    const bar = document.querySelector('[data-testid="topbar"]')!.getBoundingClientRect();
    return { innerWidth: window.innerWidth, scrollWidth: document.documentElement.scrollWidth, barRight: bar.right, gearLeft: gear.left, gearRight: gear.right };
  });
  expect(fit.scrollWidth, `no horizontal scroll: ${JSON.stringify(fit)}`).toBeLessThanOrEqual(fit.innerWidth);
  expect(fit.barRight, `the bar fits: ${JSON.stringify(fit)}`).toBeLessThanOrEqual(fit.innerWidth);
  expect(fit.gearRight, `the gear is on screen: ${JSON.stringify(fit)}`).toBeLessThanOrEqual(fit.innerWidth);
  expect(fit.gearLeft).toBeGreaterThanOrEqual(0);
  // And the solve chip is whole at the reference width: its text is not clipped.
  const chipWhole = await page.evaluate(() => {
    const el = document.querySelector('[data-testid="tb-solve-text"]')!;
    return el.scrollWidth <= el.clientWidth + 1;
  });
  expect(chipWhole, "the solve chip's text is not truncated at 1400 px").toBe(true);

  // ---- the settings menu resizes the cache live; the choice is per user.
  await page.getByTestId("tb-settings").click();
  const select = page.getByTestId("settings-display-cache");
  await expect(select).toHaveValue("");
  await expect(page.getByTestId("settings-display-cache-now")).toHaveText("session: 1G");
  await select.selectOption("512");
  await expect.poll(async () => (await debugState(page)).caches.display.budget).toBe(512 * 1024 * 1024);
  await expect(page.getByTestId("settings-display-cache-now")).toHaveText("session: 512M");
  await expect(page.getByTestId("tb-caches-text")).toHaveText(/\/ 512M ·/);
  expect((await store(page)).displayCacheMib).toBe(512);
  await page.screenshot({ path: testInfo.outputPath("display-settings.png") });
  await page.keyboard.press("Escape");
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("cicada.settings.v1") ?? "{}") as { displayCacheMib?: number });
  expect(stored.displayCacheMib).toBe(512);

  // ---- the writer re-applies its preference on connect: plant 2 GiB, reload.
  await page.evaluate(() => {
    const raw = JSON.parse(localStorage.getItem("cicada.settings.v1") ?? "{}") as Record<string, unknown>;
    localStorage.setItem("cicada.settings.v1", JSON.stringify({ ...raw, displayCacheMib: 2048 }));
  });
  await page.reload();
  await expect(page.getByTestId("app")).toBeVisible();
  await expect.poll(async () => (await store(page)).role).toBe("writer");
  await expect.poll(async () => (await debugState(page)).caches.display.budget).toBe(2048 * 1024 * 1024);
  await expect(page.getByTestId("tb-caches-text")).toHaveText(/\/ 2G ·/);

  // ---- an out-of-range ask is refused with its reason; the budget stands.
  await send(page, { type: "set_display_cache", payload: { mib: 10 } });
  await expect(page.getByTestId("notices")).toContainText("display cache must be between 64 MiB and 65536 MiB (asked for 10)");
  expect((await debugState(page)).caches.display.budget).toBe(2048 * 1024 * 1024);

  expect(errors, errors.join("\n")).toEqual([]);
});
