/**
 * The top bar's File menu (docs/16 §Application layout; docs/17 wave 4
 * O2): Open… (the dialog over `GET /api/files`), Recent (the last ten
 * pipelines this origin opened, most recent first — `state/recent.ts`),
 * Close (back to the landing picker). Every item is a route change
 * (`state/route.ts`): one history entry, the socket follows, Back returns.
 * The menu closes on a choice, on Esc and on a click outside, like the
 * settings menu.
 */
import { useEffect, useRef, useState } from "react";
import { browserStorage, readRecent } from "../state/recent";
import { closePipeline, openPipeline } from "../state/route";
import { useCicada } from "../state/store";
import "./panels.css";

export function FileMenu() {
  const pipeline = useCicada((s) => s.pipeline);
  const openFileDialog = useCicada((s) => s.openFileDialog);
  const [open, setOpen] = useState(false);
  const [recent, setRecent] = useState<string[]>([]);
  const wrapRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: PointerEvent) => {
      if (wrapRef.current !== null && !wrapRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const toggle = () => {
    if (!open) setRecent(readRecent(browserStorage()));
    setOpen((v) => !v);
  };

  return (
    <span className="tb-menu-wrap" ref={wrapRef}>
      <button
        className={`tb-esc${open ? " active" : ""}`}
        title="open another pipeline, reopen a recent one, or close this one"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
        data-testid="tb-file"
      >
        File
      </button>
      {open && (
        <div className="tb-menu tb-menu-left tb-file-menu" role="menu" aria-label="file" data-no-hotkeys data-testid="file-menu">
          <button
            role="menuitem"
            className="tb-menu-item"
            onClick={() => {
              setOpen(false);
              openFileDialog();
            }}
            data-testid="file-open"
          >
            Open…
          </button>
          <span className="menu-h">recent</span>
          {recent.length === 0 ? (
            <span className="faint tb-menu-note" data-testid="file-recent-empty">
              nothing yet
            </span>
          ) : (
            recent.map((entry) => (
              <button
                key={entry}
                role="menuitem"
                className={`tb-menu-item mono${entry === pipeline ? " active" : ""}`}
                title={entry === pipeline ? "this pipeline (open)" : `open ${entry}`}
                onClick={() => {
                  setOpen(false);
                  openPipeline(entry);
                }}
                data-testid={`file-recent-${entry}`}
              >
                {entry}
              </button>
            ))
          )}
          <span className="menu-h">this pipeline</span>
          <button
            role="menuitem"
            className="tb-menu-item"
            title="close this pipeline and return to the picker — every edit is already on disk"
            onClick={() => {
              setOpen(false);
              closePipeline();
            }}
            data-testid="file-close"
          >
            Close
          </button>
        </div>
      )}
    </span>
  );
}
