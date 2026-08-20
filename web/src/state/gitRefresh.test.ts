/**
 * The git status refresh policy (docs/17 item 2): reads on connect, once
 * per burst of writes (≤ the debounce after the last), on focus — and NEVER
 * on a timer while idle. Fake timers; the read is a spy.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GitRefreshPolicy } from "./gitRefresh";

describe("GitRefreshPolicy", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reads once on connect and never again while idle", async () => {
    const read = vi.fn(() => Promise.resolve());
    const policy = new GitRefreshPolicy(read, 1000);
    policy.onConnected();
    expect(read).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(read, "an idle app must not poll").toHaveBeenCalledTimes(1);
    expect(policy.busy).toBe(false);
    policy.dispose();
  });

  it("coalesces a burst of writes into ONE read, at most the debounce after the last", async () => {
    const read = vi.fn(() => Promise.resolve());
    const policy = new GitRefreshPolicy(read, 1000);
    for (let i = 0; i < 5; i += 1) {
      policy.onWrite();
      await vi.advanceTimersByTimeAsync(300);
    }
    // 1.5 s in, the last write was 300 ms ago: still armed, nothing read.
    expect(read).toHaveBeenCalledTimes(0);
    expect(policy.busy).toBe(true);
    await vi.advanceTimersByTimeAsync(700);
    expect(read, "one read per burst").toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(read, "no re-read without a new write").toHaveBeenCalledTimes(1);
    // A new write (a new text_hash on the server) → exactly one more read.
    policy.onWrite();
    await vi.advanceTimersByTimeAsync(1000);
    expect(read).toHaveBeenCalledTimes(2);
    policy.dispose();
  });

  it("reads immediately on focus and on connect, cancelling an armed debounce", async () => {
    const read = vi.fn(() => Promise.resolve());
    const policy = new GitRefreshPolicy(read, 1000);
    policy.onWrite();
    policy.onFocus();
    expect(read).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect(read, "the focus read replaced the pending one").toHaveBeenCalledTimes(1);
    policy.dispose();
  });

  it("runs one more read after a trigger that lands mid-read, never two at once", async () => {
    let resolveFirst: (() => void) | null = null;
    let calls = 0;
    const read = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          calls += 1;
          if (calls === 1) resolveFirst = resolve;
          else resolve();
        }),
    );
    const policy = new GitRefreshPolicy(read, 1000);
    policy.onConnected();
    expect(read).toHaveBeenCalledTimes(1);
    // Focus and several writes while the first read is in flight.
    policy.onFocus();
    policy.onWrite();
    await vi.advanceTimersByTimeAsync(1000);
    expect(read, "no concurrent read").toHaveBeenCalledTimes(1);
    expect(policy.busy).toBe(true);
    resolveFirst!();
    await vi.advanceTimersByTimeAsync(0);
    expect(read, "exactly one follow-up read for the triggers that arrived").toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(5000);
    expect(read).toHaveBeenCalledTimes(2);
    expect(policy.reads).toBe(2);
    policy.dispose();
  });

  it("a failing read does not stop later reads", async () => {
    const read = vi.fn().mockRejectedValueOnce(new Error("boom")).mockResolvedValue(undefined);
    const policy = new GitRefreshPolicy(read, 1000);
    policy.onConnected();
    await vi.advanceTimersByTimeAsync(0);
    policy.onWrite();
    await vi.advanceTimersByTimeAsync(1000);
    expect(read).toHaveBeenCalledTimes(2);
    policy.dispose();
  });

  it("a disposed policy ignores every trigger", async () => {
    const read = vi.fn(() => Promise.resolve());
    const policy = new GitRefreshPolicy(read, 1000);
    policy.onWrite();
    policy.dispose();
    policy.onConnected();
    policy.onFocus();
    await vi.advanceTimersByTimeAsync(5000);
    expect(read).toHaveBeenCalledTimes(0);
  });
});
