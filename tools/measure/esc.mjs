#!/usr/bin/env node
/**
 * Esc always works (docs/15 measurement protocol, criterion 3): ×20, dirty
 * the cone of a heavy node with a structural `set_param`, wait until that
 * node is RUNNING (the cancel lands mid-carve, not mid-nothing), send the
 * `cancel` intent, and poll `/debug/state` → `solve.busy` at ≤ 5 ms until
 * idle. Reports p50/p95 time-to-idle as the client observed it AND the
 * server-side `cancel_to_idle_ms` the loop recorded on the cancelled
 * generation (cancel() call → loop idle — no poll granularity in it).
 *
 * Usage (Node ≥ 20, no dependencies):
 *   cicada serve <scratch-copy-of-project> --port 8493 --token t     # serve a SCRATCH copy — set_param WRITES the file
 *   node tools/measure/esc.mjs --url http://127.0.0.1:8493 --token t \
 *        --pipeline 03-voronoi.cic --param up --node carved [--port factor] \
 *        [--trials 20] [--min 1.0 --max 6.0] [--poll-ms 5] [--json out.json] [--no-restore]
 *   node tools/measure/esc.mjs --url … --pipeline wall.cic --param amps --node carved   # serving a scratch copy of examples/wall/
 *
 * `--param` is the binding whose literal gets flipped (a slider or a call
 * with a Number kwarg — `--port` names the kwarg, default = the widget's);
 * every trial uses a DISTINCT value so the heavy node is never a cache hit.
 * `--node` is the heavy node whose `running` status triggers the Esc. A
 * trial in which the node finished before the Esc could land is reported
 * as `missed` (not a failure of Esc — the solve was simply too fast) and
 * retried with the next value, up to 2× the trial count. The original
 * literal is restored at the end unless `--no-restore`. Prints a JSON
 * result, then one summary line; nonzero exit = could not measure.
 */
import { writeFileSync } from "node:fs";

import { Http, Session, die, findNode, numberLiteral, parseArgs, sleep, stats } from "./lib.mjs";

const args = parseArgs(process.argv.slice(2), { trials: "20", "poll-ms": "5", url: "http://127.0.0.1:8493" });
const { url, token, pipeline, param } = args;
const heavy = args.node;
if (!token || !pipeline || !param || !heavy) die("need --token, --pipeline, --param and --node (see the header)");
const trials = Number(args.trials);
const pollMs = Number(args["poll-ms"]);
if (!(trials > 0) || !(pollMs > 0) || pollMs > 5) die("--trials must be positive and --poll-ms in (0, 5]");

const http = new Http(url, token, pipeline);
const initial = await http.debugState({ wait: true });
const paramNode = findNode(initial, param);
findNode(initial, heavy);
let port = args.port ?? paramNode.param?.port ?? null;
let original = paramNode.param?.value;
let min = args.min !== undefined ? Number(args.min) : paramNode.param?.min;
let max = args.max !== undefined ? Number(args.max) : paramNode.param?.max;
if (port !== null && paramNode.param?.port !== port) {
  // A plain call kwarg (e.g. `up = unit_z(factor=3.0)`): take the literal from the view-model input.
  const input = paramNode.inputs.find((i) => i.name === port);
  if (input === undefined || typeof input.literal_value !== "number") {
    die(`\`${param}.${port}\` is not a numeric literal kwarg`);
  }
  original = input.literal_value;
}
if (typeof original !== "number") die(`\`${param}\` carries no numeric literal to flip (pass --port)`);
if (!Number.isFinite(min) || !Number.isFinite(max) || !(max > min)) {
  // No slider bounds: sweep ±50 % around the current value.
  min = original * 0.5;
  max = original * 1.5;
  if (!(max > min)) die(`cannot derive a value range around ${original}; pass --min/--max`);
}

const session = await new Session(url, token, pipeline).open();
const setParam = (value) =>
  session.send({ type: "set_param", payload: { node: param, port, value: numberLiteral(value) } });

/** Poll `/debug/state` (no wait) at `pollMs` until `predicate(state)`; returns `{state, at, polls}`. */
async function pollUntil(predicate, timeoutMs, what) {
  const deadline = performance.now() + timeoutMs;
  let polls = 0;
  for (;;) {
    const state = await http.debugState();
    polls += 1;
    const at = performance.now();
    if (predicate(state)) return { state, at, polls };
    if (at > deadline) throw new Error(`timed out after ${timeoutMs} ms waiting for ${what}`);
    await sleep(pollMs);
  }
}

const results = [];
let missed = 0;
let attempts = 0;
// Fast cones (a small demo carve) miss often — the "saw running" → "sent
// cancel" race is lost when the whole solve is a few ms. Budget generously;
// the honest outcome when we still cannot reach `trials` is reported, not
// silently truncated. `--max-attempts` overrides.
const maxAttempts = args["max-attempts"] !== undefined ? Number(args["max-attempts"]) : trials * 8;
let seenGenerations = new Set((initial.timings ?? []).map((t) => t.generation));
while (results.length < trials && attempts < maxAttempts) {
  attempts += 1;
  // A distinct value per attempt across the whole run, so the heavy node is
  // never a cache hit (which would finish instantly and never be catchable).
  const value = min + ((max - min) * (attempts + 0.5)) / (maxAttempts + 1);
  if (numberLiteral(value) === numberLiteral(original)) continue;
  const before = await http.debugState();
  const generationBefore = before.summary.generation;
  const edit = setParam(value);
  await session.waitFor(
    (m) => m.type === "delta" && m.payload.source?.intent_id === edit.id,
    30_000,
    `the delta acknowledging set_param #${attempts}`,
  );
  // Mid-carve: the heavy node is running — or a new generation already
  // came and went without us seeing it run (missed: too fast).
  let trigger;
  try {
    trigger = await pollUntil(
      (s) =>
        s.statuses[heavy]?.state === "running" ||
        (!s.solve.busy && s.summary.generation > generationBefore && !s.summary.running),
      120_000,
      `\`${heavy}\` to start running`,
    );
  } catch (error) {
    die(String(error));
  }
  if (trigger.state.statuses[heavy]?.state !== "running") {
    missed += 1;
    await http.debugState({ wait: true });
    continue;
  }
  const t0 = performance.now();
  session.send({ type: "cancel", payload: {} });
  const idle = await pollUntil((s) => !s.solve.busy, 120_000, "solve.busy to clear after cancel");
  const clientMs = idle.at - t0;
  // The server's own measure, on the generation the Esc ended. The loop
  // flips idle a hair BEFORE it writes cancel_to_idle_ms (the annotation is
  // recorded just after the idle flip), so poll briefly for it: if it never
  // appears within the window the generation finished on its own (a raced
  // miss), not a cancel.
  let fresh = [];
  let cancelled = [];
  const annotateDeadline = performance.now() + 200;
  for (;;) {
    const settled = await http.debugState();
    fresh = (settled.timings ?? []).filter((t) => !seenGenerations.has(t.generation));
    cancelled = fresh.filter((t) => t.cancel_to_idle_ms !== undefined);
    if (cancelled.length > 0 || performance.now() > annotateDeadline) {
      seenGenerations = new Set((settled.timings ?? []).map((t) => t.generation));
      break;
    }
    await sleep(pollMs);
  }
  if (cancelled.length === 0) {
    // The Esc arrived after the generation had already finished on its own
    // (the cone is fast enough that "saw running" → "sent cancel" lost the
    // race). Esc still worked — the loop is idle — but there is no cancel to
    // time. A raced miss, retried; it is not a failure of Esc.
    missed += 1;
    continue;
  }
  if (cancelled.length > 1) {
    die(
      `trial ${results.length + 1}: more than one Esc-annotated generation appeared at once ` +
        `(fresh timings: ${JSON.stringify(fresh)}) — the harness cannot attribute the cancel`,
    );
  }
  const timing = cancelled[0];
  results.push({
    trial: results.length + 1,
    value: numberLiteral(value),
    generation: timing.generation,
    heavy_state_at_cancel: trigger.state.statuses[heavy].state,
    client_time_to_idle_ms: Math.round(clientMs * 100) / 100,
    server_cancel_to_idle_ms: Math.round(timing.cancel_to_idle_ms * 100) / 100,
    generation_elapsed_ms: Math.round((timing.elapsed_ms ?? 0) * 100) / 100,
    polls_to_idle: idle.polls,
  });
}
if (results.length < trials) {
  die(`only ${results.length}/${trials} trials caught \`${heavy}\` running (${missed} missed) — the cone is too fast for this probe; pick a heavier node or a wider range`);
}

if (!args["no-restore"]) {
  const restore = setParam(original);
  await session.waitFor((m) => m.type === "delta" && m.payload.source?.intent_id === restore.id, 30_000, "the restore delta");
  await http.debugState({ wait: true });
}

const TARGET_P95_MS = 250;
const client = stats(results.map((r) => r.client_time_to_idle_ms));
const server = stats(results.map((r) => r.server_cancel_to_idle_ms));
const final = await http.debugState();
const result = {
  harness: "esc",
  engine: final.engine,
  threads: final.threads,
  pipeline,
  param,
  port,
  heavy_node: heavy,
  trials: results.length,
  missed,
  poll_ms: pollMs,
  client_time_to_idle_ms: client,
  server_cancel_to_idle_ms: server,
  per_trial: results,
  errors: session.errors,
  target: { p95_ms: TARGET_P95_MS },
  pass: {
    client_time_to_idle: client.count > 0 && client.p95 < TARGET_P95_MS,
    server_cancel_to_idle: server.count > 0 && server.p95 < TARGET_P95_MS,
  },
};
console.log(JSON.stringify(result, null, 2));
if (args.json) writeFileSync(args.json, JSON.stringify(result, null, 2));
console.log(
  `esc ${pipeline} ${param}→${heavy} ×${results.length} (${missed} missed): time-to-idle client p50 ${client.p50} ms p95 ${client.p95} ms max ${client.max} ms;` +
    ` server cancel→idle p50 ${server.p50} ms p95 ${server.p95} ms max ${server.max} ms; errors ${session.errors.length};` +
    ` target p95<${TARGET_P95_MS}: client ${result.pass.client_time_to_idle ? "PASS" : "FAIL"}, server ${result.pass.server_cancel_to_idle ? "PASS" : "FAIL"}`,
);
session.close();
process.exit(session.errors.length === 0 ? 0 : 1);
