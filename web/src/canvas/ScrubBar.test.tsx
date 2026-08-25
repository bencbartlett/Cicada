// @vitest-environment jsdom
/**
 * The buffer bar (docs/16 §Sliders; v0.1 item 5 S2), rendered for real: a
 * segment per position, the warm ones filled, the thumb's position marked,
 * the pulse class while warming, the warn state when capped, and nothing at
 * all for a slider that is off or cannot scrub-cache.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ScrubView } from "../protocol/messages";
import { ScrubBar } from "./ScrubBar";

const warmingView: ScrubView = { on: true, positions: 19, warmed: [5, 6, 7], warming: true, bytes: 300_000 };

function segments(bar: HTMLElement) {
  return Array.from(bar.querySelectorAll<HTMLElement>(".scrub-seg"));
}

describe("the scrub buffer bar", () => {
  afterEach(cleanup);

  it("draws one segment per position, fills the warm ones, rings the current one, and pulses while warming", () => {
    render(<ScrubBar node="size" scrub={warmingView} value={2.0} min={0.5} step={0.25} />);
    const bar = screen.getByTestId("scrub-bar-size");
    expect(bar.className).toBe("scrub-bar warming");
    expect(bar.dataset).toMatchObject({ positions: "19", warmed: "3", warming: "true", current: "6" });
    expect(bar.dataset.capped).toBeUndefined();
    const segs = segments(bar);
    expect(segs).toHaveLength(19);
    expect(segs.filter((s) => s.classList.contains("warm")).map((s) => s.dataset.index)).toEqual(["5", "6", "7"]);
    expect(segs.filter((s) => s.classList.contains("current")).map((s) => s.dataset.index)).toEqual(["6"]);
    expect(bar.title).toBe("scrub cache · 3 / 19 positions warm · warming while the app is idle… · 293.0 KB stored");
    expect(bar.getAttribute("aria-label")).toBe(bar.title);
  });

  it("the current marker follows the value the thumb shows, snapped to the nearest notch", () => {
    const { rerender } = render(<ScrubBar node="size" scrub={warmingView} value={0.5} min={0.5} step={0.25} />);
    expect(screen.getByTestId("scrub-bar-size").dataset.current).toBe("0");
    rerender(<ScrubBar node="size" scrub={warmingView} value={4.9} min={0.5} step={0.25} />);
    expect(screen.getByTestId("scrub-bar-size").dataset.current).toBe("18");
    rerender(<ScrubBar node="size" scrub={warmingView} value={2.13} min={0.5} step={0.25} />);
    expect(screen.getByTestId("scrub-bar-size").dataset.current).toBe("7");
  });

  it("every position warm: no pulse; capped: the warn state, the warm ones stay", () => {
    const all = Array.from({ length: 19 }, (_, i) => i);
    const { rerender } = render(
      <ScrubBar node="size" scrub={{ ...warmingView, warmed: all, warming: false }} value={2} min={0.5} step={0.25} />,
    );
    let bar = screen.getByTestId("scrub-bar-size");
    expect(bar.className).toBe("scrub-bar");
    expect(bar.dataset).toMatchObject({ warmed: "19", warming: "false" });
    expect(segments(bar).every((s) => s.classList.contains("warm"))).toBe(true);
    rerender(<ScrubBar node="size" scrub={{ ...warmingView, warming: false, capped: true }} value={2} min={0.5} step={0.25} />);
    bar = screen.getByTestId("scrub-bar-size");
    expect(bar.className).toBe("scrub-bar capped");
    expect(bar.dataset.capped).toBe("true");
    expect(segments(bar).filter((s) => s.classList.contains("warm"))).toHaveLength(3);
    expect(bar.title).toMatch(/capped at the 256 MiB budget/);
  });

  it("renders nothing for a slider that is off, ineligible, or has no scrub view", () => {
    const { container, rerender } = render(
      <ScrubBar node="size" scrub={{ ...warmingView, on: false }} value={2} min={0.5} step={0.25} />,
    );
    expect(container.innerHTML).toBe("");
    rerender(
      <ScrubBar
        node="size"
        scrub={{ on: true, positions: 0, warmed: [], warming: false, bytes: 0, ineligible: "too many positions (51 > 32)" }}
        value={2}
        min={0.5}
        step={0.25}
      />,
    );
    expect(container.innerHTML).toBe("");
    rerender(<ScrubBar node="size" scrub={undefined} value={2} min={0.5} step={0.25} />);
    expect(container.innerHTML).toBe("");
  });
});
