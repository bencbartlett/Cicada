// @vitest-environment jsdom
/**
 * The transport bar against a seeded store (docs/13 §Animation transport;
 * docs/17 item 4): shown only with time params; every control is the
 * intent the docs name and nothing flips locally — the server's broadcast
 * does; the counter and the thumb extrapolate the playhead while playing
 * and freeze on the server's frame while paused; a scrub shows the sought
 * frame at once and hands back to the view on the answer — the broadcast
 * of an accepted seek, or the `error` of a refused one (which broadcasts
 * nothing); observers get the bar read-only with the reason on hover.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientMessage, ErrorPayload, ServerEnvelope, TransportView } from "../protocol/messages";
import { useCicada } from "../state/store";
import { DISPLAY_TICK_MS, nowMs } from "../state/transport";
import { TransportBar } from "./TransportBar";

const orbit: TransportView = {
  playing: false,
  speed: 1,
  t_ms: 1000,
  frame: 30,
  frames: 120,
  period_ms: 4000,
  driven: [{ node: "spin", port: "frame", signal: "frame", loop: { frames: 120, period_ms: 4000 } }],
};

const broadcast = (view: TransportView): ServerEnvelope => ({ v: 1, seq: 9, type: "transport", payload: view });
/** The server's unicast refusal of this client's intent `id` — the whole answer to a refused control. */
const refusal = (id: string, kind: ErrorPayload["kind"], message: string): ServerEnvelope => ({
  v: 1,
  seq: 10,
  type: "error",
  payload: { intent_id: id, kind, message },
});

function seed(view: TransportView, receivedAt = nowMs()): void {
  useCicada.setState({ transport: { view, receivedAt } });
}

describe("TransportBar", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({ connection: "open", role: "writer", transport: null, notices: [], lastError: null });
    // Ids like the real client's: "1", "2", … in sending order.
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return String(sent.length);
    });
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("renders nothing before the first snapshot and nothing without time params (driven [])", () => {
    const { rerender } = render(<TransportBar />);
    expect(screen.queryByTestId("transport")).toBeNull();
    act(() => seed({ ...orbit, driven: [] }));
    rerender(<TransportBar />);
    expect(screen.queryByTestId("transport")).toBeNull();
    act(() => seed(orbit));
    rerender(<TransportBar />);
    expect(screen.getByTestId("transport")).toBeTruthy();
  });

  it("paused: the server's frame and time, the play button, the speed, and what it drives", () => {
    seed(orbit, 0); // an old stamp changes nothing while paused
    render(<TransportBar />);
    const bar = screen.getByTestId("transport");
    expect(bar.dataset.playing).toBe("false");
    expect(screen.getByTestId("tr-frame").textContent).toBe("30 / 120");
    expect(screen.getByTestId("tr-time").textContent).toBe("1.00 s");
    expect((screen.getByTestId("tr-scrub") as HTMLInputElement).value).toBe("30");
    expect((screen.getByTestId("tr-scrub") as HTMLInputElement).max).toBe("119");
    const play = screen.getByTestId("tr-play") as HTMLButtonElement;
    expect(play.getAttribute("aria-label")).toBe("play");
    expect(play.getAttribute("aria-pressed")).toBe("false");
    expect(play.disabled).toBe(false);
    expect((screen.getByTestId("tr-speed") as HTMLSelectElement).value).toBe("1");
    expect(screen.getByTestId("tr-driven").textContent).toBe("drives spin.frame");
  });

  it("play / pause / reset send their intents and flip nothing until the broadcast says so", () => {
    seed(orbit);
    render(<TransportBar />);
    fireEvent.click(screen.getByTestId("tr-play"));
    expect(sent).toEqual([{ type: "transport_play", payload: {} }]);
    // Still paused in the DOM: the server owns the state.
    expect(screen.getByTestId("tr-play").getAttribute("aria-label")).toBe("play");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, playing: true })));
    expect(screen.getByTestId("tr-play").getAttribute("aria-label")).toBe("pause");
    expect(screen.getByTestId("transport").dataset.playing).toBe("true");
    fireEvent.click(screen.getByTestId("tr-play"));
    expect(sent.at(-1)).toEqual({ type: "transport_pause", payload: {} });
    const reset = screen.getByTestId("tr-reset");
    reset.focus();
    expect(document.activeElement).toBe(reset);
    fireEvent.click(reset);
    expect(sent.at(-1)).toEqual({ type: "transport_reset", payload: {} });
    // The button gives the keyboard back: a focused button would take the
    // next Space as its click and reset again instead of playing.
    expect(document.activeElement, "reset blurs itself after the click").not.toBe(reset);
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 0, frame: 0 })));
    expect(screen.getByTestId("tr-frame").textContent).toBe("0 / 120");
    expect(screen.getByTestId("tr-time").textContent).toBe("0.00 s");
  });

  it("the speed menu offers 0.25 … 4 (plus a foreign speed), sends transport_speed, and shows the server's speed", () => {
    seed(orbit);
    render(<TransportBar />);
    const select = screen.getByTestId("tr-speed") as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => o.textContent)).toEqual(["0.25×", "0.5×", "1×", "2×", "4×"]);
    fireEvent.change(select, { target: { value: "2" } });
    expect(sent).toEqual([{ type: "transport_speed", payload: { factor: 2 } }]);
    expect(select.value, "the menu shows the server's speed until it answers").toBe("1");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, speed: 2 })));
    expect(select.value).toBe("2");
    // An agent on the socket set 1.5: the menu shows what IS.
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, speed: 1.5 })));
    expect(select.value).toBe("1.5");
    expect(Array.from(select.options).map((o) => o.textContent)).toEqual(["0.25×", "0.5×", "1×", "1.5×", "2×", "4×"]);
  });

  it("a scrub seeks per change, shows the sought frame at once, and hands back to the view on the answer", () => {
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "57" } });
    fireEvent.change(scrub, { target: { value: "58" } });
    expect(sent).toEqual([
      { type: "transport_seek", payload: { frame: 57 } },
      { type: "transport_seek", payload: { frame: 58 } },
    ]);
    expect(scrub.value).toBe("58");
    expect(screen.getByTestId("tr-frame").textContent).toBe("58 / 120");
    // The answer to frame 57 lands mid-drag: the thumb stays with the pointer.
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 1900, frame: 57 })));
    expect(scrub.value).toBe("58");
    // The answer to 58 lands, still mid-drag; the release then hands back
    // (the view names the sought frame — nothing later would).
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 1933.4, frame: 58 })));
    fireEvent.pointerUp(scrub);
    expect(scrub.value).toBe("58");
    // A later view (Esc, another client's seek) moves the thumb: it is the server's again.
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 100, frame: 3 })));
    expect(scrub.value).toBe("3");
    expect(screen.getByTestId("tr-frame").textContent).toBe("3 / 120");
  });

  it("a release before the answer keeps the sought frame until the answer clears it — no snap back", () => {
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "90" } });
    fireEvent.pointerUp(scrub);
    expect(scrub.value, "the view still says 30; the thumb must not snap back").toBe("90");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 3000, frame: 90 })));
    expect(scrub.value).toBe("90");
  });

  it("a foreign view landing mid-drag AFTER the seek's answer is what the release shows — a held frame never outlives its answer", () => {
    // Review 2026-08-21: the release handed back only when the CURRENT view
    // named the held frame, so a reload / a control by the client that took
    // the lease, landing after the seek's own broadcast, left the thumb and
    // the counter on the sought frame until the next broadcast.
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "57" } });
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 1900, frame: 57 })));
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 100, frame: 3 })));
    expect(scrub.value, "the pointer still owns the thumb").toBe("57");
    fireEvent.pointerUp(scrub);
    expect(scrub.value, "answered, so the release shows the view that stands").toBe("3");
    expect(screen.getByTestId("tr-frame").textContent).toBe("3 / 120");
    // A foreign view BEFORE the answer is not the answer: the release still
    // hands back once the answer has landed, to the view that stands (57).
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "80" } });
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 200, frame: 6 })));
    fireEvent.pointerUp(scrub);
    expect(scrub.value, "unanswered: the sought frame holds, no snap back to 6").toBe("80");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 2666.7, frame: 80 })));
    expect(scrub.value).toBe("80");
  });

  it("an arrow step (a change with no pointer) shows the sought frame, and the broadcast hands back — the playhead extrapolates again", () => {
    // The `transport` effect is the only hand-back for a keyboard seek
    // (no pointer-up to hand back on); a no-op there survived every other
    // unit test (review 2026-08-21, mutation M4).
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "performance"] });
    seed({ ...orbit, playing: true, t_ms: 0, frame: 0 }, nowMs());
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    fireEvent.change(scrub, { target: { value: "57" } });
    expect(sent).toEqual([{ type: "transport_seek", payload: { frame: 57 } }]);
    expect(scrub.value).toBe("57");
    expect(screen.getByTestId("tr-frame").textContent).toBe("57 / 120");
    // Held, not extrapolated, until the answer: the ticks move nothing.
    act(() => {
      vi.advanceTimersByTime(DISPLAY_TICK_MS * 10);
    });
    expect(screen.getByTestId("tr-frame").textContent).toBe("57 / 120");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, playing: true, t_ms: 1900, frame: 57 })));
    expect(scrub.value).toBe("57");
    // 30 ticks = 990 ms at 1× from 1.9 s → 2.89 s → frame 86: the playhead's again.
    act(() => {
      vi.advanceTimersByTime(DISPLAY_TICK_MS * 30);
    });
    expect(screen.getByTestId("tr-frame").textContent).toBe("86 / 120");
    expect(scrub.value).toBe("86");
  });

  it("released before the answer, a broadcast naming a DIFFERENT frame moves the thumb to the view", () => {
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "90" } });
    fireEvent.pointerUp(scrub);
    expect(scrub.value).toBe("90");
    // Esc / another client's seek arrives first: the server's view is what
    // the bar shows (the seek's own answer, when it comes, moves it again).
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 100, frame: 3 })));
    expect(scrub.value).toBe("3");
    expect(screen.getByTestId("tr-frame").textContent).toBe("3 / 120");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 3000, frame: 90 })));
    expect(scrub.value).toBe("90");
  });

  it("a refused seek broadcasts nothing, so its error hands the thumb back to the view (after the release if the pointer is down)", () => {
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub") as HTMLInputElement;
    // Released before the answer: the refusal (the loop shrank under the
    // pointer — frame 90 of a loop now 40 long) is the whole answer; the
    // view still says 30 and the thumb goes back to it, the notice says why.
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "90" } });
    fireEvent.pointerUp(scrub);
    expect(sent).toEqual([{ type: "transport_seek", payload: { frame: 90 } }]);
    expect(scrub.value).toBe("90");
    act(() => useCicada.getState().applyServerMessage(refusal("1", "transport", "frame 90 is outside the loop (frames 0..40)")));
    expect(scrub.value, "the refused seek's thumb hands back to the view").toBe("30");
    expect(screen.getByTestId("tr-frame").textContent).toBe("30 / 120");
    expect(useCicada.getState().notices.map((n) => [n.level, n.message])).toEqual([
      ["error", "frame 90 is outside the loop (frames 0..40)"],
    ]);

    // Refused mid-drag (the lease went to another client under the
    // pointer): the pointer keeps the thumb until the release, which hands
    // back because the last seek's answer has landed — a refusal.
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "57" } });
    fireEvent.change(scrub, { target: { value: "58" } });
    expect(sent.length).toBe(3);
    act(() => useCicada.getState().applyServerMessage(refusal("3", "lease", "read-only observer")));
    expect(scrub.value, "the pointer still owns the thumb").toBe("58");
    fireEvent.pointerUp(scrub);
    expect(scrub.value, "the release finds the last seek answered (refused) and hands back").toBe("30");

    // A refusal of an EARLIER seek is not the last seek's answer: the held
    // frame waits for the broadcast that answers the last one.
    fireEvent.pointerDown(scrub);
    fireEvent.change(scrub, { target: { value: "10" } });
    fireEvent.change(scrub, { target: { value: "11" } });
    fireEvent.pointerUp(scrub);
    act(() => useCicada.getState().applyServerMessage(refusal("4", "transport", "frame 10 is outside the loop (frames 0..10)")));
    expect(scrub.value, "the last seek (11) is unanswered").toBe("11");
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 366.7, frame: 11 })));
    expect(scrub.value).toBe("11");
    expect(screen.getByTestId("tr-frame").textContent).toBe("11 / 120");
    // An unrelated refusal (a slider's) never touches the thumb.
    act(() => useCicada.getState().applyServerMessage(refusal("99", "refused", "unrelated")));
    expect(scrub.value).toBe("11");
  });

  it("while playing, the counter and the thumb extrapolate the playhead at the speed and tick on the display clock", () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "performance"] });
    seed({ ...orbit, playing: true, speed: 2, t_ms: 0, frame: 0 }, nowMs());
    render(<TransportBar />);
    expect(screen.getByTestId("tr-frame").textContent).toBe("0 / 120");
    act(() => {
      vi.advanceTimersByTime(DISPLAY_TICK_MS * 30);
    });
    // 30 display ticks = 990 ms of wall time at 2× = 1.98 s of playhead = frame 59.
    expect(screen.getByTestId("tr-frame").textContent).toBe("59 / 120");
    expect((screen.getByTestId("tr-scrub") as HTMLInputElement).value).toBe("59");
    expect(screen.getByTestId("tr-time").textContent).toBe("1.98 s");
    // A broadcast re-anchors: pause at 1.25 s → frame 37, and the ticker stops.
    act(() => useCicada.getState().applyServerMessage(broadcast({ ...orbit, t_ms: 1250, frame: 37 })));
    expect(screen.getByTestId("tr-frame").textContent).toBe("37 / 120");
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.getByTestId("tr-frame").textContent).toBe("37 / 120");
  });

  it("observers see the bar live with every control disabled and the reason on hover", () => {
    useCicada.setState({ role: "observer" });
    seed({ ...orbit, playing: true });
    render(<TransportBar />);
    for (const id of ["tr-play", "tr-reset", "tr-scrub", "tr-speed"]) {
      const control = screen.getByTestId(id) as HTMLButtonElement | HTMLInputElement | HTMLSelectElement;
      expect(control.disabled, id).toBe(true);
      expect(control.title, id).toMatch(/read-only observer — take the lease to drive the transport/);
    }
    expect(screen.getByTestId("tr-play").getAttribute("aria-label")).toBe("pause");
    expect(screen.getByTestId("tr-frame").textContent).toMatch(/\/ 120$/);
    fireEvent.click(screen.getByTestId("tr-play"));
    expect(sent).toEqual([]);
  });

  it("Space on the focused scrubber is the transport hotkey (the window router leaves controls alone)", () => {
    seed(orbit);
    render(<TransportBar />);
    const scrub = screen.getByTestId("tr-scrub");
    fireEvent.keyUp(scrub, { key: " ", code: "Space" });
    expect(sent).toEqual([{ type: "transport_play", payload: {} }]);
  });
});
