/**
 * The Git tab (docs/16 §Application layout "Git panel"; doc 10 §Git
 * integration, slice 1; docs/17 item 2): where the project stands in git,
 * this pipeline's per-node markers against HEAD (click → select the
 * node), the dirty files of the commit scope, the commit form and the
 * revert-to-HEAD control. Everything shown is the last `GET
 * /api/git/status` answer (`state/git.ts` decides when to re-read); the
 * two writes go through `commitFromApp` / `revertToHead`. A read-only
 * observer sees all of the status and none of the controls.
 */
import { useEffect, useRef, useState } from "react";
import type { GitState, NodeChange, RemovedNode, ScopeFile } from "../protocol/messages";
import { gitRepoInfo } from "../protocol/messages";
import { describeGitError } from "../protocol/git";
import { commitBlockReason, commitFromApp, ignoredReason, refreshGitNow, revertBlockReason, revertToHead, STALE_REASON } from "../state/git";
import { canWrite, useCicada } from "../state/store";
import { useCommitDraft } from "./commitDraft";
import { describeHead, groupMarkers, markerBadge, revertRequest, scopeNote, type ScopeNote } from "./gitFormat";
import "./panels.css";

export function GitPanel() {
  const git = useCicada((s) => s.git);
  const writer = useCicada(canWrite);
  const { status, error } = git;

  return (
    <div data-testid="git-panel">
      <div className="insp-title">
        <span className="name" style={{ fontSize: 13 }}>
          git
        </span>
        {status !== null && (
          <span className="mono" data-testid="git-head">
            {describeHead(status.state)}
          </span>
        )}
        <button
          className="tb-esc"
          style={{ marginLeft: "auto" }}
          title="re-read the git status now (it also refreshes after every edit and when the window regains focus)"
          disabled={git.loading}
          onClick={refreshGitNow}
          data-testid="git-refresh"
        >
          {git.loading ? "reading…" : "↻ refresh"}
        </button>
      </div>

      {error !== null && (
        <div className="excluded red" data-testid="git-error">
          git status could not be read: {describeGitError(error)}
        </div>
      )}
      {status === null && error === null && <div className="faint">reading git status…</div>}

      {status !== null && (
        <>
          <StateSection state={status.state} />
          {gitRepoInfo(status.state) !== null && (
            <>
              <MarkersSection />
              <ScopeSection />
              {writer ? (
                <>
                  <section className="insp-section">
                    <h3 className="insp-h">commit</h3>
                    <CommitForm />
                  </section>
                  <section className="insp-section">
                    <h3 className="insp-h">revert</h3>
                    <RevertControl scope={status.scope} />
                  </section>
                </>
              ) : (
                <ObserverNote />
              )}
            </>
          )}
        </>
      )}
    </div>
  );
}

/** Why the controls are missing for this client (observer, or a dropped socket). */
function ObserverNote() {
  const connection = useCicada((s) => s.connection);
  return (
    <section className="insp-section">
      <div className="faint" data-testid="git-observer-note">
        {connection === "open"
          ? "read-only observer — take the lease to commit or revert from the app"
          : "not connected — commit and revert are disabled until the session is back"}
      </div>
    </section>
  );
}

// --------------------------------------------------------------- state --

function StateSection({ state }: { state: GitState }) {
  const info = gitRepoInfo(state);
  if (info === null) {
    return (
      <section className="insp-section">
        <div className="insp-desc" data-testid="git-state" data-kind={state.kind}>
          {state.kind === "not_a_repo" ? (
            <>
              The project is not in a git repository. <code>git init</code> it (any shell) to see per-node
              changes and commit from the app — every edit is already on disk, nothing here is lost.
            </>
          ) : (
            <>
              No <code>git</code> on PATH. Install git (or put it on PATH) and restart <code>cicada serve</code>{" "}
              to use the git panel.
            </>
          )}
        </div>
      </section>
    );
  }
  const operation = info.operation?.replace("_", "-");
  return (
    <section className="insp-section" data-testid="git-state" data-kind={state.kind}>
      <div className="stat-grid">
        <span className="k">branch</span>
        <span className="v">{info.branch ?? <span className="warn-text">detached HEAD</span>}</span>
        <span className="k">HEAD</span>
        <span className="v">{info.head_short ?? <span className="faint">no commits yet (unborn branch)</span>}</span>
        <span className="k">upstream</span>
        <span className="v">
          {info.upstream === null ? (
            <span className="faint">none</span>
          ) : (
            `${info.upstream.name} · ahead ${info.upstream.ahead} · behind ${info.upstream.behind}`
          )}
        </span>
        <span className="k">project</span>
        <span className="v" title={`repository root (as git reports it): ${info.root}`}>
          {info.prefix === "" ? "the repository root" : `${info.prefix}/ in the repository`}
        </span>
      </div>
      {state.kind === "locked" && (
        <div className="excluded blocked" style={{ marginTop: 6 }} data-testid="git-locked">
          index.lock is held by another git process — status still reads; commit and revert wait until it is
          released.
        </div>
      )}
      {operation !== undefined && (
        <div className="excluded blocked" style={{ marginTop: 6 }} data-testid="git-operation">
          a {operation} is in progress in the repository — finish or abort it in your shell; commit and revert
          from the app refuse until then.
        </div>
      )}
    </section>
  );
}

// ------------------------------------------------------------- markers --

function MarkersSection() {
  const pipeline = useCicada((s) => s.git.status?.pipeline);
  const stale = useCicada((s) => s.git.stale);
  const graph = useCicada((s) => s.graph);
  const selected = useCicada((s) => s.selection.nodes);
  const selectNodes = useCicada((s) => s.selectNodes);
  if (pipeline === undefined) return null;
  const groups = groupMarkers(pipeline);
  const total = pipeline.nodes.length + pipeline.removed.length;
  const live = new Set(graph.nodes.map((n) => n.name));
  const row = (change: NodeChange) => {
    const badge = markerBadge(change);
    const gone = !live.has(change.name);
    return (
      <div className="git-node" key={`${change.change}:${change.name}`} data-testid={`git-node-${change.name}`} data-change={change.change}>
        <span className={`git-mark git-mark-${badge.kind}`} title={badge.title} aria-hidden>
          {badge.glyph}
        </span>
        <button
          className={`link${selected.includes(change.name) ? " selected" : ""}`}
          disabled={gone}
          title={gone ? "not in the graph any more (the status is catching up)" : `select \`${change.name}\` on the canvas`}
          onClick={() => selectNodes([change.name])}
        >
          {change.name}
        </button>
        {change.from !== undefined && (
          <span className="faint">
            was <code>{change.from}</code>
          </span>
        )}
      </div>
    );
  };
  const removedRow = (node: RemovedNode) => (
    <div className="git-node" key={`removed:${node.name}`} data-testid={`git-node-${node.name}`} data-change="removed">
      <span className="git-mark git-mark-removed" title="removed since HEAD (bound in the last commit, not in the working tree)" aria-hidden>
        −
      </span>
      <span className="mono removed-name">{node.name}</span>
      <span className="faint">HEAD line {node.line_in_head}</span>
    </div>
  );
  return (
    <section className="insp-section">
      <h3 className="insp-h">
        nodes vs HEAD
        <span className={`badge${total > 0 ? " warn" : ""}`} data-testid="git-marker-count">
          {total}
        </span>
        {!pipeline.tracked && (
          <span className="right faint" title="the pipeline file has no HEAD version: every node is new">
            {pipeline.ignored ? "ignored" : "untracked"}
          </span>
        )}
      </h3>
      {total === 0 && (
        <div className="faint" data-testid={stale ? "git-markers-refreshing" : "git-markers-clean"}>
          {stale ? STALE_REASON : "no node differs from HEAD"}
        </div>
      )}
      {groups.added.length > 0 && (
        <div className="git-group" data-testid="git-group-added">
          <div className="git-group-h">added · {groups.added.length}</div>
          {groups.added.map(row)}
        </div>
      )}
      {groups.modified.length > 0 && (
        <div className="git-group" data-testid="git-group-modified">
          <div className="git-group-h">modified · {groups.modified.length}</div>
          {groups.modified.map(row)}
        </div>
      )}
      {groups.renamed.length > 0 && (
        <div className="git-group" data-testid="git-group-renamed">
          <div className="git-group-h">renamed · {groups.renamed.length}</div>
          {groups.renamed.map(row)}
        </div>
      )}
      {groups.removed.length > 0 && (
        <div className="git-group" data-testid="git-group-removed">
          <div className="git-group-h">removed · {groups.removed.length}</div>
          {groups.removed.map(removedRow)}
        </div>
      )}
    </section>
  );
}

// --------------------------------------------------------------- scope --

/** The dirty files of the commit scope — what a commit from the app would stage. */
export function ScopeList({ scope }: { scope: ScopeFile[] }) {
  return (
    <div className="git-scope" data-testid="git-scope">
      {scope.map((file) => (
        <div className="git-file" key={file.path} data-testid={`git-file-${file.path}`} data-status={file.status}>
          <span className={`badge git-status-${file.status}`}>{file.status}</span>
          <span className="mono">{file.path}</span>
        </div>
      ))}
    </div>
  );
}

/**
 * The line (or list) under the "files to commit" heading, from the ONE
 * rule the Ctrl+S dialog uses too (`scopeNote`): an ignored pipeline never
 * reads "nothing to commit", a stale cache reads "re-reading", a current
 * empty scope reads clean, and a list carries a `refreshing` hint while an
 * edit may still be missing from it. `data-testid`s name the branch taken.
 */
export function ScopeBody({ note }: { note: ScopeNote }) {
  switch (note.kind) {
    case "ignored":
      return (
        <>
          <div className="excluded red" data-testid="git-ignored">
            {ignoredReason(note.path)} (or <code>git add -f</code> it once in a shell — git refuses to add an ignored
            file otherwise).
          </div>
          {note.files.length > 0 && <ScopeList scope={note.files} />}
        </>
      );
    case "refreshing":
      return (
        <div className="faint" data-testid="git-scope-refreshing">
          {STALE_REASON}
        </div>
      );
    case "clean":
      return (
        <div className="faint" data-testid="git-clean">
          nothing to commit — the scope matches HEAD
        </div>
      );
    case "files":
      return (
        <>
          <ScopeList scope={note.files} />
          {note.refreshing && (
            <div className="faint" data-testid="git-scope-refreshing">
              {STALE_REASON}
            </div>
          )}
        </>
      );
  }
}

function ScopeSection() {
  const status = useCicada((s) => s.git.status);
  const stale = useCicada((s) => s.git.stale);
  if (status === null) return null;
  const { scope, pipeline } = status;
  const note = scopeNote(status, stale);
  return (
    <section className="insp-section" data-testid="git-scope-section" data-note={note.kind}>
      <h3 className="insp-h">
        files to commit
        <span className={`badge${scope.length > 0 ? " warn" : ""}${stale ? " stale" : ""}`} data-testid="git-dirty-count">
          {scope.length}
        </span>
      </h3>
      <div className="faint" style={{ marginBottom: 4 }}>
        the scope: <code>{pipeline.path}</code>, its sidecar, and <code>scripts/*.py</code> beside it
      </div>
      {!pipeline.ignored && !pipeline.tracked && (
        <div className="faint">the pipeline is not tracked yet — the first commit adds it</div>
      )}
      <ScopeBody note={note} />
    </section>
  );
}

// -------------------------------------------------------------- commit --

/**
 * The commit form: the message (verbatim — a trailing newline survives),
 * the Commit button with its disabled reason as the tooltip, Ctrl+Enter
 * to commit, Esc to `onCancel`. One form, two homes (the tab, the dialog).
 */
export function CommitForm({
  autoFocus = false,
  onCommitted,
  onCancel,
}: {
  autoFocus?: boolean;
  onCommitted?: () => void;
  onCancel?: () => void;
}) {
  const draft = useCommitDraft((s) => s.draft);
  const setDraft = useCommitDraft((s) => s.setDraft);
  const clearDraft = useCommitDraft((s) => s.clear);
  const blocked = useCicada((s) => commitBlockReason(s, draft));
  const busy = useCicada((s) => s.git.busy);
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);

  const submit = async () => {
    if (blocked !== null) return;
    const commit = await commitFromApp(draft);
    if (commit !== null) {
      clearDraft();
      onCommitted?.();
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const ctrl = event.ctrlKey || event.metaKey;
    if (ctrl && event.key === "Enter") {
      event.preventDefault();
      void submit();
      return;
    }
    // Ctrl+S inside the message field: the window key router consumes it
    // before its text-entry gate (`keyboard.ts` createKeyRouter) — the
    // browser's save never opens, and the dialog is already open.
    if (event.key === "Escape" && onCancel !== undefined) {
      event.preventDefault();
      onCancel();
    }
  };

  return (
    <div className="git-commit" data-testid="git-commit">
      <textarea
        ref={ref}
        className="git-message"
        value={draft}
        placeholder="commit message (verbatim — the first line is the summary)"
        rows={3}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={onKeyDown}
        aria-label="commit message"
        data-testid="git-message"
        spellCheck={false}
      />
      <div className="actions" style={{ alignItems: "center" }}>
        <button
          className="run"
          disabled={blocked !== null}
          title={blocked ?? "commit the scope with this message (Ctrl+Enter)"}
          onClick={() => void submit()}
          data-testid="git-commit-submit"
        >
          {busy === "commit" ? "committing…" : "Commit"}
        </button>
        {onCancel !== undefined && (
          <button onClick={onCancel} data-testid="git-commit-cancel">
            cancel
          </button>
        )}
        <span className="faint" data-testid="git-commit-hint">
          {blocked ?? "Ctrl+Enter commits"}
        </span>
      </div>
    </div>
  );
}

// -------------------------------------------------------------- revert --

/**
 * Revert to HEAD with an inline confirm step: it discards EVERY uncommitted
 * edit in the scope's files that HEAD has a version of (`in_head`, the
 * server's word — files without one are left alone; it never deletes),
 * and the reload barrier clears the undo history — said in so many words
 * before the second click. The confirmation is BINDING: the POST names
 * exactly the files it listed (`revertRequest(scope).paths`), so a file
 * that joined the scope between the last status read and the click is not
 * reverted unseen — the server refuses anything outside the scope and the
 * next read lists the newcomer for a second ask. (`e2e/git.spec.ts` reads
 * the request body: a two-file scope with one file to leave alone posts
 * one path.)
 */
function RevertControl({ scope }: { scope: ScopeFile[] }) {
  const busy = useCicada((s) => s.git.busy);
  const [confirming, setConfirming] = useState(false);
  const { files, untouched, paths } = revertRequest(scope);
  const blocked = useCicada((s) => revertBlockReason(s, files.length));

  // The scope changed under the confirm step (an edit, a refresh): ask again.
  const key = paths.join("\n");
  useEffect(() => {
    setConfirming(false);
  }, [key]);

  if (!confirming) {
    return (
      <div className="actions" style={{ alignItems: "center" }}>
        <button
          className="danger"
          disabled={blocked !== null}
          title={blocked ?? "discard every uncommitted edit in this pipeline's scope and restore HEAD (asks first)"}
          onClick={() => setConfirming(true)}
          data-testid="git-revert"
        >
          {busy === "revert" ? "reverting…" : "Revert to HEAD…"}
        </button>
        {blocked !== null && <span className="faint">{blocked}</span>}
      </div>
    );
  }
  return (
    <div className="git-revert-confirm" data-testid="git-revert-confirm">
      <div>
        This discards <b>every uncommitted edit</b> in {files.length === 1 ? "this file" : `these ${files.length} files`} and
        restores the last commit (HEAD). It cannot be undone — the undo history is cleared too.
      </div>
      <ScopeList scope={files} />
      {untouched.length > 0 && (
        <div className="faint">
          left alone (no HEAD version to go back to): {untouched.map((f) => f.path).join(", ")}
        </div>
      )}
      <div className="actions">
        <button
          className="danger confirm"
          disabled={blocked !== null || busy !== null}
          onClick={() => {
            setConfirming(false);
            void revertToHead(paths);
          }}
          data-testid="git-revert-confirm-yes"
        >
          Revert now
        </button>
        <button onClick={() => setConfirming(false)} data-testid="git-revert-confirm-no">
          keep my edits
        </button>
        {blocked !== null && <span className="faint">{blocked}</span>}
      </div>
    </div>
  );
}
