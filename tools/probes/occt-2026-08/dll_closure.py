#!/usr/bin/env python3
"""Transitive DLL import closure of one or more PE files (exe or dll).

The regression tool behind the memo's "how many DLLs does a shipped
cicada.exe need" numbers (docs/probes/occt-2026-08.md §Q1). The first
version of those numbers was walked by hand from `dumpbin /dependents`
output and got the STEP chain wrong (it said `TKCAF -> TKV3d`; the real
edge is `TKDESTEP -> TKXCAF -> TKV3d -> TKService -> freetype/FreeImage`).
This script computes the closure instead of a human, so the claim is
re-checkable in one command and a wrong number is a wrong tool, not a
wrong memory.

Pure Python, no dumpbin needed: it reads the PE import directory (and the
delay-load directory) itself. Walks from the roots; descends ONLY into
DLLs found in the given search dirs (the conda `Library\\bin`, extra dirs
such as freetype's/freeimage's), never into the OS's own DLLs.

Usage:
  dll_closure.py [--dir DIR]... [--json] [--allow-missing] ROOT...
  ROOT is a path to an exe/dll, or a bare name (resolved in --dir).

Exit status: 0 when every import resolves (search dirs, System32,
Windows, or a Windows api-set); 1 when something is MISSING (the binary
would fail to load with STATUS_DLL_NOT_FOUND) unless --allow-missing;
2 on usage/parse errors. Loud by design.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
from collections import deque
from pathlib import Path


class PeError(Exception):
    pass


def _rva_to_offset(sections, rva: int) -> int:
    for _name, va, vsize, rawsize, rawptr in sections:
        size = max(vsize, rawsize)
        if va <= rva < va + size:
            return rva - va + rawptr
    raise PeError(f"RVA 0x{rva:x} is in no section")


def _cstr(data: bytes, off: int) -> str:
    end = data.index(b"\0", off)
    return data[off:end].decode("ascii", errors="replace")


def pe_imports(path: Path) -> tuple[list[str], list[str]]:
    """Return (imports, delay_imports) as DLL names, in table order."""
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise PeError(f"{path}: not an MZ executable")
    (e_lfanew,) = struct.unpack_from("<I", data, 0x3C)
    if data[e_lfanew : e_lfanew + 4] != b"PE\0\0":
        raise PeError(f"{path}: no PE signature")
    coff = e_lfanew + 4
    nsections, opt_size = struct.unpack_from("<H", data, coff + 2)[0], struct.unpack_from(
        "<H", data, coff + 16
    )[0]
    opt = coff + 20
    (magic,) = struct.unpack_from("<H", data, opt)
    if magic == 0x20B:  # PE32+
        dd_off = opt + 112
    elif magic == 0x10B:  # PE32
        dd_off = opt + 96
    else:
        raise PeError(f"{path}: unknown optional-header magic 0x{magic:x}")
    (ndirs,) = struct.unpack_from("<I", data, dd_off - 4)
    sec_off = opt + opt_size
    sections = []
    for i in range(nsections):
        o = sec_off + i * 40
        name = data[o : o + 8].rstrip(b"\0").decode("ascii", errors="replace")
        vsize, va, rawsize, rawptr = struct.unpack_from("<IIII", data, o + 8)
        sections.append((name, va, vsize, rawsize, rawptr))

    def directory(index: int) -> tuple[int, int]:
        if index >= ndirs:
            return (0, 0)
        return struct.unpack_from("<II", data, dd_off + index * 8)

    imports: list[str] = []
    imp_rva, imp_size = directory(1)
    if imp_rva:
        off = _rva_to_offset(sections, imp_rva)
        while True:
            ilt, _ts, _fwd, name_rva, iat = struct.unpack_from("<IIIII", data, off)
            if ilt == 0 and name_rva == 0 and iat == 0:
                break
            imports.append(_cstr(data, _rva_to_offset(sections, name_rva)))
            off += 20

    delay: list[str] = []
    dl_rva, _dl_size = directory(13)
    if dl_rva:
        off = _rva_to_offset(sections, dl_rva)
        while True:
            attrs, name_rva = struct.unpack_from("<II", data, off)
            if name_rva == 0:
                break
            if attrs & 1 == 0:
                raise PeError(f"{path}: delay-load descriptor without RVA attribute (unsupported)")
            delay.append(_cstr(data, _rva_to_offset(sections, name_rva)))
            off += 32
    return imports, delay


def is_api_set(name: str) -> bool:
    n = name.lower()
    return n.startswith("api-ms-win-") or n.startswith("ext-ms-")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", action="append", default=[], help="search dir to resolve and descend into (repeatable, in order)")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--allow-missing", action="store_true", help="exit 0 even if imports are unresolved")
    ap.add_argument("roots", nargs="+")
    args = ap.parse_args(argv)

    search_dirs = [Path(d) for d in args.dir]
    for d in search_dirs:
        if not d.is_dir():
            print(f"error: --dir {d} is not a directory", file=sys.stderr)
            return 2
    windir = Path(os.environ.get("SystemRoot", r"C:\Windows"))
    system_dirs = [windir / "System32", windir]

    def resolve(name: str) -> tuple[str, Path | None]:
        for d in search_dirs:
            p = d / name
            if p.is_file():
                return ("dir", p)
        for d in system_dirs:
            p = d / name
            if p.is_file():
                return ("system", p)
        if is_api_set(name):
            return ("apiset", None)
        return ("missing", None)

    roots: list[Path] = []
    for r in args.roots:
        p = Path(r)
        if not p.is_file():
            kind, found = resolve(r)
            if found is None:
                print(f"error: root {r!r} not found as a path or in --dir", file=sys.stderr)
                return 2
            p = found
        roots.append(p)

    # name (lower) -> record
    seen: dict[str, dict] = {}
    queue: deque[tuple[str, Path | None, str]] = deque()
    for p in roots:
        queue.append((p.name, p, "root"))
    edges: dict[str, list[str]] = {}
    while queue:
        name, path, kind = queue.popleft()
        key = name.lower()
        if key in seen:
            continue
        rec = {"name": name, "kind": kind, "path": str(path) if path else None, "bytes": path.stat().st_size if path else None}
        seen[key] = rec
        if kind not in ("root", "dir"):
            continue  # do not descend into the OS
        try:
            imps, delay = pe_imports(path)  # type: ignore[arg-type]
        except (PeError, OSError, ValueError) as e:
            print(f"error: {e}", file=sys.stderr)
            return 2
        rec["delay_load"] = delay
        edges[name] = imps + delay
        for dep in imps + delay:
            dkind, dpath = resolve(dep)
            queue.append((dep, dpath, dkind))

    root_names = {p.name.lower() for p in roots}
    occt = sorted((r for r in seen.values() if r["name"].lower().startswith("tk") and r["name"].lower().endswith(".dll") and r["kind"] in ("dir", "root")), key=lambda r: r["name"].lower())
    other_dir = sorted((r for r in seen.values() if r["kind"] == "dir" and r not in occt), key=lambda r: r["name"].lower())
    system = sorted((r["name"] for r in seen.values() if r["kind"] == "system"), key=str.lower)
    apisets = sorted((r["name"] for r in seen.values() if r["kind"] == "apiset"), key=str.lower)
    missing = sorted((r["name"] for r in seen.values() if r["kind"] == "missing"), key=str.lower)
    occt_bytes = sum(r["bytes"] for r in occt)
    other_bytes = sum(r["bytes"] for r in other_dir)

    result = {
        "roots": [str(p) for p in roots],
        "occt_dlls": [{"name": r["name"], "bytes": r["bytes"]} for r in occt],
        "occt_count": len(occt),
        "occt_bytes": occt_bytes,
        "other_dir_dlls": [{"name": r["name"], "bytes": r["bytes"], "path": r["path"]} for r in other_dir],
        "other_dir_bytes": other_bytes,
        "system": system,
        "apisets": apisets,
        "missing": missing,
        "edges": edges,
    }
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print(f"roots: {', '.join(p.name for p in roots)}")
        print(f"OCCT DLLs in closure: {len(occt)}  total {occt_bytes:,} bytes")
        for r in occt:
            tag = " (root)" if r["name"].lower() in root_names else ""
            print(f"  {r['name']:<18} {r['bytes']:>12,}{tag}")
        if other_dir:
            print(f"other DLLs from search dirs: {len(other_dir)}  total {other_bytes:,} bytes")
            for r in other_dir:
                print(f"  {r['name']:<18} {r['bytes']:>12,}  {r['path']}")
        print(f"system DLLs (not shipped): {' '.join(system)}")
        print(f"api-sets (OS): {len(apisets)}")
        if missing:
            print(f"MISSING (STATUS_DLL_NOT_FOUND at load): {' '.join(missing)}")
        else:
            print("MISSING: none")
    if missing and not args.allow_missing:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
