/**
 * Deferred edge-overlay builds for large pieces: the crease-edge pass runs
 * in `edgeWorker.ts` off the UI thread (one job in flight, newest-first,
 * cancelled jobs never start), and the result lands on the main thread as
 * a ready-made position buffer. Where a module worker cannot be created,
 * the build falls back to the main thread (announced once, never silent),
 * gated by the quiet policy in `edgePolicy.ts`.
 */
import { EDGE_THRESHOLD_DEG } from "./edgePolicy";
import { edgePositions, type EdgeRequest, type EdgeResponse } from "./edgeWorker";

export interface EdgeJobInput {
  positions: Float32Array;
  indices: Uint32Array;
}

export interface EdgeJobHandle {
  /** Stop caring about this job: a pending one never starts; a finished one is discarded. */
  cancel(): void;
}

interface Job {
  id: number;
  input: EdgeJobInput;
  onDone: (positions: Float32Array, ms: number) => void;
  cancelled: boolean;
}

export class EdgeBuilder {
  private worker: Worker | null = null;
  private workerFailed = false;
  private queue: Job[] = [];
  private inFlight: Job | null = null;
  private nextId = 1;
  /** Builds finished so far (worker + fallback). */
  built = 0;
  /** Total worker time spent on finished builds (ms). */
  workerMs = 0;

  constructor(private readonly notice: (level: "info" | "warning" | "error", message: string) => void) {}

  /** Is the off-thread path available (or not yet known to have failed)? */
  get offThread(): boolean {
    return !this.workerFailed && typeof Worker === "function";
  }

  /** Queue a build; `onDone` runs on the main thread with the packed edge positions. */
  submit(input: EdgeJobInput, onDone: (positions: Float32Array, ms: number) => void): EdgeJobHandle {
    const job: Job = { id: this.nextId++, input, onDone, cancelled: false };
    this.queue.push(job);
    this.pump();
    return {
      cancel: () => {
        job.cancelled = true;
        this.queue = this.queue.filter((j) => j !== job);
      },
    };
  }

  get pending(): number {
    return this.queue.length + (this.inFlight === null ? 0 : 1);
  }

  dispose(): void {
    this.queue = [];
    this.inFlight = null;
    this.worker?.terminate();
    this.worker = null;
  }

  private ensureWorker(): Worker | null {
    if (this.worker !== null) return this.worker;
    if (!this.offThread) return null;
    try {
      const worker = new Worker(new URL("./edgeWorker.ts", import.meta.url), { type: "module" });
      worker.onmessage = (event: MessageEvent<EdgeResponse>) => this.onResult(event.data);
      worker.onerror = (event) => {
        // The worker died: say so once, finish the job on the main thread,
        // and stay on the main thread from here on.
        this.workerFailed = true;
        this.notice(
          "warning",
          `viewport: edge-overlay worker failed (${event.message || "unknown error"}) — building edges on the UI thread`,
        );
        worker.terminate();
        this.worker = null;
        const job = this.inFlight;
        this.inFlight = null;
        if (job !== null && !job.cancelled) this.queue.unshift(job);
        this.pump();
      };
      this.worker = worker;
      return worker;
    } catch (error) {
      this.workerFailed = true;
      this.notice(
        "warning",
        `viewport: no edge-overlay worker (${String(error)}) — building edges on the UI thread`,
      );
      return null;
    }
  }

  private pump(): void {
    if (this.inFlight !== null) return;
    const job = this.queue.shift();
    if (job === undefined) return;
    const worker = this.ensureWorker();
    if (worker === null) {
      // Main-thread fallback: the caller already waited for quiet (scene).
      this.finish(job, () => {
        const t0 = performance.now();
        const positions = edgePositions(job.input.positions, job.input.indices, EDGE_THRESHOLD_DEG);
        return { positions, ms: performance.now() - t0 };
      });
      this.pump();
      return;
    }
    this.inFlight = job;
    // Copies: the piece keeps using the frame's views; the worker gets its own.
    const request: EdgeRequest = {
      id: job.id,
      positions: job.input.positions.slice(),
      indices: job.input.indices.slice(),
      thresholdDeg: EDGE_THRESHOLD_DEG,
    };
    worker.postMessage(request, [request.positions.buffer, request.indices.buffer]);
  }

  private onResult(response: EdgeResponse): void {
    const job = this.inFlight;
    this.inFlight = null;
    if (job !== null && job.id === response.id) {
      this.workerMs += response.ms;
      this.finish(job, () => response);
    }
    this.pump();
  }

  private finish(job: Job, run: () => { positions: Float32Array; ms: number }): void {
    if (job.cancelled) return;
    const { positions, ms } = run();
    this.built += 1;
    job.onDone(positions, ms);
  }
}
