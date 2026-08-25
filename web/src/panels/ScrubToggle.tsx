/**
 * The scrub-cache toggle (docs/16 §Sliders, §Inspector contents; v0.1 item 5
 * S2): one switch for the inspector's actions row and, compact, for the
 * params panel's row — the node context menu renders the same state as a
 * menu item (`scrubToggle`, `state/scrub.ts`). Checked = the text says
 * `scrub=True`. The click sends `set_scrub {node, on}`; the server edits
 * the kwarg (an op, undoable) and the delta brings the new view. Greyed
 * with the SERVER's reason while the slider is off and cannot scrub-cache
 * (`param.scrub.ineligible` — the same words `set_scrub` would refuse
 * with); a slider that is on but ineligible (a hand-written kwarg) can
 * always be turned off. Disabled for observers and `#off` ghosts like every
 * other write affordance. Not offered for anything but a slider.
 */
import type { NodeView } from "../protocol/messages";
import { scrubToggle } from "../state/scrub";
import { useCicada } from "../state/store";
import "../canvas/scrub.css";

interface Props {
  view: NodeView;
  /** `canWrite` (and the node is not a `#off` ghost). */
  writer: boolean;
  /** The params row's small form: `scrub` with a state dot; the actions row spells the label. */
  compact?: boolean;
}

export function ScrubToggle({ view, writer, compact = false }: Props) {
  const state = scrubToggle(view);
  if (state === null) return null;
  const send = () => useCicada.getState().send({ type: "set_scrub", payload: { node: view.name, on: state.next } });
  return (
    <button
      type="button"
      role="switch"
      aria-checked={state.on}
      className={`scrub-toggle${compact ? " compact" : ""}`}
      disabled={!writer || state.disabled}
      title={state.title}
      onClick={send}
      data-testid={`scrub-toggle-${view.name}`}
      data-blocked={state.reason ?? undefined}
      data-surface={compact ? "params" : "inspector"}
    >
      {compact ? "scrub" : state.label}
    </button>
  );
}
