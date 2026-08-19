/**
 * Playwright smoke (doc 15 stage 5 DoD; doc 14 e2e row): serve → place →
 * wire → drag → screenshot asserts geometry changed. Runs against a REAL
 * `cicada serve` over a scratch copy of `examples/` (the app writes files;
 * the repo's examples must stay clean) with a private cache dir.
 *
 * The binary: `CICADA_BIN`, else `<CARGO_TARGET_DIR|../target>/debug/cicada[.exe]`
 * (CI builds it with `--features embed`, so the SPA is inside; locally,
 * without the feature, `npm run build` first and the config passes
 * `--web-dir dist`). No SPA either way = the smoke fails loudly at `/`.
 */
import { defineConfig } from "@playwright/test";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = resolve(dirname(fileURLToPath(import.meta.url)));
const repo = resolve(web, "..");
const targetDir = process.env.CARGO_TARGET_DIR ?? join(repo, "target");
const exe = process.platform === "win32" ? "cicada.exe" : "cicada";
const bin = process.env.CICADA_BIN ?? join(targetDir, "debug", exe);
const port = Number(process.env.CICADA_E2E_PORT ?? 8471);
const token = "e2e";

// Scratch project: a fresh copy of examples/ per run, named by pid. Earlier
// runs leave theirs behind (the server still holds files when the process
// exits), so stale siblings older than an hour are swept here, best-effort —
// a sweep failure must never fail the suite.
const SCRATCH_PREFIX = "cicada-e2e-";
const STALE_MS = 60 * 60 * 1000;
function sweepStaleScratch(root: string): void {
  let entries: string[];
  try {
    entries = readdirSync(root);
  } catch {
    return;
  }
  const now = Date.now();
  for (const name of entries) {
    if (!name.startsWith(SCRATCH_PREFIX) || name === `${SCRATCH_PREFIX}${process.pid}`) continue;
    const dir = join(root, name);
    try {
      if (!statSync(dir).isDirectory() || now - statSync(dir).mtimeMs < STALE_MS) continue;
      rmSync(dir, { recursive: true, force: true });
    } catch {
      // in use by another run, or not ours to remove — leave it
    }
  }
}
sweepStaleScratch(tmpdir());
const scratch = join(tmpdir(), `${SCRATCH_PREFIX}${process.pid}`);
rmSync(scratch, { recursive: true, force: true });
mkdirSync(scratch, { recursive: true });
cpSync(join(repo, "examples"), join(scratch, "examples"), { recursive: true });

const webDir = process.env.CICADA_WEB_DIR ?? (existsSync(join(web, "dist")) ? join(web, "dist") : null);
const serveArgs = [
  "serve",
  join(scratch, "examples", "02-solids.cic"),
  "--port",
  String(port),
  "--token",
  token,
  "--cache-dir",
  join(scratch, "cache"),
  "--threads",
  "2",
  ...(webDir !== null && !process.env.CICADA_EMBEDDED ? ["--web-dir", webDir] : []),
];

export default defineConfig({
  testDir: "./e2e",
  timeout: 90_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  outputDir: "test-results",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
    viewport: { width: 1400, height: 900 },
  },
  metadata: { token, scratch, bin, serveArgs },
  webServer: {
    command: `"${bin}" ${serveArgs.map((a) => `"${a}"`).join(" ")}`,
    url: `http://127.0.0.1:${port}/health`,
    reuseExistingServer: false,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
