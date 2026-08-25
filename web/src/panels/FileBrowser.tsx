/**
 * The file browser (docs/16 §Application layout: the landing picker and
 * File → Open…): ONE directory of the served root at a time over `GET
 * /api/files` (never `/api/project` — over a home root its walk is seconds
 * and lists what the picker must not show; docs/17 O1), with breadcrumbs,
 * directories then pipelines, and keyboard navigation — arrows move,
 * Enter opens (a directory descends, a pipeline opens), Backspace goes
 * up, Home/End jump; a double-click is Enter. A refused listing shows the
 * server's typed reason in place of the list and keeps the breadcrumbs, so
 * the user can climb out of it.
 */
import { useEffect, useRef, useState } from "react";
import { describeFilesFailure, fetchFiles } from "../protocol/files";
import type { FileEntry, FilesResponse } from "../protocol/messages";
import { crumbsOf, joinPath, modifiedText, moveCursor } from "./filePaths";
import "./panels.css";

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export interface FileBrowserProps {
  token: string;
  /** The directory to start in, root-relative (`""` = the root). */
  initialDir?: string;
  /** A pipeline was chosen (root-relative path). */
  onOpen: (pipeline: string) => void;
  /** Focus the list on mount (the dialog, the picker). */
  autoFocus?: boolean;
  /** Tests: the fetch to use. */
  fetchImpl?: FetchLike;
}

export function FileBrowser({ token, initialDir = "", onOpen, autoFocus = false, fetchImpl }: FileBrowserProps) {
  const [dir, setDir] = useState(initialDir);
  const [listing, setListing] = useState<FilesResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [cursor, setCursor] = useState(-1);
  const listRef = useRef<HTMLUListElement>(null);
  // The newest request wins: an answer for a directory we have since left
  // is dropped, never shown under the wrong breadcrumbs.
  const request = useRef(0);

  useEffect(() => {
    const id = ++request.current;
    setLoading(true);
    fetchFiles({ token }, dir, fetchImpl)
      .then((answer) => {
        if (request.current !== id) return;
        setListing(answer);
        setError(null);
        setCursor(answer.entries.length > 0 ? 0 : -1);
        setLoading(false);
      })
      .catch((failure: unknown) => {
        if (request.current !== id) return;
        setListing(null);
        setError(describeFilesFailure(failure));
        setCursor(-1);
        setLoading(false);
      });
  }, [token, dir, fetchImpl]);

  useEffect(() => {
    if (autoFocus) listRef.current?.focus();
  }, [autoFocus]);

  // Keep the cursor's row in view as the keys move it.
  useEffect(() => {
    if (cursor < 0) return;
    const row = listRef.current?.children[cursor];
    // (jsdom has no `scrollIntoView`; the component tests run there.)
    if (row instanceof HTMLElement && typeof row.scrollIntoView === "function") row.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const entries = listing?.entries ?? [];
  const parent = listing?.parent ?? (dir === "" ? null : crumbsOf(dir).at(-2)?.dir ?? "");

  const activate = (entry: FileEntry) => {
    const path = joinPath(dir, entry.name);
    if (entry.kind === "dir") setDir(path);
    else onOpen(path);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp":
      case "Home":
      case "End":
        event.preventDefault();
        setCursor(moveCursor(cursor, event.key, entries.length));
        break;
      case "Enter": {
        event.preventDefault();
        const entry = entries[cursor];
        if (entry !== undefined) activate(entry);
        break;
      }
      case "Backspace":
        event.preventDefault();
        if (parent !== null) setDir(parent);
        break;
      default:
        break;
    }
  };

  const rootLabel = listing?.root ?? "root";
  return (
    <div className="files" data-testid="file-browser" data-dir={dir} data-loading={loading}>
      <nav className="files-crumbs" aria-label="directory">
        <button
          type="button"
          className={`files-crumb${dir === "" ? " current" : ""}`}
          onClick={() => setDir("")}
          title="the served root"
          data-testid="files-crumb-root"
        >
          {rootLabel}
        </button>
        {crumbsOf(dir).map((crumb) => (
          <span key={crumb.dir} className="files-crumb-wrap">
            <span className="files-crumb-sep" aria-hidden>
              ›
            </span>
            <button
              type="button"
              className={`files-crumb${crumb.dir === dir ? " current" : ""}`}
              onClick={() => setDir(crumb.dir)}
              data-testid={`files-crumb-${crumb.label}`}
            >
              {crumb.label}
            </button>
          </span>
        ))}
      </nav>
      {error !== null && (
        <div className="files-error" role="alert" data-testid="files-error">
          {error}
        </div>
      )}
      <ul
        ref={listRef}
        className="files-list"
        role="listbox"
        tabIndex={0}
        aria-label="directories and pipelines"
        aria-activedescendant={cursor >= 0 ? `files-entry-${cursor}` : undefined}
        onKeyDown={onKeyDown}
        data-testid="files-list"
      >
        {entries.map((entry, index) => (
          <li
            key={`${entry.kind}:${entry.name}`}
            id={`files-entry-${index}`}
            role="option"
            aria-selected={index === cursor}
            className={`files-entry ${entry.kind}${index === cursor ? " cursor" : ""}`}
            onClick={() => setCursor(index)}
            onDoubleClick={() => activate(entry)}
            title={entry.kind === "dir" ? "open the directory (Enter)" : "open the pipeline (Enter)"}
            data-testid={`files-entry-${entry.name}`}
            data-kind={entry.kind}
          >
            <span className="files-glyph" aria-hidden>
              {entry.kind === "dir" ? "▸" : "·"}
            </span>
            <span className={`files-name${entry.kind === "pipeline" ? " mono" : ""}`}>{entry.name}</span>
            <span className="files-when faint">{modifiedText(entry)}</span>
          </li>
        ))}
      </ul>
      {!loading && error === null && entries.length === 0 && (
        <div className="faint files-empty" data-testid="files-empty">
          nothing here — no directories and no <code>.cic</code> files
        </div>
      )}
      <div className="files-hint faint">
        ↑ ↓ move · Enter opens · Backspace goes up{parent === null ? "" : ` (to ${parent === "" ? rootLabel : parent})`}
      </div>
    </div>
  );
}
