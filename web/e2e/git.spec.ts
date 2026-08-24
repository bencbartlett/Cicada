/**
 * The git panel in the app (docs/17 item 2; docs/16 git chip · Git tab ·
 * Ctrl+S; doc 10 §Git integration slice 1): a project outside any
 * repository says `no repo` and offers no controls; after `git init` + a
 * baseline commit the chip shows the branch, an edit marks the cached
 * status STALE the moment its delta lands (the chip dims its count, the
 * tab says "re-reading" — never "nothing to commit" over an edit that is
 * on disk) until the ≤1 s re-read marks the node `modified` on the canvas
 * (a header badge; at the far zoom tier, where header badges hide, a
 * stripe and a tint on the node box) and the chip `1 dirty`, Ctrl+S opens the commit
 * dialog and Ctrl+Enter commits with the message VERBATIM (`git log`
 * agrees, the badge clears), a second edit is reverted to HEAD through the
 * confirm step — whose list is BINDING: over a two-file scope with one
 * file the server cannot restore (no HEAD version) the request names
 * exactly the one file listed, the other is left alone (the text panel
 * shows HEAD's line, the reload barrier cleared the undo history, the file
 * on disk equals HEAD, the revert reached the canvas in measured time),
 * a `.gitignore`d pipeline reads `ignored` with no dirty count and no
 * "nothing to commit" line beside the warning, and a read-only observer
 * sees the status but no commit or revert controls.
 *
 * Runs against the REAL `cicada serve` from `playwright.config.ts` over a
 * SCRATCH copy of `examples/` — the repository is created IN the scratch
 * project dir by this spec (a real `git` on PATH is required, as it is for
 * the server's own git tests), with local config pinning identity, no
 * signing, no hooks, no CRLF conversion. Later specs run inside that
 * repository; none of them cares. Oracles: `git` itself, the served file
 * on disk, `/debug/state?wait=true`, the store, the DOM.
 */
import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { STALE_REASON } from "../src/state/git";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "git.cic";
const IGNORED = "ignored.cic";
const SIDECAR = `${PIPELINE}.layout.json`;
const PROJECT = join(meta.scratch, "examples");
const FILE = join(PROJECT, PIPELINE);
const SIDECAR_FILE = join(PROJECT, SIDECAR);

const START =
  "# cicada 1\n" +
  "size = slider(value=2.0, min=0.5, max=5.0)\n" +
  "span = construct_domain(start=0.0, end=size)\n" +
  "block = box(x=span, y=span, z=span)\n";
const SIZE_3 = START.replace("value=2.0", "value=3.0");
const SIZE_4 = START.replace("value=2.0", "value=4.0");
const MESSAGE = "e2e: size to 3 — ünïcode survives";

/** Run git in the scratch project with a hygienic environment; throws on failure unless `allowFail`. */
function git(args: string[], allowFail = false): { ok: boolean; out: string } {
  try {
    const out = execFileSync("git", args, {
      cwd: PROJECT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        GIT_CONFIG_NOSYSTEM: "1",
        GIT_CONFIG_GLOBAL: join(PROJECT, ".no-global-gitconfig"),
        GIT_TERMINAL_PROMPT: "0",
        LC_ALL: "C",
      },
    });
    return { ok: true, out: out.trim() };
  } catch (error: unknown) {
    if (allowFail) return { ok: false, out: String((error as { stderr?: string }).stderr ?? error) };
    throw error;
  }
}

interface DebugState {
  text: string;
  history: { can_undo: boolean; can_redo: boolean; depth: number };
}

/** The debug hooks the page installs (`state/connection.ts` `window.__cicada`). */
interface CicadaHandle {
  state: () => StoreView;
  send: (message: unknown) => string;
}

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  role: string;
  text: string;
  commitDialog: boolean;
  selection: { nodes: string[] };
  notices: { level: string; message: string }[];
  git: { answers: number; busy: string | null; stale: boolean; status: { text_hash: string; scope: { path: string }[] } | null };
}

async function store(page: Page): Promise<StoreView> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: CicadaHandle };
    const s = w.__cicada.state();
    return {
      role: s.role,
      text: s.text,
      commitDialog: s.commitDialog,
      selection: { nodes: s.selection.nodes },
      notices: s.notices.map((n) => ({ level: n.level, message: n.message })),
      git: { answers: s.git.answers, busy: s.git.busy, stale: s.git.stale, status: s.git.status },
    };
  });
}

async function send(page: Page, message: unknown): Promise<void> {
  await page.evaluate((msg) => {
    const w = window as unknown as { __cicada: CicadaHandle };
    w.__cicada.send(msg);
  }, message);
}

/**
 * Start a stopwatch IN the page that stops when the store's text becomes
 * `want` (5 ms resolution): the browser-side half of "revert reaches the
 * canvas within the measured barrier budget" (docs/17 item 2 — the route
 * test measures POST → barrier snapshot on the server; this measures the
 * click → the reloaded text in the store, which is what the canvas and
 * the text panel render from). Call it BEFORE the click, await it after.
 */
function stopwatchUntilText(page: Page, want: string): Promise<number> {
  return page.evaluate(
    (target) =>
      new Promise<number>((resolve, reject) => {
        const w = window as unknown as { __cicada: CicadaHandle };
        const t0 = performance.now();
        const timer = setInterval(() => {
          if (w.__cicada.state().text === target) {
            clearInterval(timer);
            resolve(performance.now() - t0);
          } else if (performance.now() - t0 > 15_000) {
            clearInterval(timer);
            reject(new Error("the store never reached the expected text"));
          }
        }, 5);
      }),
    want,
  );
}

/**
 * One sample of what the user sees of the git status: the store's flag,
 * the chip (its `data-stale` and its count), the Git tab's commit hint and
 * which scope note it shows. `traceStaleWindow` records the DISTINCT
 * samples from just before an edit is sent until the re-read has landed
 * and the DOM agrees — in the page, at 5 ms, so the ≤1 s stale window is
 * caught deterministically (a Playwright poll from outside could miss it).
 */
interface StaleSample {
  /** ms since the intent was sent (the first sample is taken just before, at 0). */
  t: number;
  stale: boolean;
  chipStale: string | null;
  dirty: string | null;
  hint: string | null;
  scope: string | null;
}

function traceStaleWindow(page: Page, message: unknown): Promise<StaleSample[]> {
  return page.evaluate(
    (msg) =>
      new Promise<StaleSample[]>((resolve, reject) => {
        const w = window as unknown as { __cicada: CicadaHandle };
        const q = (id: string) => document.querySelector(`[data-testid="${id}"]`);
        const t0 = performance.now();
        const sample = (): StaleSample => ({
          t: Math.round(performance.now() - t0),
          stale: w.__cicada.state().git.stale,
          chipStale: q("tb-git")?.getAttribute("data-stale") ?? null,
          dirty: q("tb-git-dirty")?.textContent ?? null,
          hint: q("git-commit-hint")?.textContent ?? null,
          scope: q("git-scope-section")?.getAttribute("data-note") ?? null,
        });
        const phases: StaleSample[] = [];
        const same = (a: StaleSample, b: StaleSample) =>
          a.stale === b.stale && a.chipStale === b.chipStale && a.dirty === b.dirty && a.hint === b.hint && a.scope === b.scope;
        const push = () => {
          const now = sample();
          const last = phases[phases.length - 1];
          if (last === undefined || !same(last, now)) phases.push(now);
        };
        push();
        w.__cicada.send(msg);
        const timer = setInterval(() => {
          push();
          const now = phases[phases.length - 1]!;
          const sawStale = phases.some((p) => p.stale);
          const domCurrent =
            now.chipStale === "false" && now.scope !== null && now.scope !== "refreshing" && now.hint !== null && !/re-reading/.test(now.hint);
          if (sawStale && !now.stale && domCurrent) {
            clearInterval(timer);
            resolve(phases);
          } else if (performance.now() - t0 > 15_000) {
            clearInterval(timer);
            reject(new Error(`the stale window never closed: ${JSON.stringify(phases)}`));
          }
        }, 5);
      }),
    message,
  );
}

/** Wheel over the canvas until its zoom LOD tier (`data-lod`) is `tier`; throws if it never gets there. */
async function zoomToTier(page: Page, tier: "far" | "near"): Promise<void> {
  const canvas = page.locator(".cicada-canvas");
  const pane = page.locator(".react-flow__pane");
  const box = await pane.boundingBox();
  if (box === null) throw new Error("no canvas pane");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 40; i += 1) {
    if ((await canvas.getAttribute("data-lod")) === tier) return;
    await page.mouse.wheel(0, tier === "far" ? 400 : -400);
  }
  throw new Error(`the canvas never reached the ${tier} zoom tier (at ${await canvas.getAttribute("data-lod")})`);
}

/** The computed style of a node's `::before` (the far-tier git stripe lives there). */
async function stripeOf(page: Page, node: string): Promise<{ content: string; width: string; background: string }> {
  return page.evaluate((name) => {
    const el = document.querySelector(`.react-flow__node[data-id='${name}'] .cn`);
    if (el === null) throw new Error(`no node ${name}`);
    const css = getComputedStyle(el, "::before");
    return { content: css.content, width: css.width, background: css.backgroundColor };
  }, node);
}

async function open(page: Page): Promise<void> {
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.locator(".react-flow__node")).toHaveCount(3);
}

const sizeNode = (page: Page) => page.locator(".react-flow__node[data-id='size'] .cn");
const chip = (page: Page) => page.getByTestId("tb-git");

test.describe.configure({ mode: "serial" });

test.describe("git panel", () => {
  let page: Page;
  const errors: string[] = [];
  /** Typed git-route refusals Chrome narrated (`Failed to load resource … 4xx/5xx` for `/api/git/*`): by design, toasted by the app — not page errors. */
  const refusals: string[] = [];

  test.beforeAll(async ({ browser }) => {
    writeFileSync(FILE, START);
    page = await browser.newPage();
    page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
    page.on("console", (message) => {
      if (message.type() !== "error") return;
      if (/Failed to load resource/.test(message.text()) && /\/api\/git\//.test(message.location().url)) {
        refusals.push(`${message.location().url}: ${message.text()}`);
        return;
      }
      errors.push(`console: ${message.text()}`);
    });
  });

  test.afterAll(async () => {
    await page.close();
    expect(errors, "no page errors / console errors during the git flow").toEqual([]);
    // Nothing in this flow asks the server for something it refuses; a
    // spec that does asserts its toast AND its entry here.
    expect(refusals, "no git route refused anything in this flow").toEqual([]);
  });

  test("outside any repository: the chip says `no repo`, the Git tab says why, Ctrl+S explains and offers no form", async () => {
    // The scratch lives in the OS temp dir; a repository above it would make
    // `no repo` untestable — say so instead of asserting the wrong thing.
    const inside = git(["rev-parse", "--is-inside-work-tree"], true);
    if (inside.ok) {
      throw new Error(`the e2e scratch ${PROJECT} is already inside a git work tree — the no-repo case cannot be tested here`);
    }
    await open(page);

    await expect(page.getByTestId("tb-git-branch")).toHaveText("no repo");
    await expect(chip(page)).toHaveAttribute("data-kind", "not_a_repo");
    await expect(page.getByTestId("tb-git-dirty")).toHaveCount(0);
    await expect(chip(page)).toHaveAttribute("title", /git init/);

    await chip(page).click();
    await expect(page.getByTestId("insp-body-git")).toBeVisible();
    await expect(page.getByTestId("git-state")).toHaveAttribute("data-kind", "not_a_repo");
    await expect(page.getByTestId("git-state")).toContainText(/not in a git repository/);
    await expect(page.getByTestId("git-commit")).toHaveCount(0);
    await expect(page.getByTestId("git-revert")).toHaveCount(0);
    await expect(page.getByTestId("git-marker-count")).toHaveCount(0);

    // Ctrl+S: the commit dialog, never the browser's save; it says why.
    await page.keyboard.press("Control+s");
    await expect(page.getByTestId("commit-dialog")).toBeVisible();
    await expect(page.getByTestId("commit-dialog-blocked")).toContainText(/not in a git repository/);
    await expect(page.getByTestId("git-message")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("commit-dialog")).toHaveCount(0);
    expect((await store(page)).commitDialog).toBe(false);
  });

  test("after `git init` + a baseline commit: branch and clean; an edit marks the node `modified` and the chip `1 dirty`", async () => {
    git(["init", "-q", "--initial-branch=main"]);
    mkdirSync(join(PROJECT, ".no-hooks"), { recursive: true });
    for (const [key, value] of [
      ["user.name", "Cicada E2E"],
      ["user.email", "e2e@cicada.invalid"],
      ["commit.gpgsign", "false"],
      ["core.autocrlf", "false"],
      ["core.fsmonitor", "false"],
      ["core.hooksPath", join(PROJECT, ".no-hooks")],
    ]) {
      git(["config", key!, value!]);
    }
    // The whole project: the commit scope is the pipeline, its sidecar AND
    // `scripts/*.py` beside it — the example scripts would otherwise sit in
    // the scope as untracked files and the tree would never read `clean`.
    git(["add", "-A"]);
    git(["commit", "-q", "-m", "baseline"]);
    expect(git(["rev-parse", "--abbrev-ref", "HEAD"]).out).toBe("main");

    // A (re)connect reads the status now (the policy: on hello, after
    // writes, on focus — never a timer).
    await open(page);
    await expect(page.getByTestId("tb-git-branch")).toHaveText("main");
    await expect(chip(page)).toHaveAttribute("data-kind", "repo");
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("clean");
    await expect(sizeNode(page)).not.toHaveAttribute("data-git", /.+/);
    await expect(page.getByTestId("insp-tab-git-count")).toHaveCount(0);

    // The Git tab open over the clean tree: "nothing to commit" is true now.
    await chip(page).click();
    await expect(page.getByTestId("git-clean")).toBeVisible();
    await expect(page.getByTestId("git-scope-section")).toHaveAttribute("data-note", "clean");
    await expect(page.getByTestId("git-commit-hint")).toHaveText(/nothing to commit/);
    await expect(chip(page)).toHaveAttribute("data-stale", "false");

    // Edit through the same op pipeline a slider release uses — and watch
    // the ≤1 s between the delta and the re-read from inside the page: the
    // cached "clean" answer must be marked STALE the moment the delta lands
    // (the chip dims its count, the tab says "re-reading", the Commit button
    // waits), never "nothing to commit" over an edit that is already on disk.
    const phases = await traceStaleWindow(page, { type: "set_param", payload: { node: "size", port: "value", value: "3.0" } });
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_3);
    expect(phases[0], "before the edit: current and clean").toMatchObject({ stale: false, chipStale: "false", dirty: "clean", scope: "clean" });
    expect(phases[0]!.hint).toMatch(/nothing to commit/);
    const staleDom = phases.find((p) => p.stale && p.chipStale === "true" && p.scope === "refreshing");
    expect(staleDom, `a stale phase reached the DOM: ${JSON.stringify(phases)}`).toBeDefined();
    expect(staleDom!.dirty, "the last known count stays (dimmed), not blanked").toBe("clean");
    expect(staleDom!.hint, "the Commit hint says re-reading, not nothing-to-commit").toBe(STALE_REASON);
    const settled = phases[phases.length - 1]!;
    expect(settled).toMatchObject({ stale: false, chipStale: "false", dirty: "1 dirty", scope: "files" });
    expect(settled.hint).toMatch(/write a commit message/);
    console.log(
      `stale window: the delta marked the cache stale at ${staleDom!.t} ms, the re-read answered at ${settled.t} ms (${phases.length} distinct phases)`,
    );
    for (const p of phases) {
      if (p.stale) expect(p.hint, "no phase says 'nothing to commit' while stale").not.toMatch(/nothing to commit/);
    }
    expect((await store(page)).git.stale).toBe(false);

    // ≤1 s later the status is re-read: the badge, the chip, the tab count.
    await expect(sizeNode(page)).toHaveAttribute("data-git", "modified");
    const badge = page.getByTestId("git-size");
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("~");
    await expect(badge).toHaveAttribute("title", /modified since HEAD/);
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("1 dirty");
    await expect(page.getByTestId("insp-tab-git-count")).toHaveText("1");
    // The other nodes are untouched.
    await expect(page.locator(".react-flow__node[data-id='span'] .cn")).not.toHaveAttribute("data-git", /.+/);

    // At the far zoom tier the header badges hide; the marker survives as
    // a stripe down the node's left edge (`::before`, the kind's token) and
    // a tint over the node — a zoomed-out pipeline still shows what differs
    // from HEAD. The untouched node wears neither; at the near tier the
    // stripe is gone again and the badge is back.
    expect((await stripeOf(page, "size")).content, "no stripe at the near tier (the badge is on the header)").toBe("none");
    await zoomToTier(page, "far");
    await expect(badge).toBeHidden();
    const farStripe = await stripeOf(page, "size");
    expect(farStripe.content).toBe('""');
    expect(farStripe.width).toBe("10px");
    expect(farStripe.background, "the stripe is painted (the kind's token resolved)").not.toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
    expect((await stripeOf(page, "span")).content, "an unchanged node has no stripe").toBe("none");
    await zoomToTier(page, "near");
    await expect(badge).toBeVisible();
    expect((await stripeOf(page, "size")).content).toBe("none");
    // Back to the load-time view for the tests that follow (they click into the nodes).
    await open(page);
    await expect(sizeNode(page)).toHaveAttribute("data-git", "modified");
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("1 dirty");

    // The Git tab: the marker under `modified`, click → selects the node; the scope lists the file.
    await chip(page).click();
    await expect(page.getByTestId("git-marker-count")).toHaveText("1");
    const row = page.getByTestId("git-node-size");
    await expect(row).toHaveAttribute("data-change", "modified");
    await expect(page.getByTestId("git-group-modified")).toContainText("size");
    await row.getByRole("button", { name: "size" }).click();
    await expect.poll(async () => (await store(page)).selection.nodes).toEqual(["size"]);
    await expect(page.getByTestId("git-dirty-count")).toHaveText("1");
    await expect(page.getByTestId(`git-file-${PIPELINE}`)).toHaveAttribute("data-status", "modified");
    // Nothing to write yet → Commit disabled with the reason; Revert enabled.
    await expect(page.getByTestId("git-commit-submit")).toBeDisabled();
    await expect(page.getByTestId("git-commit-submit")).toHaveAttribute("title", /write a commit message/);
    await expect(page.getByTestId("git-revert")).toBeEnabled();
  });

  test("Ctrl+S → message → Ctrl+Enter commits VERBATIM: `git log` agrees, the badge clears, the toast names the short hash", async () => {
    const dialog = page.getByTestId("commit-dialog");
    const message = dialog.getByTestId("git-message");

    // Ctrl+S from a TEXT FIELD reaches the dialog too: the text-entry gate
    // that keeps typing off the hotkey map must not swallow the save reflex
    // where it lands most — and the two canvas text fields that stop their
    // keys from reaching React Flow (the search box, a literal input) must
    // let this one chord through to the window, or the browser's own save
    // dialog opens over them. Real DOM events here: a unit test that drives
    // the router directly cannot see a stopped event.
    // (Wave 4 B3: the literal is a chip; double-click opens its editor.)
    await page.getByTestId("lit-span-start").dblclick();
    const literal = page.getByTestId("lit-span-start-input");
    await expect(literal).toBeFocused();
    await literal.press("Control+s");
    await expect(dialog).toHaveCount(1);
    await expect(message).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    // The editor lost its focus to the dialog, which commits its UNCHANGED
    // value — and an unchanged value writes nothing: the chip is back with
    // the text's literal, and the file is as it was.
    await expect(literal).toHaveCount(0);
    await expect(page.getByTestId("lit-span-start")).toHaveText("0.0");
    expect((await store(page)).commitDialog).toBe(false);

    const pane = page.locator(".react-flow__pane");
    const paneBox = await pane.boundingBox();
    if (paneBox === null) throw new Error("no canvas pane");
    await pane.dblclick({ position: { x: paneBox.width * 0.9, y: paneBox.height * 0.9 } });
    const search = page.getByTestId("search-input");
    await expect(search).toBeFocused();
    await search.press("Control+s");
    await expect(dialog).toHaveCount(1);
    await expect(message).toBeFocused();
    // Esc closes the dialog and ONLY the dialog (the hotkey map closes it
    // before it would touch the search box or the selection).
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(search).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(search).toHaveCount(0);
    expect((await store(page)).commitDialog).toBe(false);
    // None of that was an edit: still the one dirty file, the same status.
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("1 dirty");

    // Focus the page (not a text field) so the hotkey map sees Ctrl+S.
    await pane.click({ position: { x: 20, y: 20 } });
    await page.keyboard.press("Control+s");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("commit-dialog-head")).toHaveText("main");
    await expect(dialog.getByTestId(`git-file-${PIPELINE}`)).toBeVisible();
    await expect(message).toBeFocused();
    const submit = dialog.getByTestId("git-commit-submit");
    await expect(submit).toBeDisabled();
    await message.fill(MESSAGE);
    await expect(submit).toBeEnabled();
    // The reflex inside the dialog's own message field: consumed, one
    // dialog, the draft and the focus untouched.
    await message.press("Control+s");
    await expect(dialog).toHaveCount(1);
    await expect(message).toBeFocused();
    await expect(message).toHaveValue(MESSAGE);
    await message.press("Control+Enter");

    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId("notices")).toContainText(/committed [0-9a-f]{7} — e2e: size to 3/);
    expect(git(["log", "-1", "--format=%s"]).out).toBe(MESSAGE);
    expect(git(["show", "--format=", "--name-only", "HEAD"]).out.split(/\r?\n/).filter(Boolean), "exactly the scope").toEqual([
      PIPELINE,
    ]);
    expect(git(["status", "--porcelain", "--", PIPELINE]).out).toBe("");

    // The commit sent no delta, so the status was re-read explicitly.
    await expect(sizeNode(page)).not.toHaveAttribute("data-git", /.+/);
    await expect(page.getByTestId("git-size")).toHaveCount(0);
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("clean");
    await expect(page.getByTestId("insp-tab-git-count")).toHaveCount(0);
    // A commit is a git action, not a document edit: the undo history stands.
    expect((await debugState(page)).history.can_undo).toBe(true);
  });

  test("edit again → Revert to HEAD through the confirm step: the list is BINDING over a two-file scope, the text panel shows HEAD's line, the barrier cleared the undo log, disk == HEAD", async () => {
    await send(page, { type: "set_param", payload: { node: "size", port: "value", value: "4.0" } });
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_4);
    await expect(sizeNode(page)).toHaveAttribute("data-git", "modified");
    expect((await debugState(page)).history.can_undo).toBe(true);

    // A second dirty file the revert must LEAVE ALONE: the first node move
    // writes the layout sidecar, which the baseline never had — dirty (in
    // the scope) with no HEAD version. The server says so per file
    // (`in_head`); the confirm step must list only the pipeline and the
    // request must name only the pipeline — a request naming the sidecar
    // would be refused whole (`409 untracked`), and a request naming
    // nothing would revert "everything dirty" behind the user's back.
    expect(git(["ls-files", "--", SIDECAR]).out, "the sidecar must have no HEAD version for this test to mean anything").toBe("");
    await send(page, { type: "move_node", payload: { node: "size", cell: [6, 3] } });
    await expect.poll(() => existsSync(SIDECAR_FILE)).toBe(true);
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("2 dirty");
    expect((await debugState(page)).history.can_undo).toBe(true);

    await chip(page).click();
    await expect(page.getByTestId(`git-file-${SIDECAR}`)).toHaveAttribute("data-status", "untracked");
    await expect(page.getByTestId("git-revert-confirm")).toHaveCount(0);
    await page.getByTestId("git-revert").click();
    const confirm = page.getByTestId("git-revert-confirm");
    await expect(confirm).toBeVisible();
    await expect(confirm).toContainText(/discards every uncommitted edit in this file/);
    await expect(confirm).toContainText(/undo history is cleared/);
    await expect(confirm.getByTestId(`git-file-${PIPELINE}`)).toBeVisible();
    await expect(confirm.getByTestId(`git-file-${SIDECAR}`)).toHaveCount(0);
    await expect(confirm).toContainText(`left alone (no HEAD version to go back to): ${SIDECAR}`);
    // Second thoughts first: the `keep my edits` path changes nothing.
    await page.getByTestId("git-revert-confirm-no").click();
    await expect(confirm).toHaveCount(0);
    expect(readFileSync(FILE, "utf8")).toBe(SIZE_4);

    await page.getByTestId("git-revert").click();
    await expect(confirm).toBeVisible();
    const posted = page.waitForRequest((request) => request.method() === "POST" && request.url().includes("/api/git/revert"));
    const reached = stopwatchUntilText(page, SIZE_3);
    await page.getByTestId("git-revert-confirm-yes").click();
    // The binding list on the wire: exactly the one path the step showed.
    const body = (await posted).postDataJSON() as { paths?: string[]; client?: number };
    expect(body.paths, "the request names exactly the files the confirm step listed").toEqual([PIPELINE]);
    expect(typeof body.client).toBe("number");
    const elapsed = await reached;
    console.log(`revert → canvas: click to the reloaded text in the store ${elapsed.toFixed(0)} ms`);
    expect(elapsed, "revert reaches the canvas within the barrier budget (debug build, generous)").toBeLessThan(2000);

    const notices = page.getByTestId("notices");
    await expect(notices).toContainText(/reverted to HEAD: git\.cic/);
    // Narrowed to the listed path, the server had nothing to report as
    // left alone — the toast that would name the sidecar is the tell of a
    // request that named no paths.
    await expect(notices).not.toContainText(/left alone/);
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_3);
    // The reload barrier (`reason: "git revert"`) cleared the op log.
    const after = await debugState(page);
    expect(after.text).toBe(SIZE_3);
    expect(after.history).toMatchObject({ can_undo: false, can_redo: false, depth: 0 });
    await expect(page.getByTestId("tb-undo")).toBeDisabled();
    // The text panel shows HEAD's line; the badge is gone; the sidecar is
    // untouched and still the one dirty file.
    await page.getByTestId("insp-tab-text").click();
    await expect(page.getByTestId("text-panel")).toContainText("size = slider(value=3.0, min=0.5, max=5.0)");
    await expect(sizeNode(page)).not.toHaveAttribute("data-git", /.+/);
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("1 dirty");
    expect(existsSync(SIDECAR_FILE), "the file without a HEAD version was left alone").toBe(true);
    expect(git(["status", "--porcelain", "--", PIPELINE]).out).toBe("");
    expect(git(["status", "--porcelain", "--", SIDECAR]).out).toBe(`?? ${SIDECAR}`);

    // Hand the next test a clean tree the way a user would: commit the
    // sidecar from a shell (the app's status re-reads on connect/focus).
    git(["add", "--", SIDECAR]);
    git(["commit", "-q", "-m", "sidecar"]);
    expect(git(["status", "--porcelain", "--", PIPELINE, SIDECAR]).out).toBe("");
  });

  test("a `.gitignore`d pipeline: the chip says `ignored` with no dirty count; the tab shows the ignored warning and NOT 'nothing to commit' beside it; Commit is disabled with the reason", async () => {
    // git honors an untracked .gitignore; the rule names the second
    // pipeline only, so `git.cic` (the other tests) is unaffected.
    writeFileSync(join(PROJECT, IGNORED), START);
    writeFileSync(join(PROJECT, ".gitignore"), `${IGNORED}\n`);
    expect(git(["check-ignore", "--", IGNORED]).out).toBe(IGNORED);

    await page.goto(`/?token=${TOKEN}&pipeline=${IGNORED}`);
    await expect(page.getByTestId("app")).toBeVisible();
    await expect(page.locator(".react-flow__node")).toHaveCount(3);

    await expect(page.getByTestId("tb-git-branch")).toHaveText("main");
    await expect(chip(page)).toContainText("ignored");
    await expect(page.getByTestId("tb-git-dirty"), "no count: git lists nothing for an ignored file").toHaveCount(0);
    await expect(chip(page)).toHaveAttribute("title", /is ignored by a \.gitignore rule/);

    await chip(page).click();
    await expect(page.getByTestId("git-ignored")).toBeVisible();
    await expect(page.getByTestId("git-ignored")).toContainText(`\`${IGNORED}\` is ignored by a .gitignore rule`);
    await expect(page.getByTestId("git-scope-section")).toHaveAttribute("data-note", "ignored");
    // The contradiction this guards against: the ignored warning AND
    // "nothing to commit — the scope matches HEAD" on one screen (the file
    // has no HEAD version; every node wears `+`).
    await expect(page.getByTestId("git-clean")).toHaveCount(0);
    await expect(page.getByTestId("git-scope-refreshing")).toHaveCount(0);
    await expect(page.getByTestId("git-marker-count")).toHaveText("3");
    await expect(page.getByTestId("git-group-added")).toContainText("size");
    await expect(page.getByTestId("git-commit-submit")).toBeDisabled();
    await expect(page.getByTestId("git-commit-submit")).toHaveAttribute("title", /is ignored by a \.gitignore rule/);
    await expect(page.getByTestId("git-revert")).toBeDisabled();

    // Ctrl+S: the dialog says why, no form.
    await page.locator(".react-flow__pane").click({ position: { x: 20, y: 20 } });
    await page.keyboard.press("Control+s");
    await expect(page.getByTestId("commit-dialog")).toBeVisible();
    await expect(page.getByTestId("commit-dialog-blocked")).toContainText(/is ignored by a \.gitignore rule/);
    // (The Git tab's own form is still in the inspector — scope to the dialog.)
    await expect(page.getByTestId("commit-dialog").getByTestId("git-message")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("commit-dialog")).toHaveCount(0);

    // Back on the main pipeline as its writer — the observer test below
    // needs this page to hold the lease.
    await open(page);
    await expect.poll(async () => (await store(page)).role).toBe("writer");
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("clean");
  });

  test("a read-only observer sees the status but no commit or revert controls; Ctrl+S says so", async ({ browser }) => {
    const context: BrowserContext = await browser.newContext();
    const observer = await context.newPage();
    try {
      await observer.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
      await expect(observer.getByTestId("app")).toBeVisible();
      await expect.poll(async () => (await store(observer)).role).toBe("observer");
      await expect(observer.getByTestId("tb-git-branch")).toHaveText("main");
      await expect(observer.getByTestId("tb-git-dirty")).toHaveText("clean");

      await chip(observer).click();
      await expect(observer.getByTestId("git-state")).toHaveAttribute("data-kind", "repo");
      await expect(observer.getByTestId("git-observer-note")).toContainText(/read-only observer/);
      await expect(observer.getByTestId("git-commit")).toHaveCount(0);
      await expect(observer.getByTestId("git-revert")).toHaveCount(0);
      await expect(observer.getByTestId("git-clean")).toBeVisible();

      await observer.locator(".react-flow__pane").click({ position: { x: 20, y: 20 } });
      await observer.keyboard.press("Control+s");
      await expect(observer.getByTestId("commit-dialog")).toBeVisible();
      await expect(observer.getByTestId("commit-dialog-blocked")).toContainText(/read-only observer/);
      await expect(observer.getByTestId("git-message")).toHaveCount(0);
      await observer.keyboard.press("Escape");
      await expect(observer.getByTestId("commit-dialog")).toHaveCount(0);
    } finally {
      await context.close();
    }
  });
});
