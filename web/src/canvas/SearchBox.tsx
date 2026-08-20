/**
 * Search-to-place (docs/16): opened by double-click, Ctrl+F (keyboard map),
 * the canvas menu, or by dropping a wire on empty canvas — in which case the
 * list is filtered to funcs with a port that accepts the probed wire and
 * placing also connects (`place_node {connect}`). Prefix/substring match
 * (v1) over the dialect name, the title and the Grasshopper name the node
 * replaces (`filterCatalog`); a row shows that GH name as a hint when it
 * differs from the title, so a migrant typing `Merge` sees why `concat` came
 * first.
 */
import { useReactFlow } from "@xyflow/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { isCommitChord } from "../keyboard";
import { categoryLabel } from "../kinds";
import { useCicada } from "../state/store";
import { sendWrite } from "./flow";
import { filterCatalog, ghHint, pxToCell, type SearchHit } from "./grid";

interface Props {
  /** Pane-relative anchor. */
  left: number;
  top: number;
}

export function SearchBox({ left, top }: Props) {
  const search = useCicada((s) => s.search);
  const catalog = useCicada((s) => s.catalog);
  const probe = useCicada((s) => s.probe);
  const closeSearch = useCicada((s) => s.closeSearch);
  const clearProbe = useCicada((s) => s.clearProbe);
  const unit = useCicada((s) => s.hello?.unitPx ?? 24);
  const rf = useReactFlow();
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const from = search?.from ?? null;
  const probeCatalog =
    from !== null && probe !== null && probe.from.node === from.node && probe.from.port === from.port
      ? probe.catalog
      : null;
  const awaitingProbe = from !== null && probeCatalog === null;

  const hits = useMemo(
    () => (awaitingProbe ? [] : filterCatalog(catalog, query, probeCatalog)),
    [catalog, query, probeCatalog, awaitingProbe],
  );

  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  useEffect(() => {
    setCursor(0);
  }, [query, hits.length]);
  useEffect(() => {
    const el = listRef.current?.children[cursor];
    if (el instanceof HTMLElement) el.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  if (search === null) return null;

  const close = () => {
    closeSearch();
    if (from !== null) clearProbe();
  };

  const place = (hit: SearchHit, port?: string) => {
    let cell = search.cell;
    if (cell === null) {
      const p = rf.screenToFlowPosition({ x: search.x, y: search.y });
      cell = pxToCell(p.x, p.y, unit);
    }
    let connect = null;
    if (from !== null) {
      const chosen = port !== undefined ? hit.ports.find(([p]) => p === port) : hit.ports[0];
      if (chosen === undefined) {
        useCicada.getState().addNotice("error", `${hit.node.name} has no port that accepts this wire`);
        return;
      }
      connect = { from, to_port: chosen[0], lift: chosen[1] === "lift" };
    }
    if (sendWrite({ type: "place_node", payload: { func: hit.node.name, cell, connect } })) close();
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    // Ctrl+S is the commit dialog (docs/16) from EVERY surface: let it
    // bubble to the window key router, which consumes it — a stopped
    // native event never reaches the window, and the browser's own save
    // dialog would open over the search box.
    if (isCommitChord(event)) return;
    // Everything else typed here is the search's: never the hotkey map's,
    // never React Flow's document-level key handling (Space = pan, etc.).
    event.stopPropagation();
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        close();
        break;
      case "ArrowDown":
        event.preventDefault();
        setCursor((c) => Math.min(hits.length - 1, c + 1));
        break;
      case "ArrowUp":
        event.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
        break;
      case "Enter": {
        event.preventDefault();
        const hit = hits[cursor];
        if (hit !== undefined) place(hit);
        break;
      }
      default:
        break;
    }
  };

  return (
    <div
      className="cv-search nodrag nopan nowheel"
      style={{ left, top }}
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.stopPropagation()}
      data-testid="search-box"
    >
      <input
        ref={inputRef}
        className="cv-search-input"
        placeholder={from !== null ? `nodes accepting ${from.node}.${from.port}…` : "search nodes…"}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={onKeyDown}
        aria-label="search nodes"
        data-testid="search-input"
      />
      <ul className="cv-search-list" ref={listRef} role="listbox">
        {catalog === null && <li className="cv-search-empty faint">catalog not loaded yet</li>}
        {awaitingProbe && <li className="cv-search-empty faint">probing compatible ports…</li>}
        {catalog !== null && !awaitingProbe && hits.length === 0 && (
          <li className="cv-search-empty faint">
            {from !== null ? "no catalog node accepts this wire" : `no node matches "${query}"`}
          </li>
        )}
        {hits.map((hit, index) => {
          const gh = ghHint(hit.node);
          return (
            <li
              key={hit.node.name}
              role="option"
              aria-selected={index === cursor}
              className={`cv-search-item${index === cursor ? " active" : ""}`}
              onMouseEnter={() => setCursor(index)}
              onClick={() => place(hit)}
              title={hit.node.description}
              data-testid="search-item"
              data-func={hit.node.name}
            >
              <span className="cv-search-name">{hit.node.name}</span>
              <span className="cv-search-title dim">{hit.node.title}</span>
              {gh !== null && (
                <span className="cv-search-gh faint" title={`replaces Grasshopper's ${gh}`} data-testid="search-gh">
                  GH {gh}
                </span>
              )}
              <span className="cv-search-cat faint">{categoryLabel(hit.node.category)}</span>
              {hit.ports.length > 0 && (
                <span className="cv-search-ports">
                  {hit.ports.map(([port, verdict]) => (
                    <button
                      type="button"
                      key={port}
                      className={`cv-search-port ${verdict}`}
                      title={
                        verdict === "lift"
                          ? `→ ${port} with each() (mapped)`
                          : `→ ${port}`
                      }
                      onClick={(event) => {
                        event.stopPropagation();
                        place(hit, port);
                      }}
                    >
                      {port}
                      {verdict === "lift" ? " · map" : ""}
                    </button>
                  ))}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
