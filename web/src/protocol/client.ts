/**
 * The WebSocket client: one socket per session (docs/13). JSON control
 * plane in, binary frames in, JSON intents out. No React here — the
 * connection module wires this into the store and the viewport's frame bus.
 */
import { decodeFrame, type Frame } from "./frames";
import { PROTOCOL_VERSION, } from "./version";
import type { ClientEnvelope, ClientMessage, Role, ServerEnvelope } from "./messages";

export interface ClientHandlers {
  onMessage: (message: ServerEnvelope) => void;
  onFrame: (frame: Frame, byteLength: number) => void;
  onOpen?: () => void;
  /** `closedByUs` = `close()` was called; false = the server/network dropped us. */
  onClose?: (reason: string, closedByUs: boolean) => void;
  onError?: (message: string) => void;
}

/** `ws(s)://host/ws?token=…&pipeline=…` for the current page. */
export function wsUrl(token: string, pipeline: string, base: Location = window.location): string {
  const scheme = base.protocol === "https:" ? "wss" : "ws";
  const params = new URLSearchParams({ token, pipeline });
  return `${scheme}://${base.host}/ws?${params.toString()}`;
}

/** How this socket joins (the `hello`'s join hint, docs/13): `role: "observer"` = a declared observer that never holds the lease. */
export interface ClientOptions {
  role?: Role;
}

export class CicadaClient {
  private socket: WebSocket | null = null;
  private nextId = 1;
  private readonly url: string;
  private readonly handlers: ClientHandlers;
  private readonly options: ClientOptions;
  private closedByUs = false;

  constructor(url: string, handlers: ClientHandlers, options: ClientOptions = {}) {
    this.url = url;
    this.handlers = handlers;
    this.options = options;
  }

  /** The `hello` this socket opens with: the protocol version, plus the join hint when one was asked for. */
  hello(): ClientMessage {
    const role = this.options.role;
    return role === undefined
      ? { type: "hello", payload: { v: PROTOCOL_VERSION } }
      : { type: "hello", payload: { v: PROTOCOL_VERSION, role } };
  }

  /** Open a (new) socket; safe to call again after a close to reconnect. */
  connect(): void {
    this.closedByUs = false;
    if (this.socket !== null) {
      // A stale socket (still connecting/open) must not keep its handlers.
      this.socket.onclose = null;
      this.socket.onerror = null;
      this.socket.onmessage = null;
      this.socket.close();
      this.socket = null;
    }
    const socket = new WebSocket(this.url);
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      this.send(this.hello());
      this.handlers.onOpen?.();
    };
    socket.onmessage = (event: MessageEvent<string | ArrayBuffer>) => {
      if (typeof event.data === "string") {
        let parsed: ServerEnvelope;
        try {
          parsed = JSON.parse(event.data) as ServerEnvelope;
        } catch (error) {
          this.handlers.onError?.(`unreadable server message: ${String(error)}`);
          return;
        }
        this.handlers.onMessage(parsed);
      } else {
        try {
          const frame = decodeFrame(event.data);
          this.handlers.onFrame(frame, event.data.byteLength);
        } catch (error) {
          this.handlers.onError?.(`bad binary frame: ${String(error)}`);
        }
      }
    };
    socket.onclose = (event) => {
      if (this.socket === socket) this.socket = null;
      const why = this.closedByUs
        ? "closed"
        : `socket closed (${event.code}${event.reason ? `: ${event.reason}` : ""})`;
      this.handlers.onClose?.(why, this.closedByUs);
    };
    socket.onerror = () => {
      this.handlers.onError?.("socket error");
    };
    this.socket = socket;
  }

  close(): void {
    this.closedByUs = true;
    this.socket?.close();
    this.socket = null;
  }

  get isOpen(): boolean {
    return this.socket?.readyState === WebSocket.OPEN;
  }

  /** Send an intent; returns its id (echoed in the resulting delta/error). */
  send(message: ClientMessage): string {
    const id = String(this.nextId++);
    const envelope: ClientEnvelope = { v: PROTOCOL_VERSION, id, ...message };
    if (this.socket === null || this.socket.readyState !== WebSocket.OPEN) {
      this.handlers.onError?.(`not connected — dropped intent ${message.type}`);
      return id;
    }
    this.socket.send(JSON.stringify(envelope));
    return id;
  }
}
