/**
 * The git panel in the app (docs/17 item 2; docs/16 git chip · Git tab ·
 * Ctrl+S; doc 10 §Git integration slice 1): a project outside any
 * repository says `no repo` and offers no controls; after `git init` + a
 * baseline commit the chip shows the branch, an edit marks the node
 * `modified` on the canvas and the chip `1 dirty`, Ctrl+S opens the commit
 * dialog and Ctrl+Enter commits with the message VERBATIM (`git log`
 * agrees, the badge clears), a second edit is reverted to HEAD through the
 * confirm step (the text panel shows HEAD's line, the reload barrier
 * cleared the undo history, the file on disk equals HEAD), and a read-only
 * observer sees the status but no commit or revert controls.
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
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import config from "../playwright.config";

const meta = config.metadata as { token: string; scratch: string };
const TOKEN = meta.token;
const PIPELINE = "git.cic";
const PROJECT = join(meta.scratch, "examples");
const FILE = join(PROJECT, PIPELINE);

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

async function debugState(page: Page): Promise<DebugState> {
  const response = await page.request.get(`/debug/state?token=${TOKEN}&pipeline=${PIPELINE}&wait=true`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()) as DebugState;
}

interface StoreView {
  role: string;
  commitDialog: boolean;
  selection: { nodes: string[] };
  notices: { level: string; message: string }[];
  git: { answers: number; busy: string | null; status: { text_hash: string; scope: { path: string }[] } | null };
}

async function store(page: Page): Promise<StoreView> {
  return page.evaluate(() => {
    const w = window as unknown as { __cicada: { state: () => StoreView } };
    const s = w.__cicada.state();
    return {
      role: s.role,
      commitDialog: s.commitDialog,
      selection: { nodes: s.selection.nodes },
      notices: s.notices.map((n) => ({ level: n.level, message: n.message })),
      git: { answers: s.git.answers, busy: s.git.busy, status: s.git.status },
    };
  });
}

async function send(page: Page, message: unknown): Promise<void> {
  await page.evaluate((msg) => {
    const w = window as unknown as { __cicada: { send: (m: unknown) => string } };
    w.__cicada.send(msg);
  }, message);
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

  test.beforeAll(async ({ browser }) => {
    writeFileSync(FILE, START);
    page = await browser.newPage();
    page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(`console: ${message.text()}`);
    });
  });

  test.afterAll(async () => {
    await page.close();
    expect(errors, "no page errors / console errors during the git flow").toEqual([]);
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

    // Edit through the same op pipeline a slider release uses.
    await send(page, { type: "set_param", payload: { node: "size", port: "value", value: "3.0" } });
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_3);

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
    // Focus the page (not a text field) so the hotkey map sees Ctrl+S.
    await page.locator(".react-flow__pane").click({ position: { x: 20, y: 20 } });
    await page.keyboard.press("Control+s");
    const dialog = page.getByTestId("commit-dialog");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("commit-dialog-head")).toHaveText("main");
    await expect(dialog.getByTestId(`git-file-${PIPELINE}`)).toBeVisible();
    const message = dialog.getByTestId("git-message");
    await expect(message).toBeFocused();
    const submit = dialog.getByTestId("git-commit-submit");
    await expect(submit).toBeDisabled();
    await message.fill(MESSAGE);
    await expect(submit).toBeEnabled();
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

  test("edit again → Revert to HEAD through the confirm step: the text panel shows HEAD's line, the barrier cleared the undo log, disk == HEAD", async () => {
    await send(page, { type: "set_param", payload: { node: "size", port: "value", value: "4.0" } });
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_4);
    await expect(sizeNode(page)).toHaveAttribute("data-git", "modified");
    expect((await debugState(page)).history.can_undo).toBe(true);

    await chip(page).click();
    await expect(page.getByTestId("git-revert-confirm")).toHaveCount(0);
    await page.getByTestId("git-revert").click();
    const confirm = page.getByTestId("git-revert-confirm");
    await expect(confirm).toBeVisible();
    await expect(confirm).toContainText(/discards every uncommitted edit/);
    await expect(confirm).toContainText(/undo history is cleared/);
    await expect(confirm.getByTestId(`git-file-${PIPELINE}`)).toBeVisible();
    // Second thoughts first: the `keep my edits` path changes nothing.
    await page.getByTestId("git-revert-confirm-no").click();
    await expect(confirm).toHaveCount(0);
    expect(readFileSync(FILE, "utf8")).toBe(SIZE_4);

    await page.getByTestId("git-revert").click();
    await page.getByTestId("git-revert-confirm-yes").click();

    await expect(page.getByTestId("notices")).toContainText(/reverted to HEAD: git\.cic/);
    await expect.poll(async () => readFileSync(FILE, "utf8")).toBe(SIZE_3);
    // The reload barrier (`reason: "git revert"`) cleared the op log.
    const after = await debugState(page);
    expect(after.text).toBe(SIZE_3);
    expect(after.history).toMatchObject({ can_undo: false, can_redo: false, depth: 0 });
    await expect(page.getByTestId("tb-undo")).toBeDisabled();
    // The text panel shows HEAD's line; the badge and the dirty count are gone.
    await page.getByTestId("insp-tab-text").click();
    await expect(page.getByTestId("text-panel")).toContainText("size = slider(value=3.0, min=0.5, max=5.0)");
    await expect(sizeNode(page)).not.toHaveAttribute("data-git", /.+/);
    await expect(page.getByTestId("tb-git-dirty")).toHaveText("clean");
    expect(git(["status", "--porcelain", "--", PIPELINE]).out).toBe("");
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
