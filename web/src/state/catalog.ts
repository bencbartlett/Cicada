/**
 * WHEN the catalog is read (docs/17 Follow-ups: the stale catalog after a
 * scripts-change reload). The catalog — the stdlib plus the pipeline's
 * script nodes — is the one piece of the server's authoritative state a
 * `snapshot` does not carry, and it changes under the clients whenever the
 * project's scripts do: an external edit of `scripts/*.py` (the watcher's
 * barrier snapshot), a `git revert` that touched a script (its barrier
 * snapshot), an `apply_text` that replaced one (the only time `apply_text`
 * answers with a snapshot instead of a delta). The server labels none of
 * these as a catalog change — the watcher's `reason` is "external file
 * change" whether the pipeline text or a script moved — so the client
 * cannot key on the reason, and a label would be one more thing a reload
 * path could forget. The rule that needs no label: EVERY snapshot re-reads
 * the catalog. That covers the join's too — the first connect, and every
 * reconnect, where the scripts may have changed while the socket was down
 * (a staleness the one-shot fetch at start never saw) — and it costs one
 * ~100 KB GET per snapshot, which is rare by construction (connects,
 * reloads); nothing else the server sends moves the catalog.
 *
 * One read in flight at a time; a snapshot that lands mid-read runs ONE
 * more read when it finishes, so reads land in order and the catalog in
 * the store is never older than the last snapshot (two quick saves cannot
 * leave the earlier catalog on top). No DOM, no timers.
 */
import { fetchCatalog } from "../protocol/catalog";
import type { CatalogSession } from "../protocol/catalog";
import type { ServerEnvelope } from "../protocol/messages";
import { useCicada } from "./store";

export class CatalogRefreshPolicy {
  private inFlight = false;
  private again = false;
  private disposed = false;
  /** Reads started (tests and the debug handle). */
  reads = 0;

  constructor(private readonly read: () => Promise<void>) {}

  /** A `snapshot` arrived — the join's, a barrier, an `apply_text` that changed scripts: read. */
  onSnapshot(): void {
    this.run();
  }

  /** Is a read in flight (or queued behind one)? */
  get busy(): boolean {
    return this.inFlight || this.again;
  }

  dispose(): void {
    this.disposed = true;
    this.again = false;
  }

  private run(): void {
    if (this.disposed) return;
    if (this.inFlight) {
      this.again = true;
      return;
    }
    this.inFlight = true;
    this.reads += 1;
    this.read()
      .catch(() => {
        // The reader reports its own failures (store notice); the policy
        // only sequences reads.
      })
      .finally(() => {
        this.inFlight = false;
        if (this.again && !this.disposed) {
          this.again = false;
          this.run();
        }
      });
  }
}

let policy: CatalogRefreshPolicy | null = null;

function session(): CatalogSession {
  const state = useCicada.getState();
  return { token: state.token, pipeline: state.pipeline };
}

type FetchLike = Parameters<typeof fetchCatalog>[1];

/**
 * One catalog read into the store: a good answer replaces the catalog
 * (the store swaps the whole object — the canvas re-indexes per object),
 * a failure is a notice and the previous catalog stays — better a stale
 * search box than an empty one mid-session. `fetchImpl` is for tests.
 */
export async function readCatalog(fetchImpl?: FetchLike): Promise<void> {
  try {
    const catalog = await fetchCatalog(session(), fetchImpl);
    useCicada.getState().setCatalog(catalog);
  } catch (error: unknown) {
    useCicada.getState().addNotice("error", String(error));
  }
}

/**
 * Install the policy for this page's connection (once, from the connection
 * module). Installing reads nothing — the socket's first `snapshot` does.
 * `read` is injectable so tests can count reads without a server.
 */
export function startCatalogRefresh(read: () => Promise<void> = readCatalog): CatalogRefreshPolicy {
  stopCatalogRefresh();
  policy = new CatalogRefreshPolicy(read);
  return policy;
}

export function stopCatalogRefresh(): void {
  policy?.dispose();
  policy = null;
}

/** The installed policy (null before `startCatalogRefresh`). */
export function catalogPolicy(): CatalogRefreshPolicy | null {
  return policy;
}

/**
 * Which server messages make the catalog policy read: a `snapshot`, every
 * one (see the module doc); nothing else — a `delta` is this session's own
 * text/sidecar write and cannot touch `scripts/`, statuses, values, probes,
 * lease changes and `hello` move no catalog (the hello's hydration ends in
 * the snapshot that does).
 */
export function feedCatalogPolicy(policy: Pick<CatalogRefreshPolicy, "onSnapshot">, envelope: ServerEnvelope): void {
  if (envelope.type === "snapshot") policy.onSnapshot();
}
