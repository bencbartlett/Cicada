/**
 * WHEN the git status is re-read (docs/17 item 2; docs/13 `/api/git/status`
 * is HTTP-only — there is no WS message, the client reads). The policy:
 *
 * - on connect (`hello`): now;
 * - after a write the server confirmed (a `delta`, a barrier `snapshot`):
 *   once, at most `debounceMs` after the LAST one — a burst of edits is one
 *   read, and the read always sees the persisted file (the server writes
 *   before it broadcasts);
 * - on window focus: now — the user may have run git in a shell;
 * - NEVER on a timer while idle: every read shells out to git on the
 *   server, and an idle app must not keep a project's git busy (nor wake
 *   a laptop disk).
 *
 * One read in flight at a time; a trigger that lands mid-read runs one
 * more read when it finishes (so the answer is never older than the last
 * trigger), and reads therefore arrive in order. No DOM here — the
 * connection module installs the focus listener; tests drive the methods
 * with fake timers.
 */
export class GitRefreshPolicy {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight = false;
  private again = false;
  private disposed = false;
  /** Reads started (tests and the debug handle). */
  reads = 0;

  constructor(
    private readonly read: () => Promise<void>,
    readonly debounceMs: number = 1000,
  ) {}

  /** The socket said hello (first connect or a reconnect): read now. */
  onConnected(): void {
    this.now();
  }

  /**
   * The server confirmed a write — a `delta` (text and/or sidecar: both are
   * in the commit scope, so a node move dirties the tree too) or a barrier
   * `snapshot` (external change, `apply_text`, `git revert`). Coalesced: the
   * read runs `debounceMs` after the last call.
   */
  onWrite(): void {
    if (this.disposed) return;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      this.run();
    }, this.debounceMs);
  }

  /** The window regained focus: read now (a shell may have moved HEAD). */
  onFocus(): void {
    this.now();
  }

  /** Read now (a commit or revert just landed; a manual refresh). */
  now(): void {
    if (this.disposed) return;
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.run();
  }

  /** Is a read pending (debounce armed) or in flight? */
  get busy(): boolean {
    return this.timer !== null || this.inFlight;
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
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
