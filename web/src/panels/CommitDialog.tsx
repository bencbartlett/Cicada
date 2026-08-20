/**
 * The Ctrl+S commit dialog (docs/16 §Application layout: "No save button.
 * Every op persists to the file immediately; Ctrl+S opens the commit
 * dialog — the git-first reflex, repurposed honestly"). The same commit
 * form as the Git tab, over the branch and the files it would stage;
 * Ctrl+Enter commits, Esc closes, a click outside closes. When this client
 * cannot commit (observer, no repo, no git, locked, an operation in
 * progress) the dialog says why instead of showing a disabled form — the
 * reflex still gets an answer, and nothing was lost: every edit is on disk.
 */
import { useEffect } from "react";
import { gitRepoInfo } from "../protocol/messages";
import { gitWriteBlockReason, ignoredReason } from "../state/git";
import { canWrite, useCicada } from "../state/store";
import { describeHead, dirtyCount, scopeNote } from "./gitFormat";
import { CommitForm, ScopeBody } from "./GitPanel";
import { useInspectorTab } from "./inspectorTab";
import "./panels.css";

export function CommitDialog() {
  const open = useCicada((s) => s.commitDialog);
  const close = useCicada((s) => s.closeCommitDialog);
  const git = useCicada((s) => s.git);
  const writer = useCicada(canWrite);
  const blocked = useCicada(gitWriteBlockReason);
  const setTab = useInspectorTab((s) => s.setTab);

  // Esc anywhere in the dialog (a focused button, the backdrop) closes it;
  // the textarea handles its own keys (Ctrl+Enter, Esc) in `CommitForm`.
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
  const status = git.status;
  const info = status === null ? null : gitRepoInfo(status.state);
  // An ignored pipeline blocks the dialog like an observer or a missing
  // repo does: one sentence, no form — a "nothing to commit — the scope
  // matches HEAD" line over a Commit button whose tooltip says "ignored"
  // would be two contradictory statements on one screen.
  const ignored = status !== null && info !== null && status.pipeline.ignored ? ignoredReason(status.pipeline.path) : null;
  const reason = blocked ?? ignored;
  const canCommitHere = writer && info !== null && reason === null;

  return (
    <div className="git-dialog-backdrop" onPointerDown={close} data-testid="commit-dialog-backdrop">
      <div
        className="git-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="commit"
        data-no-hotkeys
        data-testid="commit-dialog"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="git-dialog-h">
          <span className="git-dialog-title">Commit</span>
          {status !== null && (
            <span className="mono" data-testid="commit-dialog-head">
              {describeHead(status.state)}
            </span>
          )}
          <span className="faint git-dialog-note">every edit is already on disk — this records them in git</span>
          <button className="tb-esc" onClick={close} aria-label="close" title="close (Esc)" data-testid="commit-dialog-close">
            ×
          </button>
        </div>

        {status === null && git.error === null && <div className="faint">reading git status…</div>}

        {canCommitHere && status !== null ? (
          <>
            <div className="git-dialog-scope" data-testid="commit-dialog-scope" data-note={scopeNote(status, git.stale).kind}>
              <div className="insp-h" style={{ marginBottom: 4 }}>
                files to commit
                <span className={`badge${(dirtyCount(status) ?? 0) > 0 ? " warn" : ""}${git.stale ? " stale" : ""}`}>
                  {dirtyCount(status) ?? 0}
                </span>
                {git.stale && (
                  <span className="right faint" data-testid="commit-dialog-refreshing">
                    refreshing…
                  </span>
                )}
              </div>
              {/* The same rule as the Git tab (`scopeNote`): an empty scope reads
                  "re-reading" while the cache is stale, never "nothing to commit"
                  over an edit that is already on disk. */}
              <ScopeBody note={scopeNote(status, git.stale)} />
            </div>
            <CommitForm autoFocus onCommitted={close} onCancel={close} />
          </>
        ) : (
          <div className="git-dialog-blocked" data-testid="commit-dialog-blocked">
            <div>{reason ?? "cannot commit right now"}</div>
            <div className="actions" style={{ marginTop: 8 }}>
              <button
                onClick={() => {
                  setTab("git");
                  close();
                }}
                data-testid="commit-dialog-open-tab"
              >
                open the Git tab
              </button>
              <button onClick={close}>close</button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
