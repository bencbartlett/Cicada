/**
 * `GET /api/files` over HTTP (docs/13 §HTTP surface; wave 4 O1): ONE
 * directory of the served root per request, for the landing picker and
 * File → Open. Typed answer (`FilesResponse`), typed refusal
 * (`FilesErrorBody` — `path_not_allowed` 400, `not_found` 404, `io_error`
 * 403) carried by `FilesRouteError`; a non-JSON failure (the token
 * middleware's text 401, a proxy page) becomes a `transport` kind with the
 * text, so callers branch on `kind`, never on prose. The picker never reads
 * `/api/project` (over a home root its walk is seconds and lists what the
 * picker must not show — docs/17 O1). `fetchImpl` is injectable for tests.
 */
import type { FilesErrorBody, FilesErrorKind, FilesResponse } from "./messages";

export interface FilesSession {
  token: string;
}

/** A refused (or unreachable) file listing: the typed body plus the HTTP status (0 = no response). */
export class FilesRouteError extends Error {
  readonly status: number;
  readonly body: FilesErrorBody;
  constructor(status: number, body: FilesErrorBody) {
    super(body.message);
    this.name = "FilesRouteError";
    this.status = status;
    this.body = body;
  }
  get kind(): FilesErrorKind {
    return this.body.kind;
  }
}

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** The route for one directory: `/api/files?dir=<root-relative>` (`""` = the root). */
export function filesUrl(dir: string): string {
  return `/api/files?dir=${encodeURIComponent(dir)}`;
}

/** List `dir` (root-relative, `""` for the root). */
export async function fetchFiles(
  session: FilesSession,
  dir: string,
  fetchImpl: FetchLike = (input, init) => fetch(input, init),
): Promise<FilesResponse> {
  let response: Response;
  try {
    response = await fetchImpl(filesUrl(dir), { headers: { "X-Cicada-Token": session.token } });
  } catch (error: unknown) {
    throw new FilesRouteError(0, { kind: "transport", message: `unreachable: ${String(error)}`, path: dir });
  }
  if (!response.ok) throw await refusalOf(response, dir);
  return (await response.json()) as FilesResponse;
}

/** Read a non-OK response as the typed refusal it is, else a `transport` refusal with the text. */
export async function refusalOf(response: Response, dir: string): Promise<FilesRouteError> {
  const text = await response.text();
  try {
    const parsed = JSON.parse(text) as Partial<FilesErrorBody>;
    if (typeof parsed.kind === "string" && typeof parsed.message === "string") {
      return new FilesRouteError(response.status, { kind: parsed.kind, message: parsed.message, path: parsed.path ?? dir });
    }
  } catch {
    // not JSON — fall through
  }
  return new FilesRouteError(response.status, {
    kind: "transport",
    message: `HTTP ${response.status}${text.trim() ? ` — ${text.trim()}` : ""}`,
    path: dir,
  });
}

/** One readable sentence per refusal kind (the picker shows it in place of the list). */
export function describeFilesError(body: FilesErrorBody): string {
  const where = body.path === "" ? "the root" : `\`${body.path}\``;
  switch (body.kind) {
    case "not_found":
      return `${where} is not a directory under the served root (it may have been moved or removed)`;
    case "path_not_allowed":
      return `${where} is outside the served root — the server lists nothing above it`;
    case "io_error":
      return `${where} could not be read: ${body.message}`;
    case "transport":
      return `the file list could not be read: ${body.message}`;
    default:
      return `${body.kind}: ${body.message}`;
  }
}

/** What a failed listing becomes in the UI: the typed sentence, or the error's own text. */
export function describeFilesFailure(error: unknown): string {
  return error instanceof FilesRouteError ? describeFilesError(error.body) : String(error);
}
