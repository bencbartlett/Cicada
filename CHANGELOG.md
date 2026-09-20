# Changelog

Notable changes to Cicada, newest first. A version is the workspace version
in `Cargo.toml`; the `v<version>` tag on the commit that sets it is the
release: `.github/workflows/release.yml` builds the Windows and macOS bundles
and the Linux binary on that tag and publishes them with this file's section
for the version as the release body (`python tools/changelog.py section
<version>`; `python tools/changelog.py check` holds the first section to
`Cargo.toml`), marked a pre-release while the version carries a pre-release
suffix. Every binary also stamps its commit and build date — `cicada
--version`, `GET /api/version`, the app's About dialog. `0.1.0` proper
closes when the v0.1 plan's items do (docs/17).

## 0.1.0-alpha.1 — 2026-09-19

The first tagged pre-release: the vertical-slice spike (docs/15, gate
passed) plus v0.1's items 0–5 and waves 4 and 5 (docs/17). Pre-release
software: the `.cic` dialect, the node catalog, the protocol and the file
formats still change without notice, there is no installer and no code
signing, and Python 3 must be on the machine (the engine's script host).

### The engine

- **B-rep by default.** `box` / `sphere` / `cylinder` / `cone` / `extrude`
  / `extrude_to_point` / `loft` / `revolve` / `sweep` / `pipe`, the three
  booleans, `volume` / `bounding_box` / `deconstruct_solid` / `section`,
  `tessellate` and STEP in/out are OCCT-backed (7.8.1, prebuilt and pinned
  by `tools/fetch_occt.py`); a `Solid` is always one body and its bytes are
  canonical, so it caches and hashes like every other value. The mesh tier
  lives on as `mesh_box` / `mesh_sphere` / `mesh_extrude` / `mesh_loft`
  (Manifold booleans, prebuilt by `tools/fetch_manifold.py`).
- **The scheduler.** Content-addressed memo on disk in the user cache dir,
  cancellation at every safe point (Esc), latest-wins previews, cost
  sampling and an ETA; compute-on-release for cones over 1 s; scrub caching
  for sliders with a bounded position count (`slider(…, scrub=True)`, up
  to 32 positions, a buffer bar on the slider); the time transport —
  `cycle` / `clock` params driven by a server-side playhead with a play
  bar, `Space`, a scrubber and speed.
- **Undo/redo** as a snapshot op log (`Ctrl+Z` / `Ctrl+Y`), the atomic
  `batch` and whole-file `apply_text` paths for multi-node gestures and
  agents, `#off` as the native disable, `Del` (never Backspace) deletes.
- **Git panel, slice 1.** Status chip, per-node change markers computed
  from `git diff`, commit from the app (`Ctrl+S`), revert to HEAD.
- **The catalog.** One node per file with a self-documenting format the
  conformance test enforces (`gh = "…"` names the Grasshopper equivalent,
  `# Panics` is the red contract, `# Examples` solve in CI), sub-groups as
  a required `#[node(sub = …)]` (catalog format 3), the docs/08 S+1 rows
  through C2b — lists, maths, sequences, Point · Vector · Plane, the
  Transform rows over `Affine`, the `choice` dropdown param — and `cicada
  mcp`, the read tools for agents over the Model Context Protocol.
- **The display edge, bounded and visible** (wave 5 D1 / P1): a triangle
  budget of 1,000,000 per output per generation (over it, the preview
  tier, said so), a 1 GiB solid display cache resizable at run time
  (`--solid-cache-mib`, the settings menu) with a notice and a top-bar
  indicator when a generation cannot fit or thrashes it, display passes
  that announce themselves and are latest-wins like the solve, and the
  profiler (the `profile` read, an inspector tab: phases, every node's
  cost, what the pass drew, the caches).
- **Headless.** `cicada run` (values, `--hashes`, `--time`), `cicada
  catalog`, `cicada mcp`; the wall corpus (`examples/wall/`) reproduces
  its production 3MF/DXF nightly.

### The app

- **`cicada app`** — the server plus a Chromium-based app window (Edge or
  Chrome), or the default browser; `cicada serve` over a ROOT directory
  (no path = the home directory) with File → Open / Recent / Close and a
  landing picker; a pop-out viewport that joins as an observer; the
  double-click launchers `tools/launch/Cicada.cmd` / `Cicada.command`
  that build when stale, and `tools/launch/bundle.py` — the redistributable
  folder with the kernel's run-time libraries beside the binary, checked
  and smoked.
- **The canvas.** A menu bar of dropdown panels grouped by sub-group (wave
  5 M1) in place of the ribbon; the node face — a collapse chevron, an
  editable collapsed value, name-first layout, input values at close zoom
  (N1); two zoom states, fixed port handles, the preview eye, wire glow
  and the single / double / thick-dashed wire convention, the timing badge,
  four-significant-figure values, placements at the view's centre; PCB
  traces from a router of our own (45° corners, quarter-unit lanes); typed
  literals editable on any unconnected input; collapsed sliders and the
  `A<B<C` slider shortcut in search-to-place; search matches Grasshopper
  names; value summaries from the `near` tier; the viewport gimbal; dimmer
  grid tokens.
- **Releases and About** (wave 5 R1): the build stamps its version, commit
  (`-dirty` when built with uncommitted changes to the code) and UTC date — `cicada
  --version`, `hello.version` on the socket, `GET /api/version` — and the
  settings menu's About dialog shows them with the protocol, the engine's
  threads and links to the repository and the release notes; `v*` tags
  build and publish the bundles.

### From the user tests

Ben's two user tests (2026-08-24, findings U1–U14; 2026-08-25, U15–U34 —
docs/17 §Usage findings) drove waves 4 and 5: the root model and the file
menu, the observer pop-out, the trace router, the literal chips, the
collapsed slider, the gimbal, the grid tokens (U1–U14); the menu bar, the
node face, the two zoom states, the wire convention, the timing badge, the
compact values, the display budget and the resizable cache, the profiler,
About and releases (U15–U34). U30 / U31 were measured rather than guessed:
1,000 fine-tier spheres are 8,000 triangles each — a 160 MB frame and
2.3 s of tessellation per redraw, re-paid on every undo because two value
sets thrash a 256 MiB cache — which is why the cache is 1 GiB and visible.

### Known limits

- Pre-release: no installer, no code signing or notarization (macOS:
  right-click → Open the first time), no support.
- The macOS bundle is Apple silicon only (`-macos-arm64`); there is no
  Intel build yet. The Windows bundle and the Linux binary are x86_64.
- The Linux asset is the bare engine binary with the app embedded: it needs
  the OpenCASCADE 7.8.1 run-time libraries on `LD_LIBRARY_PATH` (`python
  tools/fetch_occt.py --print-env bash` from a checkout prints them) and
  Python 3; the Windows and macOS bundles carry the libraries.
- A `-dirty` commit in About means the binary was built with uncommitted
  changes to its sources (`crates/`, `web/`, the manifests) — a dev build,
  not a release.
- Wave 5's round 2 packages V1 (viewport modes: Split · Floating · Window)
  and T1 (the 250 ms tooltip layer) land beside this entry; they belong in
  this section when the tag includes them.
