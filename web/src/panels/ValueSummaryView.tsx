/**
 * One cached value summary (docs/02's Param-Viewer-in-place): kind ·
 * count/absent/axis · bounds · samples · facts · truncated hash with copy.
 */
import { kindColor } from "../kinds";
import type { ValueSummary } from "../protocol/messages";
import { useCicada } from "../state/store";
import { boundsText, factsList, shortHash, valueHeadline } from "./format";

export function ValueSummaryView({
  summary,
  stale = false,
}: {
  summary: ValueSummary | null;
  stale?: boolean;
}) {
  if (summary === null) {
    return (
      <div className="value-box faint" data-testid="value-box">
        no value (absent or not computed)
      </div>
    );
  }
  const facts = factsList(summary.facts);
  return (
    <div className={`value-box${stale ? " stale" : ""}`} data-testid="value-box" data-kind={summary.kind}>
      <div>
        <span className="kind-badge" style={{ color: kindColor(summary.kind) }}>
          {summary.kind}
        </span>{" "}
        <span className="dim">{valueHeadline(summary).slice(summary.kind.length).replace(/^ · /, "")}</span>
        {stale && <span className="faint"> · previous generation</span>}
      </div>
      {summary.bounds !== undefined && (
        <div className="mono dim" title="bounds">
          {boundsText(summary.bounds)}
        </div>
      )}
      {facts.length > 0 && (
        <div className="facts">
          {facts.map(([k, v]) => (
            <span key={k}>
              {k} <b data-fact={k}>{v}</b>
            </span>
          ))}
        </div>
      )}
      {summary.samples !== undefined && summary.samples.length > 0 && (
        <ul className="samples" title="samples">
          {summary.samples.map((s, i) => (
            <li key={i}>{s}</li>
          ))}
        </ul>
      )}
      <span className="hash" title={summary.hash}>
        #{shortHash(summary.hash)}
        <button
          onClick={() => {
            navigator.clipboard
              .writeText(summary.hash)
              .then(() => useCicada.getState().addNotice("info", `copied hash ${shortHash(summary.hash)}…`))
              .catch((e: unknown) => useCicada.getState().addNotice("error", `clipboard: ${String(e)}`));
          }}
          title="copy the full hash"
        >
          copy
        </button>
      </span>
    </div>
  );
}
