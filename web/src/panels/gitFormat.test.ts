import { describe, expect, it } from "vitest";
import type { GitStatusResponse, RepoInfo } from "../protocol/messages";
import type { GitSlice } from "../state/store";
import {
  describeHead,
  dirtyCount,
  gitChip,
  groupMarkers,
  markerBadge,
  markerCount,
  projectGitLine,
  revertRequest,
  revertable,
  scopeNote,
} from "./gitFormat";

const INFO: RepoInfo = {
  root: "C:/repo",
  prefix: "examples/wall",
  branch: "main",
  head_short: "abc1234",
  upstream: null,
  unborn: false,
};

const CLEAN: GitStatusResponse = {
  state: { kind: "repo", ...INFO },
  pipeline: { path: "p.cic", tracked: true, ignored: false, dirty: false, nodes: [], removed: [] },
  scope: [],
  text_hash: "00".repeat(32),
};

const slice = (status: GitStatusResponse | null, error: GitSlice["error"] = null, stale = false): GitSlice => ({
  status,
  error,
  loading: false,
  busy: null,
  answers: 1,
  stale,
  writes: stale ? 1 : 0,
});

describe("describeHead", () => {
  it("branch · detached @short · detached · no repo · git not found", () => {
    expect(describeHead({ kind: "repo", ...INFO })).toBe("main");
    expect(describeHead({ kind: "locked", ...INFO })).toBe("main");
    expect(describeHead({ kind: "repo", ...INFO, branch: null })).toBe("detached @abc1234");
    expect(describeHead({ kind: "repo", ...INFO, branch: null, head_short: null })).toBe("detached");
    expect(describeHead({ kind: "not_a_repo" })).toBe("no repo");
    expect(describeHead({ kind: "git_not_found" })).toBe("git not found");
  });
});

describe("gitChip", () => {
  it("before the first answer: reading; after a refused first read: the kind, error tone", () => {
    expect(gitChip(slice(null))).toMatchObject({ label: "git…", dirty: null, tone: "faint" });
    const refused = gitChip(slice(null, { kind: "git_failed", message: "git status failed: boom" }));
    expect(refused).toMatchObject({ label: "git: git_failed", dirty: null, tone: "error" });
    expect(refused.title).toMatch(/boom/);
  });

  it("no repo / no git: the label, no dirty count, a tooltip that says what to do", () => {
    const none = gitChip(slice({ ...CLEAN, state: { kind: "not_a_repo" } }));
    expect(none).toMatchObject({ label: "no repo", dirty: null, tone: "faint" });
    expect(none.title).toMatch(/git init/);
    const missing = gitChip(slice({ ...CLEAN, state: { kind: "git_not_found" } }));
    expect(missing).toMatchObject({ label: "git not found", dirty: null });
    expect(missing.title).toMatch(/install git/);
  });

  it("a clean repo on a branch: branch, 0 dirty, plain tone", () => {
    const chip = gitChip(slice(CLEAN));
    expect(chip).toEqual({
      label: "main",
      dirty: 0,
      notes: [],
      title: "branch main · HEAD abc1234 · no upstream · nothing to commit in this pipeline's scope · click for the Git tab",
      tone: "",
      stale: false,
    });
  });

  it("dirty count = the scope's files; upstream ahead/behind, unborn, operation and locked are notes", () => {
    const dirty: GitStatusResponse = {
      ...CLEAN,
      state: {
        kind: "locked",
        ...INFO,
        upstream: { name: "origin/main", ahead: 2, behind: 1 },
        operation: "cherry_pick",
      },
      scope: [
        { path: "p.cic", status: "modified", in_head: true },
        { path: "p.cic.layout.json", status: "untracked", in_head: false },
        { path: "scripts/helper.py", status: "modified", in_head: true },
      ],
    };
    const chip = gitChip(slice(dirty));
    expect(chip.label).toBe("main");
    expect(chip.dirty).toBe(3);
    expect(dirtyCount(dirty)).toBe(3);
    expect(chip.notes).toEqual(["↑2 ↓1", "cherry-pick in progress", "locked"]);
    expect(chip.tone).toBe("warn");
    expect(chip.title).toMatch(/3 dirty files/);
    expect(chip.title).toMatch(/upstream origin\/main \(ahead 2, behind 1\)/);
    expect(chip.title).toMatch(/index\.lock is held/);
    expect(chip.title).toMatch(/cherry-pick is in progress/);

    const unborn = gitChip(slice({ ...CLEAN, state: { kind: "repo", ...INFO, head_short: null, unborn: true } }));
    expect(unborn.notes).toEqual(["no commits yet"]);
    expect(unborn.title).toMatch(/no commits yet/);

    const upToDate = gitChip(slice({ ...CLEAN, state: { kind: "repo", ...INFO, upstream: { name: "origin/main", ahead: 0, behind: 0 } } }));
    expect(upToDate.notes).toEqual([]);
  });

  it("an IGNORED pipeline is never `clean`: no dirty count, an `ignored` note in warn tone, the tooltip says why", () => {
    // git lists nothing for an ignored file and refuses to add it: the scope
    // is empty BY CONSTRUCTION while every node wears `+` — `0` here was the
    // silent wrong answer.
    const ignored: GitStatusResponse = {
      ...CLEAN,
      pipeline: { path: "ign.cic", tracked: false, ignored: true, dirty: true, nodes: [{ name: "n", change: "added" }], removed: [] },
      scope: [],
    };
    expect(dirtyCount(ignored)).toBeNull();
    const chip = gitChip(slice(ignored));
    expect(chip).toMatchObject({ label: "main", dirty: null, notes: ["ignored"], tone: "warn" });
    expect(chip.title).toMatch(/`ign\.cic` is ignored by a \.gitignore rule/);
    expect(chip.title).not.toMatch(/nothing to commit/);
  });

  it("a STALE answer keeps its count (the last known one) but says so: `stale` for the chip to dim it, the tooltip names the re-read", () => {
    const current = gitChip(slice(CLEAN));
    expect(current.stale).toBe(false);
    expect(current.title).not.toMatch(/re-reading/);
    const stale = gitChip(slice(CLEAN, null, true));
    expect(stale).toMatchObject({ label: "main", dirty: 0, tone: "", stale: true });
    expect(stale.title).toMatch(/an edit landed since — re-reading the status now/);
    // Before any answer there is nothing to be stale about.
    expect(gitChip(slice(null, null, true)).stale).toBe(false);
    expect(gitChip(slice({ ...CLEAN, state: { kind: "not_a_repo" } }, null, true)).stale).toBe(false);
  });

  it("detached HEAD is a warning tone; a refused re-read over a good answer is error tone with the reason", () => {
    expect(gitChip(slice({ ...CLEAN, state: { kind: "repo", ...INFO, branch: null } })).tone).toBe("warn");
    const chip = gitChip(slice(CLEAN, { kind: "git_timeout", message: "git status did not finish", command: "git status --porcelain=v2 --no-optional-locks" }));
    expect(chip.label).toBe("main");
    expect(chip.tone).toBe("error");
    expect(chip.title).toMatch(/last read refused: git status did not finish/);
  });
});

describe("scopeNote — the one rule under 'files to commit' (the Git tab and the Ctrl+S dialog)", () => {
  const dirty: GitStatusResponse = {
    ...CLEAN,
    pipeline: { ...CLEAN.pipeline, dirty: true, nodes: [{ name: "size", change: "modified" }] },
    scope: [{ path: "p.cic", status: "modified", in_head: true }],
  };
  const ignored: GitStatusResponse = {
    ...CLEAN,
    pipeline: { path: "ign.cic", tracked: false, ignored: true, dirty: true, nodes: [{ name: "n", change: "added" }], removed: [] },
    scope: [],
  };

  it("a current empty scope is clean; a current dirty scope is the list", () => {
    expect(scopeNote(CLEAN, false)).toEqual({ kind: "clean" });
    expect(scopeNote(dirty, false)).toEqual({ kind: "files", files: dirty.scope, refreshing: false });
  });

  it("an IGNORED pipeline never reads 'nothing to commit — the scope matches HEAD' (every node wears `+`; git refuses the file): the ignored note, plus whatever else of the scope is dirty", () => {
    expect(scopeNote(ignored, false)).toEqual({ kind: "ignored", path: "ign.cic", files: [] });
    expect(scopeNote(ignored, true), "stale or not").toEqual({ kind: "ignored", path: "ign.cic", files: [] });
    const sidecar = { path: "ign.cic.layout.json", status: "untracked" as const, in_head: false };
    expect(scopeNote({ ...ignored, scope: [sidecar] }, false)).toEqual({ kind: "ignored", path: "ign.cic", files: [sidecar] });
    for (const note of [scopeNote(ignored, false), scopeNote(ignored, true)]) {
      expect(note.kind).not.toBe("clean");
    }
  });

  it("a STALE empty scope reads 'refreshing', never 'clean' (the previous tree's verdict over an edit already on disk); a stale list carries the hint", () => {
    expect(scopeNote(CLEAN, true)).toEqual({ kind: "refreshing" });
    expect(scopeNote(dirty, true)).toEqual({ kind: "files", files: dirty.scope, refreshing: true });
  });
});

describe("groupMarkers / markerCount / revertable / markerBadge", () => {
  it("groups by kind and lists HEAD's removals", () => {
    const pipeline = {
      path: "p.cic",
      tracked: true,
      ignored: false,
      dirty: true,
      nodes: [
        { name: "a", change: "added" as const },
        { name: "m", change: "modified" as const },
        { name: "r2", change: "renamed" as const, from: "r" },
        { name: "m2", change: "modified" as const },
      ],
      removed: [{ name: "gone", line_in_head: 7 }],
    };
    const groups = groupMarkers(pipeline);
    expect(groups.added.map((n) => n.name)).toEqual(["a"]);
    expect(groups.modified.map((n) => n.name)).toEqual(["m", "m2"]);
    expect(groups.renamed).toEqual([{ name: "r2", change: "renamed", from: "r" }]);
    expect(groups.removed).toEqual([{ name: "gone", line_in_head: 7 }]);
    expect(markerCount(pipeline)).toBe(5);
  });

  it("revertable = the scope files the server says HEAD has (`in_head`) — never inferred from `status`", () => {
    // The list is POSTed as `paths`, and the server refuses an explicit ask
    // for a file it would have to delete (`409 untracked`) — so a file
    // without a HEAD version here would fail the whole revert. `status`
    // cannot tell: porcelain `AD` (added to the index, then deleted from
    // disk) is `deleted` with nothing in HEAD, while a deleted tracked
    // file is `deleted` WITH one — only `in_head` separates them (found in
    // review: the status-based rule listed the `AD` file and the revert
    // was refused).
    const scope = [
      { path: "p.cic", status: "modified" as const, in_head: true },
      { path: "p.cic.layout.json", status: "deleted" as const, in_head: true },
      { path: "scripts/moved.py", status: "renamed" as const, in_head: false },
      { path: "scripts/new.py", status: "untracked" as const, in_head: false },
      { path: "scripts/probe_ad.py", status: "deleted" as const, in_head: false },
      { path: "scripts/staged.py", status: "added" as const, in_head: false },
      // Not a shape git produces today (an unmerged `AA` is the nearest);
      // the rule is the field, so it still holds.
      { path: "scripts/odd.py", status: "modified" as const, in_head: false },
    ];
    expect(revertable(scope).map((f) => f.path)).toEqual(["p.cic", "p.cic.layout.json"]);
  });

  it("revertRequest: the confirm list, the left-alone list and the POST's `paths` come from one object", () => {
    const pipeline = { path: "p.cic", status: "modified" as const, in_head: true };
    const sidecar = { path: "p.cic.layout.json", status: "untracked" as const, in_head: false };
    const probe = { path: "scripts/probe_ad.py", status: "deleted" as const, in_head: false };
    const scope = [pipeline, sidecar, probe];
    const request = revertRequest(scope);
    expect(request.files).toEqual([pipeline]);
    expect(request.untouched).toEqual([sidecar, probe]);
    expect(request.paths).toEqual(["p.cic"]);
    // Every scope file is in exactly one of the two lists.
    expect([...request.files, ...request.untouched].length).toBe(scope.length);
    // Nothing to restore → an empty ask (the control disables itself on it).
    expect(revertRequest([sidecar])).toEqual({ files: [], untouched: [sidecar], paths: [] });
    expect(revertRequest([])).toEqual({ files: [], untouched: [], paths: [] });
  });

  it("badges: glyph + tooltip per kind, the rename naming the HEAD name", () => {
    expect(markerBadge({ name: "a", change: "added" })).toEqual({ glyph: "+", title: "added since HEAD (not in the last commit)", kind: "added" });
    expect(markerBadge({ name: "m", change: "modified" })).toEqual({ glyph: "~", title: "modified since HEAD", kind: "modified" });
    expect(markerBadge({ name: "r2", change: "renamed", from: "r" })).toEqual({
      glyph: "→",
      title: "renamed since HEAD (was `r`)",
      kind: "renamed",
    });
  });
});

describe("projectGitLine (the landing page)", () => {
  it("branch + project-wide dirty count; locked, detached, no repo, no git, error", () => {
    expect(projectGitLine(undefined)).toBeNull();
    expect(projectGitLine({ kind: "repo", branch: "main", dirty_count: 0 })).toBe("git: main · clean");
    expect(projectGitLine({ kind: "repo", branch: "main", dirty_count: 3 })).toBe("git: main · 3 dirty");
    expect(projectGitLine({ kind: "locked", branch: "main", dirty_count: 1 })).toBe("git: main · 1 dirty · index.lock held");
    expect(projectGitLine({ kind: "repo", branch: null, dirty_count: 0 })).toBe("git: detached HEAD · clean");
    expect(projectGitLine({ kind: "not_a_repo", branch: null, dirty_count: 0 })).toBe("git: not a repository");
    expect(projectGitLine({ kind: "git_not_found", branch: null, dirty_count: 0 })).toBe("git: no `git` on PATH");
    expect(projectGitLine({ kind: "error", branch: null, dirty_count: 0, error: "git status failed" })).toBe("git: git status failed");
  });
});
