/**
 * Wires the socket to the store and the frame bus, feeds the catalog policy
 * (`state/catalog.ts`: every `snapshot` re-reads `/api/catalog`) and the git
 * status policy (`state/git.ts`) the events they refresh on, remembers the
 * pipeline under File → Recent on its `hello`, answers screenshot requests
 * via the viewport, and exposes the debug handle Playwright reads
 * (`window.__cicada`). The route drives it (`syncConnection`, docs/16
 * §Application layout): one socket at a time, joined to the pipeline the
 * URL names — opening another pipeline closes this socket, clears the
 * store's pipeline-bound state and joins the other session afresh; closing
 * the pipeline (the picker) leaves no socket. The pop-out viewport's route
 * joins as a declared observer (docs/13 — the join hint).
 */
import { CicadaClient, wsUrl } from "../protocol/client";
import type { ClientMessage, Role, ServerEnvelope } from "../protocol/messages";
import { catalogPolicy, feedCatalogPolicy, startCatalogRefresh, stopCatalogRefresh } from "./catalog";
import { frameBus } from "./frameBus";
import { gitPolicy, startGitStatus, stopGitStatus } from "./git";
import type { GitRefreshPolicy } from "./gitRefresh";
import { browserStorage, rememberRecent, type StorageLike } from "./recent";
import type { Route } from "./route";
import { useCicada } from "./store";

/**
 * Which server messages make the git policy read again (docs/17 item 2):
 * `hello` = (re)connected → now; a `delta` (the server persisted a write —
 * text and/or sidecar, both in the commit scope) and a barrier `snapshot`
 * (external change, `apply_text`, `git revert`) → the ≤1 s debounce, AND
 * the store's cached status is marked stale in the same breath (until the
 * read lands the chip's count, the tab's scope and the commit/revert gates
 * would otherwise describe the previous tree). The initial snapshot
 * follows the hello's read and adds nothing; statuses, values, probes and
 * lease changes never touch the tree.
 */
export function feedGitPolicy(policy: Pick<GitRefreshPolicy, "onConnected" | "onWrite">, envelope: ServerEnvelope): void {
  switch (envelope.type) {
    case "hello":
      policy.onConnected();
      break;
    case "delta":
      writeLanded(policy);
      break;
    case "snapshot":
      if (envelope.payload.barrier) writeLanded(policy);
      break;
    default:
      break;
  }
}

/** The server confirmed a write: the cache is stale from now on, and the debounced read is armed — never one without the other. */
function writeLanded(policy: Pick<GitRefreshPolicy, "onWrite">): void {
  useCicada.getState().markGitStale();
  policy.onWrite();
}

/**
 * File → Recent remembers a pipeline on its session's `hello` — the server
 * confirmed the file exists and named it root-relative — never on the ask
 * (a URL naming a file the server refuses must not become "recent").
 */
export function feedRecent(storage: StorageLike | null, envelope: ServerEnvelope): void {
  if (envelope.type === "hello") rememberRecent(storage, envelope.payload.pipeline);
}

export interface StartOptions {
  token: string;
  pipeline: string;
  /** `observer` = join as a declared observer that never holds the lease (the pop-out viewport). */
  role?: Role;
}

/**
 * The connection a route asks for: a pipeline with a token is a session to
 * join — as a declared observer when the view is the pop-out viewport —
 * and anything else (the picker, a page without a token) is none.
 */
export function optionsForRoute(route: Route): StartOptions | null {
  if (route.token === undefined || route.pipeline === undefined) return null;
  const options: StartOptions = { token: route.token, pipeline: route.pipeline };
  if (route.view === "viewport") options.role = "observer";
  return options;
}

function sameOptions(a: StartOptions, b: StartOptions): boolean {
  return a.token === b.token && a.pipeline === b.pipeline && a.role === b.role;
}

async function blobToBase64(blob: Blob): Promise<string> {
  const buffer = await blob.arrayBuffer();
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

let client: CicadaClient | null = null;
/** What the live socket was started with (null = no socket). */
let current: StartOptions | null = null;

/** Reconnect backoff: 0.5 s doubling to a cap of 8 s (`attempt` is 1-based). */
export const RECONNECT_BASE_MS = 500;
export const RECONNECT_CAP_MS = 8000;
export function reconnectDelayMs(attempt: number): number {
  return Math.min(RECONNECT_CAP_MS, RECONNECT_BASE_MS * 2 ** Math.max(0, attempt - 1));
}

let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectAttempt = 0;

/** Try again now (banner button); a no-op unless a retry is pending. */
export function retryConnectionNow(): void {
  if (client === null || reconnectTimer === null) return;
  clearTimeout(reconnectTimer);
  reconnectTimer = null;
  useCicada.getState().setReconnect({ attempt: reconnectAttempt, nextAt: null });
  client.connect();
}

/**
 * The socket dropped without us closing it: clear the session identity
 * (nothing may write against a dead socket), then reconnect with capped
 * exponential backoff. The server re-hydrates on join (hello + snapshot +
 * display_reset + frames), which the store and the scene already handle.
 */
function scheduleReconnect(reason: string): void {
  if (client === null || reconnectTimer !== null) return;
  reconnectAttempt += 1;
  const delay = reconnectDelayMs(reconnectAttempt);
  const attempt = reconnectAttempt;
  useCicada.getState().markDisconnected(reason, { attempt, nextAt: Date.now() + delay });
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    useCicada.getState().setReconnect({ attempt, nextAt: null });
    client?.connect();
  }, delay);
}

/**
 * Follow the route (installed as `installRouting`'s callback): join the
 * pipeline it names with the role it implies, or leave no socket. The same
 * pipeline, token and role again is a no-op — a `popstate` back to the open
 * file must not reconnect it.
 */
export function syncConnection(route: Route): void {
  const wanted = optionsForRoute(route);
  if (wanted === null) {
    if (current !== null) {
      stopConnection();
      useCicada.getState().resetSession(route.token ?? "", "");
    }
    return;
  }
  if (current !== null && sameOptions(current, wanted)) return;
  startConnection(wanted);
}

/**
 * Close the live socket (if any) and everything that follows it — the
 * reconnect timer, the git and catalog policies. The store's pipeline-bound
 * state is the caller's to reset (`resetSession`): `startConnection` does
 * it for the next pipeline, `syncConnection` for the picker.
 */
export function stopConnection(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  reconnectAttempt = 0;
  client?.close();
  client = null;
  current = null;
  stopGitStatus(window);
  stopCatalogRefresh();
}

export function startConnection(options: StartOptions): CicadaClient {
  stopConnection();
  current = { ...options };
  const store = useCicada.getState();
  store.resetSession(options.token, options.pipeline);
  store.setConnection("connecting");

  const url = wsUrl(options.token, options.pipeline);
  const git = startGitStatus(window);
  const catalog = startCatalogRefresh();
  const recent = browserStorage();
  const socket = new CicadaClient(
    url,
    {
      onMessage: (envelope: ServerEnvelope) => {
        useCicada.getState().applyServerMessage(envelope);
        feedCatalogPolicy(catalog, envelope);
        feedGitPolicy(git, envelope);
        feedRecent(recent, envelope);
        if (envelope.type === "screenshot_request") {
          const { id, target } = envelope.payload;
          frameBus
            .screenshot(target)
            .then(blobToBase64)
            .then((png_base64) => {
              socket.send({ type: "screenshot", payload: { id, png_base64 } });
            })
            .catch((error: unknown) => {
              socket.send({ type: "screenshot", payload: { id, error: String(error) } });
            });
        }
      },
      onFrame: (frame, byteLength) => frameBus.publish(frame, byteLength),
      onOpen: () => {
        reconnectAttempt = 0;
        useCicada.getState().setConnection("open");
      },
      onClose: (reason, closedByUs) => {
        if (closedByUs) {
          // Our own close: the next `startConnection` (or the picker's
          // reset) already owns the store — say nothing over it.
          if (client === socket) useCicada.getState().setConnection("closed", reason);
          return;
        }
        if (client === socket) scheduleReconnect(reason);
      },
      onError: (message) => {
        if (client !== socket) return;
        const state = useCicada.getState();
        if (state.connection === "reconnecting") {
          // A failed retry is expected while reconnecting — the banner says
          // so; no notice per attempt (`onclose` follows and reschedules).
          // Anything else (a dropped intent, a bad frame) stays loud.
          if (message !== "socket error") state.addNotice("error", message);
          return;
        }
        state.setConnection("error", message);
        state.addNotice("error", message);
      },
    },
    options.role === undefined ? {} : { role: options.role },
  );
  client = socket;
  store.installSender((message: ClientMessage) => client?.send(message) ?? "");
  // No catalog fetch here: the socket's first `snapshot` reads it (and every
  // later one re-reads it — `state/catalog.ts`).
  socket.connect();

  installDebugHandle();
  return socket;
}

/** `window.__cicada`: the agent verification hooks (doc 14). */
function installDebugHandle(): void {
  const handle = {
    /** A snapshot of the store (authoritative mirror + ui). */
    state: () => useCicada.getState(),
    /**
     * Frame counters (docs/15 measurement harness): `received`/`bytes` so
     * far, `lastAt` = `performance.now()` of the last frame (the client end
     * of a preview round-trip), `lastGeneration` = the highest generation
     * any frame carried.
     */
    frames: () => ({
      received: frameBus.received,
      bytes: frameBus.bytes,
      lastAt: frameBus.lastAt,
      lastGeneration: frameBus.lastGeneration,
    }),
    /** Send an intent (tests drive gestures through the same op pipeline). */
    send: (message: ClientMessage) => useCicada.getState().send(message),
    /** Viewport render → PNG blob (same path as /debug/screenshot). */
    screenshot: (target = "viewport") => frameBus.screenshot(target),
    /** The git status policy's counters: reads started, whether one is pending or in flight, and the cache's staleness. */
    git: () => {
      const policy = gitPolicy();
      const { stale, writes, answers } = useCicada.getState().git;
      return { reads: policy?.reads ?? 0, busy: policy?.busy ?? false, stale, writes, answers };
    },
    /** The catalog policy's counters: reads started (one per snapshot received), whether one is in flight, and how many nodes the store holds. */
    catalog: () => {
      const policy = catalogPolicy();
      const catalog = useCicada.getState().catalog;
      return { reads: policy?.reads ?? 0, busy: policy?.busy ?? false, nodes: catalog?.nodes.length ?? 0 };
    },
    /** What the live socket was started with (`{token, pipeline, role?}`), or null without one — the route's word made socket. */
    connection: () => (current === null ? null : { ...current }),
    /** Filled in by the viewport: scene statistics for assertions. */
    scene: null as null | (() => unknown),
  };
  const existing = (window as unknown as { __cicada?: typeof handle }).__cicada;
  // A re-install (the next pipeline) keeps the viewport's `scene` hook: the
  // viewport is mounted once and fills it on mount, before or after.
  if (existing !== undefined) handle.scene = existing.scene;
  (window as unknown as { __cicada: typeof handle }).__cicada = handle;
}

export function getClient(): CicadaClient | null {
  return client;
}
