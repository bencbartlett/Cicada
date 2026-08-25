/**
 * The pop-out (docs/16 §Viewport conventions; docs/17 wave 4 O3): the
 * viewport alone — no canvas, panels, ribbon or hotkeys — for a second
 * window or monitor. Its socket joined as a declared observer (docs/13 —
 * the join hint), so it follows the same pipeline's display set live and
 * can never take the lease from the main window; its camera is its own.
 * A chip names the pipeline and the role so the window is never mistaken
 * for the editor. The connection banner and the notices are the same as
 * the app's (a dead socket must not leave the window LOOKING live).
 */
import { ConnBanner } from "./panels/ConnBanner";
import { Notices } from "./panels/Notices";
import { useCicada } from "./state/store";
import { Viewport } from "./viewport/Viewport";

export function ViewportOnly() {
  const pipeline = useCicada((s) => s.hello?.pipeline ?? s.pipeline);
  const connection = useCicada((s) => s.connection);
  const role = useCicada((s) => s.role);
  const standing =
    connection !== "open" ? connection : role === "observer" ? "read-only observer" : "writer";
  return (
    <div className="viewport-only" data-testid="viewport-only">
      <ConnBanner />
      <div className="pane viewport-only-pane">
        <Viewport />
        <div className="viewport-only-chip mono" data-testid="viewport-only-chip" data-role={role} data-connection={connection}>
          <span data-testid="viewport-only-pipeline">{pipeline}</span>
          <span className="faint"> · </span>
          <span data-testid="viewport-only-role">{standing}</span>
        </div>
      </div>
      <Notices />
    </div>
  );
}
