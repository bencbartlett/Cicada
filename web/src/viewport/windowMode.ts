/**
 * The viewport-mode controller (docs/16 §Viewport conventions; docs/17 wave
 * 5 V1): runs `stepMode` against `settings.viewportMode` and owns the one
 * thing React cannot — the viewport's element leaving this document for
 * the picture-in-picture window and coming back.
 *
 * `window` mode: `documentPictureInPicture.requestWindow({width, height})`
 * (a user gesture is required, so the request is made synchronously in the
 * click); when it resolves, the app's stylesheets and theme are copied into
 * the PiP document, the element the `Viewport` registered is MOVED into its
 * body (the same three.js scene, the same WebGL context — verified 2026-09-19
 * on the headless shell and Edge), the scene is re-homed to that window,
 * and the mode becomes `window`. Coming back — the control, the
 * placeholder, the window's own close button, File → Close — moves the
 * element back into the wrapper it left BEFORE the store changes, so React
 * finds its node where it left it. Without the API (Firefox, Safari) the
 * wave-4 observer pop-out opens instead, said in a notice, and the mode
 * stays.
 */
import { useCicada } from "../state/store";
import {
  stepMode,
  windowChoice,
  windowSize,
  type DocumentPictureInPicture,
  type ModeEvent,
  type PipWindow,
  type ViewportMode,
  type WindowChoice,
} from "./modes";
import { popOutViewport, type PopoutWindow } from "./popout";
import type { SceneWindow } from "./scene";

/** What the `Viewport` component registers: its host element, and how to tell its scene the element moved windows. */
export interface ViewportHost {
  element: HTMLElement;
  rehome(win: SceneWindow): void;
}

/** What the controller needs of the main window: the pop-out's `open` + location, the PiP API if any, the document to copy styles from, and a scene home. */
export type ModeWindow = PopoutWindow & SceneWindow & { documentPictureInPicture?: unknown; document: Document };

interface Controller {
  host: ViewportHost | null;
  /** The PiP window holding the element (null = not in `window` mode). */
  pip: PipWindow | null;
  /** The wrapper the element left, to return it to. */
  home: HTMLElement | null;
  /** A `requestWindow` in flight (a second click while it resolves must not open a second window). */
  opening: boolean;
  unsubscribeTheme: (() => void) | null;
}

const c: Controller = { host: null, pip: null, home: null, opening: false, unsubscribeTheme: null };

/** The `Viewport` registers its host on mount; the returned unregister (its layout-effect cleanup) runs BEFORE React removes the node. */
export function registerViewportHost(host: ViewportHost, win: ModeWindow = window): () => void {
  c.host = host;
  return () => {
    if (c.host !== host) return;
    dispatch({ kind: "host_unmounted" }, win);
    c.host = null;
  };
}

/** The user picked a mode (the viewport's control, the settings menu, the placeholder). */
export function chooseViewportMode(mode: ViewportMode, win: ModeWindow = window): void {
  const choice = windowChoice(win);
  dispatch({ kind: "choose", mode, pip: choice.kind === "pip" }, win, choice);
}

/** Tests: is a PiP window held right now? */
export function viewportWindowOpen(): boolean {
  return c.pip !== null;
}

function dispatch(event: ModeEvent, win: ModeWindow, choice?: WindowChoice): void {
  const store = useCicada.getState();
  const before = { mode: store.settings.viewportMode, windowOpen: c.pip !== null };
  const { state, effects } = stepMode(before, event);
  const pip = c.pip;
  // The window is ours no longer once the state says so: a `pagehide` that
  // `close()` raises synchronously must not read as the window closing on
  // its own (which would land on `split` instead of the chosen mode).
  if (!state.windowOpen) c.pip = null;
  if (effects.includes("reclaim")) reclaim(win);
  if (state.mode !== before.mode) store.updateSettings({ viewportMode: state.mode });
  if (effects.includes("close_window")) pip?.close();
  if (effects.includes("open_window") && choice?.kind === "pip") void openWindow(win, choice.api);
  if (effects.includes("pop_out") && choice?.kind === "popout") {
    if (popOutViewport(win) !== null) {
      store.addNotice(
        "warning",
        `${choice.reason} — the viewport opened as a separate read-only window instead (the second-monitor pop-out)`,
      );
    }
  }
}

async function openWindow(win: ModeWindow, api: DocumentPictureInPicture): Promise<void> {
  if (c.opening) return;
  c.opening = true;
  let pip: PipWindow;
  try {
    pip = await api.requestWindow(windowSize(useCicada.getState().settings.floatingViewport));
  } catch (error) {
    c.opening = false;
    useCicada.getState().addNotice("error", `the picture-in-picture window was refused — ${String(error)}`);
    dispatch({ kind: "window_refused" }, win);
    return;
  }
  c.opening = false;
  const host = c.host;
  if (host === null || c.pip !== null) {
    // The viewport left the tree while the request resolved, or a window is somehow held: this one is not wanted.
    pip.close();
    return;
  }
  const settings = useCicada.getState().settings;
  adoptStyles(win.document, pip.document);
  pip.document.documentElement.dataset.theme = settings.theme;
  pip.document.title = `${useCicada.getState().hello?.pipeline ?? useCicada.getState().pipeline} — viewport · Cicada`;
  c.home = host.element.parentElement;
  pip.document.body.append(host.element);
  c.pip = pip;
  host.rehome(pip);
  pip.addEventListener("pagehide", () => {
    if (c.pip === pip) dispatch({ kind: "window_closed" }, win);
  });
  c.unsubscribeTheme = useCicada.subscribe((state, prev) => {
    if (state.settings.theme !== prev.settings.theme) pip.document.documentElement.dataset.theme = state.settings.theme;
  });
  dispatch({ kind: "window_opened" }, win);
}

/** The element back into the wrapper it left, the scene back on the main window. */
function reclaim(win: ModeWindow): void {
  c.unsubscribeTheme?.();
  c.unsubscribeTheme = null;
  const home = c.home;
  c.home = null;
  if (c.host === null || home === null) return;
  home.append(c.host.element);
  c.host.rehome(win);
}

/**
 * The app's stylesheets into the PiP document: a linked sheet by its href,
 * an inline one (the dev server's) by its rules. The tokens are on
 * `:root, [data-theme]`, so the PiP's own root carries them once its
 * `data-theme` is set.
 */
export function adoptStyles(from: Document, into: Document): void {
  for (const sheet of Array.from(from.styleSheets)) {
    if (sheet.href !== null) {
      const link = into.createElement("link");
      link.rel = "stylesheet";
      link.href = sheet.href;
      into.head.append(link);
      continue;
    }
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      // A sheet whose rules this origin may not read (never ours): skipped, said nowhere — it cannot be ours to copy.
      continue;
    }
    const style = into.createElement("style");
    style.textContent = Array.from(rules)
      .map((rule) => rule.cssText)
      .join("\n");
    into.head.append(style);
  }
}
