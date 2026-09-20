/**
 * The profiler tab (docs/16 §Inspector contents; docs/13 §The profiler;
 * v0.1 wave 5 P1): the last complete generation itemised — an SVG ring of
 * its phases (the top nodes by cost and the rest, tessellation, encode, the
 * socket, the client's decode and upload), the server's and the client's
 * phases as numbers, a sortable, filterable table of every node with its
 * state, time, share and elements (cached rows marked), the outputs the
 * pass drew, and the two caches with the budget control. The `profile` read
 * goes out when the tab shows and after every pass lands; observers read
 * it too; Esc closes it (the keyboard map).
 */
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { isEditableTarget } from "../keyboard";
import type { ProfileDisplay, ProfileNode } from "../protocol/messages";
import { frameBus } from "../state/frameBus";
import { useCicada } from "../state/store";
import { DisplayCachePicker } from "./DisplayCachePicker";
import { formatBytes, formatMs, formatNanos } from "./format";
import { useInspectorTab } from "./inspectorTab";
import {
  DEFAULT_NODE_SORT,
  RING_SIZE,
  RING_WIDTH,
  clientPhases,
  filterProfileNodes,
  nextSort,
  nodeShares,
  nodeTimeNanos,
  profileTitle,
  rateText,
  ringArcs,
  ringPhases,
  shareText,
  sortProfileNodes,
  type NodeSort,
  type NodeSortKey,
  type RingArc,
} from "./profile";
import "./panels.css";

const COLUMNS: [NodeSortKey, string, string][] = [
  ["name", "node", "the binding"],
  ["state", "state", "the state word of the generation (docs/16's one vocabulary)"],
  ["time", "time", "this generation's measured work for a computed node; a cached node's LAST compute, from its memo entry"],
  ["share", "share", "of this generation's measured work — computed nodes only"],
  ["elements", "elements", "elements processed (this generation's for a computed node, the last compute's for a cached one)"],
];

export function ProfilePanel() {
  const profile = useCicada((s) => s.profile);
  const display = useCicada((s) => s.display);
  const summary = useCicada((s) => s.summary);
  const connection = useCicada((s) => s.connection);
  const snapshots = useCicada((s) => s.snapshots);
  const send = useCicada((s) => s.send);
  const focus = useInspectorTab((s) => s.profileFocus);
  const consumeFocus = useInspectorTab((s) => s.consumeProfileFocus);
  const asked = useRef("");
  const cachesRef = useRef<HTMLElement>(null);

  // Ask once per landed pass (and per re-hydration): the profile is the
  // LAST COMPLETE generation's, final once its `display_end` has been
  // heard — a pass still painting is not asked for (its answer would be the
  // previous generation's, replaced moments later).
  useEffect(() => {
    if (connection !== "open") return;
    if (display !== null && display.phase !== "painted") return;
    const key = `${display?.generation ?? 0}:${summary.generation}:${summary.running ? 1 : 0}:${snapshots}`;
    if (asked.current === key) return;
    asked.current = key;
    send({ type: "profile", payload: {} });
  }, [connection, display, summary.generation, summary.running, snapshots, send]);

  // Esc closes the tab. The keyboard map does it when the key reaches it;
  // from a FOCUSED BUTTON — the top bar's `profile` button or the caches
  // indicator the user just clicked, a tab — plain keys stay with the
  // control and never reach the map (`hotkeysReach`), so the panel listens
  // for that case itself: the same one rule, never twice (a press the map
  // consumed is `defaultPrevented`; a text field keeps its Esc).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || isEditableTarget(event.target)) return;
      const tabs = useInspectorTab.getState();
      if (tabs.tab === "profile") tabs.setTab("inspect");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The caches indicator's click lands on the caches section.
  useEffect(() => {
    if (focus !== "caches" || cachesRef.current === null) return;
    // (jsdom has no `scrollIntoView`; the focus is consumed either way.)
    if (typeof cachesRef.current.scrollIntoView === "function") cachesRef.current.scrollIntoView({ block: "start" });
    consumeFocus();
  }, [focus, consumeFocus, profile]);

  // The client's phases: the frame bus's record of this generation's
  // frames (final once the pass landed) and the viewport's paint stamp.
  const client = useMemo(
    () => (profile === null ? null : clientPhases(profile, frameBus.generation(profile.generation), display)),
    [profile, display],
  );
  const shares = useMemo(() => (profile === null ? new Map<string, number>() : nodeShares(profile.nodes)), [profile]);

  if (profile === null || client === null) {
    return (
      <div data-testid="profile-view" data-generation="none">
        <div className="insp-title">
          <span className="name" style={{ fontSize: 13 }}>
            profile
          </span>
          <span className="faint">{connection === "open" ? "asking the session for the last complete generation…" : "not connected"}</span>
        </div>
        <ProfileCaches sectionRef={cachesRef} />
      </div>
    );
  }

  const phases = ringPhases(profile, client);
  const arcs = ringArcs(phases);
  const total = arcs.reduce((sum, arc) => sum + arc.ms, 0);
  const rate = rateText(profile.phases.bytes, client.rate_bytes_per_ms);

  return (
    <div data-testid="profile-view" data-generation={profile.generation} data-kind={profile.kind}>
      <div className="insp-title">
        <span className="name" style={{ fontSize: 13 }}>
          profile
        </span>
        <span className="mono" data-testid="profile-title">
          {profileTitle(profile)}
        </span>
        <span className="faint">Esc closes</span>
      </div>

      <section className="insp-section">
        <h3 className="insp-h" title="a node's time is its measured work (CPU, summed across chunks); the other phases are wall time">
          phases
        </h3>
        <div className="prof-ring-row">
          <svg
            className="prof-ring"
            viewBox={`0 0 ${RING_SIZE} ${RING_SIZE}`}
            width={RING_SIZE}
            height={RING_SIZE}
            role="img"
            aria-label={`the generation's phases: ${arcs.map((a) => `${a.label} ${shareText(a.share)}`).join(", ") || "nothing took time"}`}
            data-testid="profile-ring"
          >
            {arcs.map((arc) => (
              <path
                key={arc.id}
                d={arc.d}
                fill="none"
                stroke={arc.color}
                strokeWidth={RING_WIDTH}
                data-testid="profile-arc"
                data-phase={arc.id}
                data-kind={arc.kind}
              >
                <title>{arcTitle(arc, rate)}</title>
              </path>
            ))}
            <text x={RING_SIZE / 2} y={RING_SIZE / 2 - 2} className="prof-ring-total" textAnchor="middle" data-testid="profile-ring-total">
              {formatMs(total)}
            </text>
            <text x={RING_SIZE / 2} y={RING_SIZE / 2 + 12} className="prof-ring-caption" textAnchor="middle">
              {arcs.length === 0 ? "nothing timed" : "in all"}
            </text>
          </svg>
          <ul className="prof-legend" data-testid="profile-legend">
            {arcs.map((arc) => (
              <li key={arc.id} data-phase={arc.id} title={arcTitle(arc, rate)}>
                <i className="prof-swatch" style={{ background: arc.color }} aria-hidden />
                <span className={`prof-legend-label${arc.kind === "node" ? " mono" : ""}`}>{arc.label}</span>
                <span className="prof-legend-ms mono">{formatMs(arc.ms)}</span>
                <span className="prof-legend-share faint">{shareText(arc.share)}</span>
              </li>
            ))}
            {arcs.length === 0 && <li className="faint">nothing took time this generation</li>}
          </ul>
          <div className="faint prof-note">node times are measured work; the rest wall time</div>
        </div>
        <div className="stat-grid prof-phases" data-testid="profile-phases">
          <span className="k">queued</span>
          <span className="v">{formatMs(profile.phases.queued_ms)}</span>
          <span className="k">solve</span>
          <span className="v">{formatMs(profile.phases.solve_ms)}</span>
          <span className="k">tessellation</span>
          <span className="v">{formatMs(profile.phases.tessellate_ms)}</span>
          <span className="k">encode</span>
          <span className="v">
            {formatMs(profile.phases.encode_ms)} · {formatBytes(profile.phases.bytes)}
          </span>
          <span className="k" title="the frames' transfer and queueing, as this client measured it">
            socket
          </span>
          <span className="v" data-testid="profile-socket">
            {client.socket_ms === null ? "—" : `${formatMs(client.socket_ms)}${rate === null ? "" : ` · ${rate}`}`}
          </span>
          <span className="k">decode</span>
          <span className="v" data-testid="profile-decode">
            {client.frames === 0 ? "—" : `${formatMs(client.decode_ms)} · ${client.frames} ${client.frames === 1 ? "frame" : "frames"}`}
          </span>
          <span className="k">upload</span>
          <span className="v" data-testid="profile-upload">
            {client.frames === 0 ? "—" : formatMs(client.upload_ms)}
          </span>
          <span className="k" title="display_begin to the first render after the last frame applied — this client's wall">
            first paint
          </span>
          <span className="v" data-testid="profile-first-paint">
            {client.first_paint_ms === null ? "—" : formatMs(client.first_paint_ms)}
          </span>
        </div>
      </section>

      <NodesTable nodes={profile.nodes} shares={shares} />

      <section className="insp-section">
        <h3 className="insp-h">
          display
          <span className="right faint">{profile.display.length === 0 ? "this pass drew nothing new" : "what this pass drew"}</span>
        </h3>
        {profile.display.length > 0 && (
          <table className="prof-table prof-display" data-testid="profile-display">
            <thead>
              <tr>
                <th>output</th>
                <th className="prof-col-tier" title="the tier its solids were meshed at; — for an output without solids">
                  tier
                </th>
                <th className="prof-col-num">tris</th>
                <th className="prof-col-num">bytes</th>
                <th className="prof-col-num" title="display-cache lookups the pass made for the value: answered by the cache / by the kernel">
                  cache
                </th>
              </tr>
            </thead>
            <tbody>
              {profile.display.map((row) => (
                <DisplayRow key={`${row.node}.${row.output}`} row={row} />
              ))}
            </tbody>
          </table>
        )}
      </section>

      <ProfileCaches sectionRef={cachesRef} />
    </div>
  );
}

function arcTitle(arc: RingArc, rate: string | null): string {
  const what = arc.kind === "node" ? `${arc.label}: measured work` : arc.label;
  const line = `${what} · ${formatMs(arc.ms)} · ${shareText(arc.share)}`;
  return arc.id === "socket" && rate !== null ? `${line} · ${rate}` : line;
}

function NodesTable({ nodes, shares }: { nodes: ProfileNode[]; shares: Map<string, number> }) {
  const [sort, setSort] = useState<NodeSort>(DEFAULT_NODE_SORT);
  const [filter, setFilter] = useState("");
  const selectNodes = useCicada((s) => s.selectNodes);
  const rows = useMemo(() => sortProfileNodes(filterProfileNodes(nodes, filter), sort, shares), [nodes, filter, sort, shares]);
  return (
    <section className="insp-section">
      <h3 className="insp-h">
        nodes
        <span className="right">
          <input
            className="prof-filter"
            type="search"
            placeholder="filter"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            aria-label="filter nodes by name or state"
            data-no-hotkeys
            data-testid="profile-filter"
          />
        </span>
      </h3>
      <table className="prof-table prof-nodes" data-testid="profile-nodes" data-sort={sort.key} data-descending={sort.descending}>
        <thead>
          <tr>
            {COLUMNS.map(([key, label, title]) => (
              <th key={key} className={`prof-col-${key}`} aria-sort={sort.key === key ? (sort.descending ? "descending" : "ascending") : "none"}>
                <button
                  className={`prof-sort${sort.key === key ? " active" : ""}`}
                  title={`${title} — click to sort`}
                  onClick={() => setSort((s) => nextSort(s, key))}
                  data-testid={`profile-sort-${key}`}
                >
                  {label}
                  {sort.key === key ? (sort.descending ? " ▾" : " ▴") : ""}
                </button>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((node) => (
            <NodeRow key={node.name} node={node} share={shares.get(node.name) ?? null} onSelect={() => selectNodes([node.name])} />
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={COLUMNS.length} className="faint">
                {nodes.length === 0 ? "no nodes" : `no node matches “${filter}”`}
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}

function NodeRow({ node, share, onSelect }: { node: ProfileNode; share: number | null; onSelect: () => void }) {
  const nanos = nodeTimeNanos(node);
  const cached = node.state === "cached";
  return (
    <tr className={`prof-row state-${node.state}${cached ? " cached" : ""}`} data-testid="profile-node-row" data-node={node.name} data-state={node.state}>
      <td>
        <button className="link mono" onClick={onSelect} title={`${node.name} — select the node`}>
          {node.name}
        </button>
      </td>
      <td>
        <span className={`status-line state-${node.state}`}>
          <span className="state-dot" />
          {node.state}
        </span>
      </td>
      <td className="mono" title={cached ? "the LAST compute's time, from the memo entry — this generation paid a cache read" : undefined}>
        {nanos === null ? "—" : cached ? <span className="faint">last {formatNanos(nanos)}</span> : formatNanos(nanos)}
      </td>
      <td className="mono">{shareText(share)}</td>
      <td className="mono">{node.elements === undefined ? "—" : node.elements.toLocaleString("en-US")}</td>
    </tr>
  );
}

function DisplayRow({ row }: { row: ProfileDisplay }) {
  return (
    <tr data-testid="profile-display-row" data-output={`${row.node}.${row.output}`}>
      <td className="mono" title={`${row.node}.${row.output}`}>
        {row.node}.{row.output}
      </td>
      <td>{row.tier ?? "—"}</td>
      <td className="mono">{row.triangles.toLocaleString("en-US")}</td>
      <td className="mono">{formatBytes(row.bytes)}</td>
      <td className="mono" title={`${row.solids} ${row.solids === 1 ? "solid" : "solids"} · ${row.cache_hits} from the cache · ${row.cache_misses} from the kernel`}>
        {row.solids === 0 ? "—" : `${row.cache_hits} / ${row.cache_misses}`}
      </td>
    </tr>
  );
}

/**
 * The caches section (the D1 contract's "profiler's caches section"): both
 * caches' counters and flags, and the budget control. Rendered before the
 * first profile too — the caches view arrives with the snapshot.
 */
function ProfileCaches({ sectionRef }: { sectionRef: RefObject<HTMLElement> }) {
  const caches = useCicada((s) => s.caches);
  const d = caches?.display;
  const flagged = d !== undefined && (d.over_budget || d.thrash);
  return (
    <section className="insp-section" id="profile-caches" ref={sectionRef} data-testid="profile-caches" data-warn={flagged}>
      <h3 className="insp-h">caches</h3>
      {caches === null ? (
        <div className="faint">no caches view yet</div>
      ) : (
        <>
          <div className="stat-grid">
            <span className="k">display cache</span>
            <span className="v" data-testid="profile-cache-held">
              {formatBytes(caches.display.bytes)} of {formatBytes(caches.display.budget)}
            </span>
            <span className="k">entries</span>
            <span className="v">
              {caches.display.entries.toLocaleString("en-US")}
              {caches.display.refusals > 0 ? ` (${caches.display.refusals} cached ${caches.display.refusals === 1 ? "refusal" : "refusals"})` : ""}
            </span>
            <span className="k" title="the display meshes every output on screen holds — what a redraw of everything needs">
              on screen
            </span>
            <span className="v">{formatBytes(caches.display.working_set)}</span>
            <span className="k">lookups</span>
            <span className="v">
              hits {caches.display.hits.toLocaleString("en-US")} · misses {caches.display.misses.toLocaleString("en-US")} · evictions{" "}
              {caches.display.evictions.toLocaleString("en-US")}
              {caches.display.oversized > 0 ? ` · ${caches.display.oversized} too large to keep` : ""}
            </span>
            <span className="k">memo store</span>
            <span className="v">
              {formatBytes(caches.memo.bytes)} in {caches.memo.entries.toLocaleString("en-US")} {caches.memo.entries === 1 ? "entry" : "entries"}
            </span>
          </div>
          {caches.display.over_budget && (
            <div className="prof-flag" data-testid="profile-flag-over-budget">
              over budget: the picture does not fit — every redraw re-tessellates part of it
            </div>
          )}
          {caches.display.thrash && (
            <div className="prof-flag" data-testid="profile-flag-thrash">
              thrashing: the last pass evicted meshes the previous generation displayed
            </div>
          )}
          {flagged && <div className="faint">raise the display cache below, or draw fewer solids</div>}
          <div className="prof-budget">
            <span className="k">budget</span>
            <DisplayCachePicker testId="profile-display-cache" />
          </div>
        </>
      )}
    </section>
  );
}
