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
 *
 * The bus also keeps the client's own display phases per generation for
 * the profiler (docs/13 §The profiler; v0.1 wave 5 P1): the frames' decode
 * time as the socket measured it, and their apply time — the wall of the
 * subscribers' work on each frame, which is the scene building its
 * geometry for the GPU (`viewport/scene.ts`); the last few generations are
 * kept (`GENERATIONS_KEPT`), the profiler reads the one it shows.
 */
import type { Frame } from "../protocol/frames";

export type FrameListener = (frame: Frame, byteLength: number) => void;
export type ScreenshotHandler = (target: string) => Promise<Blob>;

const REPLAY_LIMIT = 4096;
/** Generations whose client phases are kept — the profiler shows the last complete one; a few more cover a slow reader. */
export const GENERATIONS_KEPT = 16;

/** The client's work on one generation's frames (`panels/profile.ts::clientPhases` reads it). */
export interface GenerationFrames {
  /** Frames received for this generation. */
  frames: number;
  /** Their bytes on the wire. */
  bytes: number;
  /** Wall milliseconds decoding them (measured around `decodeFrame` by the socket). */
  decodeMs: number;
  /** Wall milliseconds applying them — the subscribers' work per frame (the scene's geometry build). */
  applyMs: number;
  /** `performance.now()` when the first frame arrived. */
  firstAt: number;
  /** `performance.now()` when the last frame was applied. */
  lastAt: number;
}

class FrameBus {
  private listeners = new Set<FrameListener>();
  private replay: [Frame, number][] = [];
  private screenshotHandler: ScreenshotHandler | null = null;
  private generations = new Map<number, GenerationFrames>();
  /** Generations whose record is final (`seal` / `sealAll`); pruned with `generations`. */
  private sealed = new Set<number>();
  received = 0;
  bytes = 0;
  /** `performance.now()` of the last frame — the client end of the preview-latency measurement. */
  lastAt = 0;
  /** Highest generation seen in any frame. */
  lastGeneration = 0;
  /**
   * The clock the phases are stamped with — `performance.now()`; a test
   * injects its own and drives it from a subscriber, so the apply time and
   * the last-frame stamp are asserted against known numbers (the seam in
   * the shape of the server's `op_clock`).
   */
  now: () => number = () => performance.now();

  /**
   * A frame off the socket: counted, attributed to its generation's client
   * phases (`decodeMs` from the socket's own measurement; the apply is
   * timed here around the subscribers), replayed to a late viewport.
   */
  publish(frame: Frame, byteLength: number, decodeMs = 0): void {
    this.received += 1;
    this.bytes += byteLength;
    const arrived = this.now();
    this.lastAt = arrived;
    if (frame.header.generation > this.lastGeneration) this.lastGeneration = frame.header.generation;
    if (this.listeners.size === 0) {
      this.replay.push([frame, byteLength]);
      if (this.replay.length > REPLAY_LIMIT) this.replay.shift();
    }
    for (const listener of this.listeners) listener(frame, byteLength);
    const applied = this.now();
    const generation = frame.header.generation;
    // A sealed generation's record is final: a restream (a `resync_display`,
    // a reconnect's re-hydration) re-sends every displayed output at the
    // generation that drew it, and those frames are not that pass's work —
    // counting them doubled the decode and upload and moved the last stamp
    // to the restream, so the socket residual read a rate the pass never
    // had (review finding L3-P1-4).
    if (this.sealed.has(generation)) return;
    const stats = this.generations.get(generation) ?? {
      frames: 0,
      bytes: 0,
      decodeMs: 0,
      applyMs: 0,
      firstAt: arrived,
      lastAt: applied,
    };
    stats.frames += 1;
    stats.bytes += byteLength;
    stats.decodeMs += decodeMs;
    stats.applyMs += applied - arrived;
    stats.lastAt = applied;
    if (!this.generations.has(generation)) {
      this.generations.set(generation, stats);
      // Bounded: the oldest generation goes (insertion order is arrival order).
      while (this.generations.size > GENERATIONS_KEPT) {
        const oldest = this.generations.keys().next().value;
        if (oldest === undefined) break;
        this.generations.delete(oldest);
        this.sealed.delete(oldest);
      }
    }
  }

  /** The client's work on `generation`'s frames so far, or null when none arrived (or it has left the window). */
  generation(generation: number): GenerationFrames | null {
    const stats = this.generations.get(generation);
    return stats === undefined ? null : { ...stats };
  }

  /**
   * The pass of `generation` has landed (`display_end` heard behind its last
   * frame): its record is final — later frames of that generation are a
   * restream's and change nothing. A generation never recorded (a page
   * that joined after its pass) still records the restream's frames once,
   * so its decode and upload are measured (docs/16 §Inspector contents).
   */
  seal(generation: number): void {
    if (this.generations.has(generation)) this.sealed.add(generation);
  }

  /** A `display_reset`: every recorded generation is final — what follows is a restream. */
  sealAll(): void {
    for (const generation of this.generations.keys()) this.sealed.add(generation);
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
