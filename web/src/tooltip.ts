/**
 * The tooltip layer (docs/16 §Theme and visual language; docs/17 wave 5 T1,
 * finding U24 — "mouseover text boxes should appear ~50 % faster"). The
 * browser's own tooltip takes ~1 s in Chromium and cannot be tuned, so this
 * layer shows the same text itself after `TOOLTIP_DELAY_MS`: ONE listener set
 * on the document (`pointerover`, `pointerout`, `pointermove`, `pointerdown`,
 * `pointerup`, `keydown` Esc — all in the capture phase) and every hover
 * text in the app changes nothing — the `title=` sites and the two SVG
 * `<title>` children (every wire's, the profiler ring's arcs) alike, the
 * platform's two tooltip sources.
 *
 * The capture phase is for the DISMISS, not the hover: nothing in the app
 * stops a `pointerover`, but the dialogs and the search box stop `pointerdown`
 * and the text editors stop `keydown` at their own element, so a bubble-phase
 * document listener would never see a press inside About or an Esc typed into
 * the search box — the box would stand over a closed dialog until the next
 * hover.
 *
 * Entering an element whose closest ancestor-or-self carries a title starts
 * the delay; the box shows that text (newlines kept) until the pointer leaves
 * the element, a pointer goes down, Esc is pressed, or the element leaves the
 * document (a node deleted under the pointer fires no `pointerout`; the
 * hover's MutationObserver watches the tree for it). No hover starts while
 * a button is held — a node or wire drag crossing titled elements shows
 * nothing, as the platform shows no tooltip with a button down — and the
 * element under the pointer at the release is entered then (Chromium fires
 * no boundary event on a wire's drop target). While the element is
 * hovered its title is PARKED: the text moves to `data-title` and the source
 * is left EMPTY — an empty `title` attribute, or an empty `<title>` child, is
 * what the platform itself reads as "no tooltip here", so the native box
 * never doubles ours and no ancestor's title surfaces in its place — and it
 * is restored on leave. A title the app rewrites under the pointer (React
 * re-rendering the undo button after a click) is adopted through the same
 * MutationObserver: the box follows the new text and the restore writes
 * the NEW value, never the stale one. The records of the layer's own
 * parking writes are drained at the write, so an app-written EMPTY title
 * is read as what it is — the platform's "no tooltip here": the box goes
 * and the restore writes the empty value back — and a title removed under
 * the pointer takes the box down and restores nothing.
 *
 * What the layer does not do: it never consumes a key or a press (Esc hides
 * the box and goes on to the keyboard map and the modals; a press goes on
 * to the slider's drag and the dialogs — the layer fights none of them),
 * never special-cases disabled controls (Chromium dispatches pointer events
 * over them — measured on 151: every pointer event arrives; only the mouse
 * events `mousedown`, `mouseup` and `click` are withheld — so their "why
 * disabled" titles show at the same 250 ms; a browser that withholds the
 * pointer events keeps its native tooltip there), never shows a box on
 * keyboard focus (pointer events only, as the native tooltip; a focused
 * control's `title` stays unparked for assistive technology), and never
 * takes the pointer (the box is `pointer-events: none`).
 *
 * `installTooltips` is the controller — DOM in, a subscription out — so
 * the whole behaviour runs under jsdom with fake timers; `TooltipLayer`
 * (`TooltipLayer.tsx` — not `Tooltip.tsx`: a name differing from this
 * file's only in case resolves to THIS file on a case-insensitive file
 * system, Windows and macOS) renders what it reports. `placeTooltip` is
 * the pure placement: below the element, above when there is no room,
 * clamped to the viewport — and for an SVG `<title>` source, whose element
 * has no edge to sit under (a wire's box is the whole diagonal), below the
 * pointer where it rested.
 */

/** The hover-to-box delay. The native tooltip's is the browser's (~1 s). */
export const TOOLTIP_DELAY_MS = 250;
/** The gap between the element and the box, px. */
export const TOOLTIP_GAP_PX = 6;
/** What the box keeps from the viewport's edges, px. */
export const TOOLTIP_MARGIN_PX = 4;
/**
 * The pointer glyph's height under its hotspot, px: what a box placed at the
 * pointer (an SVG `<title>` source) sits below, so the arrow never covers it.
 */
export const POINTER_HEIGHT_PX = 18;
/** Where a hovered element's title text lives while its title is parked. */
export const PARKED_ATTR = "data-title";

const SVG_NS = "http://www.w3.org/2000/svg";

export interface TooltipPoint {
  x: number;
  y: number;
}

export interface TooltipShown {
  /** The element whose title is shown; the box is placed against it. */
  anchor: Element;
  text: string;
  /**
   * Where the pointer rested when the box was due — set for an SVG `<title>`
   * source, whose element has no edge to sit under; absent, the box sits
   * under the anchor's own box.
   */
  point?: TooltipPoint;
}

export interface TooltipController {
  /** What is shown right now (null: nothing). */
  shown(): TooltipShown | null;
  /** Called on every change of `shown()`; returns the unsubscribe. */
  subscribe(listener: (shown: TooltipShown | null) => void): () => void;
  /** Remove the listeners; a hovered element gets its title back. */
  dispose(): void;
}

/** A hovered element's title source: the `title` attribute, or an SVG `<title>` child. */
interface TitleSource {
  anchor: Element;
  /** The SVG `<title>` child carrying the text; null when the text is the attribute's. */
  titleEl: Element | null;
}

interface Session {
  source: TitleSource;
  /** The title as parked — what leave restores; null once the app removed it. */
  title: string | null;
  timer: ReturnType<typeof setTimeout> | null;
  observer: MutationObserver;
  /** For an SVG `<title>` source: where the pointer was last seen while the box was pending. */
  point: TooltipPoint | null;
}

function isElement(value: EventTarget | null): value is Element {
  return value !== null && typeof value === "object" && (value as Node).nodeType === 1;
}

function isNode(value: EventTarget | null): value is Node {
  return value !== null && typeof value === "object" && "nodeType" in value;
}

/** The element's direct SVG `<title>` child, if any — the SVG tooltip source. */
function svgTitleChild(el: Element): Element | null {
  const children = el.children;
  for (let i = 0; i < children.length; i += 1) {
    const child = children[i];
    if (child !== undefined && child.localName === "title" && child.namespaceURI === SVG_NS) return child;
  }
  return null;
}

/**
 * The title source the pointer's target shows: walking up from the target,
 * the first element with a `title` attribute or an SVG `<title>` child — the
 * platform's own walk, which stops at the closest source, empty or not.
 */
function sourceFor(target: EventTarget | null): TitleSource | null {
  if (!isElement(target)) return null;
  for (let el: Element | null = target; el !== null; el = el.parentElement) {
    if (el.hasAttribute("title")) return { anchor: el, titleEl: null };
    const titleEl = svgTitleChild(el);
    if (titleEl !== null) return { anchor: el, titleEl };
  }
  return null;
}

/** The source's current text; null once the app removed the source. */
function readTitle(source: TitleSource): string | null {
  if (source.titleEl === null) return source.anchor.getAttribute("title");
  if (source.titleEl.parentNode !== source.anchor) return null;
  return source.titleEl.textContent ?? "";
}

function writeTitle(source: TitleSource, text: string) {
  if (source.titleEl === null) source.anchor.setAttribute("title", text);
  else source.titleEl.textContent = text;
}

function pointOf(event: Event): TooltipPoint {
  const { clientX, clientY } = event as PointerEvent;
  return { x: clientX, y: clientY };
}

function buttonsHeld(event: Event): boolean {
  return ((event as PointerEvent).buttons ?? 0) !== 0;
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

  // Park: the text into `data-title`, the source emptied — and the records of
  // this write drained, so the observer never mistakes it for the app's.
  const park = (current: Session, text: string) => {
    current.source.anchor.setAttribute(PARKED_ATTR, text);
    writeTitle(current.source, "");
    current.observer.takeRecords();
  };

  const takeDown = () => {
    cancelTimer();
    setShown(null);
  };

  // The app rewrote the hovered element's title (a React re-render with a
  // new prop): adopt it — the box follows, the restore writes this one. An
  // empty one is the platform's "no tooltip here": the box goes, the restore
  // writes the empty value; a removed one takes the box down and restores
  // nothing.
  const adopt = () => {
    if (session === null) return;
    const current = session;
    const title = readTitle(current.source);
    current.title = title;
    if (title === null || title === "") {
      current.source.anchor.removeAttribute(PARKED_ATTR);
      takeDown();
      return;
    }
    park(current, title);
    if (shown !== null) setShown({ anchor: current.source.anchor, text: title, ...(current.point !== null ? { point: current.point } : {}) });
  };

  const leave = () => {
    if (session === null) return;
    cancelTimer();
    session.observer.disconnect();
    const { source, title } = session;
    source.anchor.removeAttribute(PARKED_ATTR);
    if (title !== null) writeTitle(source, title);
    session = null;
    setShown(null);
  };

  // One observer per hover: the anchor's `title` attribute (or its `<title>`
  // child's text) for a rewrite, and the document's tree for the anchor's
  // removal — a node deleted under the pointer fires no `pointerout`, and
  // its box would stand until the next hover.
  const onMutation = (records: MutationRecord[]) => {
    if (session === null) return;
    const { anchor, titleEl } = session.source;
    if (!anchor.isConnected) {
      leave();
      return;
    }
    const titled = records.some(
      (record) =>
        (record.type === "attributes" && record.target === anchor) || (titleEl !== null && titleEl.contains(record.target)),
    );
    if (titled) adopt();
  };

  const enter = (source: TitleSource, at: TooltipPoint) => {
    const title = readTitle(source) ?? "";
    // An empty title is the platform's "no tooltip here" (and none of the
    // ancestors' either): the same for us.
    if (title === "") return;
    const observer = new MutationObserver(onMutation);
    observer.observe(source.anchor, { attributes: true, attributeFilter: ["title"] });
    if (source.titleEl !== null) observer.observe(source.titleEl, { childList: true, characterData: true, subtree: true });
    observer.observe(doc, { childList: true, subtree: true });
    const started: Session = { source, title, timer: null, observer, point: source.titleEl !== null ? at : null };
    session = started;
    park(started, title);
    started.timer = setTimeout(() => {
      if (session !== started) return;
      started.timer = null;
      // Gone from the document meanwhile, or the app took the title away
      // (both end the hover through the observer; a belt for the timer):
      // nothing to show.
      if (!started.source.anchor.isConnected || started.title === null || started.title === "") {
        leave();
        return;
      }
      setShown({
        anchor: started.source.anchor,
        text: started.title,
        ...(started.point !== null ? { point: started.point } : {}),
      });
    }, delayMs);
  };

  // A pointer arriving over an element — a boundary event, or the release
  // that ends a drag over it.
  const arrive = (event: Event) => {
    const next = sourceFor(event.target);
    if (session !== null && next !== null && session.source.anchor === next.anchor) return;
    leave();
    // A button held (a node or wire drag crossing titled elements): no hover,
    // as the platform shows no tooltip with a button down.
    if (next === null || buttonsHeld(event)) return;
    enter(next, pointOf(event));
  };

  const onPointerOver = (event: Event) => arrive(event);

  const onPointerOut = (event: Event) => {
    if (session === null) return;
    // Moving between the element's own descendants is not a leave.
    const related = (event as PointerEvent).relatedTarget;
    if (isNode(related) && session.source.anchor.contains(related)) return;
    leave();
  };

  // The pointer resting on an SVG-titled element decides where its box goes:
  // the last position before the box is due.
  const onPointerMove = (event: Event) => {
    if (session === null || session.point === null || session.timer === null) return;
    session.point = pointOf(event);
  };

  // A press or Esc dismisses the box; the title stays parked, so the native
  // one does not surface in its place, until the pointer leaves. Nothing is
  // consumed: the keyboard map and the modals see the Esc as before.
  const dismiss = () => takeDown();
  const onPointerDown = () => dismiss();
  // The release that ends a drag: the element under the pointer is entered
  // as if the pointer had just arrived — a wire's drop target sees no
  // boundary event of its own. A click's release lands on the pressed
  // element, whose session it keeps (still dismissed).
  const onPointerUp = (event: Event) => arrive(event);
  const onKeyDown = (event: Event) => {
    if ((event as KeyboardEvent).key === "Escape") dismiss();
  };

  doc.addEventListener("pointerover", onPointerOver, true);
  doc.addEventListener("pointerout", onPointerOut, true);
  doc.addEventListener("pointermove", onPointerMove, true);
  doc.addEventListener("pointerdown", onPointerDown, true);
  doc.addEventListener("pointerup", onPointerUp, true);
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
      doc.removeEventListener("pointermove", onPointerMove, true);
      doc.removeEventListener("pointerdown", onPointerDown, true);
      doc.removeEventListener("pointerup", onPointerUp, true);
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
 * The rect a shown tooltip is placed against: the anchor's own box, or — for
 * a box due at the pointer — the pointer glyph under its hotspot.
 */
export function anchorRect(shown: TooltipShown): RectLike {
  if (shown.point !== undefined) return { left: shown.point.x, top: shown.point.y, width: 0, height: POINTER_HEIGHT_PX };
  return shown.anchor.getBoundingClientRect();
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
