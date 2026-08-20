---
name: verify-change
description: Verify a Cicada change with the headless-first evidence loop — fmt, clippy, tests, catalog freshness, web checks, then browser evidence for UI-facing work. Use before declaring ANY change done or committing.
---

# Verify a change

The human reviews evidence; the human is never the feedback loop (doc 14).
Run everything headless first; touch the browser only for UI-facing changes.

## The loop

1. **Format + lint** (never leave these red):

   ```
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   ```

2. **Tests** — the touched crate's first for fast iteration, then the
   workspace gate:

   ```
   cargo test -p <touched-crate>
   cargo test --workspace
   ```

3. **Catalog freshness** — any change that touches node specs, doc
   comments, or the renderer:

   ```
   cargo run -p cicada-cli -- catalog --check
   ```

   If stale: regenerate (drop `--check`), review the diff, commit it with
   the change.

4. **Web changes** (`web/` touched) — PowerShell form (bash: `&&` chain):

   ```powershell
   cd web; npm run check; if ($?) { npm run lint }; if ($?) { npm test }
   ```

5. **Pipeline-facing changes** *(stage 3+, once `cicada run` exists)*:
   run a pipeline headlessly and compare output hashes:

   ```
   cicada run <pipeline.cic> --node <sink> --hashes
   ```

   The wall (`examples/wall/wall.cic`, since stage 6) is the full-size
   pipeline; otherwise use whatever fixture pipeline the work under test provides.

6. **UI-facing changes**: drive the running app yourself — never hand the
   human a "please click around and check" step. The recipe:

   ```bash
   # serve a SCRATCH copy (the app writes the served files) with a fixed token
   SCRATCH="$LOCALAPPDATA/Temp/cicada-probe"; mkdir -p "$SCRATCH" && cp -r examples "$SCRATCH/"
   cd "$SCRATCH"
   "$CARGO_TARGET_DIR/debug/cicada" serve examples/02-solids.cic --port 8450 --token t \
       --cache-dir "$SCRATCH/cache" --web-dir <repo>/web/dist &   # after `npm run build`
   curl "http://127.0.0.1:8450/debug/state?token=t&wait=true"     # the authoritative oracle
   curl -o shot.png "http://127.0.0.1:8450/debug/screenshot?token=t"   # needs a connected page
   ```

   Open `http://127.0.0.1:8450/?token=t&pipeline=02-solids.cic` with
   Playwright (`import { chromium } from "@playwright/test"` in a .mjs under
   `web/`), drive gestures through the DOM, read `window.__cicada.state()`
   / `.scene()` / `.frames()`, compare `/debug/state` before and after, and
   LOOK at the screenshots. The durable form is `web/e2e/smoke.spec.ts`
   (`cd web && npm run build && npm run e2e`); extend it when a contract
   changes. Dev alternative: `npm run dev` (Vite proxy) instead of
   `--web-dir`.

7. **Attach evidence** to the report/commit: the failing-then-passing test
   for bug fixes, hash diffs with an explanation for determinism changes,
   screenshots for UI changes. Report outcomes faithfully — if something is
   red, say so with the output.

## Golden hashes

Determinism goldens update ONLY through the blessed path, and the commit
body explains why the hash legitimately changed. The blessed paths:
blake3 hash constants (core/stdlib determinism tests) are blessed by
running the test once and copying the actual from the failure message —
declared in the commit (see `add-stdlib-node`); insta snapshots (checker
diagnostics) via `cargo insta review` or an `INSTA_UPDATE=always` run,
reviewing the .snap diff. What is never blessed: silently editing an
expected value to make a red test green without explaining the change.
