/**
 * Text tab (docs/16 §Application layout): the read-only view of the `.cic`
 * file — mono, line numbers, the selected node's binding highlighted and
 * scrolled into view; a selected wire highlights its target line; clicking
 * a line selects the node bound there. Editing stays canvas-only (v1).
 */
import { useEffect, useMemo, useRef } from "react";
import { useCicada } from "../state/store";
import { highlightedLines, lineOwners } from "./format";

export function TextPanel() {
  const text = useCicada((s) => s.text);
  const graph = useCicada((s) => s.graph);
  const selection = useCicada((s) => s.selection);
  const selectNodes = useCicada((s) => s.selectNodes);
  const firstHighlight = useRef<HTMLDivElement>(null);

  const lines = useMemo(() => text.split("\n"), [text]);
  const owners = useMemo(() => lineOwners(graph.nodes), [graph]);
  const highlighted = useMemo(() => highlightedLines(graph.nodes, selection.nodes), [graph, selection.nodes]);
  const wireLines = useMemo(() => {
    if (selection.wire === null) return new Set<number>();
    const wire = graph.wires.find((w) => w.id === selection.wire);
    return wire === undefined ? new Set<number>() : highlightedLines(graph.nodes, [wire.to.node]);
  }, [graph, selection.wire]);
  const diagLines = useMemo(() => new Set(graph.diagnostics.map((d) => d.span.line)), [graph]);

  const firstLine = Math.min(...highlighted, ...wireLines);

  useEffect(() => {
    firstHighlight.current?.scrollIntoView({ block: "nearest" });
  }, [firstLine, text]);

  if (text.length === 0) {
    return <div className="faint">no text yet — waiting for the snapshot</div>;
  }
  return (
    <div className="text-panel" data-testid="text-panel">
      {lines.map((source, i) => {
        const line = i + 1;
        const owner = owners.get(line);
        const isHl = highlighted.has(line);
        const isWire = wireLines.has(line);
        const cls = [
          "text-line",
          owner !== undefined ? "owned" : "",
          isHl ? "hl" : "",
          isWire && !isHl ? "hl-wire" : "",
          diagLines.has(line) ? "diag-line" : "",
        ]
          .filter(Boolean)
          .join(" ");
        return (
          <div
            key={line}
            className={cls}
            data-line={line}
            data-node={owner}
            ref={line === firstLine ? firstHighlight : undefined}
            title={owner !== undefined ? `select ${owner}` : undefined}
            onClick={owner !== undefined ? () => selectNodes([owner]) : undefined}
          >
            <span className="text-ln">{line}</span>
            <span className={`text-src${source.trimStart().startsWith("#") ? " cmt" : ""}`}>
              {source.length === 0 ? " " : source}
            </span>
          </div>
        );
      })}
    </div>
  );
}
