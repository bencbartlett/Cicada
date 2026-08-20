# OCCT probe (2026-08)

The executable half of `docs/probes/occt-2026-08.md` (docs/17 §Item P).
A standalone cargo project — its empty `[workspace]` table keeps the main
workspace from adopting it, so nothing in the repo's default build needs
OCCT. Throwaway code; the memo is the deliverable.

- `src/main.rs` — `smoke` (build + tessellate the four probe shapes),
  `dump <dir>` (serialize with BinTools/BRepTools, hash files and
  triangle buffers; run twice in two processes and diff — panics unless
  the `.bin` file really is BinTools output, see patch 0002),
  `bench <parts>` (timings per op and per part), `throw` (drive OCCT
  into a `Standard_DomainError` through the binding; expected to abort
  with `0xC0000409` — the fail-loud input for the seam design).
- `src/bin/step.rs` — `occt-probe-step`, the STEP-linking twin: writes
  and reads back one STEP file. Its only purpose is the DLL closure a
  STEP-capable binary needs (memo §Q1): 26 OCCT DLLs + 21 font/codec
  DLLs from 16 more conda packages in conda-forge's build.
- `dll_closure.py` — transitive DLL import closure of exes/DLLs (pure
  Python PE reader, no dumpbin): sizes, system vs shipped, and a loud
  `MISSING` list (exit 1). The memo's DLL-count numbers come from it.
- `fetch_conda_pkg.py` — fetch + sha256-verify + extract one conda-forge
  package and append it to a manifest; the seed of Item 3 WP-A's
  `tools/fetch_occt.py`, not that script (no solver).
- `patches/` — the minimal changes the binding needed, each recorded in
  full and explained in the memo:
  - `0001-msvc-handle-aliases.diff` — source-only part of upstream PR #230
    (required to compile on MSVC at all).
  - `0002-bin-tools-symbol-collision.diff` — the binding's BinTools writer
    was silently the text writer (cxx symbol collision); required for Q2.
  - `0003-windows-static-system-libs.diff` — PR #216's link-lib hunk;
    required only for static OCCT, harmless for the shared build.
  - `0004-occt-sys-msvc-release-flags.diff` — source path only: the cmake
    crate drops `/O2` on MSVC, so `builtin` compiles an unoptimized OCCT.
    Note `opencascade-sys` takes `occt-sys` from crates.io; the fork's
    copy (and this patch) is only compiled through `[patch.crates-io]`.
- `Cargo.toml` — pins the binding by git rev and points a `[patch]` at a
  scratch clone with the patches applied (path documented inline; the
  memo's §Reproduce has the full command sequence).
