// @vitest-environment jsdom
/**
 * The viewport's wrapper across the three modes (docs/16 §Viewport
 * conventions; wave 5 V1), the `Viewport` inside it mocked (jsdom has no
 * WebGL): `split` is a plain pane; `floating` is an absolutely placed panel
 * at the stored rect clamped into the measured work area (the default when
 * none is stored), whose title strip drags it and whose corner resizes it —
 * the rect written to the DOM while the pointer moves, persisted to the
 * settings on release, never below 240 × 160; `window` parks the wrapper;
 * the placeholder's click asks for `split`.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCicada } from "../state/store";
import { ViewportFrame, ViewportPlaceholder } from "./ViewportFrame";

vi.mock("./Viewport", () => ({ Viewport: () => <div data-testid="viewport" /> }));

/** A `ResizeObserver` the test fires by hand: it records what was observed, and `fire()` runs the callback as a real one would after a layout change. */
class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  observed: Element[] = [];
  constructor(private readonly callback: ResizeObserverCallback) {
    FakeResizeObserver.instances.push(this);
  }
  observe(el: Element): void {
    this.observed.push(el);
  }
  disconnect(): void {
    this.observed = [];
  }
  unobserve(): void {}
  fire(): void {
    this.callback([], this as unknown as ResizeObserver);
  }
}

/** Declare the work area's size (jsdom lays nothing out). */
function sizeArea(el: HTMLElement, width: number, height: number): void {
  Object.defineProperty(el, "clientWidth", { value: width, configurable: true });
  Object.defineProperty(el, "clientHeight", { value: height, configurable: true });
}

/** A work area of `width` × `height`. */
function area(width: number, height: number) {
  const el = document.createElement("div");
  sizeArea(el, width, height);
  document.body.append(el);
  const ref = createRef<HTMLDivElement>() as React.MutableRefObject<HTMLDivElement>;
  ref.current = el;
  return ref;
}

const frame = () => screen.getByTestId("viewport-pane");
const rectOf = (el: HTMLElement) => ({
  x: parseInt(el.style.left, 10),
  y: parseInt(el.style.top, 10),
  width: parseInt(el.style.width, 10),
  height: parseInt(el.style.height, 10),
});

describe("ViewportFrame", () => {
  beforeEach(() => {
    FakeResizeObserver.instances = [];
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    useCicada.getState().updateSettings({ viewportMode: "split", floatingViewport: null });
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("split: a pane, the viewport inside, no strip, no corner, no inline place", () => {
    render(<ViewportFrame mode="split" areaRef={area(1000, 600)} />);
    expect(frame().className).toBe("pane");
    expect(frame().dataset.mode).toBe("split");
    expect(frame().getAttribute("style")).toBeNull();
    expect(screen.getByTestId("viewport")).toBeTruthy();
    expect(screen.queryByTestId("viewport-float-title")).toBeNull();
    expect(screen.queryByTestId("viewport-float-corner")).toBeNull();
  });

  it("window: the wrapper is parked (the element itself is in the PiP document)", () => {
    render(<ViewportFrame mode="window" areaRef={area(1000, 600)} />);
    expect(frame().className).toBe("viewport-parked");
    expect(screen.queryByTestId("viewport-float-title")).toBeNull();
  });

  it("floating without a stored rect: the default — 40 % of the area, in the lower-right corner — with strip and corner", () => {
    render(<ViewportFrame mode="floating" areaRef={area(1000, 600)} />);
    expect(frame().className).toBe("viewport-float");
    expect(rectOf(frame())).toEqual({ x: 588, y: 348, width: 400, height: 240 });
    expect(screen.getByTestId("viewport-float-title")).toBeTruthy();
    expect(screen.getByTestId("viewport-float-corner")).toBeTruthy();
    expect(screen.getByTestId("viewport")).toBeTruthy();
  });

  it("floating with a stored rect: clamped into the area at render (a shrunken window never hides the panel)", () => {
    useCicada.getState().updateSettings({ floatingViewport: { x: 900, y: 500, width: 300, height: 200 } });
    render(<ViewportFrame mode="floating" areaRef={area(1000, 600)} />);
    expect(rectOf(frame())).toEqual({ x: 700, y: 400, width: 300, height: 200 });
  });

  it("the work area shrinking re-clamps the panel through the observer it watches the area with; the stored rect stays, so growing it back restores the place", () => {
    useCicada.getState().updateSettings({ floatingViewport: { x: 600, y: 300, width: 400, height: 300 } });
    const ref = area(1000, 600);
    render(<ViewportFrame mode="floating" areaRef={ref} />);
    expect(rectOf(frame())).toEqual({ x: 600, y: 300, width: 400, height: 300 });
    // The area is observed — not just measured once at mount.
    const observer = FakeResizeObserver.instances.at(-1);
    expect(observer, "a ResizeObserver was created for the floating mode").toBeDefined();
    expect(observer!.observed).toEqual([ref.current]);
    // The window shrinks: the panel's right/bottom edges would be 300/200 px outside — it is pulled back in.
    sizeArea(ref.current, 700, 400);
    act(() => observer!.fire());
    expect(rectOf(frame())).toEqual({ x: 300, y: 100, width: 400, height: 300 });
    expect(useCicada.getState().settings.floatingViewport).toEqual({ x: 600, y: 300, width: 400, height: 300 });
    // Smaller than the panel: the size is cut to the area, the place 0.
    sizeArea(ref.current, 300, 200);
    act(() => observer!.fire());
    expect(rectOf(frame())).toEqual({ x: 0, y: 0, width: 300, height: 200 });
    // Back to the original area: the user's place and size again.
    sizeArea(ref.current, 1000, 600);
    act(() => observer!.fire());
    expect(rectOf(frame())).toEqual({ x: 600, y: 300, width: 400, height: 300 });
  });

  it("leaving floating disconnects the area's observer", () => {
    const ref = area(1000, 600);
    const { rerender } = render(<ViewportFrame mode="floating" areaRef={ref} />);
    const observer = FakeResizeObserver.instances.at(-1)!;
    expect(observer.observed).toEqual([ref.current]);
    rerender(<ViewportFrame mode="split" areaRef={ref} />);
    expect(observer.observed).toEqual([]);
  });

  it("the title strip drags: the DOM follows the pointer, the settings get the rect on release", () => {
    useCicada.getState().updateSettings({ floatingViewport: { x: 100, y: 100, width: 400, height: 300 } });
    render(<ViewportFrame mode="floating" areaRef={area(1000, 600)} />);
    const title = screen.getByTestId("viewport-float-title");
    fireEvent.pointerDown(title, { clientX: 200, clientY: 110, button: 0, pointerId: 1 });
    fireEvent.pointerMove(title, { clientX: 160, clientY: 150 });
    expect(rectOf(frame())).toEqual({ x: 60, y: 140, width: 400, height: 300 });
    // Nothing persisted while the pointer is down.
    expect(useCicada.getState().settings.floatingViewport).toEqual({ x: 100, y: 100, width: 400, height: 300 });
    fireEvent.pointerMove(title, { clientX: 5000, clientY: 5000 });
    expect(rectOf(frame())).toEqual({ x: 600, y: 300, width: 400, height: 300 });
    act(() => {
      fireEvent.pointerUp(title);
    });
    expect(useCicada.getState().settings.floatingViewport).toEqual({ x: 600, y: 300, width: 400, height: 300 });
    expect(rectOf(frame())).toEqual({ x: 600, y: 300, width: 400, height: 300 });
  });

  it("the corner resizes: the place stays, the size follows, never below 240 × 160 nor past the area", () => {
    useCicada.getState().updateSettings({ floatingViewport: { x: 100, y: 100, width: 400, height: 300 } });
    render(<ViewportFrame mode="floating" areaRef={area(1000, 600)} />);
    const corner = screen.getByTestId("viewport-float-corner");
    fireEvent.pointerDown(corner, { clientX: 500, clientY: 400, button: 0, pointerId: 1 });
    fireEvent.pointerMove(corner, { clientX: 560, clientY: 380 });
    expect(rectOf(frame())).toEqual({ x: 100, y: 100, width: 460, height: 280 });
    fireEvent.pointerMove(corner, { clientX: -1000, clientY: -1000 });
    expect(rectOf(frame())).toEqual({ x: 100, y: 100, width: 240, height: 160 });
    fireEvent.pointerMove(corner, { clientX: 5000, clientY: 5000 });
    expect(rectOf(frame())).toEqual({ x: 100, y: 100, width: 900, height: 500 });
    act(() => {
      fireEvent.pointerUp(corner);
    });
    expect(useCicada.getState().settings.floatingViewport).toEqual({ x: 100, y: 100, width: 900, height: 500 });
  });

  it("a secondary button starts no drag", () => {
    useCicada.getState().updateSettings({ floatingViewport: { x: 100, y: 100, width: 400, height: 300 } });
    render(<ViewportFrame mode="floating" areaRef={area(1000, 600)} />);
    const title = screen.getByTestId("viewport-float-title");
    fireEvent.pointerDown(title, { clientX: 200, clientY: 110, button: 2, pointerId: 1 });
    fireEvent.pointerMove(title, { clientX: 160, clientY: 150 });
    expect(rectOf(frame())).toEqual({ x: 100, y: 100, width: 400, height: 300 });
    act(() => {
      fireEvent.pointerUp(title);
    });
    expect(useCicada.getState().settings.floatingViewport).toEqual({ x: 100, y: 100, width: 400, height: 300 });
  });

  it("the mode changing keeps the one wrapper element (the viewport inside never remounts)", () => {
    const ref = area(1000, 600);
    const { rerender } = render(<ViewportFrame mode="split" areaRef={ref} />);
    const wrapper = frame();
    const viewport = screen.getByTestId("viewport");
    rerender(<ViewportFrame mode="floating" areaRef={ref} />);
    expect(frame()).toBe(wrapper);
    expect(screen.getByTestId("viewport")).toBe(viewport);
    expect(wrapper.className).toBe("viewport-float");
    rerender(<ViewportFrame mode="window" areaRef={ref} />);
    expect(frame()).toBe(wrapper);
    expect(screen.getByTestId("viewport")).toBe(viewport);
    rerender(<ViewportFrame mode="split" areaRef={ref} />);
    expect(frame()).toBe(wrapper);
    expect(screen.getByTestId("viewport")).toBe(viewport);
    expect(wrapper.getAttribute("style")).toBe("");
  });
});

describe("ViewportPlaceholder", () => {
  afterEach(cleanup);
  it("says where the viewport went and a click asks for split", () => {
    useCicada.getState().updateSettings({ viewportMode: "window" });
    render(<ViewportPlaceholder />);
    const button = screen.getByTestId("viewport-placeholder");
    expect(button.textContent).toBe("the viewport is in its own window — click to bring it back");
    fireEvent.click(button);
    expect(useCicada.getState().settings.viewportMode).toBe("split");
  });
});
