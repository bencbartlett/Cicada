// @vitest-environment jsdom
/**
 * The tooltip box rendered for real (docs/17 wave 5 T1): nothing until a
 * titled element has been hovered for 250 ms, then one `role="tooltip"` box
 * with the title's text placed against the element, gone on leave and on
 * unmount — with the hovered title parked and restored around it.
 */
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipLayer } from "./TooltipLayer";
import { PARKED_ATTR, TOOLTIP_DELAY_MS, TOOLTIP_GAP_PX, TOOLTIP_MARGIN_PX } from "./tooltip";

const TITLE = "undo: place sphere_1 (Ctrl+Z)";

function move(from: Element | null, to: Element | null) {
  act(() => {
    if (from !== null) from.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: to }));
    if (to !== null) to.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, relatedTarget: from }));
  });
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("the tooltip layer", () => {
  it("renders the hovered title in one box after the delay, placed below the element, and takes it down on leave", () => {
    const { unmount } = render(
      <>
        <div data-testid="pane">
          <button title={TITLE} data-testid="undo">
            undo
          </button>
        </div>
        <TooltipLayer />
      </>,
    );
    const undo = screen.getByTestId("undo");
    expect(screen.queryByRole("tooltip")).toBeNull();

    move(screen.getByTestId("pane"), undo);
    act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS - 1));
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(undo.getAttribute("title")).toBe("");
    act(() => vi.advanceTimersByTime(1));
    const box = screen.getByRole("tooltip");
    expect(box.textContent).toBe(TITLE);
    expect(box.dataset.testid).toBe("tooltip");
    // jsdom lays nothing out (every rect is zero): the placement is the
    // pure function's on those zeros — below, at the margin and the gap.
    expect(box.dataset.side).toBe("below");
    expect(box.style.left).toBe(`${TOOLTIP_MARGIN_PX}px`);
    expect(box.style.top).toBe(`${TOOLTIP_GAP_PX}px`);
    expect(screen.getAllByRole("tooltip")).toHaveLength(1);

    move(undo, screen.getByTestId("pane"));
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(undo.getAttribute("title")).toBe(TITLE);
    expect(undo.hasAttribute(PARKED_ATTR)).toBe(false);

    // Unmounting mid-hover puts the title back too.
    move(screen.getByTestId("pane"), undo);
    act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
    expect(screen.getByRole("tooltip").textContent).toBe(TITLE);
    unmount();
    expect(undo.getAttribute("title")).toBe(TITLE);
  });
});
