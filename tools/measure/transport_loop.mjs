#!/usr/bin/env node
/**
 * Transport playback loop (docs/13 §Latency targets: "warmed `cycle` loop
 * playback — 60 fps sustained"; docs/17 item 4 "done when": the second
 * pass of a loop is 100 % cached): play a SERVED pipeline's primary loop
 * for N passes and report, per pass, how many transport generations the
 * ticker produced, how many of them computed anything, the solve latency,
 * and the cadence — generations per second and the gaps between
 * consecutive generation starts, read from `/debug/state` timings
 * (kind = transport).
 *
 * Usage (Node ≥ 20, no dependencies):
 *   cicada serve <scratch-copy-of-project> --port 8493 --token t     # serve a SCRATCH copy — the app writes files
 *   node tools/measure/transport_loop.mjs --url http://127.0.0.1:8493 --token t \
 *        --pipeline 08-orbit.cic [--passes 2] [--speed 1] [--expect warm] [--json out.json]
 *
 * The pipeline's primary loop (`transport.frames` / `period_ms`) sets the
 * expected frame rate: a 240-frame / 4 s loop is a 60 fps loop, the
 * ticker's own rate, so every one of its frames must be visited on the
 * first pass for the second to be pure cache reads. Prints a JSON result,
 * then one summary line. Exit code 0 = measured; with `--expect warm` the
 * docs/17 property is ASSERTED: every generation of every pass after the
 * first computed nothing, and each of those passes visited at least 95 %
 * of the frames the ticker can show (min(frames, 60 × period)) — a
 * mismatch exits 1. Nonzero = could not measure (a refused intent counts).
 *
 * How passes are told apart: the play's own generation (the first
 * transport generation after `transport_play`) starts the clock; pass k
 * is the generations that started within [k, k + 1) × period_ms / speed
 * of it, on the server's own `started_ms`.
 */
import { writeFileSync } from "node:fs";

import { Http, Session, die, parseArgs, sleep, stats } from "./lib.mjs";

const args = parseArgs(process.argv.slice(2), { passes: "2", speed: "1", url: "http://127.0.0.1:8493" });
const { url, token, pipeline } = args;
if (!token || !pipeline) die("need --token and --pipeline (see the header)");
const passes = Number(args.passes);
const speed = Number(args.speed);
if (!Number.isInteger(passes) || passes < 1) die("--passes must be a positive integer");
if (!(speed > 0) || !Number.isFinite(speed)) die("--speed must be a positive finite number");
const expect = args.expect;
if (expect !== undefined && expect !== "warm") die(`--expect takes warm, not ${JSON.stringify(expect)}`);

const http = new Http(url, token, pipeline);
const initial = await http.debugState({ wait: true });
const loop = initial.transport;
if (!loop || !Array.isArray(loop.driven)) die("the server's /debug/state carries no transport — an engine before v0.1 item 4?");
if (loop.driven.length === 0) die(`${pipeline} has no time params (cycle / clock): playback moves nothing`);
const frames = loop.frames;
const periodMs = loop.period_ms;
const passMs = periodMs / speed;
// The frames the ticker can show per pass: the loop's, bounded by its own
// rate (TRANSPORT_TICK = 1/60 s in session.rs).
const TICKER_HZ = 60;
const visitable = Math.min(frames, Math.floor((TICKER_HZ * passMs) / 1000));

const session = await new Session(url, token, pipeline).open();
const control = async (type, payload = {}) => {
  const sent = session.send({ type, payload });
  // A control answers with the `transport` broadcast; a refusal with an
  // `error` carrying the intent id.
  const answer = await session.waitFor(
    (m) => m.type === "transport" || (m.type === "error" && m.payload.intent_id === sent.id),
    10_000,
    `${type}'s answer`,
  );
  if (answer.type === "error") die(`${type} refused: ${answer.payload.message}`);
  return answer.payload;
};

// Rewind to frame 0, paused, and set the speed — every pass then starts at
// the loop's first frame.
await control("transport_reset");
if (speed !== 1) await control("transport_speed", { factor: speed });
const atRest = await http.debugState({ wait: true });
const baselineGeneration = atRest.solve.last_complete_generation ?? 0;

const played = await control("transport_play");
if (!played.playing) die("transport_play did not start playback");
// Watch the playhead: done when it has covered `passes` loops, with a
// margin of one frame so the last frame's tick lands.
const framesMs = periodMs / frames;
const deadline = performance.now() + passes * passMs + 30_000;
for (;;) {
  const state = await http.debugState();
  if (state.transport.t_ms >= passes * periodMs + framesMs) break;
  if (performance.now() > deadline) die("the playhead never covered the requested passes — is playback advancing?");
  await sleep(50);
}
await control("transport_pause");
const finalState = await http.debugState({ wait: true });

// ---- the generations, split into passes on the server's clock.
const allTimings = finalState.timings ?? [];
const transport = allTimings.filter((t) => t.kind === "transport" && t.generation > baselineGeneration);
if (transport.length === 0) die("no transport generations were recorded — did playback run?");
const ringOverflow = allTimings.length > 0 && allTimings[0].generation > baselineGeneration;
if (ringOverflow) {
  console.error(
    `warning: the server's timing ring (${allTimings.length} entries) no longer holds the play generation — ` +
      "the earliest generations are missing from the statistics",
  );
}
const playStart = transport[0].started_ms;
const perPass = [];
for (let k = 0; k < passes; k += 1) {
  const from = playStart + k * passMs;
  const to = from + passMs;
  const gens = transport.filter((t) => t.started_ms >= from && t.started_ms < to);
  const starts = gens.map((t) => t.started_ms);
  const gaps = starts.slice(1).map((s, i) => s - starts[i]);
  perPass.push({
    pass: k + 1,
    generations: gens.length,
    visitable_frames: visitable,
    computed_generations: gens.filter((t) => (t.computed ?? 0) > 0).length,
    nodes_computed: gens.reduce((n, t) => n + (t.computed ?? 0), 0),
    nodes_cached: gens.reduce((n, t) => n + (t.cached ?? 0), 0),
    cancelled: gens.filter((t) => t.cancelled).length,
    generations_per_second: Math.round((gens.length / (passMs / 1000)) * 10) / 10,
    start_gap_ms: stats(gaps),
    elapsed_ms: stats(gens.map((t) => t.elapsed_ms ?? 0)),
    queued_ms: stats(gens.map((t) => t.queued_ms ?? 0)),
  });
}

const warmPasses = perPass.slice(1);
const warm =
  warmPasses.length > 0 &&
  warmPasses.every((p) => p.computed_generations === 0 && p.generations >= Math.ceil(visitable * 0.95));
const result = {
  harness: "transport_loop",
  engine: finalState.engine,
  threads: finalState.threads,
  pipeline,
  loop: { frames, period_ms: periodMs, fps: Math.round((frames / periodMs) * 1000 * 100) / 100 },
  speed,
  passes,
  driven: loop.driven,
  ticker_hz: TICKER_HZ,
  transport_generations: transport.length,
  timings_ring_overflow: ringOverflow,
  frames_received: session.frames.length,
  per_pass: perPass,
  warm_after_first_pass: warm,
  errors: session.errors,
};
if (args.json) writeFileSync(args.json, `${JSON.stringify(result, null, 2)}\n`);
console.log(JSON.stringify(result, null, 2));
const last = perPass[perPass.length - 1];
console.log(
  `transport_loop: ${frames} frames / ${periodMs} ms (${result.loop.fps} fps) × ${passes} passes at ${speed}× — ` +
    `pass ${last.pass}: ${last.generations} generations (${last.generations_per_second}/s, gap p50 ${last.start_gap_ms.p50} ms, max ${last.start_gap_ms.max} ms), ` +
    `${last.computed_generations} computed, solve p50 ${last.elapsed_ms.p50} ms — warm after the first pass: ${warm ? "YES" : "NO"}`,
);
session.close();
if (expect === "warm" && !warm) {
  console.error("error: --expect warm: a pass after the first computed something or skipped frames (see per_pass)");
  process.exit(1);
}
process.exit(session.errors.length === 0 ? 0 : 1);
