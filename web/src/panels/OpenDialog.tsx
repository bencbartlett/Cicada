/**
 * File → Open… (docs/16 §Application layout): the file browser in a dialog,
 * starting in the open pipeline's own directory. Enter or a double-click on
 * a pipeline opens it in THIS tab (the route changes, Back returns here);
 * Esc or a click outside closes. `data-no-hotkeys`: the canvas's keys stay
 * off while the list has the keyboard.
 */
import { useEffect } from "react";
import { openPipeline } from "../state/route";
import { useCicada } from "../state/store";
import { dirnameOf } from "./filePaths";
import { FileBrowser } from "./FileBrowser";
import "./panels.css";

export function OpenDialog() {
  const open = useCicada((s) => s.fileDialog);
  const close = useCicada((s) => s.closeFileDialog);
  const token = useCicada((s) => s.token);
  const pipeline = useCicada((s) => s.pipeline);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, close]);

  if (!open) return null;
  return (
    <div className="app-dialog-backdrop" onPointerDown={close} data-testid="open-dialog-backdrop">
      <div
        className="app-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="open a pipeline"
        data-no-hotkeys
        data-testid="open-dialog"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="app-dialog-h">
          <span className="app-dialog-title">Open</span>
          <span className="faint app-dialog-note">a pipeline under the served root — Enter or double-click opens it here</span>
          <button className="tb-esc" onClick={close} aria-label="close" title="close (Esc)" data-testid="open-dialog-close">
            ×
          </button>
        </div>
        <FileBrowser
          token={token}
          initialDir={dirnameOf(pipeline)}
          autoFocus
          onOpen={(chosen) => {
            close();
            openPipeline(chosen);
          }}
        />
      </div>
    </div>
  );
}
