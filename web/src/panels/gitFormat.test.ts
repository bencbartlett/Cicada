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
  revertable,
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

const slice = (status: GitStatusResponse | null, error: GitSlice["error"] = null): GitSlice => ({
  status,
  error,
  loading: false,
  busy: null,
  answers: 1,
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
        { path: "p.cic", status: "modified" },
        { path: "p.cic.layout.json", status: "untracked" },
        { path: "scripts/helper.py", status: "modified" },
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

  it("detached HEAD is a warning tone; a refused re-read over a good answer is error tone with the reason", () => {
    expect(gitChip(slice({ ...CLEAN, state: { kind: "repo", ...INFO, branch: null } })).tone).toBe("warn");
    const chip = gitChip(slice(CLEAN, { kind: "git_timeout", message: "git status did not finish", command: "status" }));
    expect(chip.label).toBe("main");
    expect(chip.tone).toBe("error");
    expect(chip.title).toMatch(/last read refused: git status did not finish/);
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

  it("revertable = the scope files with a HEAD version", () => {
    expect(
      revertable([
        { path: "p.cic", status: "modified" },
        { path: "p.cic.layout.json", status: "untracked" },
        { path: "scripts/new.py", status: "added" },
        { path: "scripts/old.py", status: "deleted" },
        { path: "scripts/moved.py", status: "renamed" },
      ]).map((f) => f.path),
    ).toEqual(["p.cic", "scripts/old.py", "scripts/moved.py"]);
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
