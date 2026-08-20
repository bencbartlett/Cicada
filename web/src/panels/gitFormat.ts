/**
 * What the git chip and the Git tab SAY about a status answer (docs/16
 * §Application layout: "branch/git status"; doc 10 §Git integration's
 * status strip). Pure functions over the mirror shapes — the chip's label
 * and tooltip, the marker groups, the revertable subset of the scope — so
 * the wording is tested without a DOM.
 */
import type {
  ChangeKind,
  GitState,
  GitStatusResponse,
  NodeChange,
  PipelineGitStatus,
  ProjectGit,
  RemovedNode,
  ScopeFile,
} from "../protocol/messages";
import { gitRepoInfo } from "../protocol/messages";
import type { GitSlice } from "../state/store";

/** The chip's tone: plain · `warn` (locked, an operation, detached) · `error` (a refused read) · `faint` (no repo). */
export type ChipTone = "" | "warn" | "error" | "faint";

export interface GitChip {
  /** The main label: `main`, `detached @abc1234`, `no repo`, `git not found`, `git…`. */
  label: string;
  /** Dirty files in this pipeline's commit scope; null when there is no repository to count against. */
  dirty: number | null;
  /** Extra facts after the label: `↑1 ↓2`, `locked`, `merge in progress`, `no commits yet`. */
  notes: string[];
  title: string;
  tone: ChipTone;
  /**
   * The count is the PREVIOUS tree's: an edit landed after the last read
   * and the re-read is on its way (`GitSlice.stale`). The chip dims the
   * count rather than hiding it — the last known answer with an honest
   * caveat beats a blank — and the tooltip says so.
   */
  stale: boolean;
}

/** The branch (or where HEAD is) for a repo-ish state. */
export function describeHead(state: GitState): string {
  const info = gitRepoInfo(state);
  if (info === null) return state.kind === "not_a_repo" ? "no repo" : "git not found";
  if (info.branch !== null) return info.branch;
  return info.head_short !== null ? `detached @${info.head_short}` : "detached";
}

/**
 * The dirty files of the pipeline's commit scope (what `commit` would
 * stage) — or null for an IGNORED pipeline: git lists nothing for it and
 * `git add` refuses it, so the scope is empty by construction and `0` /
 * `clean` would be the silent wrong answer (every node wears `+`).
 */
export function dirtyCount(status: GitStatusResponse): number | null {
  return status.pipeline.ignored ? null : status.scope.length;
}

/** The chip for the current slice (loading · refused · no repo · repo facts). */
export function gitChip(slice: GitSlice): GitChip {
  const { status, error, stale } = slice;
  if (status === null) {
    if (error !== null) {
      return {
        label: `git: ${error.kind}`,
        dirty: null,
        notes: [],
        title: `git status could not be read — ${error.message}`,
        tone: "error",
        stale: false,
      };
    }
    return { label: "git…", dirty: null, notes: [], title: "reading git status…", tone: "faint", stale: false };
  }
  const info = gitRepoInfo(status.state);
  if (info === null) {
    const title =
      status.state.kind === "not_a_repo"
        ? "the project is not in a git repository — `git init` it to commit from the app"
        : "no `git` on PATH — install git (or put it on PATH) to use the git panel";
    return { label: describeHead(status.state), dirty: null, notes: [], title, tone: "faint", stale: false };
  }
  const notes: string[] = [];
  let tone: ChipTone = info.branch === null ? "warn" : "";
  if (info.unborn) notes.push("no commits yet");
  if (info.upstream !== null) {
    const parts: string[] = [];
    if (info.upstream.ahead > 0) parts.push(`↑${info.upstream.ahead}`);
    if (info.upstream.behind > 0) parts.push(`↓${info.upstream.behind}`);
    if (parts.length > 0) notes.push(parts.join(" "));
  }
  if (info.operation !== undefined) {
    notes.push(`${info.operation.replace("_", "-")} in progress`);
    tone = "warn";
  }
  if (status.state.kind === "locked") {
    notes.push("locked");
    tone = "warn";
  }
  if (status.pipeline.ignored) {
    notes.push("ignored");
    tone = "warn";
  }
  const dirty = dirtyCount(status);
  const titleParts = [
    info.branch !== null ? `branch ${info.branch}` : "detached HEAD",
    info.head_short !== null ? `HEAD ${info.head_short}` : "no commits yet",
    info.upstream !== null
      ? `upstream ${info.upstream.name} (ahead ${info.upstream.ahead}, behind ${info.upstream.behind})`
      : "no upstream",
    dirty === null
      ? `\`${status.pipeline.path}\` is ignored by a .gitignore rule — git will not track it; nothing can be committed from the app`
      : dirty === 0
        ? "nothing to commit in this pipeline's scope"
        : `${dirty} dirty ${dirty === 1 ? "file" : "files"} in this pipeline's scope`,
    info.operation !== undefined ? `a ${info.operation.replace("_", "-")} is in progress — finish it in your shell` : null,
    status.state.kind === "locked" ? "index.lock is held — commit and revert wait" : null,
    error !== null ? `last read refused: ${error.message}` : null,
    stale ? "an edit landed since — re-reading the status now" : null,
    "click for the Git tab",
  ].filter((s): s is string => s !== null);
  if (error !== null) tone = "error";
  return { label: describeHead(status.state), dirty, notes, title: titleParts.join(" · "), tone, stale };
}

/**
 * What the "files to commit" section says under its heading — ONE rule for
 * the Git tab and the Ctrl+S dialog, so they cannot disagree:
 *
 * - `ignored`: the pipeline matches a `.gitignore` rule — git lists
 *   nothing for it and refuses to add it, so an empty scope is git's
 *   refusal, NEVER a clean tree ("nothing to commit — the scope matches
 *   HEAD" beside the ignored warning were two contradictory statements on
 *   one screen; every node wears `+`). `files` still lists whatever else
 *   of the scope is dirty (a sidecar the rule does not match) — shown,
 *   not committable;
 * - `refreshing`: the scope is empty but the cache is stale (an edit
 *   landed after the last read) — "clean" would be the previous tree's
 *   verdict over an edit already on disk;
 * - `clean`: an empty scope from a current read;
 * - `files`: the dirty files, with `refreshing` set when the list may be
 *   short of the latest edit.
 */
export type ScopeNote =
  | { kind: "ignored"; path: string; files: ScopeFile[] }
  | { kind: "refreshing" }
  | { kind: "clean" }
  | { kind: "files"; files: ScopeFile[]; refreshing: boolean };

export function scopeNote(status: GitStatusResponse, stale: boolean): ScopeNote {
  if (status.pipeline.ignored) return { kind: "ignored", path: status.pipeline.path, files: status.scope };
  if (status.scope.length > 0) return { kind: "files", files: status.scope, refreshing: stale };
  return stale ? { kind: "refreshing" } : { kind: "clean" };
}

/** The markers grouped the way the tab lists them; `removed` comes from HEAD. */
export interface MarkerGroups {
  added: NodeChange[];
  modified: NodeChange[];
  renamed: NodeChange[];
  removed: RemovedNode[];
}

export function groupMarkers(pipeline: PipelineGitStatus): MarkerGroups {
  const groups: MarkerGroups = { added: [], modified: [], renamed: [], removed: [...pipeline.removed] };
  for (const change of pipeline.nodes) {
    switch (change.change) {
      case "added":
        groups.added.push(change);
        break;
      case "modified":
        groups.modified.push(change);
        break;
      case "renamed":
        groups.renamed.push(change);
        break;
      case "removed":
        // Never on a working-tree node (the server lists HEAD's removals
        // separately); tolerated so an unexpected marker is still shown.
        groups.removed.push({ name: change.name, line_in_head: 0 });
        break;
    }
  }
  return groups;
}

/** Total markers (the tab's count badge). */
export function markerCount(pipeline: PipelineGitStatus): number {
  return pipeline.nodes.length + pipeline.removed.length;
}

/**
 * The scope files `revert` can put back: those the server says HEAD has a
 * version of (`in_head` — ITS rule, read off the status, never re-derived
 * from `status` here: a `deleted` file can be an index addition that went
 * missing from disk, porcelain `AD`, with nothing in HEAD to go back to).
 * The rest — untracked, index-only, a rename's new path — the server
 * leaves alone (it never deletes) and refuses as an explicit ask (`409
 * untracked`), so listing one would fail the whole revert.
 */
export function revertable(scope: ScopeFile[]): ScopeFile[] {
  return scope.filter((f) => f.in_head);
}

/** What the confirm step shows and what the request then names — one object, so they cannot drift. */
export interface RevertRequestView {
  /** Listed in the confirm step; restored by the request. */
  files: ScopeFile[];
  /** Named in the confirm step as left alone (no HEAD version). */
  untouched: ScopeFile[];
  /** The request's `paths`: exactly `files`, in scope order. */
  paths: string[];
}

/**
 * The BINDING confirm list: the revert the user agrees to is exactly the
 * files shown, and the POST names exactly those `paths` — never the whole
 * scope — so a file that joined the scope after the last status read is
 * not reverted unseen (the next read lists it for a second ask).
 */
export function revertRequest(scope: ScopeFile[]): RevertRequestView {
  const files = revertable(scope);
  return {
    files,
    untouched: scope.filter((f) => !f.in_head),
    paths: files.map((f) => f.path),
  };
}

/** The canvas badge for a marker: glyph + tooltip (docs/16 canvas badges; removed nodes are never on the canvas). */
export function markerBadge(change: NodeChange): { glyph: string; title: string; kind: ChangeKind } {
  switch (change.change) {
    case "added":
      return { glyph: "+", title: "added since HEAD (not in the last commit)", kind: "added" };
    case "modified":
      return { glyph: "~", title: "modified since HEAD", kind: "modified" };
    case "renamed":
      return {
        glyph: "→",
        title: `renamed since HEAD${change.from !== undefined ? ` (was \`${change.from}\`)` : ""}`,
        kind: "renamed",
      };
    case "removed":
      return { glyph: "−", title: "removed since HEAD", kind: "removed" };
  }
}

/**
 * The landing page's line for `GET /api/project`'s git summary: the branch
 * and the project-wide dirty count (every `git status` entry under the
 * project dir — not one pipeline's scope), or why there is none.
 */
export function projectGitLine(git: ProjectGit | undefined): string | null {
  if (git === undefined) return null;
  switch (git.kind) {
    case "repo":
    case "locked": {
      const where = git.branch ?? "detached HEAD";
      const dirty = git.dirty_count === 0 ? "clean" : `${git.dirty_count} dirty`;
      return `git: ${where} · ${dirty}${git.kind === "locked" ? " · index.lock held" : ""}`;
    }
    case "not_a_repo":
      return "git: not a repository";
    case "git_not_found":
      return "git: no `git` on PATH";
    default:
      return `git: ${git.error ?? git.kind}`;
  }
}
