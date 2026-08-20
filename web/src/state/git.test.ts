/**
 * The store's git slice and the policy's feed (docs/17 item 2): which
 * server messages trigger a status read, the `text_hash` dedupe, the
 * canvas-facing marker index, and the gate every commit/revert affordance
 * reads (`gitWriteBlockReason`).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GitStatusResponse, RepoInfo, ServerEnvelope } from "../protocol/messages";
import { feedGitPolicy } from "./connection";
import {
  commitBlockReason,
  GIT_REFRESH_DEBOUNCE_MS,
  gitPolicy,
  gitWriteBlockReason,
  ignoredReason,
  startGitStatus,
  stopGitStatus,
} from "./git";
import { markersByName, sameGitStatus, useCicada } from "./store";

const INFO: RepoInfo = {
  root: "C:/repo",
  prefix: "",
  branch: "main",
  head_short: "abc1234",
  upstream: { name: "origin/main", ahead: 1, behind: 0 },
  unborn: false,
};

const REPO: GitStatusResponse = {
  state: { kind: "repo", ...INFO },
  pipeline: {
    path: "p.cic",
    tracked: true,
    ignored: false,
    dirty: true,
    nodes: [
      { name: "size", change: "modified" },
      { name: "extra", change: "added" },
      { name: "span2", change: "renamed", from: "span" },
    ],
    removed: [{ name: "old", line_in_head: 5 }],
  },
  scope: [{ path: "p.cic", status: "modified" }],
  text_hash: "11".repeat(32),
};

const HISTORY = { can_undo: false, can_redo: false, undo_label: null, redo_label: null, depth: 0 };
const GRAPH = { nodes: [], wires: [], diagnostics: [] };
const SUMMARY = {
  generation: 0,
  running: false,
  cancelled: false,
  computed: 0,
  cached: 0,
  pending: 0,
  red: 0,
  blocked: 0,
  elapsed_ms: 0,
  eta_rough: false,
};

describe("feedGitPolicy", () => {
  it("hello → now; delta and barrier snapshot → debounced write; everything else → nothing", () => {
    const policy = { onConnected: vi.fn(), onWrite: vi.fn() };
    const feed = (message: ServerEnvelope) => feedGitPolicy(policy, message);
    feed({
      v: 1,
      seq: 0,
      type: "hello",
      payload: { client_id: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unit_px: 24 },
    });
    expect(policy.onConnected).toHaveBeenCalledTimes(1);
    expect(policy.onWrite).toHaveBeenCalledTimes(0);

    const snapshot = (barrier: boolean): ServerEnvelope => ({
      v: 1,
      seq: 1,
      type: "snapshot",
      payload: {
        graph: GRAPH,
        text: "# cicada 1\n",
        statuses: {},
        summary: SUMMARY,
        lease: { writer: 1, clients: [[1, "writer"]] },
        barrier,
        reason: barrier ? "git revert" : "initial",
        history: HISTORY,
      },
    });
    feed(snapshot(false));
    expect(policy.onWrite, "the initial snapshot follows the hello's read").toHaveBeenCalledTimes(0);
    feed(snapshot(true));
    expect(policy.onWrite, "a barrier = the tree changed under us").toHaveBeenCalledTimes(1);

    feed({
      v: 1,
      seq: 2,
      type: "delta",
      payload: {
        source: { client: 1, label: "move size" },
        graph: GRAPH,
        text: "# cicada 1\n",
        dirty: [],
        history: HISTORY,
      },
    });
    expect(policy.onWrite, "a sidecar-only delta dirties the scope too").toHaveBeenCalledTimes(2);

    feed({ v: 1, seq: 3, type: "status", payload: { generation: 1, nodes: {}, summary: SUMMARY } });
    feed({ v: 1, seq: 4, type: "lease", payload: { lease: { writer: 1, clients: [] }, role: "writer" } });
    feed({ v: 1, seq: 5, type: "notice", payload: { level: "info", message: "x" } });
    feed({ v: 1, seq: 6, type: "display_reset", payload: { generation: 1 } });
    expect(policy.onConnected).toHaveBeenCalledTimes(1);
    expect(policy.onWrite).toHaveBeenCalledTimes(2);
  });
});

/**
 * The INSTALLED policy (what the app runs), not a hand-built one: the
 * debounce it is constructed with is the exported ≤1 s constant, the window
 * `focus` listener reads once, idle time reads nothing, and `stopGitStatus`
 * takes the listener down. A fake window collects the listeners.
 */
describe("startGitStatus — the installed policy", () => {
  const fakeWindow = () => {
    const listeners = new Map<string, EventListenerOrEventListenerObject>();
    return {
      listeners,
      addEventListener: vi.fn((type: string, listener: EventListenerOrEventListenerObject) => {
        listeners.set(type, listener);
      }),
      removeEventListener: vi.fn((type: string) => {
        listeners.delete(type);
      }),
      focus() {
        const listener = listeners.get("focus");
        if (listener === undefined) throw new Error("no focus listener installed");
        (listener as () => void)();
      },
    };
  };
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    stopGitStatus(null);
    vi.useRealTimers();
  });

  it("debounces writes by the exported constant, which is ≤ 1 s (docs/17: '≤1 s debounced after every delta')", async () => {
    expect(GIT_REFRESH_DEBOUNCE_MS).toBeLessThanOrEqual(1000);
    expect(GIT_REFRESH_DEBOUNCE_MS).toBeGreaterThan(0);
    const read = vi.fn(() => Promise.resolve());
    const policy = startGitStatus(null, read);
    expect(policy.debounceMs).toBe(GIT_REFRESH_DEBOUNCE_MS);
    expect(gitPolicy()).toBe(policy);
    expect(read, "installing reads nothing — the hello does").toHaveBeenCalledTimes(0);
    // The way the connection module feeds it: one delta → one read, exactly the debounce later.
    feedGitPolicy(policy, {
      v: 1,
      seq: 2,
      type: "delta",
      payload: { source: { client: 1, label: "set size" }, graph: GRAPH, text: "# cicada 1\n", dirty: [], history: HISTORY },
    });
    await vi.advanceTimersByTimeAsync(GIT_REFRESH_DEBOUNCE_MS - 1);
    expect(read).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(read).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(read, "never on a timer while idle").toHaveBeenCalledTimes(1);
  });

  it("installs ONE window `focus` listener that reads now; stopGitStatus removes it and disposes the policy", async () => {
    const win = fakeWindow();
    const read = vi.fn(() => Promise.resolve());
    const policy = startGitStatus(win, read);
    expect(win.addEventListener).toHaveBeenCalledTimes(1);
    expect(win.addEventListener.mock.calls[0]?.[0]).toBe("focus");
    win.focus();
    expect(read, "focus → read now (a shell may have moved HEAD)").toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(0);
    win.focus();
    expect(read).toHaveBeenCalledTimes(2);
    expect(policy.reads).toBe(2);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(read, "no timer between focus events").toHaveBeenCalledTimes(2);

    stopGitStatus(win);
    expect(win.removeEventListener).toHaveBeenCalledWith("focus", expect.any(Function));
    expect(win.listeners.has("focus")).toBe(false);
    expect(gitPolicy()).toBeNull();
    policy.onWrite();
    policy.onFocus();
    await vi.advanceTimersByTimeAsync(5000);
    expect(read, "a disposed policy reads no more").toHaveBeenCalledTimes(2);
  });

  it("a second start replaces the first: the old listener is gone, one policy is live", () => {
    const win = fakeWindow();
    const first = startGitStatus(win, () => Promise.resolve());
    const second = startGitStatus(win, () => Promise.resolve());
    expect(second).not.toBe(first);
    expect(gitPolicy()).toBe(second);
    expect(win.addEventListener).toHaveBeenCalledTimes(2);
    expect(win.removeEventListener).toHaveBeenCalledTimes(1);
    expect(win.listeners.size).toBe(1);
    stopGitStatus(win);
  });
});

describe("the git slice", () => {
  beforeEach(() => {
    useCicada.setState({
      git: { status: null, error: null, loading: false, busy: null, answers: 0 },
      gitMarkers: {},
      notices: [],
    });
  });

  it("a status answer fills the cache and the marker index (removed nodes are not badged)", () => {
    useCicada.getState().setGitLoading(true);
    useCicada.getState().setGitStatus(REPO);
    const s = useCicada.getState();
    expect(s.git.status).toEqual(REPO);
    expect(s.git.loading).toBe(false);
    expect(s.git.answers).toBe(1);
    expect(s.gitMarkers).toEqual({
      size: { name: "size", change: "modified" },
      extra: { name: "extra", change: "added" },
      span2: { name: "span2", change: "renamed", from: "span" },
    });
    expect(markersByName(REPO).old).toBeUndefined();
  });

  it("the same text_hash with the same facts is deduped: identities survive, answers still count", () => {
    useCicada.getState().setGitStatus(REPO);
    const before = useCicada.getState();
    useCicada.getState().setGitStatus(JSON.parse(JSON.stringify(REPO)) as GitStatusResponse);
    const after = useCicada.getState();
    expect(after.git.status).toBe(before.git.status);
    expect(after.gitMarkers).toBe(before.gitMarkers);
    expect(after.git.answers).toBe(2);
  });

  it("a new text_hash replaces the cache; the same hash with a changed scope does too", () => {
    useCicada.getState().setGitStatus(REPO);
    const cleaner: GitStatusResponse = {
      ...REPO,
      pipeline: { ...REPO.pipeline, dirty: false, nodes: [], removed: [] },
      scope: [],
      text_hash: "22".repeat(32),
    };
    useCicada.getState().setGitStatus(cleaner);
    expect(useCicada.getState().gitMarkers).toEqual({});
    // A node move dirties the sidecar: same pipeline bytes, a longer scope.
    const moved: GitStatusResponse = { ...cleaner, scope: [{ path: "p.cic.layout.json", status: "untracked" }] };
    expect(sameGitStatus(cleaner, moved)).toBe(false);
    useCicada.getState().setGitStatus(moved);
    expect(useCicada.getState().git.status?.scope).toEqual([{ path: "p.cic.layout.json", status: "untracked" }]);
  });

  it("a refused read keeps the last good answer beside the error; the next good answer clears it", () => {
    useCicada.getState().setGitStatus(REPO);
    useCicada.getState().setGitError({ kind: "git_failed", message: "git status failed", command: "git status --porcelain=v2 --no-optional-locks", code: 128 });
    let s = useCicada.getState();
    expect(s.git.status).toEqual(REPO);
    expect(s.git.error?.kind).toBe("git_failed");
    expect(s.git.answers).toBe(2);
    useCicada.getState().setGitStatus(REPO);
    s = useCicada.getState();
    expect(s.git.error).toBeNull();
    expect(s.git.answers).toBe(3);
  });
});

describe("gitWriteBlockReason", () => {
  const writer = () =>
    useCicada.setState({
      connection: "open",
      role: "writer",
      hello: { clientId: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
    });
  beforeEach(() => {
    writer();
    useCicada.setState({ git: { status: REPO, error: null, loading: false, busy: null, answers: 1 } });
  });
  afterEach(() => {
    useCicada.setState({ git: { status: null, error: null, loading: false, busy: null, answers: 0 } });
  });

  it("allows the writer of a plain repo", () => {
    expect(gitWriteBlockReason()).toBeNull();
  });

  it("names the observer, the dropped socket, the in-flight write, and every refusing state", () => {
    useCicada.setState({ role: "observer" });
    expect(gitWriteBlockReason()).toBe("read-only observer");
    writer();
    useCicada.setState({ connection: "reconnecting" });
    expect(gitWriteBlockReason()).toBe("not connected");
    writer();
    useCicada.getState().setGitBusy("commit");
    expect(gitWriteBlockReason()).toMatch(/commit is in progress/);
    useCicada.getState().setGitBusy(null);

    const withState = (state: GitStatusResponse["state"]) =>
      useCicada.setState({ git: { status: { ...REPO, state }, error: null, loading: false, busy: null, answers: 1 } });
    withState({ kind: "not_a_repo" });
    expect(gitWriteBlockReason()).toMatch(/not in a git repository/);
    withState({ kind: "git_not_found" });
    expect(gitWriteBlockReason()).toMatch(/no `git` on PATH/);
    withState({ kind: "locked", ...INFO });
    expect(gitWriteBlockReason()).toMatch(/index\.lock/);
    withState({ kind: "repo", ...INFO, operation: "rebase" });
    expect(gitWriteBlockReason()).toMatch(/a rebase is in progress/);
    withState({ kind: "repo", ...INFO, operation: "cherry_pick" });
    expect(gitWriteBlockReason()).toMatch(/a cherry-pick is in progress/);
  });

  it("commitBlockReason: the shared gate, then an IGNORED pipeline (one sentence, shared with the dialog and the tab), then the empty scope, then the blank message", () => {
    const state = () => useCicada.getState();
    expect(commitBlockReason(state(), "")).toBe("write a commit message first");
    expect(commitBlockReason(state(), "m")).toBeNull();
    const ignored: GitStatusResponse = {
      ...REPO,
      pipeline: { path: "ign.cic", tracked: false, ignored: true, dirty: true, nodes: [{ name: "n", change: "added" }], removed: [] },
      scope: [],
    };
    useCicada.setState({ git: { status: ignored, error: null, loading: false, busy: null, answers: 1 } });
    expect(commitBlockReason(state(), "m")).toBe(ignoredReason("ign.cic"));
    expect(ignoredReason("ign.cic")).toMatch(/^`ign\.cic` is ignored by a \.gitignore rule/);
    useCicada.setState({ git: { status: { ...REPO, scope: [] }, error: null, loading: false, busy: null, answers: 1 } });
    expect(commitBlockReason(state(), "m")).toMatch(/^nothing to commit/);
    useCicada.setState({ role: "observer" });
    expect(commitBlockReason(state(), "m")).toBe("read-only observer");
  });

  it("before the first answer it says so; after a refused first read it quotes the refusal", () => {
    useCicada.setState({ git: { status: null, error: null, loading: true, busy: null, answers: 0 } });
    expect(gitWriteBlockReason()).toMatch(/reading git status/);
    useCicada.setState({
      git: { status: null, error: { kind: "no_such_pipeline", message: "gone", path: "p.cic" }, loading: false, busy: null, answers: 1 },
    });
    expect(gitWriteBlockReason()).toMatch(/no pipeline `p\.cic`/);
  });
});
