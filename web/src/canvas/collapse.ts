/**
 * The client's MIRROR of the server's collapse rule (wave 4 B4;
 * `viewmodel::collapse_refusal`): a slider collapses to one row only while
 * `min`, `max` and `step` are literals — the collapsed row has no port a
 * wire into them could reach — and nothing but a slider collapses. The
 * server decides and refuses (`set_collapsed`, kind `refused`, the notice
 * is its message); this mirror only gives the menu item and the inspector
 * button their hint, so the user reads why before the refusal says so.
 * Pure, unit-tested.
 */
import type { NodeView } from "../protocol/messages";

/** Why collapsing would be refused — short, for a hint — or `null` when it would not. */
export function collapseHint(view: Pick<NodeView, "param" | "inputs">): string | null {
  if (view.param?.kind !== "slider") return "not a slider";
  const wired = view.inputs
    .filter((input) => (input.name === "min" || input.name === "max" || input.name === "step") && input.wired !== undefined)
    .map((input) => input.name);
  if (wired.length === 0) return null;
  return `${wired.join(" and ")} ${wired.length === 1 ? "is" : "are"} wired`;
}
