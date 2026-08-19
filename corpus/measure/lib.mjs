/**
 * Shared plumbing for the docs/15 measurement harness (slider_loop.mjs,
 * esc.mjs): argument parsing, the token-gated HTTP + WebSocket client
 * against a running `cicada serve`, percentiles. Node ≥ 20, no
 * dependencies — the global `WebSocket` and `fetch` are enough.
 *
 * Protocol facts used here (mirrors crates/cicada-server/src/protocol.rs
 * and web/src/protocol/*.ts — change them together):
 *   - `GET /debug/state?token=…&pipeline=…[&wait=true]` is the authoritative
 *     JSON oracle (graph, statuses, solve.busy, timings, …).
 *   - `ws://host/ws?token=…&pipeline=…`; the first intent must be
 *     `{v:1,type:"hello",payload:{v:1}}`; server envelopes are
 *     `{v,seq,type,payload}`; binary frames carry a 32-byte header with the
 *     generation as a little-endian u64 at byte 8.
 *   - The first client is the writer; later ones are observers and must
 *     `take_lease` before writing (previews, params, cancel).
 */

export const PROTOCOL_VERSION = 1;

/** `--key value` / `--flag` → `{ key: value, flag: true }` (+ positionals). */
export function parseArgs(argv, defaults = {}) {
  const out = { ...defaults, _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) {
        out[key] = true;
      } else {
        out[key] = next;
        i += 1;
      }
    } else {
      out._.push(arg);
    }
  }
  return out;
}

/** Fail loudly: print the reason and exit nonzero. */
export function die(message) {
  console.error(`error: ${message}`);
  process.exit(2);
}

export function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[rank];
}

/** `{count, min, p50, p95, max, mean}` of a list (ms rounded to 0.01). */
export function stats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const round = (x) => (x === null ? null : Math.round(x * 100) / 100);
  const mean = sorted.length === 0 ? null : sorted.reduce((a, b) => a + b, 0) / sorted.length;
  return {
    count: sorted.length,
    min: round(sorted[0] ?? null),
    p50: round(percentile(sorted, 50)),
    p95: round(percentile(sorted, 95)),
    max: round(sorted[sorted.length - 1] ?? null),
    mean: round(mean),
  };
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Token-gated HTTP against the server. */
export class Http {
  constructor(base, token, pipeline) {
    this.base = base.replace(/\/$/, "");
    this.token = token;
    this.pipeline = pipeline;
  }

  async debugState({ wait = false, values = false } = {}) {
    const params = new URLSearchParams({ token: this.token, pipeline: this.pipeline });
    if (wait) params.set("wait", "true");
    if (values) params.set("values", "true");
    const response = await fetch(`${this.base}/debug/state?${params}`);
    if (!response.ok) {
      throw new Error(`GET /debug/state → HTTP ${response.status}: ${await response.text()}`);
    }
    return response.json();
  }
}

/**
 * One WebSocket session: hello handshake, lease, intents out, messages and
 * frames in (timestamped with `performance.now()`).
 */
export class Session {
  constructor(base, token, pipeline) {
    const url = new URL(base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/ws";
    url.search = new URLSearchParams({ token, pipeline }).toString();
    this.url = url.toString();
    this.socket = null;
    this.nextId = 1;
    this.clientId = null;
    this.role = null;
    /** Every text message: `{at, envelope}` in arrival order. */
    this.messages = [];
    /** Every binary frame: `{at, generation, bytes}` in arrival order. */
    this.frames = [];
    this.errors = [];
    this.waiters = [];
    this.closed = false;
  }

  async open() {
    await new Promise((resolve, reject) => {
      const socket = new WebSocket(this.url);
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        this.socket = socket;
        socket.send(JSON.stringify({ v: PROTOCOL_VERSION, type: "hello", payload: { v: PROTOCOL_VERSION } }));
        resolve();
      };
      socket.onerror = () => reject(new Error(`cannot connect to ${this.url}`));
      socket.onmessage = (event) => this.receive(event);
      socket.onclose = () => {
        this.closed = true;
        this.wake();
      };
    });
    const hello = await this.waitFor((m) => m.type === "hello", 10_000, "hello", 0);
    this.clientId = hello.payload.client_id;
    this.role = hello.payload.role;
    await this.waitFor((m) => m.type === "snapshot", 30_000, "snapshot", 0);
    if (this.role !== "writer") {
      this.send({ type: "take_lease", payload: {} });
      await this.waitFor(
        (m) => m.type === "lease" && m.payload.role === "writer",
        10_000,
        "lease (another client holds the write lease and will not yield it)",
      );
      this.role = "writer";
    }
    return this;
  }

  receive(event) {
    const at = performance.now();
    if (typeof event.data === "string") {
      const envelope = JSON.parse(event.data);
      this.messages.push({ at, envelope });
      if (envelope.type === "error") this.errors.push(envelope.payload);
    } else {
      const view = new DataView(event.data);
      const magic = view.getUint32(0, true);
      if (magic !== 0x46434943) throw new Error(`bad frame magic 0x${magic.toString(16)}`);
      const generation = Number(view.getBigUint64(8, true));
      this.frames.push({ at, generation, bytes: event.data.byteLength });
    }
    this.wake();
  }

  wake() {
    const waiters = this.waiters;
    this.waiters = [];
    for (const waiter of waiters) waiter();
  }

  /** Resolve with the first message matching `predicate`, scanning from `from` (default: from now on). */
  waitFor(predicate, timeoutMs, what, from = this.messages.length) {
    return new Promise((resolve, reject) => {
      const deadline = performance.now() + timeoutMs;
      const check = () => {
        for (let i = from; i < this.messages.length; i += 1) {
          const { envelope } = this.messages[i];
          if (predicate(envelope)) {
            resolve(envelope);
            return true;
          }
        }
        if (this.closed) {
          reject(new Error(`socket closed while waiting for ${what}`));
          return true;
        }
        if (performance.now() > deadline) {
          reject(new Error(`timed out after ${timeoutMs} ms waiting for ${what}`));
          return true;
        }
        return false;
      };
      const arm = () => {
        if (check()) return;
        this.waiters.push(arm);
      };
      arm();
      setTimeout(() => {
        if (check()) return;
      }, timeoutMs + 1);
    });
  }

  /** Send an intent; returns `{id, at}` (`at` = `performance.now()` just before the send). */
  send(message) {
    if (this.socket === null) throw new Error("not connected");
    const id = String(this.nextId++);
    const at = performance.now();
    this.socket.send(JSON.stringify({ v: PROTOCOL_VERSION, id, ...message }));
    return { id, at };
  }

  close() {
    this.socket?.close();
  }
}

/** Find a node in the debug state's graph by binding name (loud when absent). */
export function findNode(state, name) {
  const node = state.graph.nodes.find((n) => n.name === name);
  if (node === undefined) {
    const names = state.graph.nodes.map((n) => n.name).join(", ");
    throw new Error(`no node named \`${name}\` in ${state.pipeline} (nodes: ${names})`);
  }
  return node;
}

/** Spell a Number literal the way the canvas does (docs/10: keep the point). */
export function numberLiteral(x) {
  if (!Number.isFinite(x)) throw new Error(`not a finite number: ${x}`);
  return Number.isInteger(x) ? x.toFixed(1) : String(x);
}
