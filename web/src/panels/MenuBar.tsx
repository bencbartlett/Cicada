/**
 * The menu bar (docs/16 §Application layout; docs/17 wave 5 M1, finding
 * U27): one tab per docs/08 category (label · count) populated from the
 * JSON catalog. A click opens a dropdown panel under the tab whose columns
 * are the category's sub-groups in the catalog's table order
 * (`ribbonTabs`), each a titled list of node buttons — title and name on
 * the button; the description, the GH name and the node's contract ("Red
 * when") in the hover. Hovering another tab while a panel is open switches
 * to it. The panel closes on a pointerdown outside the bar, on Esc, on a
 * re-click of its tab, and on a placement — `place_node` at the cell under
 * the centre of the canvas view (U29: the canvas keeps it in the store; the
 * user must see what a click did). Nothing below the top bar is persistent
 * any more: the wave-4 `ribbonCollapsed` setting is gone. Observers see
 * the node buttons disabled with the reason in the hover.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ghHint } from "../canvas/grid";
import { kindColor } from "../kinds";
import type { CatalogNode } from "../protocol/messages";
import { canWrite, useCicada, writeBlockReason } from "../state/store";
import { ribbonTabs } from "./ribbonTabs";
import "./panels.css";

/** The panel keeps this many pixels from the bar's edges when a tab sits too far right for it. */
const PANEL_MARGIN_PX = 6;

export function MenuBar() {
  const catalog = useCicada((s) => s.catalog);
  const writer = useCicada(canWrite);
  const send = useCicada((s) => s.send);
  const center = useCicada((s) => s.canvasCenter);
  /** The open tab's category; null = every panel closed. */
  const [open, setOpen] = useState<string | null>(null);
  const navRef = useRef<HTMLElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef(new Map<string, HTMLButtonElement>());
  const [panelLeft, setPanelLeft] = useState(0);

  const tabs = useMemo(() => ribbonTabs(catalog?.nodes ?? [], catalog?.subgroups ?? []), [catalog]);
  // A catalog reload can drop the open category (a script's category
  // changed under us): then there is nothing to show and the panel is gone.
  const openTab = open === null ? null : (tabs.find((t) => t.category === open) ?? null);
  const isOpen = openTab !== null;

  // Outside pointerdown and Esc close the panel — like the top bar's File
  // and settings menus. A pointerdown on the bar itself is a tab's own
  // business (re-click closes, another tab opens).
  useEffect(() => {
    if (!isOpen) return;
    const onDown = (event: PointerEvent) => {
      if (navRef.current !== null && !navRef.current.contains(event.target as Node)) setOpen(null);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(null);
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  // The panel hangs under its tab, pulled left when it would overrun the
  // bar's right edge (the last tabs), never past the left one.
  useLayoutEffect(() => {
    if (openTab === null) return;
    const nav = navRef.current;
    const panel = panelRef.current;
    const tab = tabRefs.current.get(openTab.category);
    if (nav === null || panel === null || tab === undefined) return;
    const navRect = nav.getBoundingClientRect();
    const wanted = tab.getBoundingClientRect().left - navRect.left;
    const maxLeft = navRect.width - panel.getBoundingClientRect().width - PANEL_MARGIN_PX;
    setPanelLeft(Math.max(PANEL_MARGIN_PX, Math.min(wanted, maxLeft)));
  }, [openTab]);

  const place = (func: string) => {
    if (!writer) {
      const state = useCicada.getState();
      state.addNotice("warning", `${writeBlockReason(state) ?? "cannot write"} — placing nodes ignored`);
      return;
    }
    // The centre of the view when the canvas has reported one; else the
    // server's auto-layout (no canvas mounted yet).
    send({ type: "place_node", payload: { func, cell: center } });
    setOpen(null);
  };

  return (
    <nav className="menubar" data-testid="menubar" aria-label="node catalog" ref={navRef}>
      <div className="mb-tabs" role="menubar">
        {tabs.map((tab) => {
          const active = openTab?.category === tab.category;
          return (
            <button
              key={tab.category}
              ref={(el) => {
                if (el === null) tabRefs.current.delete(tab.category);
                else tabRefs.current.set(tab.category, el);
              }}
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={active}
              className={`mb-tab${active ? " open" : ""}`}
              title={tab.category}
              data-testid={`menu-tab-${tab.label}`}
              onClick={() => setOpen(active ? null : tab.category)}
              onPointerEnter={() => {
                if (isOpen && !active) setOpen(tab.category);
              }}
            >
              {tab.label}
              <span className="count">{tab.nodes.length}</span>
            </button>
          );
        })}
        {tabs.length === 0 && <span className="mb-empty">catalog loading…</span>}
      </div>
      {openTab !== null && (
        <div
          className="mb-panel"
          role="menu"
          aria-label={openTab.category}
          data-testid="menu-panel"
          data-no-hotkeys
          ref={panelRef}
          style={{ left: panelLeft }}
        >
          {openTab.columns.map((column) => (
            <div className="mb-col" role="group" aria-label={column.sub} data-testid={`menu-col-${column.sub}`} key={column.sub}>
              <span className="mb-col-h">{column.sub}</span>
              {column.nodes.map((node) => (
                <button
                  key={node.name}
                  role="menuitem"
                  className="mb-node"
                  style={{ borderLeftColor: kindColor(node.outputs[0]?.base ?? "") }}
                  disabled={!writer}
                  title={nodeTooltip(node, writer)}
                  data-testid={`menu-node-${node.name}`}
                  onClick={() => place(node.name)}
                >
                  <span className="mb-node-title">{node.title}</span>
                  <span className="mb-node-name">{node.name}</span>
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </nav>
  );
}

/** The hover: the description, the GH name when it says something the title does not, the contract, and why an observer cannot place. */
function nodeTooltip(node: CatalogNode, writer: boolean): string {
  const lines = [node.description.trim()];
  const gh = ghHint(node);
  if (gh !== null) lines.push(`GH: ${gh}`);
  if (node.panics) lines.push(`Red when: ${node.panics.trim()}`);
  if (!writer) lines.push("(read-only — take the lease, or wait for the connection, to place)");
  return lines.join("\n");
}
