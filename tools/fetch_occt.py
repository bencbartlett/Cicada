#!/usr/bin/env python3
"""Fetch the pinned prebuilt OCCT into the user cache dir (docs/17 Item 3 WP-A).

Cicada links OCCT through the `occt` feature of `cicada-geom` against a
PREBUILT OpenCASCADE found via `DEP_OCCT_ROOT` (DECISIONS.md rented-kernels
row; docs/probes/occt-2026-08.md). The prebuilt is conda-forge's `occt 7.8.1`
build 103 ("novtk") per platform, plus the run-time packages its shared
libraries load. This script downloads those packages, verifies every byte
against the sha256 pinned in `tools/fetch_occt_manifest.json`, extracts them
into ONE conda-style prefix under the user cache dir (never inside a repo),
and prints the environment a shell needs. It is idempotent: a prefix whose
stamp matches the manifest is left alone.

    python tools/fetch_occt.py                      # fetch for this platform, print the env
    python tools/fetch_occt.py --print-env bash     # `export ...` lines   (also: powershell)
    python tools/fetch_occt.py --github-env         # append to $GITHUB_ENV / $GITHUB_PATH
    python tools/fetch_occt.py --dest DIR           # cache root (default below)
    python tools/fetch_occt.py --subdir linux-64    # another platform's prefix (prefetch / inspection)
    python tools/fetch_occt.py --check-closure      # static import-closure check of the prefix
    python tools/fetch_occt.py --manifest-hash      # the CI cache key
    python tools/fetch_occt.py regenerate-manifest  # MAINTAINER: re-resolve from anaconda.org

Default cache root: `%LOCALAPPDATA%\\cicada-occt` on Windows, else
`$XDG_CACHE_HOME/cicada-occt` (`~/.cache/cicada-occt`). The prefix for a
platform is `<root>/occt-<version>-<subdir>`; downloads sit in
`<root>/downloads/<subdir>`. `DEP_OCCT_ROOT` is `<prefix>/Library` on Windows
(conda's Windows layout) and `<prefix>` elsewhere; the shared libraries are in
`<DEP_OCCT_ROOT>/bin` (Windows, goes on PATH) or `<prefix>/lib` (goes on
LD_LIBRARY_PATH / DYLD_LIBRARY_PATH).

Why the run-time packages: conda-forge builds OCCT with USE_FREETYPE and
USE_FREEIMAGE on, so `TKDESTEP -> TKXCAF -> TKV3d -> TKService ->
freetype + FreeImage (+ FreeImage's codec stack)`. A binary that only uses the
modeling toolkits needs none of it, but the binding's link list carries the
STEP toolkits and an unoptimized (debug/test) link keeps them, so every test
binary loads the whole closure (measured 2026-08-20: exit 0xC0000135 without
it). The manifest therefore carries the full closure from day one; trimming it
is the own-built-OCCT work of docs/17 Item 3 WP-C, not this script's.

Dependencies: Python 3 standard library. Extracting `.conda` archives needs a
zstd decoder: the standard library's `compression.zstd` (Python 3.14+) or the
`zstandard` package (`python -m pip install zstandard`); the script tells you
which, it never guesses. `regenerate-manifest` needs network access and is
the ONLY operation that does anything unpinned.

Exit status: 0 on success; 1 on any mismatch (size, sha256, manifest,
closure) or missing tool -- never a silent fallback.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import fnmatch
import hashlib
import io
import json
import os
import platform
import re
import shutil
import struct
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path
from typing import Iterable

HERE = Path(__file__).resolve().parent
MANIFEST_PATH = HERE / "fetch_occt_manifest.json"
STAMP_NAME = ".cicada-occt-stamp.json"
META_DIR_NAME = ".cicada-occt-meta"
MANIFEST_FORMAT = 1

OCCT_VERSION = "7.8.1"
OCCT_BUILD_NUMBER = 103
OCCT_BUILD_PREFIX = "novtk"
SUPPORTED_SUBDIRS = ("win-64", "linux-64", "osx-64", "osx-arm64")

LISTING_URL = "https://api.anaconda.org/package/conda-forge/{name}/files"
DOWNLOAD_URL = "https://api.anaconda.org/download/conda-forge/{name}/{version}/{subdir}/{filename}"

# Dependencies the resolver does NOT put in the prefix: conda virtual packages
# (`__glibc`, `__osx`, ...), font metapackages and font files (fontconfig lists
# them; no shared library needs them), and the MSVC redistributable packages
# (the VC++ runtime is a machine-level install; the memo records MSVCP140 as
# the one genuinely new requirement on Windows).
IGNORED_DEPENDENCIES = ("fonts-conda-ecosystem", "fonts-conda-forge", "vc", "vc14_runtime", "ucrt")
IGNORED_DEPENDENCY_GLOBS = ("__*", "font-ttf-*", "vs20*_runtime")

# The binding's own cmake project says `cmake_minimum_required(3.1)`; cmake 4
# hosts refuse that unless told the floor (docs/probes/occt-2026-08.md Q1).
CMAKE_POLICY_VERSION_MINIMUM = "3.5"


class FetchError(Exception):
    """Anything that must stop the run: printed as `error: ...`, exit 1."""


# ---------------------------------------------------------------------------
# Platform + layout
# ---------------------------------------------------------------------------


def detect_subdir(system: str | None = None, machine: str | None = None) -> str:
    """conda subdir for this (or the given) platform; loud on anything else."""
    system = (system or platform.system()).lower()
    machine = (machine or platform.machine()).lower()
    table = {
        ("windows", "amd64"): "win-64",
        ("windows", "x86_64"): "win-64",
        ("linux", "x86_64"): "linux-64",
        ("linux", "amd64"): "linux-64",
        ("darwin", "x86_64"): "osx-64",
        ("darwin", "arm64"): "osx-arm64",
        ("darwin", "aarch64"): "osx-arm64",
    }
    subdir = table.get((system, machine))
    if subdir is None:
        raise FetchError(
            f"no pinned OCCT prebuilt for platform {system}/{machine}; supported: {', '.join(SUPPORTED_SUBDIRS)} "
            "(linux-aarch64 has a conda-forge build but is not pinned -- run `regenerate-manifest` with it added)"
        )
    return subdir


def default_cache_root(environ: dict[str, str] | None = None, system: str | None = None) -> Path:
    """`%LOCALAPPDATA%\\cicada-occt` on Windows, `$XDG_CACHE_HOME/cicada-occt` elsewhere."""
    environ = os.environ if environ is None else environ
    system = (system or platform.system()).lower()
    if system == "windows":
        local = environ.get("LOCALAPPDATA")
        if not local:
            raise FetchError("LOCALAPPDATA is not set; pass --dest")
        return Path(local) / "cicada-occt"
    xdg = environ.get("XDG_CACHE_HOME")
    base = Path(xdg) if xdg else Path(environ.get("HOME", "~")).expanduser() / ".cache"
    return base / "cicada-occt"


class Layout:
    """Where things are for one platform's prefix."""

    def __init__(self, cache_root: Path, subdir: str, occt_version: str = OCCT_VERSION):
        if subdir not in SUPPORTED_SUBDIRS:
            raise FetchError(f"unsupported subdir {subdir!r}; supported: {', '.join(SUPPORTED_SUBDIRS)}")
        self.cache_root = cache_root
        self.subdir = subdir
        self.prefix = cache_root / f"occt-{occt_version}-{subdir}"
        self.downloads = cache_root / "downloads" / subdir
        self.is_windows = subdir.startswith("win-")
        self.is_macos = subdir.startswith("osx-")

    @property
    def dep_occt_root(self) -> Path:
        return self.prefix / "Library" if self.is_windows else self.prefix

    @property
    def library_dir(self) -> Path:
        return self.dep_occt_root / "bin" if self.is_windows else self.prefix / "lib"

    @property
    def loader_variable(self) -> str:
        if self.is_windows:
            return "PATH"
        return "DYLD_LIBRARY_PATH" if self.is_macos else "LD_LIBRARY_PATH"

    @property
    def stamp(self) -> Path:
        return self.prefix / STAMP_NAME


def windows_to_msys(path: str) -> str:
    """`C:\\Users\\x` -> `/c/Users/x` for PATH entries in Git Bash / MSYS shells."""
    match = re.match(r"^([A-Za-z]):[\\/](.*)$", path)
    if not match:
        return path.replace("\\", "/")
    drive, rest = match.groups()
    rest = rest.replace("\\", "/")
    return f"/{drive.lower()}/{rest}"


def env_lines(layout: Layout, shell: str) -> list[str]:
    """The lines a shell needs: DEP_OCCT_ROOT, the loader path, the cmake policy floor."""
    root = str(layout.dep_occt_root)
    libdir = str(layout.library_dir)
    if shell == "bash":
        if layout.is_windows:
            # Git Bash: PATH entries are POSIX; DEP_OCCT_ROOT is read by cmake
            # (a native program), which takes forward-slash Windows paths.
            root = root.replace("\\", "/")
            libdir = windows_to_msys(libdir)
        var = layout.loader_variable
        return [
            f"export DEP_OCCT_ROOT='{root}'",
            f"export {var}='{libdir}'\"${{{var}:+:${var}}}\"",
            f"export CMAKE_POLICY_VERSION_MINIMUM='{CMAKE_POLICY_VERSION_MINIMUM}'",
        ]
    if shell == "powershell":
        var = layout.loader_variable
        ps_var = "Path" if var == "PATH" else var
        separator = ";" if layout.is_windows else ":"
        return [
            f"$env:DEP_OCCT_ROOT = '{root}'",
            f"$env:{ps_var} = '{libdir}{separator}' + $env:{ps_var}",
            f"$env:CMAKE_POLICY_VERSION_MINIMUM = '{CMAKE_POLICY_VERSION_MINIMUM}'",
        ]
    raise FetchError(f"unknown shell {shell!r}; use bash or powershell")


def github_env_entries(layout: Layout, existing_loader_value: str) -> tuple[list[str], list[str]]:
    """(lines for $GITHUB_ENV, lines for $GITHUB_PATH)."""
    env = [
        f"DEP_OCCT_ROOT={layout.dep_occt_root}",
        f"CMAKE_POLICY_VERSION_MINIMUM={CMAKE_POLICY_VERSION_MINIMUM}",
    ]
    path: list[str] = []
    if layout.is_windows:
        path.append(str(layout.library_dir))
    else:
        value = str(layout.library_dir)
        if existing_loader_value:
            value = f"{value}:{existing_loader_value}"
        env.append(f"{layout.loader_variable}={value}")
    return env, path


# ---------------------------------------------------------------------------
# Manifest
# ---------------------------------------------------------------------------


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest(path: Path = MANIFEST_PATH) -> dict:
    try:
        raw = path.read_bytes()
    except FileNotFoundError as error:
        raise FetchError(f"manifest {path} is missing; run `regenerate-manifest`") from error
    manifest = json.loads(raw.decode("utf-8"))
    validate_manifest(manifest)
    manifest["_sha256"] = sha256_bytes(raw)
    return manifest


def validate_manifest(manifest: dict) -> None:
    """Shape check: a broken manifest is refused before any byte is fetched."""
    if manifest.get("format") != MANIFEST_FORMAT:
        raise FetchError(f"manifest format {manifest.get('format')!r} != {MANIFEST_FORMAT}")
    for key in ("occt_version", "occt_build_number", "download_url", "platforms"):
        if key not in manifest:
            raise FetchError(f"manifest lacks {key!r}")
    for subdir, entry in manifest["platforms"].items():
        packages = entry.get("packages")
        if not isinstance(packages, list) or not packages:
            raise FetchError(f"manifest platform {subdir}: no packages")
        names = [package.get("name") for package in packages]
        if "occt" not in names:
            raise FetchError(f"manifest platform {subdir}: occt itself is missing")
        for package in packages:
            for key in ("name", "version", "build", "filename", "size", "sha256", "license"):
                if key not in package:
                    raise FetchError(f"manifest platform {subdir}: package {package.get('name')!r} lacks {key!r}")
            if not re.fullmatch(r"[0-9a-f]{64}", package["sha256"]):
                raise FetchError(f"manifest platform {subdir}: {package['filename']}: malformed sha256")
            if not isinstance(package["size"], int) or package["size"] <= 0:
                raise FetchError(f"manifest platform {subdir}: {package['filename']}: malformed size")
            if package["name"] == "occt" and package["version"] != manifest["occt_version"]:
                raise FetchError(
                    f"manifest platform {subdir}: occt version {package['version']} != {manifest['occt_version']}"
                )
        if len(set(names)) != len(names):
            raise FetchError(f"manifest platform {subdir}: duplicate package names")


def manifest_packages(manifest: dict, subdir: str) -> list[dict]:
    entry = manifest["platforms"].get(subdir)
    if entry is None:
        raise FetchError(
            f"no pinned OCCT prebuilt for {subdir} in the manifest (pinned: {', '.join(manifest['platforms'])})"
        )
    return entry["packages"]


def package_url(manifest: dict, subdir: str, package: dict) -> str:
    return manifest["download_url"].format(
        name=package["name"], version=package["version"], subdir=subdir, filename=package["filename"]
    )


# ---------------------------------------------------------------------------
# Download + verify
# ---------------------------------------------------------------------------


def verify_archive(path: Path, expected_size: int, expected_sha256: str) -> None:
    size = path.stat().st_size
    if size != expected_size:
        raise FetchError(f"{path.name}: size {size} != pinned {expected_size}")
    digest = sha256_file(path)
    if digest != expected_sha256:
        raise FetchError(f"{path.name}: sha256 {digest} != pinned {expected_sha256}")


def download(url: str, destination: Path, log) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(destination.suffix + ".part")
    log(f"downloading {url}")
    with urllib.request.urlopen(url) as response, partial.open("wb") as handle:
        shutil.copyfileobj(response, handle, 1 << 20)
    partial.replace(destination)


def ensure_archive(manifest: dict, subdir: str, package: dict, downloads: Path, log) -> Path:
    """The verified archive on disk, downloading it only when absent or wrong."""
    path = downloads / package["filename"]
    if path.exists():
        try:
            verify_archive(path, package["size"], package["sha256"])
            return path
        except FetchError as error:
            log(f"{error}; re-downloading")
            path.unlink()
    download(package_url(manifest, subdir, package), path, log)
    verify_archive(path, package["size"], package["sha256"])
    return path


# ---------------------------------------------------------------------------
# Extraction (.conda = zip of zstd tarballs)
# ---------------------------------------------------------------------------


def open_zstd(fileobj):
    """A readable binary stream of the decompressed data, or a loud, specific refusal."""
    try:
        from compression import zstd as stdlib_zstd  # Python 3.14+
    except ImportError:
        stdlib_zstd = None
    if stdlib_zstd is not None:
        return stdlib_zstd.ZstdFile(fileobj, mode="rb")
    try:
        import zstandard
    except ImportError as error:
        raise FetchError(
            "extracting .conda archives needs a zstd decoder: Python 3.14+ has `compression.zstd` built in; "
            f"on this interpreter ({sys.version.split()[0]}) run:  {sys.executable} -m pip install zstandard"
        ) from error
    return zstandard.ZstdDecompressor().stream_reader(fileobj)


def safe_member_path(prefix: Path, name: str) -> Path:
    """Refuse archive members that would land outside `prefix`."""
    normalized = name.replace("\\", "/")
    if normalized.startswith("/") or re.match(r"^[A-Za-z]:", normalized):
        raise FetchError(f"archive member with an absolute path: {name!r}")
    parts = [part for part in normalized.split("/") if part not in ("", ".")]
    if any(part == ".." for part in parts):
        raise FetchError(f"archive member escaping the prefix: {name!r}")
    return prefix.joinpath(*parts) if parts else prefix


Symlink = tuple[Path, str]


def extract_tar_stream(stream, destination: Path) -> list[Symlink]:
    """Extract a tar stream under `destination`, refusing escapes. Symlinks
    are NOT created here but returned: a link may point at a file of another
    package (`_openmp_mutex` ships `libgomp.so.1 -> libgomp.so.1.0.0`, the
    target is libgomp's), so they are materialized once every package is in."""
    symlinks: list[Symlink] = []
    with tarfile.open(fileobj=stream, mode="r|") as archive:
        for member in archive:
            target = safe_member_path(destination, member.name)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.issym():
                symlinks.append((target, member.linkname))
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                if not target.parent.resolve().is_relative_to(destination.resolve()):
                    raise FetchError(f"archive member {member.name!r} would be written through a link")
                source = archive.extractfile(member)
                if source is None:
                    raise FetchError(f"unreadable archive member {member.name!r}")
                with target.open("wb") as handle:
                    shutil.copyfileobj(source, handle, 1 << 20)
                if member.mode & 0o111 and os.name != "nt":
                    target.chmod(target.stat().st_mode | 0o111)
            else:
                raise FetchError(f"unsupported archive member type for {member.name!r}")
    return symlinks


def materialize_symlinks(symlinks: list[Symlink]) -> None:
    """Create the links where the platform allows and copies where it does not."""
    pending = list(symlinks)
    while pending:
        remaining = []
        for target, linkname in pending:
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() or target.is_symlink():
                target.unlink()
            try:
                os.symlink(linkname, target)
                continue
            except (OSError, NotImplementedError):
                pass
            # Windows without symlink privilege (inspecting a Unix prefix): a
            # copy of the link target keeps the closure check honest. Links
            # to links resolve once their target has been materialized, so
            # unresolved ones are retried after the others.
            resolved = (target.parent / linkname).resolve()
            if resolved.is_file():
                shutil.copyfile(resolved, target)
            else:
                remaining.append((target, linkname))
        if len(remaining) == len(pending):
            unresolved = ", ".join(f"{target.name} -> {linkname}" for target, linkname in remaining)
            raise FetchError(f"cannot create symlinks and their targets are absent: {unresolved}")
        pending = remaining


def extract_conda(archive: Path, prefix: Path, meta_dir: Path) -> list[Symlink]:
    """Payload (`pkg-*.tar.zst`) into the prefix, metadata (`info-*.tar.zst`,
    incl. the license files) into `meta_dir`. Returns the symlinks to make."""
    symlinks: list[Symlink] = []
    with zipfile.ZipFile(archive) as bundle:
        names = bundle.namelist()
        payloads = [name for name in names if name.startswith("pkg-") and name.endswith(".tar.zst")]
        infos = [name for name in names if name.startswith("info-") and name.endswith(".tar.zst")]
        if len(payloads) != 1 or len(infos) != 1:
            raise FetchError(f"{archive.name}: not a .conda archive (members: {names})")
        for name, destination in ((infos[0], meta_dir), (payloads[0], prefix)):
            destination.mkdir(parents=True, exist_ok=True)
            with bundle.open(name) as compressed:
                data = io.BytesIO(compressed.read())
            symlinks.extend(extract_tar_stream(open_zstd(data), destination))
    return symlinks


def read_stamp(layout: Layout) -> dict | None:
    try:
        return json.loads(layout.stamp.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError):
        return {}


def stamp_matches(stamp: dict | None, manifest: dict, subdir: str) -> bool:
    if not stamp:
        return False
    expected = {package["filename"]: package["sha256"] for package in manifest_packages(manifest, subdir)}
    return stamp.get("manifest_sha256") == manifest["_sha256"] and stamp.get("packages") == expected


def fetch(manifest: dict, layout: Layout, log) -> bool:
    """Bring the prefix up to the manifest. Returns True when work was done."""
    packages = manifest_packages(manifest, layout.subdir)
    if stamp_matches(read_stamp(layout), manifest, layout.subdir):
        log(f"prefix {layout.prefix} is present and verified (stamp matches manifest {manifest['_sha256'][:12]})")
        return False
    archives = [ensure_archive(manifest, layout.subdir, package, layout.downloads, log) for package in packages]
    if layout.prefix.exists():
        log(f"prefix {layout.prefix} is stale or incomplete; re-extracting")
        shutil.rmtree(layout.prefix)
    layout.prefix.mkdir(parents=True)
    symlinks: list[Symlink] = []
    for package, archive in zip(packages, archives):
        log(f"extracting {package['filename']}")
        symlinks.extend(extract_conda(archive, layout.prefix, layout.prefix / META_DIR_NAME / package["name"]))
    materialize_symlinks(symlinks)
    marker = layout.dep_occt_root / "include" / "opencascade" / "Standard_Version.hxx"
    if not marker.is_file():
        raise FetchError(f"extraction finished but {marker} is missing -- not an OCCT prefix")
    check_occt_version(marker, manifest["occt_version"])
    stamp = {
        "manifest_sha256": manifest["_sha256"],
        "subdir": layout.subdir,
        "packages": {package["filename"]: package["sha256"] for package in packages},
        "written": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
    }
    layout.stamp.write_text(json.dumps(stamp, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return True


def check_occt_version(standard_version_hxx: Path, expected: str) -> None:
    """The binding accepts any 7.x >= 7.8; the pin lives here: the extracted
    headers must say exactly the manifest's version."""
    text = standard_version_hxx.read_text(encoding="utf-8", errors="replace")
    found = parse_occt_version(text)
    if found != expected:
        raise FetchError(f"extracted OCCT reports version {found}, manifest pins {expected}")


def parse_occt_version(standard_version_hxx_text: str) -> str:
    values = {}
    for key in ("OCC_VERSION_MAJOR", "OCC_VERSION_MINOR", "OCC_VERSION_MAINTENANCE"):
        match = re.search(rf"#define\s+{key}\s+(\d+)", standard_version_hxx_text)
        if not match:
            raise FetchError(f"Standard_Version.hxx lacks {key}")
        values[key] = match.group(1)
    return ".".join(values[k] for k in ("OCC_VERSION_MAJOR", "OCC_VERSION_MINOR", "OCC_VERSION_MAINTENANCE"))


# ---------------------------------------------------------------------------
# Static import-closure check (PE / ELF / Mach-O), no execution needed
# ---------------------------------------------------------------------------

WINDOWS_SYSTEM_DLLS = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "comdlg32.dll", "crypt32.dll", "dbghelp.dll",
    "gdi32.dll", "kernel32.dll", "msvcp140.dll", "msvcp140_1.dll", "msvcp140_2.dll", "ntdll.dll", "ole32.dll",
    "oleaut32.dll", "opengl32.dll", "psapi.dll", "rpcrt4.dll", "secur32.dll", "shell32.dll", "shlwapi.dll",
    "user32.dll", "userenv.dll", "vcomp140.dll", "vcruntime140.dll", "vcruntime140_1.dll", "version.dll",
    "winmm.dll", "ws2_32.dll", "wsock32.dll", "glu32.dll", "imm32.dll", "setupapi.dll", "winspool.drv",
}
MACHO_MAGICS = (
    b"\xcf\xfa\xed\xfe",  # 64-bit little-endian (as stored)
    b"\xce\xfa\xed\xfe",  # 32-bit little-endian
    b"\xfe\xed\xfa\xcf",  # 64-bit big-endian
    b"\xfe\xed\xfa\xce",  # 32-bit big-endian
    b"\xca\xfe\xba\xbe",  # fat
    b"\xca\xfe\xba\xbf",  # fat, 64-bit entries
)
LINUX_SYSTEM_SONAMES = {
    "libc.so.6", "libm.so.6", "libpthread.so.0", "libdl.so.2", "librt.so.1", "libutil.so.1", "libresolv.so.2",
    "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "linux-vdso.so.1", "libgcc_s.so.1", "libstdc++.so.6",
}


def pe_imports(data: bytes) -> list[str]:
    """Import DLL names of a PE file (pure Python; enough for import tables)."""
    if data[:2] != b"MZ":
        raise FetchError("not a PE file")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise FetchError("not a PE file (no PE signature)")
    sections_count = struct.unpack_from("<H", data, pe_offset + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe_offset + 20)[0]
    optional = pe_offset + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directories = optional + (112 if magic == 0x20B else 96)
    import_rva, import_size = struct.unpack_from("<II", data, directories + 8)
    sections = []
    at = optional + optional_size
    for _ in range(sections_count):
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, at + 8)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_pointer))
        at += 40

    def rva_to_offset(rva: int) -> int:
        for virtual_address, size, raw_pointer in sections:
            if virtual_address <= rva < virtual_address + size:
                return raw_pointer + (rva - virtual_address)
        raise FetchError(f"RVA {rva:#x} outside every section")

    names = []
    if import_rva == 0 or import_size == 0:
        return names
    at = rva_to_offset(import_rva)
    while True:
        entry = struct.unpack_from("<IIIII", data, at)
        if entry[3] == 0:
            break
        name_at = rva_to_offset(entry[3])
        end = data.index(b"\0", name_at)
        names.append(data[name_at:end].decode("ascii", errors="replace"))
        at += 20
    return names


def elf_needed(data: bytes) -> list[str]:
    """DT_NEEDED entries of an ELF shared object or executable."""
    if data[:4] != b"\x7fELF":
        raise FetchError("not an ELF file")
    is_64 = data[4] == 2
    little = data[5] == 1
    endian = "<" if little else ">"
    if is_64:
        _, _, _, _, _, e_shoff = struct.unpack_from(endian + "HHIQQQ", data, 16)
        e_shentsize, e_shnum = struct.unpack_from(endian + "HH", data, 16 + 42)
    else:
        _, _, _, _, _, e_shoff = struct.unpack_from(endian + "HHIIII", data, 16)
        e_shentsize, e_shnum = struct.unpack_from(endian + "HH", data, 16 + 30)
    sections = []
    for index in range(e_shnum):
        at = e_shoff + index * e_shentsize
        if is_64:
            sh_type, _, _, sh_offset, sh_size, sh_link, _, _, sh_entsize = struct.unpack_from(
                endian + "IQQQQIIQQ", data, at + 4
            )
        else:
            sh_type, _, _, sh_offset, sh_size, sh_link, _, _, sh_entsize = struct.unpack_from(
                endian + "IIIIIIIII", data, at + 4
            )
        sections.append((sh_type, sh_offset, sh_size, sh_link, sh_entsize))
    needed = []
    for sh_type, sh_offset, sh_size, sh_link, sh_entsize in sections:
        if sh_type != 6:  # SHT_DYNAMIC
            continue
        _, str_offset, str_size, _, _ = sections[sh_link]
        strings = data[str_offset : str_offset + str_size]
        entry_size = sh_entsize or (16 if is_64 else 8)
        for at in range(sh_offset, sh_offset + sh_size, entry_size):
            tag, value = struct.unpack_from(endian + ("qQ" if is_64 else "iI"), data, at)
            if tag == 0:
                break
            if tag == 1:  # DT_NEEDED
                end = strings.index(b"\0", value)
                needed.append(strings[value:end].decode("utf-8", errors="replace"))
    return needed


def macho_dylibs(data: bytes) -> list[str]:
    """LC_LOAD_DYLIB (+ weak/reexport) install names of a Mach-O file (thin or fat)."""
    magic = struct.unpack_from(">I", data, 0)[0]
    if magic in (0xCAFEBABE, 0xCAFEBABF):  # fat: take the first slice
        count = struct.unpack_from(">I", data, 4)[0]
        if count == 0:
            return []
        if magic == 0xCAFEBABE:
            _, _, offset, size, _ = struct.unpack_from(">IIIII", data, 8)
        else:
            _, _, offset, size, _, _ = struct.unpack_from(">IIQQII", data, 8)
        return macho_dylibs(data[offset : offset + size])
    # Read big-endian, a little-endian Mach-O (every Apple platform today)
    # shows its magic byte-swapped.
    if magic in (0xCFFAEDFE, 0xCEFAEDFE):
        endian = "<"
    elif magic in (0xFEEDFACF, 0xFEEDFACE):
        endian = ">"
    else:
        raise FetchError("not a Mach-O file")
    is_64 = magic in (0xCFFAEDFE, 0xFEEDFACF)
    ncmds = struct.unpack_from(endian + "I", data, 16)[0]
    at = 32 if is_64 else 28
    names = []
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from(endian + "II", data, at)
        if cmd in (0xC, 0x18 | 0x80000000, 0x1F | 0x80000000, 0xD):  # LOAD, LOAD_WEAK, REEXPORT, ID (skip)
            name_offset = struct.unpack_from(endian + "I", data, at + 8)[0]
            start = at + name_offset
            end = data.index(b"\0", start)
            name = data[start:end].decode("utf-8", errors="replace")
            if cmd != 0xD:
                names.append(name)
        at += cmdsize
    return names


def shared_library_files(layout: Layout) -> list[Path]:
    libdir = layout.library_dir
    if not libdir.is_dir():
        raise FetchError(f"{libdir} does not exist; fetch first")
    if layout.is_windows:
        pattern = "*.dll"
    elif layout.is_macos:
        pattern = "*.dylib"
    else:
        pattern = "*.so*"
    return sorted(path for path in libdir.iterdir() if path.is_file() and fnmatch.fnmatch(path.name.lower(), pattern))


def check_closure(layout: Layout, log) -> list[str]:
    """Every shared library the prefix's libraries import must be in the
    prefix or a known system library. Returns the list of unresolved names
    (empty = closed)."""
    files = shared_library_files(layout)
    present = {path.name.lower() for path in files}
    present |= {path.name for path in files}
    unresolved: dict[str, set[str]] = {}
    for path in files:
        data = path.read_bytes()
        if not (data[:2] == b"MZ" or data[:4] == b"\x7fELF" or data[:4] in MACHO_MAGICS):
            # GNU ld scripts (`libgcc_s.so` is 4 lines of text) are not loaded.
            log(f"  skipping {path.name}: not a shared object")
            continue
        try:
            if layout.is_windows:
                imports = pe_imports(data)
            elif layout.is_macos:
                imports = macho_dylibs(data)
            else:
                imports = elf_needed(data)
        except FetchError as error:
            raise FetchError(f"{path.name}: {error}") from error
        for name in imports:
            if layout.is_windows:
                lower = name.lower()
                ok = lower in present or lower in WINDOWS_SYSTEM_DLLS or lower.startswith("api-ms-win-")
            elif layout.is_macos:
                base = name.rsplit("/", 1)[-1]
                ok = (
                    name.startswith("/usr/lib/")
                    or name.startswith("/System/Library/")
                    or (name.startswith(("@rpath/", "@loader_path/")) and base in present)
                )
            else:
                ok = name in present or name in LINUX_SYSTEM_SONAMES
            if not ok:
                unresolved.setdefault(name, set()).add(path.name)
    log(f"{layout.subdir}: {len(files)} shared libraries in {layout.library_dir}")
    for name in sorted(unresolved):
        log(f"  UNRESOLVED {name}  (needed by {', '.join(sorted(unresolved[name]))})")
    return sorted(unresolved)


# ---------------------------------------------------------------------------
# regenerate-manifest: the only unpinned operation (maintainer, network)
# ---------------------------------------------------------------------------


def version_key(version: str) -> tuple:
    """Ordering close enough to conda's VersionOrder for the constraints in
    these packages: dotted numeric components, pre-release tags sort before
    the release, `post` after."""
    parts = re.split(r"[._+-]", version.lower())
    key = []
    for part in parts:
        for token in re.findall(r"\d+|[a-z]+", part):
            if token.isdigit():
                key.append((1, int(token), ""))
            elif token == "post":
                key.append((2, 0, token))
            else:
                key.append((0, 0, token))  # alpha/beta/rc/dev: before the release component
    return tuple(key)


def compare_versions(left: str, right: str) -> int:
    """-1 / 0 / 1; the shorter key is padded with release components so that
    a pre-release tag sorts BELOW the bare release (`3.0a0` < `3.0`), which is
    what conda's `<3.0a0` upper bounds rely on."""
    a, b = list(version_key(left)), list(version_key(right))
    pad = (1, 0, "")
    while len(a) < len(b):
        a.append(pad)
    while len(b) < len(a):
        b.append(pad)
    return (a > b) - (a < b)


def version_satisfies(version: str, spec: str) -> bool:
    """`spec` is conda's version spec: `>=1.2,<2.0a0`, `1.2.*`, `1.2.3`,
    alternatives joined by `|`. Anything else is refused loudly."""
    if spec in ("", "*"):
        return True
    for alternative in spec.split("|"):
        if all(_single_version_constraint(version, part.strip()) for part in alternative.split(",")):
            return True
    return False


def _single_version_constraint(version: str, constraint: str) -> bool:
    match = re.fullmatch(r"(>=|<=|==|!=|>|<|=)?\s*([0-9A-Za-z._+-]+?)(\.\*|\*)?", constraint)
    if not match:
        raise FetchError(f"unsupported version constraint {constraint!r}")
    operator, bound, star = match.groups()
    if star or operator == "=":
        return version == bound or version.startswith(bound + ".")
    if operator is None or operator == "==":
        return version == bound
    order = compare_versions(version, bound)
    return {">=": order >= 0, "<=": order <= 0, ">": order > 0, "<": order < 0, "!=": order != 0}[operator]


def parse_depends(spec: str) -> tuple[str, str, str]:
    """`name [version_spec [build_glob]]`."""
    parts = spec.split()
    if not parts:
        raise FetchError("empty dependency spec")
    name = parts[0]
    version_spec = parts[1] if len(parts) > 1 else ""
    build_glob = parts[2] if len(parts) > 2 else "*"
    if len(parts) > 3:
        raise FetchError(f"unsupported dependency spec {spec!r}")
    return name, version_spec, build_glob


def is_ignored_dependency(name: str) -> bool:
    return name in IGNORED_DEPENDENCIES or any(
        fnmatch.fnmatchcase(name, glob) for glob in IGNORED_DEPENDENCY_GLOBS
    )


def fetch_listing(name: str, cache: dict[str, list[dict]], listing_dir: Path | None, log) -> list[dict]:
    if name in cache:
        return cache[name]
    if listing_dir is not None:
        saved = listing_dir / f"{name}_files.json"
        if saved.exists():
            cache[name] = json.loads(saved.read_text(encoding="utf-8"))
            return cache[name]
    url = LISTING_URL.format(name=name)
    log(f"listing {url}")
    with urllib.request.urlopen(url) as response:
        raw = response.read()
    if listing_dir is not None:
        listing_dir.mkdir(parents=True, exist_ok=True)
        (listing_dir / f"{name}_files.json").write_bytes(raw)
    cache[name] = json.loads(raw.decode("utf-8"))
    return cache[name]


def candidate_records(listing: list[dict], subdir: str) -> list[dict]:
    records = [
        record
        for record in listing
        if record.get("attrs", {}).get("subdir") == subdir
        and record["basename"].endswith(".conda")
        and "main" in (record.get("labels") or ["main"])
    ]
    records.sort(
        key=lambda record: (
            version_key(record["version"]),
            record["attrs"].get("build_number", 0),
            record["attrs"].get("timestamp", 0),
        ),
        reverse=True,
    )
    return records


def pick(records: list[dict], constraints: list[tuple[str, str]], name: str, subdir: str) -> dict:
    for record in records:
        version = record["version"]
        build = record["attrs"].get("build", "")
        if all(
            version_satisfies(version, vspec) and fnmatch.fnmatchcase(build, bglob) for vspec, bglob in constraints
        ):
            return record
    raise FetchError(f"{subdir}: no {name} build satisfies {constraints}")


def resolve_platform(subdir: str, listing_cache: dict, listing_dir: Path | None, log) -> list[dict]:
    """occt (pinned version + build number + build prefix) and the transitive
    closure of its `depends`, newest satisfying build each; a fixed-point
    iteration re-picks packages whose constraints tightened later."""
    occt_listing = fetch_listing("occt", listing_cache, listing_dir, log)
    occt_records = [
        record
        for record in candidate_records(occt_listing, subdir)
        if record["version"] == OCCT_VERSION
        and record["attrs"].get("build_number") == OCCT_BUILD_NUMBER
        and record["attrs"].get("build", "").startswith(OCCT_BUILD_PREFIX)
    ]
    if len(occt_records) != 1:
        raise FetchError(
            f"{subdir}: expected exactly one occt {OCCT_VERSION} {OCCT_BUILD_PREFIX}_*_{OCCT_BUILD_NUMBER} build, "
            f"found {[record['basename'] for record in occt_records]}"
        )
    chosen: dict[str, dict] = {"occt": occt_records[0]}
    for _ in range(50):
        constraints: dict[str, list[tuple[str, str]]] = {}
        order: list[str] = []
        for dependent in list(chosen.values()):
            for spec in dependent["attrs"].get("depends", []):
                name, vspec, bglob = parse_depends(spec)
                if is_ignored_dependency(name) or name == "occt":
                    continue
                constraints.setdefault(name, []).append((vspec, bglob))
                if name not in order:
                    order.append(name)
        changed = False
        for name in order:
            records = candidate_records(fetch_listing(name, listing_cache, listing_dir, log), subdir)
            best = pick(records, constraints[name], name, subdir)
            if chosen.get(name, {}).get("basename") != best["basename"]:
                chosen[name] = best
                changed = True
        for name in [name for name in chosen if name != "occt" and name not in constraints]:
            del chosen[name]
            changed = True
        if not changed:
            break
    else:
        raise FetchError(f"{subdir}: dependency resolution did not converge")
    # occt first, then dependencies deepest-last is irrelevant for extraction
    # (conda packages do not overlap); keep a stable, readable order.
    ordered = [chosen["occt"]] + [chosen[name] for name in sorted(chosen) if name != "occt"]
    return [
        {
            "name": record["full_name"].split("/")[1],
            "version": record["version"],
            "build": record["attrs"].get("build"),
            "filename": record["basename"].split("/")[-1],
            "size": record["size"],
            "sha256": record["sha256"],
            "license": record["attrs"].get("license"),
            "depends": record["attrs"].get("depends", []),
        }
        for record in ordered
    ]


def regenerate_manifest(subdirs: Iterable[str], listing_dir: Path | None, log, path: Path = MANIFEST_PATH) -> dict:
    listing_cache: dict[str, list[dict]] = {}
    platforms = {}
    for subdir in subdirs:
        packages = resolve_platform(subdir, listing_cache, listing_dir, log)
        total = sum(package["size"] for package in packages)
        log(f"{subdir}: {len(packages)} packages, {total:,} bytes")
        platforms[subdir] = {"packages": packages}
    manifest = {
        "format": MANIFEST_FORMAT,
        "generated": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
        "generator": "tools/fetch_occt.py regenerate-manifest",
        "occt_version": OCCT_VERSION,
        "occt_build_number": OCCT_BUILD_NUMBER,
        "download_url": DOWNLOAD_URL,
        "ignored_dependencies": list(IGNORED_DEPENDENCIES) + list(IGNORED_DEPENDENCY_GLOBS),
        "platforms": platforms,
    }
    validate_manifest(manifest)
    # LF regardless of host: the manifest sha256 is the CI cache key and the
    # prefix stamp, so its bytes must not depend on the checkout's line endings.
    path.write_bytes((json.dumps(manifest, indent=1) + "\n").encode("utf-8"))
    log(f"wrote {path}")
    return manifest


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", nargs="?", choices=["fetch", "regenerate-manifest"], default="fetch")
    parser.add_argument("--dest", type=Path, help="cache root (default: the user cache dir, see above)")
    parser.add_argument("--subdir", choices=SUPPORTED_SUBDIRS, action="append", help="platform(s); default: this one")
    parser.add_argument("--print-env", choices=["bash", "powershell"], help="emit shell lines instead of the summary")
    parser.add_argument("--github-env", action="store_true", help="append to $GITHUB_ENV and $GITHUB_PATH")
    parser.add_argument("--check-closure", action="store_true", help="after fetching, verify the import closure")
    parser.add_argument("--manifest-hash", action="store_true", help="print the manifest sha256 and exit")
    parser.add_argument("--listing-dir", type=Path, help="regenerate-manifest: cache anaconda.org listings here")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    def log(message: str) -> None:
        if not args.quiet:
            print(message, file=sys.stderr)

    try:
        if args.command == "regenerate-manifest":
            regenerate_manifest(args.subdir or SUPPORTED_SUBDIRS, args.listing_dir, log)
            return 0
        manifest = load_manifest()
        if args.manifest_hash:
            print(manifest["_sha256"])
            return 0
        cache_root = args.dest or default_cache_root()
        subdirs = args.subdir or [detect_subdir()]
        for subdir in subdirs:
            layout = Layout(cache_root, subdir, manifest["occt_version"])
            fetch(manifest, layout, log)
            if args.check_closure:
                unresolved = check_closure(layout, log)
                if unresolved:
                    raise FetchError(f"{subdir}: import closure is not closed: {', '.join(unresolved)}")
                log(f"{subdir}: import closure is closed")
            if args.print_env:
                print("\n".join(env_lines(layout, args.print_env)))
            elif args.github_env:
                env_file = os.environ.get("GITHUB_ENV")
                path_file = os.environ.get("GITHUB_PATH")
                if not env_file or not path_file:
                    raise FetchError("--github-env needs GITHUB_ENV and GITHUB_PATH in the environment")
                env, path = github_env_entries(layout, os.environ.get(layout.loader_variable, ""))
                with open(env_file, "a", encoding="utf-8") as handle:
                    handle.write("".join(line + "\n" for line in env))
                with open(path_file, "a", encoding="utf-8") as handle:
                    handle.write("".join(line + "\n" for line in path))
                log("wrote " + ", ".join(env + path))
            else:
                print(f"DEP_OCCT_ROOT={layout.dep_occt_root}")
                print(f"{layout.loader_variable}+={layout.library_dir}")
                print(f"CMAKE_POLICY_VERSION_MINIMUM={CMAKE_POLICY_VERSION_MINIMUM}")
        return 0
    except FetchError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
