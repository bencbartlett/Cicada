/**
 * The git panel's state machine (docs/17 item 2; docs/10 §Git integration;
 * docs/13 `/api/git/*`): ONE `GitRefreshPolicy` per connection reads
 * `GET /api/git/status` into the store's `git` slice (on connect, ≤1/s
 * after writes, on window focus — never on a timer while idle), and the
 * two writes — `commitFromApp`, `revertToHead` — go out over HTTP with the
 * writer's client id, toast their outcome, and re-read. The revert's real
 * answer is the barrier snapshot the server broadcasts (op log cleared);
 * the HTTP response only says what went back.
 */
import {
  describeGitError,
  describeGitFailure,
  fetchGitStatus,
  GitRouteError,
  postGitCommit,
  postGitRevert,
} from "../protocol/git";
import type { GitSession } from "../protocol/git";
import type { CommitResponse, RevertResponse } from "../protocol/messages";
import { GitRefreshPolicy } from "./gitRefresh";
import { canWrite, useCicada, writeBlockReason, type CicadaState } from "./store";

let policy: GitRefreshPolicy | null = null;
let focusListener: (() => void) | null = null;

/** The debounce after a write (docs/17: "≤1 s debounced after every delta"). */
export const GIT_REFRESH_DEBOUNCE_MS = 1000;

function session(): GitSession {
  const state = useCicada.getState();
  return { token: state.token, pipeline: state.pipeline };
}

/** One status read into the store: a good answer replaces the cache, a refusal is kept beside it. */
export async function readGitStatus(): Promise<void> {
  const store = useCicada.getState();
  store.setGitLoading(true);
  try {
    const status = await fetchGitStatus(session());
    useCicada.getState().setGitStatus(status);
  } catch (error: unknown) {
    const state = useCicada.getState();
    if (error instanceof GitRouteError) {
      state.setGitError(error.body);
    } else {
      state.setGitError({ kind: "transport", message: String(error) });
    }
  }
}

/** What `startGitStatus` needs of the window: the `focus` event. */
export type FocusSource = Pick<Window, "addEventListener" | "removeEventListener">;

/**
 * Install the policy for this page's connection (once, from the connection
 * module): the window focus listener and the policy itself, with the ≤1 s
 * debounce. Installing reads nothing — the socket's `hello` does (a read
 * before the session exists would race the server's own hydration for no
 * answer the hello's read will not give). Returns the policy so the
 * connection module can feed it `hello` / `delta` / barrier events;
 * `window` is optional so tests can run without a DOM, and `read` is
 * injectable so they can count reads without a server.
 */
export function startGitStatus(win: FocusSource | null, read: () => Promise<void> = readGitStatus): GitRefreshPolicy {
  stopGitStatus(win);
  policy = new GitRefreshPolicy(read, GIT_REFRESH_DEBOUNCE_MS);
  if (win !== null) {
    focusListener = () => policy?.onFocus();
    win.addEventListener("focus", focusListener);
  }
  return policy;
}

export function stopGitStatus(win: FocusSource | null): void {
  policy?.dispose();
  policy = null;
  if (win !== null && focusListener !== null) win.removeEventListener("focus", focusListener);
  focusListener = null;
}

/** The installed policy (null before `startGitStatus`). */
export function gitPolicy(): GitRefreshPolicy | null {
  return policy;
}

/** Re-read the status now (after a commit; a manual refresh button). */
export function refreshGitNow(): void {
  policy?.now();
}

/**
 * Why this client cannot commit or revert right now — null when it can.
 * Display + gating only: the server is the authority and refuses with a
 * typed kind that `describeGitError` turns into the same sentences. Pure
 * over the state so components subscribe to it (`useCicada(gitWriteBlockReason)`).
 */
export function gitWriteBlockReason(
  state: Pick<CicadaState, "role" | "connection" | "git"> = useCicada.getState(),
): string | null {
  if (!canWrite(state)) return writeBlockReason(state) ?? "cannot write";
  if (state.git.busy !== null) return `a ${state.git.busy} is in progress`;
  const status = state.git.status;
  if (status === null) {
    return state.git.error === null ? "reading git status…" : describeGitError(state.git.error);
  }
  switch (status.state.kind) {
    case "not_a_repo":
      return "the project is not in a git repository — `git init` it to commit from the app";
    case "git_not_found":
      return "no `git` on PATH — install git (or put it on PATH) to use the git panel";
    case "locked":
      return "git is busy (index.lock is held) — try again in a moment";
    case "repo":
      if (status.state.operation !== undefined) {
        return `a ${status.state.operation.replace("_", "-")} is in progress in the repository — finish or abort it in your shell first`;
      }
      return null;
  }
}

/** The sentence for a pipeline `.gitignore` matches: git refuses to add it, so nothing here can be committed. */
export function ignoredReason(path: string): string {
  return `\`${path}\` is ignored by a .gitignore rule — un-ignore it to commit from the app`;
}

/**
 * Why the Commit button is disabled (null = it is enabled): the shared
 * write gate, then this pipeline's own refusals, then the empty scope and
 * the blank message — in the order the user can fix them.
 */
export function commitBlockReason(
  state: Pick<CicadaState, "role" | "connection" | "git">,
  message: string,
): string | null {
  const shared = gitWriteBlockReason(state);
  if (shared !== null) return shared;
  const status = state.git.status;
  if (status === null) return "reading git status…";
  if (status.pipeline.ignored) return ignoredReason(status.pipeline.path);
  if (status.scope.length === 0) return "nothing to commit — the scope matches HEAD";
  if (message.trim() === "") return "write a commit message first";
  return null;
}

/**
 * `POST /api/git/commit` with the message verbatim. Toasts the short hash
 * on success (and re-reads the status — a commit sends no delta) or the
 * typed refusal's sentence. Resolves to the commit, or null when refused.
 */
export async function commitFromApp(message: string): Promise<CommitResponse | null> {
  const state = useCicada.getState();
  const blocked = gitWriteBlockReason();
  if (blocked !== null) {
    state.addNotice("warning", `cannot commit: ${blocked}`);
    return null;
  }
  const client = state.hello?.clientId;
  if (client === undefined) {
    state.addNotice("error", "cannot commit: no session identity yet");
    return null;
  }
  state.setGitBusy("commit");
  try {
    const commit = await postGitCommit(session(), client, message);
    const files = commit.files.length === 1 ? "1 file" : `${commit.files.length} files`;
    useCicada.getState().addNotice("info", `committed ${commit.short} — ${commit.summary} (${files})`);
    return commit;
  } catch (error: unknown) {
    useCicada.getState().addNotice("error", `commit refused: ${describeGitFailure(error)}`);
    return null;
  } finally {
    useCicada.getState().setGitBusy(null);
    refreshGitNow();
  }
}

/**
 * `POST /api/git/revert` for `paths` — the files the confirm step LISTED,
 * so what the user agreed to is exactly what goes back (absent = the whole
 * dirty scope, for a caller with no list to show): every uncommitted edit
 * in those files is discarded. The session reloads through the barrier
 * (the store's snapshot handler already says "reloaded from disk (git
 * revert)"); this toasts what went back and what was left alone.
 */
export async function revertToHead(paths?: string[]): Promise<RevertResponse | null> {
  const state = useCicada.getState();
  const blocked = gitWriteBlockReason();
  if (blocked !== null) {
    state.addNotice("warning", `cannot revert: ${blocked}`);
    return null;
  }
  const client = state.hello?.clientId;
  if (client === undefined) {
    state.addNotice("error", "cannot revert: no session identity yet");
    return null;
  }
  state.setGitBusy("revert");
  try {
    const result = await postGitRevert(session(), client, paths);
    const reverted =
      result.reverted.length === 0
        ? "nothing reverted"
        : `reverted to HEAD: ${result.reverted.join(", ")}`;
    const untracked =
      result.untracked.length === 0 ? "" : ` — left alone (no HEAD version): ${result.untracked.join(", ")}`;
    useCicada.getState().addNotice("info", `${reverted}${untracked}`);
    return result;
  } catch (error: unknown) {
    useCicada.getState().addNotice("error", `revert refused: ${describeGitFailure(error)}`);
    return null;
  } finally {
    useCicada.getState().setGitBusy(null);
    refreshGitNow();
  }
}
