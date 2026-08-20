/**
 * The git routes' client side: the request shapes on the wire (docs/13
 * §HTTP surface `/api/git/*`), the typed refusal every failure becomes,
 * and the sentences the toasts use per `kind`.
 */
import { describe, expect, it, vi } from "vitest";
import {
  describeGitError,
  describeGitFailure,
  fetchGitStatus,
  GitRouteError,
  postGitCommit,
  postGitRevert,
  refusalOf,
} from "./git";
import type { GitStatusResponse } from "./messages";

const SESSION = { token: "t0k", pipeline: "examples/wall/p.cic" };

const STATUS: GitStatusResponse = {
  state: {
    kind: "repo",
    root: "C:/repo",
    prefix: "examples/wall",
    branch: "main",
    head_short: "abc1234",
    upstream: null,
    unborn: false,
  },
  pipeline: { path: "p.cic", tracked: true, ignored: false, dirty: true, nodes: [{ name: "size", change: "modified" }], removed: [] },
  scope: [{ path: "p.cic", status: "modified" }],
  text_hash: "ff".repeat(32),
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("fetchGitStatus", () => {
  it("GETs /api/git/status with the pipeline and the token header", async () => {
    const fetchImpl = vi.fn(() => Promise.resolve(json(200, STATUS)));
    const status = await fetchGitStatus(SESSION, fetchImpl);
    expect(status).toEqual(STATUS);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/git/status?pipeline=examples%2Fwall%2Fp.cic");
    expect(init.method).toBeUndefined();
    expect((init.headers as Record<string, string>)["X-Cicada-Token"]).toBe("t0k");
  });

  it("turns a JSON refusal into a typed GitRouteError with the body's facts", async () => {
    const fetchImpl = vi.fn(() =>
      Promise.resolve(json(404, { kind: "no_such_pipeline", message: "no pipeline `x.cic`", path: "x.cic" })),
    );
    const error = await fetchGitStatus(SESSION, fetchImpl).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(GitRouteError);
    const typed = error as GitRouteError;
    expect(typed.status).toBe(404);
    expect(typed.kind).toBe("no_such_pipeline");
    expect(typed.body.path).toBe("x.cic");
    expect(typed.message).toBe("no pipeline `x.cic`");
  });

  it("a text failure (the 401 middleware) is a `transport` refusal carrying the text", async () => {
    const fetchImpl = vi.fn(() => Promise.resolve(new Response("bad token", { status: 401 })));
    const error = (await fetchGitStatus(SESSION, fetchImpl).catch((e: unknown) => e)) as GitRouteError;
    expect(error.kind).toBe("transport");
    expect(error.status).toBe(401);
    expect(error.message).toBe("HTTP 401 — bad token");
  });

  it("a network failure is a `transport` refusal with status 0", async () => {
    const fetchImpl = vi.fn(() => Promise.reject(new TypeError("Failed to fetch")));
    const error = (await fetchGitStatus(SESSION, fetchImpl).catch((e: unknown) => e)) as GitRouteError;
    expect(error.kind).toBe("transport");
    expect(error.status).toBe(0);
    expect(error.message).toMatch(/Failed to fetch/);
  });

  it("refusalOf: a JSON body without kind/message is not mistaken for a refusal body", async () => {
    const error = await refusalOf(new Response(JSON.stringify({ error: "nope" }), { status: 500 }));
    expect(error.kind).toBe("transport");
    expect(error.message).toBe('HTTP 500 — {"error":"nope"}');
  });
});

describe("postGitCommit / postGitRevert", () => {
  it("POSTs {message, client} verbatim — a trailing newline and unicode survive", async () => {
    const answer = { hash: "a".repeat(40), short: "aaaaaaa", summary: "wall: thicker", files: ["p.cic"] };
    const fetchImpl = vi.fn(() => Promise.resolve(json(200, answer)));
    const message = "wall: thicker — 12 → 14\n\nbody ünïcode\n";
    const commit = await postGitCommit(SESSION, 7, message, fetchImpl);
    expect(commit).toEqual(answer);
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/git/commit?pipeline=examples%2Fwall%2Fp.cic");
    expect(init.method).toBe("POST");
    const headers = init.headers as Record<string, string>;
    expect(headers["X-Cicada-Token"]).toBe("t0k");
    expect(headers["Content-Type"]).toBe("application/json");
    expect(JSON.parse(init.body as string)).toEqual({ message, client: 7 });
  });

  it("revert POSTs {client} for the whole scope and {paths, client} for a subset", async () => {
    const answer = { reverted: ["p.cic"], untracked: [], reloaded: true };
    const fetchImpl = vi.fn(() => Promise.resolve(json(200, answer)));
    await postGitRevert(SESSION, 3, undefined, fetchImpl);
    await postGitRevert(SESSION, 3, ["p.cic"], fetchImpl);
    const bodies = fetchImpl.mock.calls.map((call) => JSON.parse((call as unknown as [string, RequestInit])[1].body as string) as unknown);
    expect(bodies).toEqual([{ client: 3 }, { paths: ["p.cic"], client: 3 }]);
    expect((fetchImpl.mock.calls[0] as unknown as [string])[0]).toBe("/api/git/revert?pipeline=examples%2Fwall%2Fp.cic");
  });

  it("a 403 lease refusal keeps its kind (and the `path` when nobody has the pipeline open)", async () => {
    const fetchImpl = vi.fn(() =>
      Promise.resolve(json(403, { kind: "lease", message: "nobody holds it: no client has `p.cic` open", path: "p.cic" })),
    );
    const error = (await postGitCommit(SESSION, 1, "m", fetchImpl).catch((e: unknown) => e)) as GitRouteError;
    expect(error.status).toBe(403);
    expect(error.kind).toBe("lease");
    expect(describeGitError(error.body)).toBe("nobody has `p.cic` open — open it in the app first");
  });
});

describe("describeGitError — one readable sentence per kind", () => {
  const cases: [Parameters<typeof describeGitError>[0], RegExp][] = [
    [{ kind: "lease", message: "x" }, /do not hold the write lease/],
    [{ kind: "nothing_to_commit", message: "x" }, /^nothing to commit/],
    [{ kind: "nothing_to_revert", message: "x" }, /^nothing to revert/],
    [{ kind: "locked", message: "x" }, /index\.lock/],
    [{ kind: "not_a_repo", message: "x" }, /not in a git repository/],
    [{ kind: "git_not_found", message: "x" }, /no `git` on PATH/],
    [{ kind: "untracked", message: "x", path: "p.cic" }, /`p\.cic` has no HEAD version/],
    [{ kind: "ignored", message: "x", path: "p.cic" }, /`p\.cic` is ignored by a \.gitignore rule/],
    [{ kind: "operation_in_progress", message: "x", operation: "cherry_pick" }, /a cherry-pick is in progress/],
    [{ kind: "empty_message", message: "x" }, /write a commit message/],
    [{ kind: "no_such_pipeline", message: "x", path: "gone.cic" }, /`gone\.cic`/],
    [{ kind: "git_failed", message: "x", command: "commit", code: 1, stderr: "hook said no" }, /^git commit failed \(exit 1\): hook said no$/],
    [{ kind: "git_failed", message: "x", command: "add", code: null, stderr: "" }, /^git add failed \(killed\)$/],
    [{ kind: "git_timeout", message: "x", command: "status" }, /^git status did not finish in time/],
    // Server-side failures and unknown kinds keep the server's sentence.
    [{ kind: "reload_failed", message: "the files are back but the session could not load them" }, /^the files are back/],
    [{ kind: "internal", message: "the git task did not complete" }, /^the git task did not complete$/],
    [{ kind: "something_new", message: "server says so" }, /^server says so$/],
    [{ kind: "transport", message: "HTTP 401 — bad token" }, /^HTTP 401/],
  ];
  for (const [body, expected] of cases) {
    it(`${body.kind}`, () => {
      expect(describeGitError(body)).toMatch(expected);
    });
  }

  it("describeGitFailure handles typed and untyped throws", () => {
    expect(describeGitFailure(new GitRouteError(409, { kind: "locked", message: "x" }))).toMatch(/index\.lock/);
    expect(describeGitFailure(new Error("boom"))).toBe("Error: boom");
  });
});
