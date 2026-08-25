/**
 * The scrub-cache toggle as a node-menu item (docs/16 §Sliders §Scrub-cached;
 * v0.1 item 5 S2). The SAME state the inspector's switch and the params
 * pill render (`state/scrub.ts::scrubToggle`), spelled as a `MenuItem`: the
 * label (`scrub-cache this slider` / `stop scrub-caching`), greyed with the
 * SERVER's reason while the slider is off and cannot scrub-cache, the hint
 * the reason or the warm count — read off the MERGED view, the slider's
 * `param.scrub` with its `scrub_progress` overlay laid over it, as the bar
 * is drawn. The review of 2026-08-24 found the first cut reading the raw
 * graph view here: the overlay never writes the graph (docs/13), so the
 * menu said `0 / 19 positions warm` under a full bar. Pure, so the menu's
 * greying and hint are pinned without React Flow — `scrubMenu.test.tsx`
 * renders the items through the real `ContextMenu`; `Canvas.tsx` spreads
 * them into the node menu (nothing, for a node that is no slider).
 */
import type { ClientMessage, NodeView, ScrubProgressPayload } from "../protocol/messages";
import { scrubToggle } from "../state/scrub";
import type { MenuItem } from "./ContextMenu";

export function scrubMenuItems(
  view: Pick<NodeView, "name" | "func" | "param">,
  progress: ScrubProgressPayload | undefined,
  send: (message: ClientMessage) => unknown,
): MenuItem[] {
  const toggle = scrubToggle(view, progress);
  if (toggle === null) return [];
  return [
    {
      label: toggle.label,
      disabled: toggle.disabled,
      hint: toggle.hint,
      onClick: () => send({ type: "set_scrub", payload: { node: view.name, on: toggle.next } }),
    },
  ];
}
