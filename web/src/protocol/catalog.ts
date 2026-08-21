/**
 * `GET /api/catalog?pipeline=…` (docs/13 §HTTP surface): the project-aware
 * node catalog — the stdlib plus the served pipeline's `scripts/*.py` — in
 * format 2 (`Catalog` in `messages.ts`; `catalog.test.ts` pins the shape to
 * the server's own rendering). A read, token-gated like every API route;
 * `fetchImpl` is injectable for tests, the app passes nothing.
 *
 * WHEN the app reads it is the state layer's business (`state/catalog.ts`):
 * the catalog is the one piece of authoritative state a `snapshot` does not
 * carry, so every snapshot re-reads it.
 */
import type { Catalog } from "./messages";

/** What the read needs: the session token and the pipeline the catalog is for. */
export interface CatalogSession {
  token: string;
  pipeline: string;
}

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** One catalog answer: the parsed object and the bytes it was parsed from (the state layer compares answers by text). */
export interface CatalogAnswer {
  catalog: Catalog;
  /** The response body verbatim — two snapshots whose answers are byte-identical need no second catalog object. */
  text: string;
}

/** The catalog the server serves for `session.pipeline`, with its text; any non-OK answer throws with the HTTP status. */
export async function fetchCatalog(session: CatalogSession, fetchImpl: FetchLike = fetch): Promise<CatalogAnswer> {
  const response = await fetchImpl(`/api/catalog?pipeline=${encodeURIComponent(session.pipeline)}`, {
    headers: { "X-Cicada-Token": session.token },
  });
  if (!response.ok) throw new Error(`catalog: HTTP ${response.status}`);
  const text = await response.text();
  return { catalog: JSON.parse(text) as Catalog, text };
}
