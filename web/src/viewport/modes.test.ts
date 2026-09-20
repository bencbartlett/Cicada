/**
 * The viewport modes' pure rules (docs/16 §Viewport conventions; wave 5 V1):
 * the floating panel's clamp, default, move and resize; what a stored mode
 * and rect load as; the window decision over a real / fake / missing
 * `documentPictureInPicture`; and the mode reducer's whole table.
 */
import { describe, expect, it } from "vitest";
import {
  clampFloating,
  defaultFloating,
  FLOATING_MARGIN,
  FLOATING_MIN_HEIGHT,
  FLOATING_MIN_WIDTH,
  floatingRectFrom,
  moveFloating,
  resizeFloating,
  stepMode,
  viewportModeFrom,
  windowChoice,
  windowSize,
  DEFAULT_WINDOW_SIZE,
  type ModeState,
} from "./modes";

const area = { width: 1000, height: 600 };

describe("clampFloating", () => {
  it("keeps a panel that fits", () => {
    expect(clampFloating({ x: 100, y: 50, width: 400, height: 300 }, area)).toEqual({ x: 100, y: 50, width: 400, height: 300 });
  });
  it("holds the minimum 240 × 160", () => {
    expect(clampFloating({ x: 0, y: 0, width: 10, height: 10 }, area)).toEqual({ x: 0, y: 0, width: FLOATING_MIN_WIDTH, height: FLOATING_MIN_HEIGHT });
    expect(FLOATING_MIN_WIDTH).toBe(240);
    expect(FLOATING_MIN_HEIGHT).toBe(160);
  });
  it("caps the size at the area and pulls the place back inside", () => {
    expect(clampFloating({ x: 900, y: 500, width: 2000, height: 2000 }, area)).toEqual({ x: 0, y: 0, width: 1000, height: 600 });
    expect(clampFloating({ x: 900, y: 500, width: 300, height: 200 }, area)).toEqual({ x: 700, y: 400, width: 300, height: 200 });
    expect(clampFloating({ x: -50, y: -50, width: 300, height: 200 }, area)).toEqual({ x: 0, y: 0, width: 300, height: 200 });
  });
  it("the minimum wins over an area smaller than it (the panel overflows at 0 rather than shrinking below usable)", () => {
    expect(clampFloating({ x: 30, y: 30, width: 300, height: 200 }, { width: 200, height: 100 })).toEqual({ x: 0, y: 0, width: 240, height: 160 });
  });
  it("rounds to integers", () => {
    expect(clampFloating({ x: 10.4, y: 10.6, width: 300.5, height: 200.49 }, area)).toEqual({ x: 10, y: 11, width: 301, height: 200 });
  });
});

describe("defaultFloating", () => {
  it("is 40 % of the area each way, one margin in from the lower-right corner", () => {
    const rect = defaultFloating(area);
    expect(rect).toEqual({ x: 1000 - 400 - FLOATING_MARGIN, y: 600 - 240 - FLOATING_MARGIN, width: 400, height: 240 });
  });
  it("never goes below the minimum", () => {
    expect(defaultFloating({ width: 300, height: 200 })).toEqual({ x: 300 - 240 - FLOATING_MARGIN, y: 200 - 160 - FLOATING_MARGIN, width: 240, height: 160 });
  });
});

describe("moveFloating / resizeFloating", () => {
  const rect = { x: 100, y: 100, width: 400, height: 300 };
  it("a move keeps the size and clamps the place", () => {
    expect(moveFloating(rect, -30, 20, area)).toEqual({ x: 70, y: 120, width: 400, height: 300 });
    expect(moveFloating(rect, 5000, -5000, area)).toEqual({ x: 600, y: 0, width: 400, height: 300 });
  });
  it("a resize keeps the place, holds the minimum and stops at the area's edge", () => {
    expect(resizeFloating(rect, 50, -20, area)).toEqual({ x: 100, y: 100, width: 450, height: 280 });
    expect(resizeFloating(rect, -1000, -1000, area)).toEqual({ x: 100, y: 100, width: 240, height: 160 });
    // Overshooting the corner grows to the edge and never slides the panel left.
    expect(resizeFloating(rect, 5000, 5000, area)).toEqual({ x: 100, y: 100, width: 900, height: 500 });
  });
});

describe("stored values → this build's", () => {
  it("a stored `window` loads as split (the PiP window does not survive a reload; reopening needs a click); unknown is split", () => {
    expect(viewportModeFrom("floating")).toBe("floating");
    expect(viewportModeFrom("split")).toBe("split");
    expect(viewportModeFrom("window")).toBe("split");
    expect(viewportModeFrom("popout")).toBe("split");
    expect(viewportModeFrom(undefined)).toBe("split");
  });
  it("a floating rect needs four finite numbers", () => {
    expect(floatingRectFrom({ x: 1, y: 2, width: 300, height: 200 })).toEqual({ x: 1, y: 2, width: 300, height: 200 });
    expect(floatingRectFrom({ x: 1, y: 2, width: "300", height: 200 })).toBeNull();
    expect(floatingRectFrom({ x: 1, y: 2, width: Infinity, height: 200 })).toBeNull();
    expect(floatingRectFrom({ x: 1, y: 2, width: 300 })).toBeNull();
    expect(floatingRectFrom(null)).toBeNull();
    expect(floatingRectFrom([1, 2, 3, 4])).toBeNull();
    expect(floatingRectFrom("x")).toBeNull();
  });
  it("the window's requested size is the floating size, else the default", () => {
    expect(windowSize({ x: 0, y: 0, width: 500, height: 320 })).toEqual({ width: 500, height: 320 });
    expect(windowSize(null)).toEqual(DEFAULT_WINDOW_SIZE);
  });
});

describe("windowChoice", () => {
  it("the PiP API when requestWindow is a function", () => {
    const api = { requestWindow: () => Promise.reject(new Error("fake")), window: null };
    expect(windowChoice({ documentPictureInPicture: api })).toEqual({ kind: "pip", api });
  });
  it("the observer pop-out with the reason when the API is missing or not an API", () => {
    for (const value of [undefined, null, {}, { requestWindow: 3 }, "yes"]) {
      const choice = windowChoice({ documentPictureInPicture: value });
      expect(choice.kind, String(value)).toBe("popout");
      if (choice.kind === "popout") expect(choice.reason).toMatch(/no picture-in-picture window/);
    }
    expect(windowChoice({}).kind).toBe("popout");
  });
});

describe("stepMode (the mode reducer)", () => {
  const split: ModeState = { mode: "split", windowOpen: false };
  const floating: ModeState = { mode: "floating", windowOpen: false };
  const windowed: ModeState = { mode: "window", windowOpen: true };

  it("split ↔ floating: a mode change and nothing else", () => {
    expect(stepMode(split, { kind: "choose", mode: "floating", pip: true })).toEqual({ state: floating, effects: [] });
    expect(stepMode(floating, { kind: "choose", mode: "split", pip: false })).toEqual({ state: split, effects: [] });
    expect(stepMode(split, { kind: "choose", mode: "split", pip: true })).toEqual({ state: split, effects: [] });
  });
  it("choosing window asks for the PiP window and changes nothing yet; window_opened is what enters the mode", () => {
    expect(stepMode(split, { kind: "choose", mode: "window", pip: true })).toEqual({ state: split, effects: ["open_window"] });
    expect(stepMode(floating, { kind: "choose", mode: "window", pip: true })).toEqual({ state: floating, effects: ["open_window"] });
    expect(stepMode(split, { kind: "window_opened" })).toEqual({ state: windowed, effects: [] });
    expect(stepMode(floating, { kind: "window_opened" })).toEqual({ state: windowed, effects: [] });
  });
  it("choosing window without the API is the observer pop-out and NO change of mode", () => {
    expect(stepMode(split, { kind: "choose", mode: "window", pip: false })).toEqual({ state: split, effects: ["pop_out"] });
    expect(stepMode(floating, { kind: "choose", mode: "window", pip: false })).toEqual({ state: floating, effects: ["pop_out"] });
  });
  it("choosing window while the window is open is a no-op (never a second window)", () => {
    expect(stepMode(windowed, { kind: "choose", mode: "window", pip: true })).toEqual({ state: windowed, effects: [] });
  });
  it("leaving window by the control reclaims the element first, then closes the window, and lands on the CHOSEN mode", () => {
    expect(stepMode(windowed, { kind: "choose", mode: "floating", pip: true })).toEqual({
      state: floating,
      effects: ["reclaim", "close_window"],
    });
    expect(stepMode(windowed, { kind: "choose", mode: "split", pip: true })).toEqual({ state: split, effects: ["reclaim", "close_window"] });
  });
  it("the window closing on its own returns to split (the contract) and reclaims; when none is open it is nothing", () => {
    expect(stepMode(windowed, { kind: "window_closed" })).toEqual({ state: split, effects: ["reclaim"] });
    expect(stepMode(floating, { kind: "window_closed" })).toEqual({ state: floating, effects: [] });
    expect(stepMode(split, { kind: "window_closed" })).toEqual({ state: split, effects: [] });
  });
  it("a refused request changes nothing (the controller says why)", () => {
    expect(stepMode(split, { kind: "window_refused" })).toEqual({ state: split, effects: [] });
    expect(stepMode(floating, { kind: "window_refused" })).toEqual({ state: floating, effects: [] });
  });
  it("the host leaving the tree while the window is open reclaims and closes it, back to split; otherwise nothing", () => {
    expect(stepMode(windowed, { kind: "host_unmounted" })).toEqual({ state: split, effects: ["reclaim", "close_window"] });
    expect(stepMode(floating, { kind: "host_unmounted" })).toEqual({ state: floating, effects: [] });
  });
});
