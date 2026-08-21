#!/usr/bin/env node
/**
 * Control-plane priority over the display restream, on the wire (docs/13
 * §Two lanes, one socket; docs/17 §Follow-ups). No browser: a writer opens
 * the SERVED pipeline and waits for it to solve; an observer then joins
 * and records, per message, when it arrived and what it was; the writer
 * sends one `param_preview` tick at a chosen moment of the observer's join
 * and the report says where the observer's `preview_policy` (or `status`)
 * landed among its restream frames — and how long the joiner waited for
 * `hello`, `snapshot`, `display_reset`, its first and its last frame.
 *
 * Usage (Node ≥ 20, no dependencies):
 *   cicada serve <scratch-copy-of-examples> --port 8493 --token t --cache-dir <dir>   # a SCRATCH copy — the app writes files
 *   node tools/measure/lanes.mjs --url http://127.0.0.1:8493 --token t \
 *        --pipeline wall/wall.cic --param deboss --value 1.1 \
 *        [--tick at-snapshot|<ms-after-open>] [--busy-ms-per-mb 0] [--json out.json]
 *
 * `--param`/`--value`: the slider the writer ticks (a cold value for the
 * wall's `deboss` answers with `preview_policy`; a cheap slider answers
 * with `status` + a frame). `--tick at-snapshot` (default) sends the tick
 * the moment the observer has its snapshot; a number sends it that many ms
 * after the observer's socket opened — while the server is still building
 * the restream (the wall: ~3 s on a debug engine). `--busy-ms-per-mb`
 * makes the observer's handler burn that long per MB of each frame — a
 * page's decode + render, crudely; note a busy handler blocks THIS Node
 * process's writer socket too, so read the writer's numbers only when it
 * is 0. Prints a JSON result, then one summary line; nonzero exit = could
 * not measure.
 *
 * What to expect (measured 2026-08-20, debug engine, the wall cached,
 * loopback): one queue per client (24d558b) — observer open → hello
 * ~3.0 s (the restream was built under the session lock before the
 * joiner's texts went out), the tick's text after ALL 26 frames / 368 MB;
 * the lanes — open → hello ~10 ms, the text behind the one frame in
 * flight. A text sent after the server has WRITTEN the restream lands
 * last on the wire whatever the engine: the lanes reorder what is still
 * queued at the server, never what the client already holds.
 */
import { writeFileSync } from "node:fs";

import { Http, Session, die, findNode, numberLiteral, parseArgs, sleep } from "./lib.mjs";

const args = parseArgs(process.argv.slice(2), {
  url: "http://127.0.0.1:8493",
  tick: "at-snapshot",
  "busy-ms-per-mb": "0",
});
const { url, token, pipeline, param } = args;
if (!token || !pipeline || !param || args.value === undefined) {
  die("need --token, --pipeline, --param and --value (see the header)");
}
const tickAtSnapshot = args.tick === "at-snapshot";
const tickDelayMs = tickAtSnapshot ? null : Number(args.tick);
if (!tickAtSnapshot && !(tickDelayMs >= 0)) die("--tick must be `at-snapshot` or a number of ms");
const busyMsPerMb = Number(args["busy-ms-per-mb"]);
if (!(busyMsPerMb >= 0)) die("--busy-ms-per-mb must be ≥ 0");

const http = new Http(url, token, pipeline);

// ---- the pipeline solves first (`/debug/state?wait=true` opens the
// session and blocks until its initial generation is done — the wall's
// cold open is minutes on a debug engine), then the writer joins and
// drains its own restream.
const solved = await http.debugState({ wait: true });
const writer = new Session(url, token, pipeline);
await writer.open();
const node = findNode(solved, param);
if (!node.param) die(`\`${param}\` has no parameter widget (it is a ${node.kind}); name a slider`);
const port = node.param.port ?? null;
const red = Object.entries(solved.statuses).filter(([, s]) => s.state === "red" || s.state === "blocked");
if (red.length > 0) die(`the pipeline is not green: ${red.map(([n, s]) => `${n}: ${s.state}`).join(", ")}`);
const restreamBytes = Object.values(solved.display).reduce((sum, d) => sum + d.stats.bytes, 0);
let last = -1;
while (writer.frames.length !== last) {
  last = writer.frames.length;
  await sleep(1000);
}

// ---- the observer: a raw socket (it must stay an observer), timestamped.
const events = [];
let tOpen = null;
let tTick = null;
const busyWait = (ms) => {
  const end = performance.now() + ms;
  while (performance.now() < end) {
    /* a page busy with a frame */
  }
};
const tick = () => {
  tTick = performance.now();
  writer.send({ type: "param_preview", payload: { node: param, port, value: numberLiteral(Number(args.value)) } });
};
const observerUrl = new URL(url);
observerUrl.protocol = observerUrl.protocol === "https:" ? "wss:" : "ws:";
observerUrl.pathname = "/ws";
observerUrl.search = new URLSearchParams({ token, pipeline }).toString();
await new Promise((resolve, reject) => {
  const socket = new WebSocket(observerUrl);
  socket.binaryType = "arraybuffer";
  tOpen = performance.now();
  socket.onopen = () => {
    socket.send(JSON.stringify({ v: 1, type: "hello", payload: { v: 1 } }));
    if (!tickAtSnapshot) setTimeout(tick, tickDelayMs);
    resolve(socket);
  };
  socket.onerror = () => reject(new Error(`cannot connect to ${observerUrl}`));
  socket.onmessage = (event) => {
    const at = performance.now();
    if (typeof event.data === "string") {
      const type = JSON.parse(event.data).type;
      events.push({ at, kind: "text", type });
      if (type === "snapshot" && tickAtSnapshot && tTick === null) tick();
    } else {
      const bytes = event.data.byteLength;
      events.push({ at, kind: "frame", bytes });
      if (busyMsPerMb > 0) busyWait((busyMsPerMb * bytes) / 1e6);
    }
  };
});

// Until the observer has its whole display set and a text has answered
// the tick (or 240 s).
const started = performance.now();
for (;;) {
  await sleep(250);
  const received = events.filter((e) => e.kind === "frame").reduce((sum, e) => sum + e.bytes, 0);
  const answered = tTick !== null && events.some((e) => e.kind === "text" && (e.type === "preview_policy" || e.type === "status") && e.at > tTick);
  if (received >= restreamBytes && answered) break;
  if (performance.now() - started > 240_000) break;
}

const frames = events.filter((e) => e.kind === "frame");
const texts = events.filter((e) => e.kind === "text");
const first = (type) => events.find((e) => e.kind === "text" && e.type === type) ?? null;
const answer = events.find((e) => e.kind === "text" && (e.type === "preview_policy" || e.type === "status") && tTick !== null && e.at > tTick) ?? null;
const framesBeforeAnswer = answer === null ? null : frames.filter((f) => f.at < answer.at);
const ms = (t) => (t === null ? null : +(t).toFixed(1));
const result = {
  pipeline,
  param,
  tick: tickAtSnapshot ? "at-snapshot" : `${tickDelayMs} ms after open`,
  busy_ms_per_mb: busyMsPerMb,
  restream_bytes_server: restreamBytes,
  observer_frames: frames.length,
  observer_bytes: frames.reduce((sum, f) => sum + f.bytes, 0),
  largest_frame_bytes: frames.length > 0 ? Math.max(...frames.map((f) => f.bytes)) : 0,
  texts_in_order: texts.map((t) => t.type),
  display_reset_before_first_frame:
    first("display_reset") !== null && frames.length > 0 ? first("display_reset").at < frames[0].at : null,
  open_to_hello_ms: ms(first("hello") === null ? null : first("hello").at - tOpen),
  open_to_snapshot_ms: ms(first("snapshot") === null ? null : first("snapshot").at - tOpen),
  open_to_display_reset_ms: ms(first("display_reset") === null ? null : first("display_reset").at - tOpen),
  open_to_first_frame_ms: ms(frames.length > 0 ? frames[0].at - tOpen : null),
  open_to_last_frame_ms: ms(frames.length > 0 ? frames.at(-1).at - tOpen : null),
  answer:
    answer === null
      ? null
      : {
          type: answer.type,
          after_tick_ms: ms(answer.at - tTick),
          frames_before: framesBeforeAnswer.length,
          bytes_before: framesBeforeAnswer.reduce((sum, f) => sum + f.bytes, 0),
          landed_mid_restream: framesBeforeAnswer.length < frames.length,
        },
  writer_errors: writer.errors,
};
if (args.json) writeFileSync(args.json, `${JSON.stringify(result, null, 2)}\n`);
console.log(JSON.stringify(result, null, 2));
if (result.answer === null) {
  console.log(`lanes: no text answered the tick within the wait — could not measure`);
  writer.close();
  process.exit(2);
}
console.log(
  `lanes: open→hello ${result.open_to_hello_ms} ms · ${result.observer_frames} frames / ${(result.observer_bytes / 1e6).toFixed(0)} MB · ` +
    `${result.answer.type} ${result.answer.after_tick_ms} ms after the tick, behind ${result.answer.frames_before} frames / ` +
    `${(result.answer.bytes_before / 1e6).toFixed(1)} MB (${result.answer.landed_mid_restream ? "mid-restream" : "after the whole restream"})`,
);
writer.close();
process.exit(0);
