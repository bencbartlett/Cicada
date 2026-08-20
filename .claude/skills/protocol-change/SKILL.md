---
name: protocol-change
description: Change the app protocol between cicada-server and the web client — JSON control-plane messages, the graph view-model, binary frame layout, HTTP/debug routes — updating server, client mirror, and tests together. Use for ANY change to crates/cicada-server/src/{protocol,viewmodel,frames,http}.rs or web/src/protocol/*.
---

# Change the protocol

The engine server owns all authoritative state; the browser sends intents
and renders deltas (DECISIONS.md app-protocol row, docs/13). Both sides
mirror one set of shapes — a change that lands on one side only is a bug
by definition. Doc 13 is the spec; `crates/cicada-server/src/frames.rs`
IS the byte-exact frame spec.

## Where things live

| Concern | Server | Client mirror |
|---|---|---|
| Control-plane messages, envelope, versions | `crates/cicada-server/src/protocol.rs` (`PROTOCOL_VERSION`) | `web/src/protocol/messages.ts`, `version.ts` |
| Graph view-model (nodes/ports/wires/params) | `crates/cicada-server/src/viewmodel.rs` | `messages.ts` (`GraphView` & co.) |
| Binary frames | `crates/cicada-server/src/frames.rs` (encoder + decoder + tests) | `web/src/protocol/frames.ts` (+ vitest) |
| Frame construction from values, pick ids, summaries | `crates/cicada-server/src/display.rs` | `web/src/viewport/*` (consumer) |
| Intents → text edits, statuses, display set | `crates/cicada-server/src/session.rs` | `web/src/state/store.ts` (`applyServerMessage`) |
| Undo/redo op log (`undo`, `redo`, `batch`, `apply_text`; `history` on delta/snapshot; `error` details `current_text_hash` / `diagnostics` / `index`) | `session.rs` (`OpLog`, `apply_gesture` → `commit`, `apply_text`), `protocol.rs` (`Actor`, `HistoryView`, `ApplyTextRequest`) | `store.ts` (history state), the toolbar/keymap consumers |
| Routes, auth, WS loop, debug endpoints (`/api/edit/text`, `/api/edit/apply_text` included) | `crates/cicada-server/src/http.rs` | `web/src/state/connection.ts` |

## Checklist

1. **Decide the version impact.** Additive fields (new optional payload
   fields, new message types the client can ignore) keep `PROTOCOL_VERSION`;
   any change that makes an old client misread a message bumps it on BOTH
   sides (`protocol.rs` + `version.ts`) in the same commit. Frame layout
   changes bump `frames::VERSION` + `FRAME_VERSION`.
2. **Server first**: change the Rust type; serde attributes keep JSON
   stable (`snake_case`, `skip_serializing_if` for optionals). Add/extend a
   unit test in the touched module (`protocol.rs` round-trips intents;
   `frames.rs` round-trips bytes; `viewmodel.rs` builds views from a
   source string; `session.rs` drives intents end to end).
3. **Client mirror second**: mirror the shape in `messages.ts` /
   `frames.ts`; extend `frames.test.ts` for byte layout changes; update
   `applyServerMessage` in `store.ts` when a new message carries state.
4. **Integration**: `crates/cicada-server/tests/http_e2e.rs` starts a real
   server and drives HTTP + WS — extend it when routes or the handshake
   change. Playwright smoke (`web/e2e/smoke.spec.ts`) covers the browser
   end to end (`npm run e2e`).
5. **Docs**: docs/13 tables (messages, frames, HTTP surface) — the frame
   section points at `frames.rs`; keep both truthful. A change that
   contradicts a ledger row revises the row in the same commit.
6. Run the `verify-change` loop: `cargo test -p cicada-server`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cd web && npm run
   check && npm run lint && npm test`, and — for anything the browser
   renders — the Playwright smoke or a screenshot from `/debug/screenshot`.

## Rules worth repeating

- Never let the client compute semantics the server owns (wire
  compatibility, layout defaults, param bounds): add a read intent
  (`probe_wire`-style) instead of duplicating rules in TypeScript.
- Frames stay zero-parse: every array 4-byte aligned, counts in the
  header/body, no variable-length text.
- Every refusal is a typed `error` message with `intent_id` echoed —
  never a silent drop.
