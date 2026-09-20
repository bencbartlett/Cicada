/**
 * The viewport's three modes (docs/16 §Viewport conventions; docs/17 wave 5
 * V1, finding U16): `split` — its pane under the canvas; `floating` — a
 * panel over the canvas, dragged by its title strip, resized from its
 * corner, place and size per-user settings clamped into the work area;
 * `window` — the browser's Document Picture-in-Picture window with the
 * viewport's element MOVED into it (the same scene, the same WebGL
 * context), the wave-4 observer pop-out where the API is missing.
 *
 * Pure functions only: the floating panel's geometry, the stored-value
 * normalisation, the window decision, and the mode reducer (`stepMode`)
 * that `windowMode.ts` runs — every rule here is a table test.
 */

export type ViewportMode = "split" | "floating" | "window";
export const VIEWPORT_MODES: readonly ViewportMode[] = ["split", "floating", "window"];

export interface Size {
  width: number;
  height: number;
}

/** The floating panel's place and size, CSS pixels relative to the work area. */
export interface FloatingRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export const FLOATING_MIN_WIDTH = 240;
export const FLOATING_MIN_HEIGHT = 160;
/** The default panel keeps this far from the area's lower-right corner. */
export const FLOATING_MARGIN = 12;
/** The default panel's share of the work area, each way. */
export const FLOATING_DEFAULT_SHARE = 0.4;
/** `requestWindow`'s size when the user has no floating size yet. */
export const DEFAULT_WINDOW_SIZE: Size = { width: 640, height: 400 };

/**
 * Clamp a panel into the area: the size at least the minimum and at most the
 * area (the minimum wins over an area smaller than it — the panel then
 * overflows rather than shrinking below usable), the position such that the
 * panel lies inside (0 when it cannot). Integers, so a persisted rect and
 * the DOM agree to the pixel.
 */
export function clampFloating(rect: FloatingRect, area: Size): FloatingRect {
  const width = Math.round(Math.max(FLOATING_MIN_WIDTH, Math.min(rect.width, Math.max(area.width, FLOATING_MIN_WIDTH))));
  const height = Math.round(
    Math.max(FLOATING_MIN_HEIGHT, Math.min(rect.height, Math.max(area.height, FLOATING_MIN_HEIGHT))),
  );
  const x = Math.round(Math.max(0, Math.min(rect.x, area.width - width)));
  const y = Math.round(Math.max(0, Math.min(rect.y, area.height - height)));
  return { x, y, width, height };
}

/** The panel a user without a stored one gets: 40 % of the area each way (at least the minimum), one margin in from the lower-right corner. */
export function defaultFloating(area: Size): FloatingRect {
  const width = Math.max(FLOATING_MIN_WIDTH, Math.round(area.width * FLOATING_DEFAULT_SHARE));
  const height = Math.max(FLOATING_MIN_HEIGHT, Math.round(area.height * FLOATING_DEFAULT_SHARE));
  return clampFloating(
    { x: area.width - width - FLOATING_MARGIN, y: area.height - height - FLOATING_MARGIN, width, height },
    area,
  );
}

/** A drag of the title strip by (dx, dy): the same size, the place clamped into the area. */
export function moveFloating(rect: FloatingRect, dx: number, dy: number, area: Size): FloatingRect {
  return clampFloating({ ...rect, x: rect.x + dx, y: rect.y + dy }, area);
}

/**
 * A drag of the corner by (dx, dy): the same place, the size at least the
 * minimum and at most what fits between the place and the area's edge — the
 * place never moves under a resize (clamping the size first is what keeps
 * `clampFloating` from sliding the panel left when the corner overshoots).
 */
export function resizeFloating(rect: FloatingRect, dx: number, dy: number, area: Size): FloatingRect {
  const width = Math.min(rect.width + dx, Math.max(FLOATING_MIN_WIDTH, area.width - rect.x));
  const height = Math.min(rect.height + dy, Math.max(FLOATING_MIN_HEIGHT, area.height - rect.y));
  return clampFloating({ ...rect, width, height }, area);
}

/**
 * A stored mode → this build's. `window` never rests: the PiP window closes
 * with the page that opened it and reopening one needs a click (the API is
 * gated on a user gesture), so a stored `window` loads as `split`, as does
 * anything this build does not know.
 */
export function viewportModeFrom(raw: unknown): ViewportMode {
  return raw === "floating" ? "floating" : "split";
}

/** A stored floating rect → this build's: four finite numbers, or no rect (the default is computed from the area). */
export function floatingRectFrom(raw: unknown): FloatingRect | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
  if (!finite(r.x) || !finite(r.y) || !finite(r.width) || !finite(r.height)) return null;
  return { x: r.x, y: r.y, width: r.width, height: r.height };
}

/** The size `requestWindow` asks for: the user's floating size (the one detached size they have shaped), else the default. */
export function windowSize(floating: FloatingRect | null): Size {
  return floating === null ? DEFAULT_WINDOW_SIZE : { width: floating.width, height: floating.height };
}

// ------------------------------------------------------------ the window --

export interface PipWindowOptions {
  width: number;
  height: number;
}

/** A window with its globals (the PiP window's `ResizeObserver` is its own realm's). */
export type PipWindow = Window & typeof globalThis;

/** The Document Picture-in-Picture API as a page sees it (Chromium ≥ 116; the DOM lib does not type it yet). */
export interface DocumentPictureInPicture {
  requestWindow(options?: PipWindowOptions): Promise<PipWindow>;
  readonly window: PipWindow | null;
}

export type WindowChoice = { kind: "pip"; api: DocumentPictureInPicture } | { kind: "popout"; reason: string };

/** What the window decision reads off the main window: the API if any, whether the origin is a secure context, and the origin to name. */
export interface WindowChoiceInput {
  documentPictureInPicture?: unknown;
  isSecureContext?: boolean;
  location?: { origin: string };
}

/**
 * What `window` mode does in this browser: the PiP window when the API is
 * there, the observer pop-out otherwise (said in a notice, with the cause):
 * the API is `[SecureContext]`-gated, so on a plain-http non-loopback origin
 * (`cicada serve --host` reached over the LAN) a browser that HAS it shows
 * none — the reason then names the origin, not the browser.
 */
export function windowChoice(win: WindowChoiceInput): WindowChoice {
  const api = win.documentPictureInPicture;
  if (typeof api === "object" && api !== null && typeof (api as { requestWindow?: unknown }).requestWindow === "function") {
    return { kind: "pip", api: api as DocumentPictureInPicture };
  }
  if (win.isSecureContext === false) {
    const origin = win.location === undefined ? "this page's origin" : win.location.origin;
    return {
      kind: "popout",
      reason: `picture-in-picture needs a secure origin — localhost or https — and ${origin} is not one`,
    };
  }
  return {
    kind: "popout",
    reason: "this browser has no picture-in-picture window (documentPictureInPicture, Chromium 116+)",
  };
}

// ------------------------------------------------------- the mode reducer --

export interface ModeState {
  /** The setting: what the window shows. */
  mode: ViewportMode;
  /** The controller holds a PiP window with the viewport's element in it. */
  windowOpen: boolean;
}

export type ModeEvent =
  /** The user picked a mode (the control, the settings menu); `pip` = the API is available here. */
  | { kind: "choose"; mode: ViewportMode; pip: boolean }
  /** `requestWindow` resolved and the element was moved. */
  | { kind: "window_opened" }
  /** The PiP window went away on its own (its close button, the opener leaving). */
  | { kind: "window_closed" }
  /** `requestWindow` rejected (no gesture, a policy) — said in a notice by the controller. */
  | { kind: "window_refused" }
  /** The viewport's element is about to leave the React tree (File → Close; the picker). */
  | { kind: "host_unmounted" };

export type ModeEffect =
  /** Move the element back where it came from — BEFORE the store changes, so React finds it at home. */
  | "reclaim"
  /** Close the PiP window we hold. */
  | "close_window"
  /** Ask for a PiP window; the mode changes only on `window_opened`. */
  | "open_window"
  /** Open the wave-4 observer pop-out instead, and say why. */
  | "pop_out";

export interface ModeStep {
  state: ModeState;
  effects: ModeEffect[];
}

/**
 * The mode machine. `window` is entered only by `window_opened` (the element
 * is in the PiP document by then) and left to `split` when the window closes
 * on its own — the contract — or to the mode the user chose when they leave
 * it by the control. Choosing `window` without the API is the pop-out and
 * no change of mode: the observer window is a second socket, not a place
 * this window's viewport can be.
 */
export function stepMode(state: ModeState, event: ModeEvent): ModeStep {
  switch (event.kind) {
    case "choose": {
      if (event.mode === "window") {
        if (state.windowOpen) return { state, effects: [] };
        return { state, effects: [event.pip ? "open_window" : "pop_out"] };
      }
      if (state.windowOpen) {
        return { state: { mode: event.mode, windowOpen: false }, effects: ["reclaim", "close_window"] };
      }
      return { state: { ...state, mode: event.mode }, effects: [] };
    }
    case "window_opened":
      return { state: { mode: "window", windowOpen: true }, effects: [] };
    case "window_closed":
      return state.windowOpen ? { state: { mode: "split", windowOpen: false }, effects: ["reclaim"] } : { state, effects: [] };
    case "window_refused":
      return { state, effects: [] };
    case "host_unmounted":
      return state.windowOpen
        ? { state: { mode: "split", windowOpen: false }, effects: ["reclaim", "close_window"] }
        : { state, effects: [] };
  }
}
