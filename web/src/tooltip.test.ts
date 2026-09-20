// @vitest-environment jsdom
/**
 * The tooltip controller (docs/16 §Theme and visual language; docs/17 wave 5
 * T1) against a real DOM under fake timers: the box at 250 ms and not at
 * 249, the title parked while hovered and back on leave, an element's own
 * descendants keeping the hover, an ancestor's title serving an untitled
 * child, an empty title showing nothing, a press or Esc dismissing without
 * consuming, a title rewritten or removed under the pointer, newlines
 * kept, a disabled control treated like any other, and the placement math.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  installTooltips,
  PARKED_ATTR,
  placeTooltip,
  TOOLTIP_DELAY_MS,
  type TooltipController,
  type TooltipShown,
} from "./tooltip";

const UNDO = "undo: wire size.out → sphere_1.radius (Ctrl+Z)";
const REDO = "nothing to redo (Ctrl+Shift+Z / Ctrl+Y)";
const TWO_LINES = "out: [Number] — the values\n[1, 2, 3]";

let tooltips: TooltipController;
let seen: (TooltipShown | null)[];

function el(id: string): Element {
  const found = document.getElementById(id);
  if (found === null) throw new Error(`no #${id}`);
  return found;
}

// The browser's order on a move from `from` to `to`: `pointerout` on the
// old element with the new one related, then `pointerover` on the new one.
function move(from: Element | null, to: Element | null) {
  if (from !== null) from.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: to }));
  if (to !== null) to.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, relatedTarget: from }));
}

// MutationObserver delivery is a microtask; the fake timers leave those alone.
async function flush() {
  for (let i = 0; i < 3; i += 1) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  document.body.innerHTML = `
    <div id="pane">
      <span id="wrap" title="3 undoable">
        <button id="undo" title="${UNDO}"><i id="glyph">↶</i> undo</button>
        <button id="redo" title="${REDO}" disabled>↷ redo</button>
        <em id="plain">·</em>
      </span>
      <b id="empty" title="">no tooltip here</b>
      <div id="port" title="${TWO_LINES.replace("\n", "&#10;")}">out</div>
    </div>`;
  seen = [];
  tooltips = installTooltips(document);
  tooltips.subscribe((shown) => seen.push(shown));
});

afterEach(() => {
  tooltips.dispose();
  vi.useRealTimers();
});

describe("the tooltip controller", () => {
  it("shows the hovered element's title at the delay, not a millisecond before, and puts the title back on leave", () => {
    const undo = el("undo");
    move(el("pane"), undo);
    expect(undo.getAttribute("title"), "parked while hovered: the native tooltip has nothing to show").toBe("");
    expect(undo.getAttribute(PARKED_ATTR)).toBe(UNDO);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS - 1);
    expect(tooltips.shown()).toBeNull();
    expect(seen).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(tooltips.shown()).toEqual({ anchor: undo, text: UNDO });
    expect(seen).toEqual([{ anchor: undo, text: UNDO }]);

    move(undo, el("pane"));
    expect(tooltips.shown()).toBeNull();
    expect(seen).toEqual([{ anchor: undo, text: UNDO }, null]);
    expect(undo.getAttribute("title")).toBe(UNDO);
    expect(undo.hasAttribute(PARKED_ATTR)).toBe(false);
  });

  it("the element's own descendants keep the hover; a sibling starts its own delay from zero", () => {
    const undo = el("undo");
    const glyph = el("glyph");
    move(el("pane"), undo);
    vi.advanceTimersByTime(200);
    move(undo, glyph);
    expect(undo.getAttribute("title"), "still the same hover").toBe("");
    vi.advanceTimersByTime(50);
    expect(tooltips.shown()).toEqual({ anchor: undo, text: UNDO });

    // Onto the (disabled) sibling: the undo title is back, nothing shows
    // for redo before ITS 250 ms — and a disabled control is no exception.
    const redo = el("redo");
    move(glyph, redo);
    expect(tooltips.shown()).toBeNull();
    expect(undo.getAttribute("title")).toBe(UNDO);
    expect(redo.getAttribute("title")).toBe("");
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS - 1);
    expect(tooltips.shown()).toBeNull();
    vi.advanceTimersByTime(1);
    expect(tooltips.shown()).toEqual({ anchor: redo, text: REDO });
    move(redo, null);
    expect(redo.getAttribute("title")).toBe(REDO);
  });

  it("an untitled child shows its closest ancestor's title; an empty title is no tooltip and no ancestor's", () => {
    const wrap = el("wrap");
    move(el("pane"), el("plain"));
    expect(wrap.getAttribute("title")).toBe("");
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    expect(tooltips.shown()).toEqual({ anchor: wrap, text: "3 undoable" });
    move(el("plain"), el("pane"));
    expect(wrap.getAttribute("title")).toBe("3 undoable");

    const empty = el("empty");
    move(el("pane"), empty);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS * 4);
    expect(tooltips.shown()).toBeNull();
    expect(empty.getAttribute("title")).toBe("");
    expect(empty.hasAttribute(PARKED_ATTR)).toBe(false);
    expect(seen).toEqual([{ anchor: wrap, text: "3 undoable" }, null]);
  });

  it("a pointer down dismisses the box and keeps the title parked until the pointer leaves", () => {
    const undo = el("undo");
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    expect(tooltips.shown()).not.toBeNull();
    undo.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    expect(tooltips.shown()).toBeNull();
    expect(undo.getAttribute("title"), "the native tooltip must not surface in place of the dismissed one").toBe("");
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS * 4);
    expect(tooltips.shown()).toBeNull();
    move(undo, el("pane"));
    expect(undo.getAttribute("title")).toBe(UNDO);
  });

  it("Esc dismisses a shown or pending box without consuming the key", () => {
    const undo = el("undo");
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    undo.dispatchEvent(escape);
    expect(tooltips.shown()).toBeNull();
    expect(escape.defaultPrevented, "the keyboard map and the modals see the same Esc").toBe(false);
    move(undo, el("pane"));

    // Pending: an Esc inside the delay means no box at all for this hover.
    move(el("pane"), undo);
    vi.advanceTimersByTime(100);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS * 4);
    expect(tooltips.shown()).toBeNull();
    // Another key is not a dismissal.
    move(undo, el("pane"));
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    expect(tooltips.shown()).toEqual({ anchor: undo, text: UNDO });
  });

  it("a title rewritten under the pointer is what the box shows and what leave restores", async () => {
    const undo = el("undo");
    const NEXT = "undo: delete sphere_1 (Ctrl+Z)";
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    // What React does after the click changed the history: a new prop, one setAttribute.
    undo.setAttribute("title", NEXT);
    await flush();
    expect(tooltips.shown()).toEqual({ anchor: undo, text: NEXT });
    expect(undo.getAttribute("title"), "parked again").toBe("");
    expect(undo.getAttribute(PARKED_ATTR)).toBe(NEXT);
    move(undo, el("pane"));
    expect(undo.getAttribute("title"), "the NEW title, never the stale one").toBe(NEXT);

    // Rewritten inside the delay: the box shows the new text when it comes.
    move(el("pane"), undo);
    vi.advanceTimersByTime(100);
    undo.setAttribute("title", UNDO);
    await flush();
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS - 100);
    expect(tooltips.shown()).toEqual({ anchor: undo, text: UNDO });
    move(undo, el("pane"));
    expect(undo.getAttribute("title")).toBe(UNDO);
  });

  it("a title removed under the pointer takes the box down and restores nothing", async () => {
    const undo = el("undo");
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    undo.removeAttribute("title");
    await flush();
    expect(tooltips.shown()).toBeNull();
    expect(undo.hasAttribute(PARKED_ATTR)).toBe(false);
    move(undo, el("pane"));
    expect(undo.hasAttribute("title"), "a title the app took away does not come back").toBe(false);
  });

  it("keeps the title's newlines", () => {
    move(el("pane"), el("port"));
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    expect(tooltips.shown()?.text).toBe(TWO_LINES);
  });

  it("an element removed from the document before the delay shows nothing and ends the hover", () => {
    const undo = el("undo");
    move(el("pane"), undo);
    undo.remove();
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    expect(tooltips.shown()).toBeNull();
    expect(seen).toEqual([]);
  });

  it("dispose puts a hovered title back and stops listening", () => {
    const undo = el("undo");
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS);
    tooltips.dispose();
    expect(tooltips.shown()).toBeNull();
    expect(undo.getAttribute("title")).toBe(UNDO);
    move(el("pane"), undo);
    vi.advanceTimersByTime(TOOLTIP_DELAY_MS * 4);
    expect(tooltips.shown()).toBeNull();
    expect(undo.getAttribute("title"), "no listener parks anything any more").toBe(UNDO);
  });
});

describe("placeTooltip", () => {
  const viewport = { width: 1400, height: 900 };
  const box = { width: 200, height: 40 };

  it("centres the box under the element with the gap", () => {
    expect(placeTooltip({ left: 500, top: 100, width: 100, height: 20 }, box, viewport)).toEqual({
      left: 450,
      top: 126,
      side: "below",
    });
  });

  it("goes above when there is no room below", () => {
    // 880 + 20 + 6 + 40 = 946 > 896 (the height less the margin): above.
    expect(placeTooltip({ left: 500, top: 880, width: 100, height: 20 }, box, viewport)).toEqual({
      left: 450,
      top: 834,
      side: "above",
    });
    // Exactly the margin left below: still below.
    expect(placeTooltip({ left: 500, top: 830, width: 100, height: 20 }, box, viewport).side).toBe("below");
  });

  it("clamps to the viewport's edges", () => {
    expect(placeTooltip({ left: 0, top: 100, width: 30, height: 20 }, box, viewport).left).toBe(4);
    expect(placeTooltip({ left: 1380, top: 100, width: 20, height: 20 }, box, viewport).left).toBe(1196);
    // Wider than the viewport: the left margin, whatever the element.
    expect(placeTooltip({ left: 700, top: 100, width: 20, height: 20 }, { width: 2000, height: 40 }, viewport).left).toBe(4);
  });

  it("with room on neither side takes the roomier one, clamped", () => {
    const tall = { width: 200, height: 600 };
    const low = placeTooltip({ left: 500, top: 500, width: 100, height: 20 }, tall, { width: 1400, height: 700 });
    expect(low.side).toBe("above");
    expect(low.top).toBe(4);
    const high = placeTooltip({ left: 500, top: 150, width: 100, height: 20 }, tall, { width: 1400, height: 700 });
    expect(high.side).toBe("below");
    expect(high.top).toBe(96);
  });
});
