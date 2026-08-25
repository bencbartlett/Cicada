#!/usr/bin/env python3
"""The redistributable bundle (docs/17 wave 4, L3): make it, check it, smoke it.

    python tools/launch/bundle.py --out dist/                 # make the bundle from the release build
    python tools/launch/bundle.py --out dist/ --binary PATH   # ... from this binary (CI names its own)
    python tools/launch/bundle.py --check dist/               # verify a bundle
    python tools/launch/bundle.py --check dist/ --smoke       # ... and run its `cicada app --no-browser`

`--out DIR` produces the folder a user double-clicks, from an EXISTING
release build (`cargo build --release -p cicada-cli --features embed` -- this
script builds nothing):

  Windows                         macOS
  DIR/                            DIR/
    cicada.exe                      Cicada.app/Contents/Info.plist
    TK*.dll ...  (the closure)        Cicada.app/Contents/MacOS/Cicada.command   (the launcher)
    .cicada-occt-bundle.json        Cicada.app/Contents/MacOS/cicada          (the binary)
    Cicada.cmd                      Cicada.app/Contents/MacOS/lib/*.dylib      (the closure)
    README.txt                      Cicada.app/Contents/MacOS/.cicada-occt-bundle.json
    .cicada-bundle.json             README.txt
                                    .cicada-bundle.json

The libraries and the macOS rpath come from `fetch_occt.bundle` (L2): the
binary starts with NO loader path in its environment. On macOS everything
lives INSIDE `Cicada.app` because Gatekeeper's app translocation runs a
downloaded app from a random read-only copy, and anything the bundle
referenced outside itself would not be there; the launcher script is named
`Cicada.command`, never `Cicada`, because the binary is `cicada` and the
default file system is case-insensitive. `Cicada.cmd` / the `.command`
run `cicada app` with the arguments they were given (none when
double-clicked) and keep their window open on a non-zero exit. The README
states what the bundle does NOT carry: Python 3 (the engine's script host
finds `CICADA_PYTHON`, else `python`, else `python3` on PATH -- started at
launch whether or not the pipeline has script nodes), the VC++ runtime on
Windows, and a first right-click -> Open on macOS for an app Apple has not
notarized.

Idempotent: a second `--out` over the same build changes nothing (the binary
is copied again only when the SOURCE binary changed -- size or mtime --
recorded in `.cicada-bundle.json`, a pure function of the inputs; launcher,
plist and README are rewritten only when their bytes would change).

The binary must embed the SPA (`embeds_spa`: two lines of `web/index.html`
an `embed` build carries verbatim -- rust-embed stores the files
uncompressed, `debug-embed` on -- and no other build does): a plain
`cargo build --release` is the easy mistake, and its bundle would die at
the first double-click with `cicada app has nothing to open`. `--out`
refuses such a binary BEFORE copying anything, unless `--allow-no-spa`
asks for an engine-only bundle (CI's debug binaries): then the README says
ENGINE ONLY and what still works (run, serve, mcp), and the stamp records
`"spa": false`. The README is ASCII like the launchers, so `type
README.txt` in a cp1252 console and Notepad agree.

`--check DIR` verifies a bundle and exits non-zero naming every problem:
the launcher files are present; the L2 stamp's libraries are present at
their recorded sizes; the binary's own imports resolve inside the bundle
or the OS (the static read `fetch_occt --check-closure` does); on macOS the
binary's rpath is `@executable_path/lib` and no prefix rpath remains;
`Info.plist` names the launcher as the executable; the binary agrees with
the stamp about the SPA (a binary swapped in after the bundle was made is
caught); and the binary answers `--help` from inside the bundle under a
MINIMAL environment (Windows: PATH = System32 alone; macOS: the system
PATH, no loader variable) -- the sentence the contract asks for. `--smoke`
adds the process-level proof: the bundle's `cicada app --no-browser` over a
scratch pipeline prints its URL, `/health` answers `ok` over it and `/` is
the SPA, never the server's "API only" page; then the server is stopped.
An engine-only bundle refuses the smoke up front (nothing to open). CI's
Windows and macOS jobs run `--out --allow-no-spa` + `--check` on the
binaries they built (debug, no SPA -- so not `--smoke`); the release
bundle's smoke is the launcher work's own evidence.

Exit status: 0 on success; 1 on any refusal or failed check, always through
an `error:` line, never a silent fallback.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import plistlib
import queue
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

HERE = Path(__file__).resolve().parent  # <repo>/tools/launch
TOOLS = HERE.parent
REPO = TOOLS.parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import fetch_occt as fo  # noqa: E402  (needs TOOLS on sys.path first)

APP_NAME = "Cicada"
MACOS_APP = "Cicada.app"
#: The macOS launcher inside Contents/MacOS -- NOT `Cicada`: the binary there
#: is `cicada`, and APFS is case-insensitive by default.
MACOS_LAUNCHER = "Cicada.command"
WINDOWS_LAUNCHER = "Cicada.cmd"
README_NAME = "README.txt"
#: L3's own stamp at the bundle's root (L2's sits beside the binary).
STAMP_NAME = ".cicada-bundle.json"
#: How long one console line of the smoke's server may take before the
#: smoke fails -- a bound on a hang, never a wait the pass path takes.
LINE_TIMEOUT_SECONDS = 60
#: The scratch pipeline the smoke serves.
DEMO = "# cicada 1\nnums = series(count=3)\n"
#: The `--help` run must name the subcommand the launchers use.
HELP_MUST_MENTION = "app"

Run = Callable[..., "subprocess.CompletedProcess[str]"]


class BundleError(Exception):
    """A refusal; the message is printed as `error: ...`."""


# ---------------------------------------------------------------------------
# Where things go
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Places:
    """The bundle's layout for one platform."""

    out: Path
    subdir: str

    @property
    def is_windows(self) -> bool:
        return self.subdir.startswith("win-")

    @property
    def binary_dir(self) -> Path:
        return self.out if self.is_windows else self.out / MACOS_APP / "Contents" / "MacOS"

    @property
    def binary(self) -> Path:
        return self.binary_dir / ("cicada.exe" if self.is_windows else "cicada")

    @property
    def launcher(self) -> Path:
        return self.out / WINDOWS_LAUNCHER if self.is_windows else self.binary_dir / MACOS_LAUNCHER

    @property
    def plist(self) -> Path | None:
        return None if self.is_windows else self.out / MACOS_APP / "Contents" / "Info.plist"

    @property
    def readme(self) -> Path:
        return self.out / README_NAME

    @property
    def stamp(self) -> Path:
        return self.out / STAMP_NAME

    @property
    def occt_stamp(self) -> Path:
        return self.binary_dir / fo.BUNDLE_STAMP_NAME

    def required_files(self) -> list[Path]:
        files = [self.binary, self.launcher, self.readme, self.stamp, self.occt_stamp]
        if self.plist is not None:
            files.insert(2, self.plist)
        return files


def places(out: Path, subdir: str) -> Places:
    if not (subdir.startswith("win-") or subdir.startswith("osx-")):
        raise BundleError(
            f"the bundle supports win-64 and osx-* binaries; {subdir} is not one of them "
            "(a Linux bundle is an rpath set at link time, `$ORIGIN/lib` -- not written here)"
        )
    return Places(out=out, subdir=subdir)


def detect_places(out: Path) -> Places:
    """Which platform's bundle `out` holds, from what is there."""
    if (out / "cicada.exe").is_file():
        return Places(out=out, subdir="win-64")
    if (out / MACOS_APP).is_dir():
        stamp = read_json(Places(out=out, subdir="osx-arm64").occt_stamp)
        subdir = stamp.get("subdir") if stamp else None
        return Places(out=out, subdir=subdir if isinstance(subdir, str) and subdir.startswith("osx-") else "osx-arm64")
    raise BundleError(f"{out} is not a bundle: neither cicada.exe nor {MACOS_APP} is there")


def read_json(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError):
        return {}


#: Two lines of the SPA's `index.html` (web/index.html, kept by Vite's build)
#: that an `embed` build carries verbatim and no other build does: the
#: server's own "API only" page has another title and no mount point.
SPA_MARKERS = (b"<title>Cicada</title>", b'<div id="root"></div>')


def embeds_spa(binary: Path) -> bool:
    """Whether `binary` carries the SPA -- every marker present in its bytes."""
    data = binary.read_bytes()
    return all(marker in data for marker in SPA_MARKERS)


# ---------------------------------------------------------------------------
# The files the bundle writes (pure)
# ---------------------------------------------------------------------------


def windows_launcher_text() -> str:
    """`Cicada.cmd`: runs the bundled `cicada app` with the window's
    arguments; the window stays open on a non-zero exit. CRLF -- cmd.exe
    misreads labels and multi-line blocks in LF-only batch files."""
    lines = [
        "@echo off",
        "setlocal",
        f"title {APP_NAME}",
        "rem Cicada -- double-click to start. The engine serves on localhost and",
        "rem opens the app window (Edge or Chrome in app mode, else the default",
        "rem browser); THIS window is the server console: Ctrl-C or closing it stops",
        "rem the server. Arguments go to `cicada app` (a project folder or a .cic",
        "rem file). Python 3 must be on PATH (or CICADA_PYTHON set) -- see README.txt.",
        'cd /d "%~dp0"',
        '"%~dp0cicada.exe" app %*',
        'set "CODE=%ERRORLEVEL%"',
        'if not "%CODE%"=="0" (',
        "  echo.",
        "  echo Cicada stopped with exit code %CODE% -- see the messages above.",
        "  pause",
        ")",
        "exit /b %CODE%",
    ]
    return "\r\n".join(lines) + "\r\n"


def macos_launcher_text() -> str:
    """`Cicada.app/Contents/MacOS/Cicada.command`: Finder starts it without a
    terminal, so without a tty it re-opens itself in Terminal (the server
    console) and, in Terminal, runs the bundled `cicada app`. bash 3.2 (the
    system's) and no GNU-only flags."""
    lines = [
        "#!/bin/bash",
        "# Cicada -- the app bundle's launcher. Finder runs this without a terminal,",
        "# so it re-opens itself in Terminal (the server console); in Terminal it",
        "# starts `cicada app`, which serves on localhost and opens the app window",
        "# (Chrome or Edge in app mode, else the default browser). Ctrl-C or closing",
        "# the window stops the server. Arguments go to `cicada app` (a project",
        "# folder or a .cic file). Python 3 must be on PATH (or CICADA_PYTHON set),",
        "# see README.txt.",
        'here="$(cd "$(dirname "$0")" && pwd)"',
        "if [ ! -t 1 ]; then",
        '  exec open -a Terminal "$0"',
        "fi",
        'cd "$here" || exit 1',
        '"$here/cicada" app "$@"',
        "code=$?",
        'if [ "$code" -ne 0 ]; then',
        "  echo",
        '  echo "Cicada stopped with exit code $code -- see the messages above."',
        '  read -r -p "Press Return to close this window. "',
        "fi",
        'exit "$code"',
    ]
    return "\n".join(lines) + "\n"


def info_plist_text(version: str) -> str:
    """A minimal `Info.plist`: Finder needs the executable's name and the
    package type; the rest names the app."""
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": APP_NAME,
        "CFBundleExecutable": MACOS_LAUNCHER,
        "CFBundleIdentifier": "cicada.Cicada",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": APP_NAME,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
    }
    return plistlib.dumps(info, sort_keys=True).decode("utf-8")


def readme_text(subdir: str, version: str, commit: str | None, spa: bool = True) -> str:
    """`README.txt` -- ASCII only, like the launchers: `type README.txt` in a
    cp1252 console and Notepad must agree. `spa=False` is the engine-only
    bundle (`--allow-no-spa`), whose launcher would stop at `cicada app`."""
    windows = subdir.startswith("win-")
    launcher = WINDOWS_LAUNCHER if windows else MACOS_APP
    binary = "cicada.exe" if windows else f"{MACOS_APP}/Contents/MacOS/cicada"
    lines = [
        f"Cicada {version} -- {subdir}" + (f", commit {commit}" if commit else ""),
        "",
        "WORK IN PROGRESS. Cicada is pre-release software under daily development;",
        "nothing is stable, there is no support, and the file formats change without",
        "notice. This folder is a development build for trying it, not a release.",
        "",
    ]
    if spa:
        lines += [
            "Run it",
            f"  Double-click {launcher}. A terminal window opens -- it is the server",
            "  console -- and the app window follows in Edge or Chrome (app mode) when",
            "  one is installed, else in your default browser. Close the console window",
            "  or press Ctrl-C in it to stop the server.",
            "",
            f"  From a terminal: {binary} app [FOLDER-or-FILE.cic]  -- the same thing;",
            f"  {binary} --help lists everything else (run, mcp, catalog, serve).",
            "",
            "What this folder carries",
            "  The cicada engine with the app built in, and the geometry kernel's run-time",
            "  libraries beside it -- it needs no environment variable and no install.",
        ]
    else:
        lines += [
            "ENGINE ONLY",
            f"  This build embeds no app: double-clicking {launcher} stops with",
            "  `cicada app has nothing to open`. Use the engine from a terminal --",
            f"  {binary} run FILE.cic, {binary} serve (API only), {binary} mcp;",
            f"  {binary} --help lists everything -- or make the bundle from a build with",
            "  the app embedded (cd web && npm run build, then",
            "  cargo build --release -p cicada-cli --features embed).",
            "",
            "What this folder carries",
            "  The cicada engine WITHOUT the app, and the geometry kernel's run-time",
            "  libraries beside it -- it needs no environment variable and no install.",
        ]
    lines += [
        "",
        "What it needs from your machine",
        "  Python 3 on PATH (or CICADA_PYTHON naming the interpreter): the engine's",
        "  script host starts at launch whether or not a pipeline has script nodes.",
    ]
    if windows:
        lines += [
            "  The Microsoft Visual C++ 2015-2022 x64 runtime (msvcp140.dll, vcruntime140.dll);",
            "  most machines have it, Microsoft's vc_redist.x64.exe installs it.",
        ]
    else:
        lines += [
            "  The first time, macOS will refuse an app Apple has not notarized: right-click",
            f"  {MACOS_APP} -> Open (or System Settings -> Privacy & Security -> Open Anyway).",
            "  The binary is signed ad hoc; nothing in this folder phones home.",
        ]
    lines += [
        "  Optional: Microsoft Edge or Google Chrome for the dedicated app window.",
        "",
        "Where things go",
        "  The engine's cache lives in your user cache directory, never beside your",
        "  files. The app WRITES the project it serves (the .cic file and a layout",
        "  sidecar) as you edit -- open a copy if you only want to look.",
        "",
        "The source, its documentation and the design ledger are in the Cicada",
        "repository (README.md, AGENTS.md, docs/).",
    ]
    text = "\n".join(lines) + "\n"
    assert all(ord(c) < 128 for c in text), "README.txt is ASCII (module docstring)"
    return text


def write_if_changed(path: Path, text: str, executable: bool = False) -> bool:
    """Write `text` (UTF-8, bytes exactly as given) unless the file already
    holds it; set the executable bits when asked. Returns True when the file
    changed."""
    data = text.encode("utf-8")
    try:
        current = path.read_bytes()
    except OSError:
        current = None
    changed = current != data
    if changed:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    if executable and os.name != "nt":
        mode = path.stat().st_mode
        if mode & 0o111 != 0o111:
            path.chmod(mode | 0o755)
    return changed


# ---------------------------------------------------------------------------
# Environments and process helpers
# ---------------------------------------------------------------------------


def clean_environment(system: str, environ: dict[str, str]) -> dict[str, str]:
    """The MINIMAL environment a bundled binary is run under to prove it
    needs none: no loader variable, no `DEP_OCCT_ROOT`; Windows keeps only
    what the OS itself needs (SystemRoot and a PATH of System32 alone -- the
    VC++ runtime lives there); macOS / Linux the system PATH and HOME."""
    if system == "windows":
        system_root = environ.get("SystemRoot") or environ.get("SYSTEMROOT") or r"C:\Windows"
        env = {
            "SystemRoot": system_root,
            "windir": system_root,
            "PATH": f"{system_root}\\System32;{system_root}",
            "SystemDrive": environ.get("SystemDrive", system_root[:2]),
            "TEMP": environ.get("TEMP", system_root + "\\Temp"),
            "TMP": environ.get("TMP", environ.get("TEMP", system_root + "\\Temp")),
        }
        for key in ("USERPROFILE", "LOCALAPPDATA", "APPDATA", "HOMEDRIVE", "HOMEPATH"):
            if key in environ:
                env[key] = environ[key]
        return env
    env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"}
    for key in ("HOME", "TMPDIR", "USER", "LANG"):
        if key in environ:
            env[key] = environ[key]
    return env


def run_capture(argv: list[str], **kwargs) -> subprocess.CompletedProcess[str]:
    """Run a program, capturing text output; `FileNotFoundError` becomes a
    failed result so callers have ONE shape to look at."""
    try:
        return subprocess.run(argv, capture_output=True, text=True, check=False, **kwargs)
    except FileNotFoundError as error:
        return subprocess.CompletedProcess(argv, 127, "", str(error))


def binary_version(binary: Path, env: dict[str, str], run: Run) -> str:
    """`cicada --version` -> the version word; refused when the binary does
    not answer (a bundled binary that cannot start is the first thing this
    script must say)."""
    completed = run([str(binary), "--version"], cwd=str(binary.parent), env=env, timeout=60)
    if completed.returncode != 0:
        raise BundleError(
            f"{binary.name} --version exited {completed.returncode} from inside the bundle under a clean environment: "
            f"{(completed.stderr or completed.stdout).strip() or 'no output'}"
        )
    words = completed.stdout.split()
    if len(words) < 2 or words[0] != "cicada":
        raise BundleError(f"{binary.name} --version answered {completed.stdout.strip()!r}, not `cicada <version>`")
    return words[1]


def git_commit(repo: Path, run: Run) -> str | None:
    """The repository's HEAD (short), or None when this is not a checkout."""
    completed = run(["git", "-C", str(repo), "rev-parse", "--short", "HEAD"], timeout=60)
    if completed.returncode != 0:
        return None
    word = completed.stdout.strip()
    return word or None


def parse_url_line(line: str) -> str | None:
    """`cicada app -- http://127.0.0.1:PORT/?token=...` -> the URL; None for any
    other console line."""
    head, separator, url = line.strip().partition(" — ")
    if not separator or not head.startswith("cicada ") or not url.startswith("http://"):
        return None
    return url


def default_binary(target_dir: Path, system: str) -> Path:
    return target_dir / "release" / ("cicada.exe" if system == "windows" else "cicada")


def cargo_target_dir(run: Run, repo: Path, environ: dict[str, str]) -> Path:
    completed = run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=str(repo), env=environ, timeout=120)
    if completed.returncode != 0:
        raise BundleError(f"cargo metadata failed ({completed.returncode}): {completed.stderr.strip()}")
    try:
        return Path(json.loads(completed.stdout)["target_directory"])
    except (ValueError, KeyError, TypeError) as error:
        raise BundleError(f"cargo metadata did not report a target_directory: {error}") from error


# ---------------------------------------------------------------------------
# Make
# ---------------------------------------------------------------------------


def source_record(binary: Path) -> dict:
    stat = binary.stat()
    return {"size": stat.st_size, "mtime_ns": stat.st_mtime_ns}


def make_bundle(
    binary: Path,
    out: Path,
    layout: fo.Layout,
    manifest: dict,
    log: Callable[[str], None],
    environ: dict[str, str] | None = None,
    run: Run = run_capture,
    which=shutil.which,
    commit: str | None = None,
    allow_no_spa: bool = False,
) -> Places:
    """Produce (or refresh) the bundle in `out` from `binary`; refused, before
    anything is written, when the binary embeds no SPA and `allow_no_spa` is
    not set (module docstring)."""
    environ = dict(os.environ if environ is None else environ)
    system = "windows" if layout.is_windows else "darwin"
    if not binary.is_file():
        raise BundleError(
            f"no release binary at {binary}: build it first (cargo build --release -p cicada-cli --features embed) "
            "or name one with --binary"
        )
    spots = places(out, layout.subdir)
    spa = embeds_spa(binary)
    if not spa and not allow_no_spa:
        raise BundleError(
            f"{binary} embeds no SPA, so `cicada app` -- what {spots.launcher.name} runs -- would refuse to open. "
            "Build it with `cd web && npm run build` then `cargo build --release -p cicada-cli --features embed`, "
            "or pass --allow-no-spa for an engine-only bundle (run, serve, mcp; its README says so)"
        )
    previous = read_json(spots.stamp) or {}
    source = source_record(binary)
    spots.binary_dir.mkdir(parents=True, exist_ok=True)
    copied = False
    if not spots.binary.is_file() or previous.get("binary_source") != source or spots.binary.stat().st_size != source["size"]:
        shutil.copy2(binary, spots.binary)
        copied = True
        log(f"copied {binary} -> {spots.binary}")
    else:
        log(f"{spots.binary.name} is the current build (source unchanged)")
    if layout.is_macos:
        log(f"rpaths before: {fo.macho_rpaths(spots.binary.read_bytes())}")
    try:
        fo.bundle(manifest, layout, spots.binary_dir, log, run=lambda argv: _run_tool(argv, run), which=which)
    except fo.FetchError as error:
        raise BundleError(str(error)) from error
    if layout.is_macos:
        log(f"rpaths after: {fo.macho_rpaths(spots.binary.read_bytes())}")
    version = binary_version(spots.binary, clean_environment(system, environ), run)
    changed = [copied]
    changed.append(write_if_changed(spots.launcher, windows_launcher_text() if spots.is_windows else macos_launcher_text(), executable=True))
    if spots.plist is not None:
        changed.append(write_if_changed(spots.plist, info_plist_text(version)))
    changed.append(write_if_changed(spots.readme, readme_text(layout.subdir, version, commit, spa)))
    stamp = {"binary_source": source, "commit": commit, "spa": spa, "subdir": layout.subdir, "version": version}
    changed.append(write_if_changed(spots.stamp, json.dumps(stamp, indent=1, sort_keys=True) + "\n"))
    log(
        f"bundle {out}: cicada {version}{'' if spa else ' (engine only -- no SPA)'}, {spots.launcher.relative_to(out)}, {README_NAME}"
        + ("" if any(changed) else " -- unchanged")
    )
    return spots


def _run_tool(argv: list[str], run: Run) -> None:
    """`fetch_occt.run_tool`'s contract over an injectable runner."""
    completed = run(argv, timeout=300)
    if completed.returncode != 0:
        raise fo.FetchError(f"{' '.join(argv)} failed ({completed.returncode}): {(completed.stderr or '').strip()}")


# ---------------------------------------------------------------------------
# Check
# ---------------------------------------------------------------------------


def check_bundle(out: Path, log: Callable[[str], None], environ: dict[str, str] | None = None, run: Run = run_capture) -> list[str]:
    """Every problem with the bundle in `out` (empty = OK); see the module
    docstring for the list."""
    environ = dict(os.environ if environ is None else environ)
    spots = detect_places(out)
    system = "windows" if spots.is_windows else "darwin"
    problems: list[str] = []
    for path in spots.required_files():
        if not path.is_file():
            problems.append(f"missing {path.relative_to(out)}")
    if problems:
        return problems
    stamp = read_json(spots.occt_stamp) or {}
    libraries = stamp.get("libraries")
    if not isinstance(libraries, dict) or not libraries:
        return [f"{spots.occt_stamp.relative_to(out)} records no libraries"]
    layout = fo.Layout(Path("unused"), spots.subdir)
    plan = fo.BundlePlan(layout, spots.binary_dir)
    for name, size in sorted(libraries.items()):
        library = plan.library_dir / name
        if not library.is_file():
            problems.append(f"library missing: {library.relative_to(out)}")
        elif library.stat().st_size != size:
            problems.append(f"library size differs: {library.relative_to(out)} is {library.stat().st_size} bytes, the stamp says {size}")
    if problems:
        return problems
    log(f"{len(libraries)} libraries present at their recorded sizes")
    try:
        unresolved = fo.bundle_unresolved_imports(plan, libraries)
    except fo.FetchError as error:
        return [f"{spots.binary.relative_to(out)}: {error}"]
    if unresolved:
        problems.append(f"{spots.binary.name} imports {', '.join(unresolved)}, which neither the bundle nor the OS provides")
    if not spots.is_windows:
        rpaths = fo.macho_rpaths(spots.binary.read_bytes())
        if fo.BUNDLE_RPATH not in rpaths:
            problems.append(f"{spots.binary.name} carries no {fo.BUNDLE_RPATH} rpath (rpaths: {rpaths})")
        for rpath in rpaths:
            if fo.is_prefix_rpath(rpath, layout):
                problems.append(f"{spots.binary.name} still carries the build prefix's rpath {rpath}")
    if problems:
        return problems
    log(f"{spots.binary.name}: every import resolves inside the bundle or the OS")
    # The binary and the bundle's own stamp agree about the SPA: a plain
    # release build swapped in after the bundle was made would pass every
    # check above and die at the first double-click.
    recorded = (read_json(spots.stamp) or {}).get("spa")
    if not isinstance(recorded, bool):
        return [f"{spots.stamp.relative_to(out)} does not record whether the SPA is embedded -- remake the bundle (bundle.py --out)"]
    actual = embeds_spa(spots.binary)
    if actual != recorded:
        return [
            f"{spots.binary.name} {'embeds the' if actual else 'embeds no'} SPA but {STAMP_NAME} says it "
            f"{'does' if recorded else 'does not'} -- not the binary this bundle was made from; remake the bundle (bundle.py --out)"
        ]
    log(f"{spots.binary.name} {'embeds the SPA' if actual else 'embeds no SPA (engine only, as the bundle records)'}")
    if spots.plist is not None:
        try:
            info = plistlib.loads(spots.plist.read_bytes())
        except Exception as error:  # noqa: BLE001 -- any malformed plist is the same finding
            return [f"{spots.plist.relative_to(out)} does not parse: {error}"]
        if info.get("CFBundleExecutable") != MACOS_LAUNCHER:
            problems.append(f"Info.plist names {info.get('CFBundleExecutable')!r} as the executable, not {MACOS_LAUNCHER}")
        if os.name != "nt" and not os.access(spots.launcher, os.X_OK):
            problems.append(f"{spots.launcher.relative_to(out)} is not executable")
    completed = run([str(spots.binary), "--help"], cwd=str(spots.binary_dir), env=clean_environment(system, environ), timeout=60)
    if completed.returncode != 0:
        problems.append(
            f"{spots.binary.name} --help exited {completed.returncode} under a clean environment: "
            f"{(completed.stderr or completed.stdout).strip() or 'no output'}"
        )
    elif HELP_MUST_MENTION not in completed.stdout.split():
        problems.append(f"{spots.binary.name} --help does not list `{HELP_MUST_MENTION}`:\n{completed.stdout}")
    else:
        log(f"{spots.binary.name} --help answers from inside the bundle with no loader path")
    return problems


# ---------------------------------------------------------------------------
# Smoke
# ---------------------------------------------------------------------------


class Console:
    """A child's stdout, line by line, through a bounded wait."""

    def __init__(self, process):
        self.process = process
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.stderr: list[str] = []
        self.readers = [threading.Thread(target=self._pump, daemon=True), threading.Thread(target=self._drain, daemon=True)]
        for reader in self.readers:
            reader.start()

    def _pump(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line.rstrip("\r\n"))
        self.lines.put(None)

    def _drain(self) -> None:
        assert self.process.stderr is not None
        self.stderr.append(self.process.stderr.read())

    def line(self, expected: str) -> str:
        try:
            line = self.lines.get(timeout=LINE_TIMEOUT_SECONDS)
        except queue.Empty:
            self.stop()
            raise BundleError(f"smoke: no {expected} within {LINE_TIMEOUT_SECONDS} s; stderr:\n{''.join(self.stderr)}") from None
        if line is None:
            self.stop()
            raise BundleError(
                f"smoke: the server exited ({self.process.returncode}) before {expected}; stderr:\n{''.join(self.stderr)}"
            )
        return line

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=LINE_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        # The pipes hit EOF once the process is gone: wait for the readers so
        # an error message built after `stop` carries the WHOLE stderr.
        for reader in self.readers:
            reader.join(timeout=LINE_TIMEOUT_SECONDS)


def http_get(url: str) -> tuple[int, str]:
    try:
        with urllib.request.urlopen(url, timeout=30) as response:  # noqa: S310 -- our own loopback server
            return response.status, response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", errors="replace")


def smoke(
    out: Path,
    log: Callable[[str], None],
    environ: dict[str, str] | None = None,
    python: str | None = None,
    start: Callable[..., object] = subprocess.Popen,
    get: Callable[[str], tuple[int, str]] = http_get,
) -> str:
    """Start the bundle's `cicada app --no-browser` over a scratch pipeline,
    read its URL, prove `/health` and `/` over it, stop it. Returns the URL.
    `start` (a `Popen`) and `get` (an HTTP GET) are injectable so the
    assertions have an offline test; refused up front for an engine-only
    bundle, whose `cicada app` has nothing to open."""
    environ = dict(os.environ if environ is None else environ)
    spots = detect_places(out)
    recorded = (read_json(spots.stamp) or {}).get("spa")
    if recorded is not True:
        raise BundleError(
            f"smoke: {out} is an engine-only bundle (its {STAMP_NAME} "
            f"{'says the SPA is not embedded' if recorded is False else 'does not record the SPA'}): "
            "`cicada app` has nothing to open; make the bundle from a `--features embed` build"
        )
    system = "windows" if spots.is_windows else "darwin"
    env = clean_environment(system, environ)
    # The engine's script host needs an interpreter: the one running this.
    env["CICADA_PYTHON"] = python or sys.executable
    token = secrets.token_hex(8)
    with tempfile.TemporaryDirectory(prefix="cicada-smoke-") as scratch_name:
        scratch = Path(scratch_name)
        (scratch / "demo.cic").write_text(DEMO, encoding="utf-8")
        argv = [
            str(spots.binary),
            "app",
            "--no-browser",
            "--port",
            "0",
            "--token",
            token,
            "--threads",
            "2",
            "--cache-dir",
            str(scratch / "cache"),
            str(scratch / "demo.cic"),
        ]
        log("smoke: " + " ".join(argv[1:]))
        process = start(
            argv,
            cwd=str(spots.binary_dir),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        console = Console(process)
        try:
            url = None
            while url is None:
                url = parse_url_line(console.line("the URL line"))
            log(f"smoke: {url}")
            base = url.split("/?", 1)[0]
            status, body = get(f"{base}/health?token={token}")
            if status != 200 or body.strip() != "ok":
                raise BundleError(f"smoke: GET /health answered {status} {body.strip()!r}, not 200 ok")
            log("smoke: /health -> ok")
            status, body = get(f"{base}/?token={token}")
            if status != 200 or "API only" in body or "<html" not in body.lower():
                raise BundleError(f"smoke: GET / answered {status} and is not the SPA:\n{body[:400]}")
            log("smoke: / is the SPA (never the API-only page)")
        finally:
            console.stop()
            log("smoke: server stopped")
        return url


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str], environ: dict[str, str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--out", type=Path, metavar="DIR", help="make (or refresh) the bundle in DIR")
    mode.add_argument("--check", type=Path, metavar="DIR", help="verify the bundle in DIR")
    parser.add_argument("--binary", type=Path, help="the built cicada binary (default: the release build in cargo's target dir)")
    parser.add_argument("--cache-root", type=Path, help="fetch_occt.py's --dest (default: the user cache dir)")
    parser.add_argument("--smoke", action="store_true", help="with --check: run the bundle's `cicada app --no-browser` and read /health")
    parser.add_argument(
        "--allow-no-spa",
        action="store_true",
        help="with --out: bundle a binary that embeds no SPA -- engine only (run, serve, mcp; `cicada app` refuses); CI's debug binaries",
    )
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)
    environ = dict(os.environ if environ is None else environ)

    def log(message: str) -> None:
        if not args.quiet:
            print(message, file=sys.stderr, flush=True)

    try:
        if args.check is not None:
            if args.allow_no_spa:
                raise BundleError("--allow-no-spa goes with --out")
            problems = check_bundle(args.check, log, environ)
            if problems:
                raise BundleError(f"bundle {args.check} fails its check:\n  " + "\n  ".join(problems))
            if args.smoke:
                smoke(args.check, log, environ)
            print(f"bundle {args.check}: OK" + (" (smoke passed)" if args.smoke else ""))
            return 0
        if args.smoke:
            raise BundleError("--smoke goes with --check")
        system = platform.system().lower()
        manifest = fo.load_manifest()
        layout = fo.Layout(args.cache_root or fo.default_cache_root(environ), fo.detect_subdir(), manifest["occt_version"])
        binary = args.binary if args.binary is not None else default_binary(cargo_target_dir(run_capture, REPO, environ), system)
        fo.fetch(manifest, layout, log)
        spots = make_bundle(
            binary, args.out, layout, manifest, log, environ, commit=git_commit(REPO, run_capture), allow_no_spa=args.allow_no_spa
        )
        print(f"bundle {args.out}: {spots.launcher.relative_to(args.out)} runs `cicada app`; `bundle.py --check {args.out}` verifies it")
        return 0
    except (BundleError, fo.FetchError) as error:
        print(f"error: {error}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
