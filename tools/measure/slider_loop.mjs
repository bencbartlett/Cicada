#!/usr/bin/env node
/**
 * Live slider loop (docs/15 measurement protocol, criterion 2): stream
 * `param_preview` at ~60 Hz for 5 s on one slider of a SERVED pipeline and
 * report preview-generation latency — server-side from `/debug/state`
 * timings (kind = preview: `queued_ms` + `elapsed_ms`) and the client
 * round-trip from each send to the first frame of the generation it
 * produced. Also reports progress/no-freeze evidence for heavy pipelines.
 *
 * Usage (Node ≥ 20, no dependencies):
 *   cicada serve <scratch-copy-of-project> --port 8493 --token t     # serve a SCRATCH copy — the app writes files
 *   node tools/measure/slider_loop.mjs --url http://127.0.0.1:8493 --token t \
 *        --pipeline 02-solids.cic --param size [--seconds 5] [--hz 60] \
 *        [--min 0.5 --max 5.0] [--json out.json]
 *   node tools/measure/slider_loop.mjs --url … --pipeline wall.cic --param amps   # serving a scratch copy of examples/wall/
 *
 * `--param` is the slider's binding name; the kwarg, min and max come from
 * the view-model (`--min/--max` override). Prints a JSON result, then one
 * summary line. Exit code 0 = measured (pass/fail against the doc-15
 * targets is IN the JSON, not the exit code); nonzero = could not measure
 * (a refused intent counts as "could not measure").
 *
 * How the client round-trip is paired with a generation: the first preview
 * (sent while the loop is idle) calibrates the session clock — its timing's
 * `started_ms − queued_ms` is the server's acceptance time of that very
 * message — then every later preview generation's acceptance time maps to
 * the last send before it (the loop is latest-wins: the job it ran IS the
 * newest accepted message). Round-trip = first frame of that generation −
 * that send. Server-side latency needs no alignment at all.
 */
import { writeFileSync } from "node:fs";

import { Http, Session, die, findNode, numberLiteral, parseArgs, sleep, stats } from "./lib.mjs";

const args = parseArgs(process.argv.slice(2), { seconds: "5", hz: "60", url: "http://127.0.0.1:8493" });
const { url, token, pipeline, param } = args;
if (!token || !pipeline || !param) die("need --token, --pipeline and --param (see the header)");
const seconds = Number(args.seconds);
const hz = Number(args.hz);
if (!(seconds > 0) || !(hz > 0)) die("--seconds and --hz must be positive numbers");

const http = new Http(url, token, pipeline);
const initial = await http.debugState({ wait: true });
const node = findNode(initial, param);
// Stream on a slider/param widget, OR on a call's numeric-literal kwarg
// named by `--port` (e.g. `up = unit_z(factor=3.0)` in a pipeline with no
// slider — a heavier fan-out cone still gets exercised).
let port;
let center;
let min = args.min !== undefined ? Number(args.min) : undefined;
let max = args.max !== undefined ? Number(args.max) : undefined;
if (node.param && (args.port === undefined || node.param.port === args.port)) {
  port = node.param.port ?? null;
  center = Number(node.param.value);
  if (min === undefined) min = node.param.min;
  if (max === undefined) max = node.param.max;
} else if (args.port !== undefined) {
  port = args.port;
  const input = node.inputs.find((i) => i.name === port);
  if (input === undefined || typeof input.literal_value !== "number") {
    die(`\`${param}.${port}\` is not a numeric literal kwarg (pass a slider param, or a call's Number kwarg via --port)`);
  }
  center = input.literal_value;
} else {
  die(`\`${param}\` has no parameter widget (it is a ${node.kind}); name a call's numeric kwarg with --port`);
}
if (!Number.isFinite(min) || !Number.isFinite(max) || !(max > min)) {
  // No slider bounds (a plain kwarg): sweep ±25 % around the current value.
  min = center * 0.75;
  max = center * 1.25;
  if (!(max > min)) die(`\`${param}\` needs a numeric range: min=${min} max=${max} (pass --min/--max)`);
}

const session = await new Session(url, token, pipeline).open();
const baselineGeneration = initial.solve.last_complete_generation ?? 0;
const preview = (value) =>
  session.send({ type: "param_preview", payload: { node: param, port, value: numberLiteral(value) } });

// ---- calibration: one preview from idle → the session epoch in client time.
const calibration = preview(min + (max - min) * 0.25);
const calibrationDeadline = performance.now() + 60_000;
let calibrationTiming = null;
while (calibrationTiming === null) {
  const state = await http.debugState({ wait: true });
  calibrationTiming = (state.timings ?? []).find(
    (t) => t.kind === "preview" && t.generation > baselineGeneration,
  ) ?? null;
  if (calibrationTiming === null && performance.now() > calibrationDeadline) {
    die("the calibration preview never produced a generation — is the slider wired to anything?");
  }
}
// accept_ms (ms since the session epoch) = started − queued; it happened
// right after the send, so epoch ≈ send − accept.
const epochClient = calibration.at - (calibrationTiming.started_ms - calibrationTiming.queued_ms);
const afterCalibration = calibrationTiming.generation;

// ---- the stream: a linear ramp min → max at `hz` for `seconds` — every
// value distinct, so no generation is a memo hit pretending to be a solve.
const sends = [];
const intervalMs = 1000 / hz;
const total = Math.round(seconds * hz);
const polls = [];
const streamStart = performance.now();
let pollAt = streamStart + 500;
for (let i = 0; i < total; i += 1) {
  const due = streamStart + i * intervalMs;
  const now = performance.now();
  if (due > now) await sleep(due - now);
  const value = min + ((max - min) * (i + 0.5)) / total;
  sends.push({ ...preview(value), value, i });
  if (performance.now() >= pollAt) {
    // Responsiveness while streaming: the oracle must keep answering.
    const t0 = performance.now();
    await http.debugState();
    polls.push(performance.now() - t0);
    pollAt += 500;
  }
}
const streamEnd = performance.now();
const finalState = await http.debugState({ wait: true });
const settledAt = performance.now();

// ---- server-side: every preview generation after the calibration. The
// server keeps a bounded ring of timings; a full ring whose oldest entry is
// newer than the calibration means the stream outran it — reported, never
// silently truncated statistics.
const allTimings = finalState.timings ?? [];
const timings = allTimings.filter((t) => t.kind === "preview" && t.generation > afterCalibration);
const ringOverflow = allTimings.length > 0 && allTimings[0].generation > afterCalibration;
if (ringOverflow) {
  console.error(
    `warning: the server's timing ring (${allTimings.length} entries) no longer holds the calibration generation — ` +
      "the earliest generations of this stream are missing from the statistics",
  );
}
const elapsed = timings.map((t) => t.elapsed_ms ?? 0);
const queued = timings.map((t) => t.queued_ms ?? 0);
const serverTotal = timings.map((t) => (t.queued_ms ?? 0) + (t.elapsed_ms ?? 0));

// ---- client-side: send → first frame of the generation it produced.
const firstFrameAt = new Map();
for (const frame of session.frames) {
  const seen = firstFrameAt.get(frame.generation);
  if (seen === undefined || frame.at < seen) firstFrameAt.set(frame.generation, frame.at);
}
const roundTrips = [];
let unmatched = 0;
let frameless = 0;
for (const t of timings) {
  const acceptClient = epochClient + (t.started_ms - t.queued_ms);
  let send = null;
  for (let i = sends.length - 1; i >= 0; i -= 1) {
    if (sends[i].at <= acceptClient + 2) {
      send = sends[i];
      break;
    }
  }
  if (send === null) {
    unmatched += 1;
    continue;
  }
  const frameAt = firstFrameAt.get(t.generation);
  if (frameAt === undefined) {
    frameless += 1;
    continue;
  }
  roundTrips.push(frameAt - send.at);
}

// ---- no-freeze evidence: the longest silence from the server while streaming.
const arrivals = [...session.messages.map((m) => m.at), ...session.frames.map((f) => f.at)]
  .filter((at) => at >= streamStart && at <= settledAt)
  .sort((a, b) => a - b);
let longestSilence = arrivals.length === 0 ? settledAt - streamStart : arrivals[0] - streamStart;
for (let i = 1; i < arrivals.length; i += 1) longestSilence = Math.max(longestSilence, arrivals[i] - arrivals[i - 1]);
longestSilence = Math.max(longestSilence, settledAt - (arrivals[arrivals.length - 1] ?? streamStart));
const statusMessages = session.messages.filter((m) => m.envelope.type === "status" && m.at >= streamStart).length;
const running = session.messages.filter(
  (m) => m.envelope.type === "status" && m.at >= streamStart && m.envelope.payload.summary?.running,
).length;

const TARGET = { p50_ms: 16, p95_ms: 33 };
const server = stats(serverTotal);
const client = stats(roundTrips);
const passes = (s) => s.count > 0 && s.p50 <= TARGET.p50_ms && s.p95 <= TARGET.p95_ms;
const result = {
  harness: "slider_loop",
  engine: finalState.engine,
  threads: finalState.threads,
  pipeline,
  param,
  port,
  range: [min, max],
  seconds,
  hz,
  sends: sends.length,
  stream_ms: Math.round((streamEnd - streamStart) * 10) / 10,
  settle_after_stream_ms: Math.round((settledAt - streamEnd) * 10) / 10,
  preview_generations: timings.length,
  superseded: sends.length - timings.length,
  generations_per_second: Math.round((timings.length / ((settledAt - streamStart) / 1000)) * 10) / 10,
  cancelled_generations: timings.filter((t) => t.cancelled).length,
  nodes_computed: timings.reduce((n, t) => n + (t.computed ?? 0), 0),
  nodes_cached: timings.reduce((n, t) => n + (t.cached ?? 0), 0),
  timings_ring_overflow: ringOverflow,
  frames: session.frames.filter((f) => f.at >= streamStart).length,
  frame_bytes: timings.reduce((n, t) => n + (t.frame_bytes ?? 0), 0),
  server: {
    elapsed_ms: stats(elapsed),
    queued_ms: stats(queued),
    queued_plus_elapsed_ms: server,
  },
  client_round_trip_ms: client,
  pairing: { matched: roundTrips.length, unmatched_sends: unmatched, frameless_generations: frameless },
  no_freeze: {
    status_messages: statusMessages,
    running_statuses: running,
    longest_server_silence_ms: Math.round(longestSilence * 10) / 10,
    debug_state_polls_while_streaming: stats(polls),
  },
  errors: session.errors,
  target: TARGET,
  pass: { server_queued_plus_elapsed: passes(server), client_round_trip: passes(client) },
};
console.log(JSON.stringify(result, null, 2));
if (args.json) writeFileSync(args.json, JSON.stringify(result, null, 2));
console.log(
  `slider_loop ${pipeline} ${param}: ${sends.length} sends/${seconds}s → ${timings.length} preview generations` +
    ` (${result.generations_per_second}/s); server queued+elapsed p50 ${server.p50} ms p95 ${server.p95} ms` +
    ` (elapsed p50 ${result.server.elapsed_ms.p50}, queued p50 ${result.server.queued_ms.p50});` +
    ` client round-trip p50 ${client.p50} ms p95 ${client.p95} ms (${roundTrips.length} paired);` +
    ` longest silence ${result.no_freeze.longest_server_silence_ms} ms; errors ${session.errors.length};` +
    ` target p50≤${TARGET.p50_ms}/p95≤${TARGET.p95_ms}: server ${result.pass.server_queued_plus_elapsed ? "PASS" : "FAIL"}, client ${result.pass.client_round_trip ? "PASS" : "FAIL"}`,
);
session.close();
process.exit(session.errors.length === 0 ? 0 : 1);
