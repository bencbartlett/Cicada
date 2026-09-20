/**
 * About (docs/16 §Settings; docs/17 wave 5 R1, finding U15): the build
 * behind this session — Cicada · version · commit (a click copies it) ·
 * built · protocol · the engine's threads — and two links, the repository
 * and this version's release notes. Everything comes from `hello`:
 * `version` and `threads` are what the engine reported (the same object
 * `GET /api/version` answers; what `cicada --version` prints, in fields).
 * An engine from before R1 reports neither, and the dialog says so rather
 * than showing a stale or invented number. Esc, the × and a click on the
 * backdrop close it; a failed copy says why (a clipboard the browser
 * withholds), never a silent "copied".
 *
 * A modal: it is open while `store.aboutDialog` is, so the keyboard map
 * closes it on Esc FIRST and keeps every canvas hotkey off behind it (fix
 * round 2026-09-20, finding L5-1); it takes focus on mount — the
 * `data-no-hotkeys` gate then applies to what the user types, and a screen
 * reader lands in the `aria-modal` region — and hands focus back to the
 * element that opened it when it closes.
 */
import { useEffect, useRef, useState } from "react";
import { useCicada } from "../state/store";
import { COPIED_MS, NOT_REPORTED, REPOSITORY_URL, UNKNOWN_COMMIT, releaseNotesUrl } from "./about";
import "./panels.css";

export function AboutDialog() {
  const hello = useCicada((s) => s.hello);
  const onClose = useCicada((s) => s.closeAboutDialog);
  const [copied, setCopied] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  // Esc from a focus inside the dialog (the router's `data-no-hotkeys` gate
  // stops it before the keyboard map); a focus behind it is the map's
  // (`keyboard.ts`, `modalOpen`).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Focus: into the dialog on mount, back to the opener on unmount (when it
  // is still in the document).
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialogRef.current?.focus();
    return () => {
      if (opener !== null && opener.isConnected) opener.focus();
    };
  }, []);

  useEffect(() => {
    if (copied === null) return;
    const timer = window.setTimeout(() => setCopied(null), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const version = hello?.version ?? null;
  const commit = version?.commit ?? null;
  // Not connected yet: nothing to say. Connected to an engine that reports
  // no build: say that — the two are different facts.
  const missing = hello === null ? "—" : NOT_REPORTED;
  // A hash is offered to copy; `unknown` (a build git could not name) is a
  // word, shown as text — a click that "copied" it would report a success
  // for something that is not a commit.
  const copyable = commit !== null && commit !== UNKNOWN_COMMIT;

  const copy = async () => {
    if (!copyable) return;
    try {
      await navigator.clipboard.writeText(commit);
      setCopied("copied");
    } catch (error) {
      setCopied(`copy failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  return (
    <div className="app-dialog-backdrop" onPointerDown={onClose} data-testid="about-backdrop">
      <div
        className="app-dialog about-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="about"
        tabIndex={-1}
        ref={dialogRef}
        data-no-hotkeys
        data-testid="about-dialog"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="app-dialog-h">
          <span className="app-dialog-title">Cicada</span>
          <span className="faint app-dialog-note">code-first parametric design</span>
          <button className="tb-esc" onClick={onClose} aria-label="close" title="close (Esc)" data-testid="about-close">
            ×
          </button>
        </div>
        <div className="stat-grid about-grid">
          <span className="k">version</span>
          <span className="v" data-testid="about-version">
            {version?.semver ?? missing}
          </span>
          <span className="k">commit</span>
          <span className="v about-commit-row">
            {!copyable ? (
              <span data-testid="about-commit">{commit ?? missing}</span>
            ) : (
              <button
                className="about-copy"
                onClick={() => void copy()}
                title="click to copy the commit hash"
                aria-label={`commit ${commit} — click to copy`}
                data-testid="about-commit"
              >
                {commit}
              </button>
            )}
            {copied !== null && (
              <span className="faint about-copied" role="status" data-testid="about-copied">
                {copied}
              </span>
            )}
          </span>
          <span className="k">built</span>
          <span className="v" data-testid="about-built">
            {version?.built ?? missing}
          </span>
          <span className="k">protocol</span>
          <span className="v" data-testid="about-protocol">
            {hello?.protocol ?? "—"}
          </span>
          <span className="k">engine threads</span>
          <span className="v" data-testid="about-threads">
            {hello?.threads ?? missing}
          </span>
        </div>
        <div className="about-links">
          <a href={REPOSITORY_URL} target="_blank" rel="noreferrer" data-testid="about-repo">
            repository
          </a>
          <span className="tb-sep">·</span>
          <a href={releaseNotesUrl(version?.semver ?? null)} target="_blank" rel="noreferrer" data-testid="about-notes">
            release notes
          </a>
          {hello !== null && (
            <span className="faint about-engine" data-testid="about-engine">
              {hello.engine}
            </span>
          )}
        </div>
        {hello === null && <div className="faint">not connected — the build shows once the engine says hello</div>}
      </div>
    </div>
  );
}
