/**
 * Wires the socket to the store and the frame bus, fetches the catalog,
 * feeds the git status policy (`state/git.ts`) the events it refreshes on,
 * answers screenshot requests via the viewport, and exposes the debug
 * handle Playwright reads (`window.__cicada`). Called once from `main.tsx`.
 */
import { CicadaClient, wsUrl } from "../protocol/client";
import type { Catalog, ClientMessage, ServerEnvelope } from "../protocol/messages";
import { frameBus } from "./frameBus";
import { gitPolicy, startGitStatus } from "./git";
import type { GitRefreshPolicy } from "./gitRefresh";
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

export interface StartOptions {
  token: string;
  pipeline: string;
}

/** Read `?token=…&pipeline=…` from the page URL (Jupyter-style). */
export function readUrlOptions(search: string = window.location.search): Partial<StartOptions> {
  const params = new URLSearchParams(search);
  const token = params.get("token") ?? undefined;
  const pipeline = params.get("pipeline") ?? undefined;
  return { token, pipeline };
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

export function startConnection(options: StartOptions): CicadaClient {
  const store = useCicada.getState();
  store.setIdentity(options.token, options.pipeline);
  store.setConnection("connecting");

  const url = wsUrl(options.token, options.pipeline);
  const git = startGitStatus(window);
  client = new CicadaClient(url, {
    onMessage: (envelope: ServerEnvelope) => {
      useCicada.getState().applyServerMessage(envelope);
      feedGitPolicy(git, envelope);
      if (envelope.type === "screenshot_request") {
        const { id, target } = envelope.payload;
        frameBus
          .screenshot(target)
          .then(blobToBase64)
          .then((png_base64) => {
            client?.send({ type: "screenshot", payload: { id, png_base64 } });
          })
          .catch((error: unknown) => {
            client?.send({ type: "screenshot", payload: { id, error: String(error) } });
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
        useCicada.getState().setConnection("closed", reason);
        return;
      }
      scheduleReconnect(reason);
    },
    onError: (message) => {
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
  });
  store.installSender((message: ClientMessage) => client?.send(message) ?? "");
  client.connect();

  void fetch(`/api/catalog?pipeline=${encodeURIComponent(options.pipeline)}`, {
    headers: { "X-Cicada-Token": options.token },
  })
    .then(async (response) => {
      if (!response.ok) throw new Error(`catalog: HTTP ${response.status}`);
      return (await response.json()) as Catalog;
    })
    .then((catalog) => useCicada.getState().setCatalog(catalog))
    .catch((error: unknown) => useCicada.getState().addNotice("error", String(error)));

  installDebugHandle();
  return client;
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
    /** Filled in by the viewport: scene statistics for assertions. */
    scene: null as null | (() => unknown),
  };
  (window as unknown as { __cicada: typeof handle }).__cicada = handle;
}

export function getClient(): CicadaClient | null {
  return client;
}
