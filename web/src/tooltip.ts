/**
 * The tooltip layer (docs/16 §Theme and visual language; docs/17 wave 5 T1,
 * finding U24 — "mouseover text boxes should appear ~50 % faster"). The
 * browser's own `title` tooltip takes ~1 s in Chromium and cannot be tuned,
 * so this layer shows the same text itself after `TOOLTIP_DELAY_MS`: ONE
 * listener set on the document (`pointerover`, `pointerout`, `pointerdown`,
 * `keydown` Esc — all in the capture phase, so a component that stops a
 * pointer event's propagation, as the dialogs do, still hovers) and the
 * 125 `title=` sites change nothing.
 *
 * Entering an element whose closest `[title]` ancestor-or-self carries text
 * starts the delay; the box shows that text (newlines kept) until the
 * pointer leaves the element, a pointer goes down, or Esc is pressed. While
 * the element is hovered its `title` is PARKED: the text moves to
 * `data-title` and the attribute is left EMPTY — an empty `title` is what
 * the platform itself reads as "no tooltip here", so the native box never
 * doubles ours and no ancestor's title surfaces in its place — and it is
 * restored on leave. A title the app rewrites under the pointer (React
 * re-rendering the undo button after a click) is adopted through a
 * MutationObserver: the box follows the new text and the restore writes
 * the NEW value, never the stale one; a title removed under the pointer
 * takes the box down and restores nothing.
 *
 * What the layer does not do: it never consumes a key (Esc hides the box
 * and goes on to the keyboard map and the modals — the layer fights
 * neither), never special-cases disabled controls (Chromium dispatches
 * pointer events over them — measured on 151: every mouse and pointer
 * event but `click` — so their "why disabled" titles show at the same
 * 250 ms; a browser that withholds those events keeps its native tooltip
 * there), and never takes the pointer (the box is `pointer-events: none`).
 *
 * `installTooltips` is the controller — DOM in, a subscription out — so
 * the whole behaviour runs under jsdom with fake timers; `TooltipLayer`
 * (`TooltipLayer.tsx` — not `Tooltip.tsx`: a name differing from this
 * file's only in case resolves to THIS file on a case-insensitive file
 * system, Windows and macOS) renders what it reports. `placeTooltip` is
 * the pure placement: below the element, above when there is no room,
 * clamped to the viewport.
 */

/** The hover-to-box delay. The native tooltip's is the browser's (~1 s). */
export const TOOLTIP_DELAY_MS = 250;
/** The gap between the element and the box, px. */
export const TOOLTIP_GAP_PX = 6;
/** What the box keeps from the viewport's edges, px. */
export const TOOLTIP_MARGIN_PX = 4;
/** Where a hovered element's title text lives while its `title` is parked. */
export const PARKED_ATTR = "data-title";

export interface TooltipShown {
  /** The element whose title is shown; the box is placed against it. */
  anchor: Element;
  text: string;
}

export interface TooltipController {
  /** What is shown right now (null: nothing). */
  shown(): TooltipShown | null;
  /** Called on every change of `shown()`; returns the unsubscribe. */
  subscribe(listener: (shown: TooltipShown | null) => void): () => void;
  /** Remove the listeners; a hovered element gets its title back. */
  dispose(): void;
}

interface Session {
  anchor: Element;
  /** The title as parked — what leave restores; null once the app removed it. */
  title: string | null;
  timer: ReturnType<typeof setTimeout> | null;
  observer: MutationObserver;
}

/** The element whose title the pointer's target shows: the closest `[title]` ancestor-or-self. */
function anchorFor(target: EventTarget | null): Element | null {
  if (target === null || typeof target !== "object" || typeof (target as Element).closest !== "function") return null;
  return (target as Element).closest("[title]");
}

function isNode(value: EventTarget | null): value is Node {
  return value !== null && typeof value === "object" && "nodeType" in value;
}

/**
 * Install the layer on `doc`. One controller per document: a second install
 * would park and restore the same titles twice.
 */
export function installTooltips(doc: Document, delayMs = TOOLTIP_DELAY_MS): TooltipController {
  const listeners = new Set<(shown: TooltipShown | null) => void>();
  let session: Session | null = null;
  let shown: TooltipShown | null = null;

  const setShown = (next: TooltipShown | null) => {
    if (next === null && shown === null) return;
    shown = next;
    for (const listener of listeners) listener(next);
  };

  const cancelTimer = () => {
    if (session !== null && session.timer !== null) {
      clearTimeout(session.timer);
      session.timer = null;
    }
  };

  const park = (el: Element, text: string) => {
    el.setAttribute(PARKED_ATTR, text);
    el.setAttribute("title", "");
  };

  // The app rewrote the hovered element's `title` (a React re-render with a
  // new prop): adopt it — the box follows, the restore writes this one.
  const adopt = () => {
    if (session === null) return;
    const el = session.anchor;
    const title = el.getAttribute("title");
    // Our own parking write: nothing to adopt.
    if (title === "") return;
    if (title === null) {
      // Removed by the app: nothing to show and nothing to restore.
      session.title = null;
      el.removeAttribute(PARKED_ATTR);
      cancelTimer();
      setShown(null);
      return;
    }
    session.title = title;
    park(el, title);
    if (shown !== null) setShown({ anchor: el, text: title });
  };

  const leave = () => {
    if (session === null) return;
    cancelTimer();
    session.observer.disconnect();
    const { anchor, title } = session;
    anchor.removeAttribute(PARKED_ATTR);
    if (title !== null) anchor.setAttribute("title", title);
    session = null;
    setShown(null);
  };

  const enter = (anchor: Element) => {
    const title = anchor.getAttribute("title") ?? "";
    // An empty title is the platform's "no tooltip here" (and none of the
    // ancestors' either): the same for us.
    if (title === "") return;
    const observer = new MutationObserver(adopt);
    observer.observe(anchor, { attributes: true, attributeFilter: ["title"] });
    park(anchor, title);
    const started: Session = { anchor, title, timer: null, observer };
    session = started;
    started.timer = setTimeout(() => {
      if (session !== started) return;
      started.timer = null;
      // Gone from the document meanwhile (a node deleted under the pointer),
      // or the app took the title away: nothing to show.
      if (!started.anchor.isConnected || started.title === null) {
        leave();
        return;
      }
      setShown({ anchor: started.anchor, text: started.title });
    }, delayMs);
  };

  const onPointerOver = (event: Event) => {
    const next = anchorFor(event.target);
    if (session !== null && session.anchor === next) return;
    leave();
    if (next !== null) enter(next);
  };

  const onPointerOut = (event: Event) => {
    if (session === null) return;
    // Moving between the element's own descendants is not a leave.
    const related = (event as PointerEvent).relatedTarget;
    if (isNode(related) && session.anchor.contains(related)) return;
    leave();
  };

  // A press or Esc dismisses the box; the title stays parked, so the native
  // one does not surface in its place, until the pointer leaves. Nothing is
  // consumed: the keyboard map and the modals see the Esc as before.
  const dismiss = () => {
    cancelTimer();
    setShown(null);
  };
  const onPointerDown = () => dismiss();
  const onKeyDown = (event: Event) => {
    if ((event as KeyboardEvent).key === "Escape") dismiss();
  };

  doc.addEventListener("pointerover", onPointerOver, true);
  doc.addEventListener("pointerout", onPointerOut, true);
  doc.addEventListener("pointerdown", onPointerDown, true);
  doc.addEventListener("keydown", onKeyDown, true);

  return {
    shown: () => shown,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dispose() {
      leave();
      doc.removeEventListener("pointerover", onPointerOver, true);
      doc.removeEventListener("pointerout", onPointerOut, true);
      doc.removeEventListener("pointerdown", onPointerDown, true);
      doc.removeEventListener("keydown", onKeyDown, true);
      listeners.clear();
    },
  };
}

export interface RectLike {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface SizeLike {
  width: number;
  height: number;
}

export interface TooltipPlacement {
  left: number;
  top: number;
  side: "below" | "above";
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), Math.max(low, high));
}

/**
 * Where the box goes: centred under the element, `gap` below it; above it
 * when there is no room below; when neither side has room, the roomier one.
 * Either way inside the viewport by `margin` — a box wider than the
 * viewport sits at the left margin.
 */
export function placeTooltip(
  anchor: RectLike,
  box: SizeLike,
  viewport: SizeLike,
  gap = TOOLTIP_GAP_PX,
  margin = TOOLTIP_MARGIN_PX,
): TooltipPlacement {
  const below = anchor.top + anchor.height + gap;
  const above = anchor.top - gap - box.height;
  const fitsBelow = below + box.height <= viewport.height - margin;
  const fitsAbove = above >= margin;
  const roomBelow = viewport.height - (anchor.top + anchor.height);
  const side: TooltipPlacement["side"] = fitsBelow
    ? "below"
    : fitsAbove
      ? "above"
      : roomBelow >= anchor.top
        ? "below"
        : "above";
  const top = clamp(side === "below" ? below : above, margin, viewport.height - margin - box.height);
  const left = clamp(anchor.left + anchor.width / 2 - box.width / 2, margin, viewport.width - margin - box.width);
  return { left, top, side };
}
