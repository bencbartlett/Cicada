/**
 * The git routes over HTTP (docs/13 §HTTP surface `/api/git/*`): typed
 * requests, typed answers, typed refusals. Every failure the server sends
 * is a `{kind, message, …}` body (`GitErrorBody`) — except the token
 * middleware's text 401 — and every failure this module throws is a
 * `GitRouteError` carrying that body (a non-JSON failure becomes a
 * `transport` kind with the text), so callers branch on `kind`, never on
 * prose. `fetchImpl` is injectable for tests; the app passes nothing.
 */
import type {
  CommitRequest,
  CommitResponse,
  GitErrorBody,
  GitErrorKind,
  GitStatusResponse,
  RevertRequest,
  RevertResponse,
} from "./messages";

/** What every git call needs: the session token, the pipeline, and (writes) the writer's client id. */
export interface GitSession {
  token: string;
  pipeline: string;
}

/** A refused (or unreachable) git route: the typed body plus the HTTP status (0 = no response). */
export class GitRouteError extends Error {
  readonly status: number;
  readonly body: GitErrorBody;
  constructor(status: number, body: GitErrorBody) {
    super(body.message);
    this.name = "GitRouteError";
    this.status = status;
    this.body = body;
  }
  get kind(): GitErrorKind {
    return this.body.kind;
  }
}

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

function statusUrl(route: string, session: GitSession): string {
  return `${route}?pipeline=${encodeURIComponent(session.pipeline)}`;
}

/**
 * Read the body of a non-OK response as the typed refusal it is: JSON
 * `{kind, message, …}` when the server answered (every git route does), else
 * a `transport` refusal with the text (the 401 middleware, a proxy page).
 */
export async function refusalOf(response: Response): Promise<GitRouteError> {
  const text = await response.text();
  try {
    const parsed = JSON.parse(text) as Partial<GitErrorBody>;
    if (typeof parsed.kind === "string" && typeof parsed.message === "string") {
      return new GitRouteError(response.status, parsed as GitErrorBody);
    }
  } catch {
    // not JSON — fall through
  }
  return new GitRouteError(response.status, {
    kind: "transport",
    message: `HTTP ${response.status}${text.trim() ? ` — ${text.trim()}` : ""}`,
  });
}

async function call<T>(
  fetchImpl: FetchLike,
  url: string,
  init: RequestInit,
): Promise<T> {
  let response: Response;
  try {
    response = await fetchImpl(url, init);
  } catch (error: unknown) {
    throw new GitRouteError(0, { kind: "transport", message: `unreachable: ${String(error)}` });
  }
  if (!response.ok) throw await refusalOf(response);
  return (await response.json()) as T;
}

/** `GET /api/git/status?pipeline=` — a read: no session is opened, nothing is written. */
export function fetchGitStatus(
  session: GitSession,
  fetchImpl: FetchLike = fetch,
): Promise<GitStatusResponse> {
  return call<GitStatusResponse>(fetchImpl, statusUrl("/api/git/status", session), {
    headers: { "X-Cicada-Token": session.token },
  });
}

/**
 * `POST /api/git/commit` — writer-gated: `client` must hold the lease of the
 * pipeline's OPEN session. The message goes verbatim (the server commits
 * with `--cleanup=verbatim`).
 */
export function postGitCommit(
  session: GitSession,
  client: number,
  message: string,
  fetchImpl: FetchLike = fetch,
): Promise<CommitResponse> {
  const body: CommitRequest = { message, client };
  return call<CommitResponse>(fetchImpl, statusUrl("/api/git/commit", session), {
    method: "POST",
    headers: { "X-Cicada-Token": session.token, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

/**
 * `POST /api/git/revert` — writer-gated; `paths` narrows the scope (absent =
 * everything dirty that has a HEAD version). The session reloads under its
 * write hold: ONE barrier snapshot (`reason: "git revert"`) reaches the
 * socket before this resolves or right after — the caller treats the
 * snapshot, not this answer, as the new state.
 */
export function postGitRevert(
  session: GitSession,
  client: number,
  paths: string[] | undefined,
  fetchImpl: FetchLike = fetch,
): Promise<RevertResponse> {
  const body: RevertRequest = paths === undefined ? { client } : { paths, client };
  return call<RevertResponse>(fetchImpl, statusUrl("/api/git/revert", session), {
    method: "POST",
    headers: { "X-Cicada-Token": session.token, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

/**
 * The sentence a refusal deserves in a toast — one per kind the panel can
 * meet, in the user's terms (the server's messages are written for any
 * HTTP caller and talk about client ids and pathspecs). Unknown kinds and
 * the server-side failures keep the server's message, which already says
 * what happened; `git_failed` adds the command and its stderr.
 */
export function describeGitError(error: GitErrorBody): string {
  switch (error.kind) {
    case "lease":
      return error.path !== undefined
        ? `nobody has \`${error.path}\` open — open it in the app first`
        : "you do not hold the write lease — take it to commit or revert";
    case "nothing_to_commit":
      return "nothing to commit — this pipeline, its sidecar and its scripts match HEAD";
    case "nothing_to_revert":
      return "nothing to revert — this pipeline's files already match HEAD";
    case "locked":
      return "git is busy (index.lock is held) — try again in a moment";
    case "not_a_repo":
      return "the project is not in a git repository — `git init` it to commit from the app";
    case "git_not_found":
      return "no `git` on PATH — install git (or put it on PATH) to use the git panel";
    case "untracked":
      return `\`${error.path ?? "the pipeline"}\` has no HEAD version — nothing to revert to (commit it first)`;
    case "ignored":
      return `\`${error.path ?? "the pipeline"}\` is ignored by a .gitignore rule — git refuses to add it; un-ignore it to commit from the app`;
    case "operation_in_progress":
      return `a ${(error.operation ?? "git operation").replace("_", "-")} is in progress in the repository — finish or abort it in your shell first`;
    case "empty_message":
      return "write a commit message first";
    case "no_such_pipeline":
      return `no pipeline \`${error.path ?? "?"}\` in the project any more`;
    case "git_failed": {
      const code = error.code === null || error.code === undefined ? "killed" : `exit ${error.code}`;
      const stderr = error.stderr?.trim() ? `: ${error.stderr.trim()}` : "";
      return `${gitCommand(error)} failed (${code})${stderr}`;
    }
    case "git_timeout":
      return `${gitCommand(error)} did not finish in time — is the repository healthy?`;
    default:
      return error.message;
  }
}

/**
 * The command a `git_failed` / `git_timeout` body names, as the server
 * wrote it — `git commit --quiet --cleanup=verbatim --file=- -- p.cic`
 * (`git.rs` formats `git <args>`; it already starts with `git`) — else
 * `git` when the body carries none.
 */
function gitCommand(error: GitErrorBody): string {
  const command = error.command?.trim();
  return command ? command : "git";
}

/** The toast sentence for anything a git action threw (typed or not). */
export function describeGitFailure(error: unknown): string {
  if (error instanceof GitRouteError) return describeGitError(error.body);
  return String(error);
}
