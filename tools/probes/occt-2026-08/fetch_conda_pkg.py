#!/usr/bin/env python3
"""Fetch + verify + extract ONE conda-forge package for the OCCT probe.

Probe-grade helper (docs/probes/occt-2026-08.md §Q1 runtime closure). It
is the seed of the `tools/fetch_occt.py` Item 3 WP-A will write, not that
script: no solver, no dependency walk — the caller names the package and
(optionally) a version/build, this picks the newest matching win-64 (or
--subdir) build from the anaconda.org listing, downloads it, checks the
sha256 the listing publishes, extracts the `.conda` (zip of .tar.zst
parts; needs `pip install zstandard`) and appends one line to
`manifest.tsv` in the cache dir so every byte fetched is on record.

Usage:
  fetch_conda_pkg.py [--cache DIR] [--subdir win-64] [--version V] [--build B] NAME...

Exit status 1 on any mismatch (size, sha256, no matching file) — never a
silent fallback.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

LISTING = "https://api.anaconda.org/package/conda-forge/{name}/files"
DOWNLOAD = "https://api.anaconda.org/download/conda-forge/{name}/{version}/{subdir}/{basename}"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def extract_conda(pkg: Path, out: Path) -> None:
    import zstandard  # pip install zstandard

    out.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(pkg) as z:
        for name in z.namelist():
            if not name.endswith(".tar.zst"):
                continue
            dctx = zstandard.ZstdDecompressor()
            with dctx.stream_reader(io.BytesIO(z.read(name))) as reader:
                with tarfile.open(fileobj=reader, mode="r|") as tar:
                    tar.extractall(out)


def pick(files: list[dict], subdir: str, version: str | None, build: str | None) -> dict:
    cands = [
        f
        for f in files
        if f.get("attrs", {}).get("subdir") == subdir
        and f["basename"].endswith(".conda")
        and (version is None or f["version"] == version)
        and (build is None or f["attrs"].get("build") == build)
    ]
    if not cands:
        raise SystemExit(f"error: no .conda file for subdir={subdir} version={version} build={build}")
    cands.sort(key=lambda f: f.get("upload_time", ""), reverse=True)
    return cands[0]


def fetch_one(name: str, cache: Path, subdir: str, version: str | None, build: str | None) -> Path:
    listing_path = cache / f"{name}_files.json"
    if not listing_path.exists():
        with urllib.request.urlopen(LISTING.format(name=name)) as r:
            listing_path.write_bytes(r.read())
    files = json.loads(listing_path.read_text(encoding="utf-8"))
    f = pick(files, subdir, version, build)
    basename = Path(f["basename"]).name
    url = DOWNLOAD.format(name=name, version=f["version"], subdir=subdir, basename=basename)
    pkg = cache / basename
    if not pkg.exists():
        print(f"downloading {url}")
        with urllib.request.urlopen(url) as r:
            pkg.write_bytes(r.read())
    size = pkg.stat().st_size
    digest = sha256_file(pkg)
    if size != f["size"]:
        raise SystemExit(f"error: {basename}: size {size} != listing {f['size']}")
    if digest != f["sha256"]:
        raise SystemExit(f"error: {basename}: sha256 {digest} != listing {f['sha256']}")
    out = cache / basename.removesuffix(".conda")
    if not (out / "info" / "index.json").exists():
        extract_conda(pkg, out)
    index = json.loads((out / "info" / "index.json").read_text(encoding="utf-8"))
    with (cache / "manifest.tsv").open("a", encoding="utf-8") as m:
        m.write(f"{name}\t{f['version']}\t{f['attrs'].get('build')}\t{subdir}\t{basename}\t{size}\t{digest}\t{url}\t{index.get('license')}\t{' | '.join(index.get('depends', []))}\n")
    print(f"{basename}  {size:,} B  sha256={digest}  license={index.get('license')}")
    print(f"  depends: {index.get('depends')}")
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument(
        "--cache",
        default=os.path.join(os.environ.get("LOCALAPPDATA") or os.path.expanduser("~"), "cicada-occt", "deps"),
        help="download cache dir (default: %%LOCALAPPDATA%%/cicada-occt/deps)",
    )
    ap.add_argument("--subdir", default="win-64")
    ap.add_argument("--version")
    ap.add_argument("--build")
    ap.add_argument("names", nargs="+")
    args = ap.parse_args(argv)
    cache = Path(args.cache)
    cache.mkdir(parents=True, exist_ok=True)
    for name in args.names:
        fetch_one(name, cache, args.subdir, args.version, args.build)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
