/**
 * Binary frames bypass React: the connection pushes decoded frames here and
 * the viewport (imperative three.js) subscribes. Also carries the two
 * server → viewport asks that are not state: screenshot requests and
 * display resets. Late subscribers get the frames received before they
 * mounted (bounded replay), so a viewport that mounts after `snapshot`
 * still paints the initial display set.
 *
 * Frames arrive in the server's display-lane order (docs/13 §Two lanes,
 * one socket) and are handed on in that order; control-plane texts may
 * overtake them on the wire and apply nothing here — the ledger behind the
 * subscribers (`viewport/sceneStore`) converges by its per-output
 * generation rules (`frameBus.test.ts`).
 */
import type { Frame } from "../protocol/frames";

export type FrameListener = (frame: Frame, byteLength: number) => void;
export type ScreenshotHandler = (target: string) => Promise<Blob>;

const REPLAY_LIMIT = 4096;

class FrameBus {
  private listeners = new Set<FrameListener>();
  private replay: [Frame, number][] = [];
  private screenshotHandler: ScreenshotHandler | null = null;
  received = 0;
  bytes = 0;
  /** `performance.now()` of the last frame — the client end of the preview-latency measurement. */
  lastAt = 0;
  /** Highest generation seen in any frame. */
  lastGeneration = 0;

  publish(frame: Frame, byteLength: number): void {
    this.received += 1;
    this.bytes += byteLength;
    this.lastAt = performance.now();
    if (frame.header.generation > this.lastGeneration) this.lastGeneration = frame.header.generation;
    if (this.listeners.size === 0) {
      this.replay.push([frame, byteLength]);
      if (this.replay.length > REPLAY_LIMIT) this.replay.shift();
    }
    for (const listener of this.listeners) listener(frame, byteLength);
  }

  subscribe(listener: FrameListener): () => void {
    this.listeners.add(listener);
    if (this.replay.length > 0) {
      const pending = this.replay;
      this.replay = [];
      for (const [frame, bytes] of pending) listener(frame, bytes);
    }
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** The viewport registers how to render a PNG for `/debug/screenshot`. */
  setScreenshotHandler(handler: ScreenshotHandler | null): void {
    this.screenshotHandler = handler;
  }

  async screenshot(target: string): Promise<Blob> {
    if (this.screenshotHandler === null) {
      throw new Error("no viewport mounted to render a screenshot");
    }
    return this.screenshotHandler(target);
  }
}

export const frameBus = new FrameBus();
