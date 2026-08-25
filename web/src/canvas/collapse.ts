/**
 * The client's MIRROR of the server's collapse rule (wave 4 B4;
 * `viewmodel::collapse_refusal`): nothing but a slider collapses, and a
 * slider collapses to one row only while `value`, `min`, `max` and `step`
 * are literals — the collapsed row is name · track · value · output, so a
 * wire into any of the four has no port to reach (the track IS `value`).
 * The server decides and refuses (`set_collapsed`, kind `refused`, the
 * notice is its message, decided off the document); this mirror only gives
 * the menu item and the inspector button their hint, so the user reads why
 * before the refusal says so. The hint is the server's own words — the
 * notice contains it verbatim (`web/e2e/slider.spec.ts` holds the two
 * spellings together). Pure, unit-tested.
 */
import type { NodeView } from "../protocol/messages";

/** The slider's ports the collapsed row has no handle for, in the server's (spec) order. */
export const COLLAPSED_ROW_PORTS = ["value", "min", "max", "step"] as const;

/** Why collapsing would be refused — short, for a hint — or `null` when it would not. */
export function collapseHint(view: Pick<NodeView, "func" | "inputs">): string | null {
  if (view.func !== "slider") return "not a slider";
  const wired = COLLAPSED_ROW_PORTS.filter((port) =>
    view.inputs.some((input) => input.name === port && input.wired !== undefined),
  );
  if (wired.length === 0) return null;
  return `${wired.join(" and ")} ${wired.length === 1 ? "is" : "are"} wired`;
}
