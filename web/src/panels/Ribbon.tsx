/**
 * Ribbon (docs/16 §Application layout): the GH tab bar — one tab per docs/08
 * category populated from the JSON catalog; the active tab lists its nodes
 * as buttons; click → `place_node` (the server auto-lays it out).
 * Collapsible to tab names only (`settings.ribbonCollapsed`).
 */
import { useMemo, useState } from "react";
import { kindColor } from "../kinds";
import type { CatalogNode } from "../protocol/messages";
import { canWrite, useCicada, writeBlockReason } from "../state/store";
import { ribbonTabs } from "./ribbonTabs";
import "./panels.css";

export function Ribbon() {
  const catalog = useCicada((s) => s.catalog);
  const collapsed = useCicada((s) => s.settings.ribbonCollapsed);
  const updateSettings = useCicada((s) => s.updateSettings);
  const writer = useCicada(canWrite);
  const send = useCicada((s) => s.send);
  const [active, setActive] = useState<string | null>(null);

  const tabs = useMemo(() => ribbonTabs(catalog?.nodes ?? []), [catalog]);
  const activeTab = tabs.find((t) => t.category === active) ?? tabs[0] ?? null;

  const place = (func: string) => {
    if (!writer) {
      const state = useCicada.getState();
      state.addNotice("warning", `${writeBlockReason(state) ?? "cannot write"} — placing nodes ignored`);
      return;
    }
    send({ type: "place_node", payload: { func, cell: null } });
  };

  return (
    <nav className="ribbon" data-testid="ribbon" aria-label="node catalog">
      <div className="rb-tabs" role="tablist">
        {tabs.map((tab) => (
          <button
            key={tab.category}
            role="tab"
            aria-selected={activeTab?.category === tab.category}
            className={`rb-tab${activeTab?.category === tab.category ? " active" : ""}`}
            title={tab.category}
            data-testid={`rb-tab-${tab.label}`}
            onClick={() => {
              setActive(tab.category);
              if (collapsed) updateSettings({ ribbonCollapsed: false });
            }}
          >
            {tab.label}
            <span className="count">{tab.nodes.length}</span>
          </button>
        ))}
        {tabs.length === 0 && <span className="rb-empty">catalog loading…</span>}
        <button
          className="rb-collapse"
          title={collapsed ? "expand the ribbon" : "collapse to tab names"}
          aria-expanded={!collapsed}
          onClick={() => updateSettings({ ribbonCollapsed: !collapsed })}
        >
          {collapsed ? "▾ expand" : "▴ collapse"}
        </button>
      </div>
      {!collapsed && activeTab !== null && (
        <div className="rb-nodes" role="tabpanel" data-testid="rb-nodes">
          {activeTab.nodes.map((node) => (
            <button
              key={node.name}
              className="rb-node"
              style={{ borderLeftColor: kindColor(node.outputs[0]?.base ?? "") }}
              disabled={!writer}
              title={nodeTooltip(node, writer)}
              data-testid={`rb-node-${node.name}`}
              onClick={() => place(node.name)}
            >
              <span className="rb-node-title">{node.title}</span>
              <span className="rb-node-name">{node.name}</span>
            </button>
          ))}
        </div>
      )}
    </nav>
  );
}

function nodeTooltip(node: CatalogNode, writer: boolean): string {
  const lines = [node.description.trim()];
  if (node.panics) lines.push(`Red when: ${node.panics.trim()}`);
  if (!writer) lines.push("(read-only — take the lease, or wait for the connection, to place)");
  return lines.join("\n");
}
