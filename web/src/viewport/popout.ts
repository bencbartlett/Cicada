/**
 * The pop-out viewport (docs/16 §Viewport conventions; docs/17 wave 4 O3):
 * `window.open(<this page> + "&view=viewport", "cicada-viewport")` — a
 * second window on the same pipeline's display set, joined as a declared
 * observer that never takes the lease (docs/13 — the join hint), with its
 * own camera. The fixed name means a second click focuses the existing
 * window instead of opening a third. A blocked pop-up is said, not lost.
 */
import { popoutUrl } from "../state/route";
import { useCicada } from "../state/store";

export const POPOUT_NAME = "cicada-viewport";

export type PopoutWindow = Pick<Window, "open"> & { location: Pick<Location, "origin" | "pathname" | "search"> };

/** Open (or focus) the pop-out for the page at `win.location`; null when the browser refused. */
export function popOutViewport(win: PopoutWindow): Window | null {
  const opened = win.open(popoutUrl(win.location), POPOUT_NAME);
  if (opened === null) {
    useCicada
      .getState()
      .addNotice("warning", "the browser blocked the pop-out window — allow pop-ups for this page and try again");
  }
  return opened;
}
