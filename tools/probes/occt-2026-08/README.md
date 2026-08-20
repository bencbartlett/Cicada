# OCCT probe (2026-08)

The executable half of `docs/probes/occt-2026-08.md` (docs/17 §Item P).
A standalone cargo project — its empty `[workspace]` table keeps the main
workspace from adopting it, so nothing in the repo's default build needs
OCCT. Throwaway code; the memo is the deliverable.

- `src/main.rs` — `smoke` (build + tessellate the four probe shapes),
  `dump <dir>` (serialize with BinTools/BRepTools, hash files and
  triangle buffers; run twice in two processes and diff), `bench <parts>`
  (timings per op and per part).
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
