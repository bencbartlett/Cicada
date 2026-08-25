#!/usr/bin/env python3
"""The dev launcher's core (docs/17 wave 4, L3): build if needed, bundle, run.

`tools/launch/Cicada.cmd` (Windows) and `tools/launch/Cicada.command`
(macOS) are the double-clickable faces of this script: each opens a visible
terminal, finds Python 3 and runs this file, which

  1. builds `cicada` in RELEASE with the SPA embedded when it is missing or
     stale -- `npm run build` (after `npm ci` when `web/node_modules` is
     missing) and `cargo build --release -p cicada-cli --features embed`,
     with the OCCT env and cmake found the way AGENTS.md says (the
     `tools/fetch_occt.py` prefix -- fetched on first use -- and the VS Build
     Tools / Homebrew cmake when none is on PATH);
  2. bundles the kernel's run-time libraries beside the binary
     (`fetch_occt.bundle`, L2) so it needs no loader path;
  3. runs `cicada app` -- with no arguments, or with whatever arguments the
     launcher itself was given (`Cicada.cmd examples/02-solids.cic`).

Every failure is printed with the step it came from and a non-zero exit, and
the wrappers keep the window open on one. This file is the Windows AND the
macOS launcher: OS differences are data (the cmake directories, the binary's
name, the env the build needs per OS from `fetch_occt.github_env_entries`),
never a separate script per OS.

What "stale" means (`decide`): the SPA is rebuilt when `web/dist` is missing
or any web source is newer than it; the engine is rebuilt when the release
binary is missing, when the launcher's stamp beside it does not match the
binary (so a binary somebody else built -- maybe without the SPA -- is never
trusted), when any engine source is newer than the binary, or when the SPA
was rebuilt. Cargo and Vite are incremental on top, so a forced rebuild on a
warm tree costs seconds. After a successful build the binary's mtime is
touched BEFORE it is stamped (`mark_built`): the rule watches files cargo
does not consider build inputs (`Cargo.lock` after a checkout, tests, docs
under `crates/`), and cargo leaves an up-to-date binary untouched -- without
the touch such a file would make every later launch "stale" and run a no-op
cargo build for good.

Where `cicada app` runs (`app_cwd`): with arguments, in the directory the
launcher was started from -- a relative path means what it would mean had
you typed `cicada app` there; with none (a double-click), in the repository,
so that on a branch where a path-less `cicada app` serves the current
directory it serves the repository, never `tools/launch/` (with O1's home
root the no-argument case is cwd-independent either way).

The build's git on Windows gets `core.longpaths=true` through
`GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_0`/`GIT_CONFIG_VALUE_0` (appended after
any entries the caller already set): the kernel build clones oneTBB, whose
doc assets exceed MAX_PATH, and the clone fails deterministically without it
on a machine that lacks the global setting (AGENTS.md, Dev machine notes).

The engine's Python: `cicada app` starts the script host at launch whether
or not the pipeline has script nodes, and finds the interpreter through
`CICADA_PYTHON`, else `python`, else `python3` on PATH. The launcher hands
the engine the interpreter running this script as `CICADA_PYTHON`, so the
Python the launcher found is the Python the engine uses.

    python tools/launch/launch.py                 # build if stale, bundle, run `cicada app`
    python tools/launch/launch.py --plan          # print what it WOULD do, do nothing
    python tools/launch/launch.py --no-run        # build + bundle, do not start the server
    python tools/launch/launch.py path/to/x.cic   # everything else goes to `cicada app`

Exit status: 0 when `cicada app` exited 0 (Ctrl-C included); 1 on any
failure of the launcher's own steps (a missing tool, a failed build, a
refused bundle) -- always through an `error:` line, never a silent fallback.
"""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable

HERE = Path(__file__).resolve().parent  # <repo>/tools/launch
TOOLS = HERE.parent
REPO = TOOLS.parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import fetch_manifold as fm  # noqa: E402  (needs TOOLS on sys.path first)
import fetch_occt as fo  # noqa: E402

#: The launcher's stamp beside the release binary: the build it made.
STAMP_NAME = "cicada.launch-stamp.json"
#: The cargo features the launcher builds with -- the shipped shape.
FEATURES = ("embed",)
PROFILE = "release"
#: Sources whose change makes the release binary stale (repo-relative).
ENGINE_SOURCES = ("Cargo.toml", "Cargo.lock", "crates")
#: Sources whose change makes `web/dist` stale (repo-relative).
WEB_SOURCES = (
    "web/index.html",
    "web/package.json",
    "web/package-lock.json",
    "web/vite.config.ts",
    "web/tsconfig.json",
    "web/tsconfig.node.json",
    "web/src",
    "web/public",
)
#: Never walked when looking for the newest source.
SKIP_DIRS = frozenset({"node_modules", "target", "__pycache__", ".git", "dist", "test-results", "playwright-report"})
#: The launcher's own flags; everything else is `cicada app`'s.
LAUNCHER_FLAGS = ("--plan", "--no-run", "--launcher-help")

#: Where cmake lives when it is not on PATH -- data the one code path reads.
#: Windows entries are (environment variable, path under it).
WINDOWS_CMAKE_DIRS = (
    ("ProgramFiles(x86)", r"Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"),
    ("ProgramFiles", r"Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"),
    ("ProgramFiles", r"Microsoft Visual Studio\2022\Professional\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"),
    ("ProgramFiles", r"Microsoft Visual Studio\2022\Enterprise\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"),
    ("ProgramFiles", r"CMake\bin"),
)
MACOS_CMAKE_DIRS = ("/opt/homebrew/bin", "/usr/local/bin", "/Applications/CMake.app/Contents/bin")

#: What to install when a tool is missing, per tool.
TOOL_HINTS = {
    "npm": "Node.js 20+ (https://nodejs.org) -- `npm` comes with it",
    "cargo": "Rust stable (https://rustup.rs)",
    "cmake": "cmake -- on Windows the VS Build Tools' copy (AGENTS.md, Dev machine notes) or Kitware's installer; "
    "on macOS `brew install cmake` or CMake.app; it must be on PATH or in one of the usual places",
}


class LaunchError(Exception):
    """A launcher step that failed; the message is printed as `error: ...`."""


# ---------------------------------------------------------------------------
# Pure parts: the state of the tree, the plan, the environments
# ---------------------------------------------------------------------------


def host_system(system: str | None = None) -> str:
    """`windows`, `darwin` or `linux` -- `platform.system()` lower-cased."""
    return (system or platform.system()).lower()


def binary_name(system: str) -> str:
    return "cicada.exe" if system == "windows" else "cicada"


def cargo_target_dir(metadata_json: str) -> Path:
    """`cargo metadata`'s `target_directory` -- cargo's own answer, which
    honours CARGO_TARGET_DIR and any `.cargo/config.toml`."""
    try:
        value = json.loads(metadata_json)["target_directory"]
    except (ValueError, KeyError, TypeError) as error:
        raise LaunchError(f"cargo metadata did not report a target_directory: {error}") from error
    return Path(value)


def newest_mtime_ns(root: Path, relative: Iterable[str], skip_dirs: frozenset[str] = SKIP_DIRS) -> int:
    """The newest modification time (ns) under the given repo-relative files
    and directories; entries that do not exist are skipped; 0 when nothing
    exists. Directories named in `skip_dirs` are never entered."""
    newest = 0
    pending = [root / entry for entry in relative]
    while pending:
        path = pending.pop()
        try:
            stat = path.lstat()
        except FileNotFoundError:
            continue
        if path.is_dir() and not path.is_symlink():
            with os.scandir(path) as entries:
                for entry in entries:
                    if entry.is_dir(follow_symlinks=False):
                        if entry.name not in skip_dirs:
                            pending.append(Path(entry.path))
                    else:
                        newest = max(newest, entry.stat(follow_symlinks=False).st_mtime_ns)
        else:
            newest = max(newest, stat.st_mtime_ns)
    return newest


def stamp_for(binary: Path) -> dict:
    """The launcher's stamp: which build this binary is, pinned to the
    binary's size and mtime so a binary rebuilt by anything else (maybe
    without the SPA) invalidates it."""
    stat = binary.stat()
    return {
        "profile": PROFILE,
        "features": list(FEATURES),
        "size": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
    }


def stamp_valid(stamp: dict | None, binary: Path) -> bool:
    """True when `stamp` describes exactly this binary, built the way the
    launcher builds (release, the SPA embedded)."""
    if not stamp or not binary.is_file():
        return False
    stat = binary.stat()
    return (
        stamp.get("profile") == PROFILE
        and stamp.get("features") == list(FEATURES)
        and stamp.get("size") == stat.st_size
        and stamp.get("mtime_ns") == stat.st_mtime_ns
    )


def read_stamp(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError):
        return {}


def mark_built(binary: Path, release_dir: Path) -> dict:
    """After a successful `cargo build`: touch the binary, then stamp it.

    The touch is what keeps a no-op build from recurring (module docstring):
    cargo rewrites the binary only when an input changed, and the staleness
    rule compares it against files cargo does not call inputs. Touched first,
    the stamp pins the new mtime."""
    os.utime(binary, None)
    stamp = stamp_for(binary)
    (release_dir / STAMP_NAME).write_text(json.dumps(stamp, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return stamp


@dataclass(frozen=True)
class State:
    """What the tree looks like -- the inputs of [`decide`]."""

    binary_exists: bool
    binary_mtime_ns: int
    stamp_valid: bool
    node_modules: bool
    dist_exists: bool
    dist_mtime_ns: int
    web_newest_ns: int
    engine_newest_ns: int


@dataclass(frozen=True)
class Plan:
    """What to do, and why -- in the order the steps run."""

    install: bool
    build_spa: bool
    build_engine: bool
    reasons: tuple[str, ...] = field(default_factory=tuple)

    @property
    def fresh(self) -> bool:
        return not (self.install or self.build_spa or self.build_engine)


def decide(state: State) -> Plan:
    """The staleness rule (module docstring); a pure function of the tree."""
    reasons: list[str] = []
    install = not state.node_modules
    if install:
        reasons.append("web/node_modules is missing (npm ci)")
    build_spa = False
    if not state.dist_exists:
        build_spa = True
        reasons.append("web/dist is missing")
    elif state.web_newest_ns > state.dist_mtime_ns:
        build_spa = True
        reasons.append("web sources are newer than web/dist")
    build_engine = False
    if not state.binary_exists:
        build_engine = True
        reasons.append(f"no {PROFILE} binary")
    elif not state.stamp_valid:
        build_engine = True
        reasons.append("the binary was not built by the launcher with the SPA embedded (no matching stamp)")
    elif state.engine_newest_ns > state.binary_mtime_ns:
        build_engine = True
        reasons.append("engine sources are newer than the binary")
    elif state.dist_mtime_ns > state.binary_mtime_ns:
        build_engine = True
        reasons.append("web/dist is newer than the binary (the SPA is embedded at build time)")
    if build_spa and not build_engine:
        build_engine = True
        reasons.append("the SPA will be rebuilt, so the binary embedding it must be too")
    return Plan(install=install, build_spa=build_spa, build_engine=build_engine, reasons=tuple(reasons))


def observe(repo: Path, release_dir: Path, system: str) -> State:
    """Read the tree into a [`State`]."""
    binary = release_dir / binary_name(system)
    dist_index = repo / "web" / "dist" / "index.html"
    binary_exists = binary.is_file()
    dist_exists = dist_index.is_file()
    return State(
        binary_exists=binary_exists,
        binary_mtime_ns=binary.stat().st_mtime_ns if binary_exists else 0,
        stamp_valid=stamp_valid(read_stamp(release_dir / STAMP_NAME), binary),
        node_modules=(repo / "web" / "node_modules").is_dir(),
        dist_exists=dist_exists,
        dist_mtime_ns=dist_index.stat().st_mtime_ns if dist_exists else 0,
        web_newest_ns=newest_mtime_ns(repo, WEB_SOURCES),
        engine_newest_ns=newest_mtime_ns(repo, ENGINE_SOURCES),
    )


def program_files(variable: str, environ: dict[str, str]) -> str:
    """`%ProgramFiles%` / `%ProgramFiles(x86)%`, or their conventional
    locations when the variable is absent -- Git Bash cannot export a name
    with parentheses, so a launcher started from one never sees the x86
    variable even though cmd.exe always sets it."""
    value = environ.get(variable)
    if value:
        return value
    drive = environ.get("SystemDrive") or "C:"
    return f"{drive}\\Program Files (x86)" if "x86" in variable else f"{drive}\\Program Files"


def cmake_candidates(system: str, environ: dict[str, str]) -> list[Path]:
    """The directories to look in for cmake after PATH, per OS (data)."""
    if system == "windows":
        return [Path(program_files(variable, environ)) / tail for variable, tail in WINDOWS_CMAKE_DIRS]
    if system == "darwin":
        return [Path(d) for d in MACOS_CMAKE_DIRS]
    return []


def find_tool(name: str, environ: dict[str, str], extra_dirs: Iterable[Path] = ()) -> Path | None:
    """`name` on the environment's PATH, else in `extra_dirs` (in order)."""
    found = shutil.which(name, path=environ.get("PATH", ""))
    if found:
        return Path(found)
    for directory in extra_dirs:
        found = shutil.which(name, path=str(directory))
        if found:
            return Path(found)
    return None


def build_environment(
    base: dict[str, str], layout: fo.Layout, system: str, cmake_dir: Path | None, manifold: fm.Layout | None = None
) -> dict[str, str]:
    """The env the BUILD runs in: `fetch_occt.github_env_entries`'s answer
    for this OS (Windows: the library dir on PATH; macOS: an rpath in
    RUSTFLAGS and NEVER a loader variable -- conda's libiconv ahead of the
    system's made cargo segfault on CI; Linux: LD_LIBRARY_PATH) plus
    `DEP_OCCT_ROOT`, the cmake policy floor and cmake's directory on PATH —
    and, when a warm prebuilt-Manifold layout is given, `MANIFOLD_CSG_LIB_DIR`
    / `MANIFOLD_CSG_LIB_KIND` so cargo links it instead of compiling the
    kernel with cmake inside the target dir (AGENTS.md iteration-speed
    rule 6)."""
    env = dict(base)
    entries, path_entries = fo.github_env_entries(layout, base.get(layout.loader_variable, ""), base.get("RUSTFLAGS", ""))
    if manifold is not None:
        entries = [*entries, *fm.github_env_entries(manifold)]
    for line in entries:
        key, _, value = line.partition("=")
        env[key] = value
    prefix = [str(p) for p in path_entries]
    if cmake_dir is not None:
        prefix.insert(0, str(cmake_dir))
    if prefix:
        separator = ";" if system == "windows" else ":"
        current = base.get("PATH", "")
        env["PATH"] = separator.join(prefix + ([current] if current else []))
    if system == "windows":
        add_git_config(env, GIT_LONGPATHS)
    return env


#: The git setting the Windows kernel build needs per process (module docstring).
GIT_LONGPATHS = ("core.longpaths", "true")


def add_git_config(env: dict[str, str], setting: tuple[str, str]) -> None:
    """Append one `key=value` to the `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` /
    `GIT_CONFIG_VALUE_n` block git reads from the environment, after any
    entries already there (they are kept)."""
    count_text = env.get("GIT_CONFIG_COUNT", "0")
    try:
        count = int(count_text)
    except ValueError as error:
        raise LaunchError(f"GIT_CONFIG_COUNT is {count_text!r}, not a number -- git itself would refuse it") from error
    key, value = setting
    env[f"GIT_CONFIG_KEY_{count}"] = key
    env[f"GIT_CONFIG_VALUE_{count}"] = value
    env["GIT_CONFIG_COUNT"] = str(count + 1)


def run_environment(base: dict[str, str], layout: fo.Layout, system: str, python: Path) -> dict[str, str]:
    """The env `cicada app` runs in: the launcher's own, with the kernel's
    loader path REMOVED (an entry naming the prefix's library dir, wherever
    the shell got it) and the build variables dropped -- the bundle beside
    the binary is what makes it start, and running it any other way would
    hide a broken bundle. `CICADA_PYTHON` names the interpreter running the
    launcher so the engine's script host uses the Python that was found."""
    env = {k: v for k, v in base.items() if k not in ("DEP_OCCT_ROOT", "CMAKE_POLICY_VERSION_MINIMUM")}
    library_dir = str(layout.library_dir)
    separator = ";" if system == "windows" else ":"

    def same(entry: str) -> bool:
        a, b = entry.rstrip("\\/"), library_dir.rstrip("\\/")
        return a.lower() == b.lower() if system == "windows" else a == b

    for variable in {"PATH", layout.loader_variable}:
        value = env.get(variable)
        if value is None:
            continue
        kept = [entry for entry in value.split(separator) if not same(entry)]
        if variable == "PATH":
            env[variable] = separator.join(kept)
        elif kept:
            env[variable] = separator.join(kept)
        else:
            del env[variable]
    env["CICADA_PYTHON"] = str(python)
    return env


def split_args(argv: list[str]) -> tuple[set[str], list[str]]:
    """The launcher's own flags (anywhere on the line) and the rest, which
    goes to `cicada app` in order."""
    flags = {arg for arg in argv if arg in LAUNCHER_FLAGS}
    rest = [arg for arg in argv if arg not in LAUNCHER_FLAGS]
    return flags, rest


def app_cwd(app_args: list[str], caller_cwd: Path, repo: Path) -> Path:
    """Where `cicada app` runs (module docstring): the caller's directory when
    the launcher was given arguments (their relative paths keep their
    meaning), the repository when it was given none (the double-click)."""
    return caller_cwd if app_args else repo


# ---------------------------------------------------------------------------
# The steps
# ---------------------------------------------------------------------------


def say(message: str) -> None:
    print(f"==> {message}", flush=True)


def run_step(name: str, argv: list[str], cwd: Path, env: dict[str, str]) -> None:
    """Run one build step with the console inherited; a non-zero exit or a
    missing program is a [`LaunchError`] naming the step."""
    say(f"{name}: {' '.join(argv)}  (in {cwd})")
    try:
        completed = subprocess.run(argv, cwd=str(cwd), env=env, check=False)
    except FileNotFoundError as error:
        raise LaunchError(f"{name}: {argv[0]} could not be started ({error})") from error
    if completed.returncode != 0:
        raise LaunchError(f"{name} failed with exit code {completed.returncode} -- see its output above")


def tool_version(path: Path, env: dict[str, str]) -> str:
    try:
        completed = subprocess.run([str(path), "--version"], env=env, capture_output=True, text=True, check=False)
    except OSError as error:
        return f"({error})"
    return (completed.stdout or completed.stderr).strip().splitlines()[0] if (completed.stdout or completed.stderr).strip() else "(no version output)"


def launch(argv: list[str], environ: dict[str, str] | None = None, system: str | None = None) -> int:
    environ = dict(os.environ if environ is None else environ)
    system = host_system(system)
    flags, app_args = split_args(argv)
    if "--launcher-help" in flags:
        print(__doc__)
        return 0
    python = Path(sys.executable)
    say(f"Cicada launcher -- repository {REPO}")
    say(f"python {platform.python_version()} ({python})")

    # Tools. Every missing one is named with what to install; nothing is guessed.
    missing = []
    npm = find_tool("npm", environ)
    cargo = find_tool("cargo", environ)
    cmake = find_tool("cmake", environ, cmake_candidates(system, environ))
    for name, found in (("npm", npm), ("cargo", cargo), ("cmake", cmake)):
        if found is None:
            missing.append(f"{name}: not found -- install {TOOL_HINTS[name]}")
    if missing:
        raise LaunchError("missing tools:\n  " + "\n  ".join(missing))
    assert npm is not None and cargo is not None and cmake is not None
    cmake_dir = None if shutil.which("cmake", path=environ.get("PATH", "")) else cmake.parent
    say(f"npm {tool_version(npm, environ)} ({npm})")
    say(f"{tool_version(cargo, environ)} ({cargo})")
    say(f"{tool_version(cmake, environ)} ({cmake})" + ("" if cmake_dir is None else " -- not on PATH, added for the build"))

    # Where cargo puts the build.
    try:
        metadata = subprocess.run(
            [str(cargo), "metadata", "--format-version", "1", "--no-deps"],
            cwd=str(REPO),
            env=environ,
            capture_output=True,
            text=True,
            check=True,
        )
    except subprocess.CalledProcessError as error:
        raise LaunchError(f"cargo metadata failed ({error.returncode}): {error.stderr.strip()}") from error
    target_dir = cargo_target_dir(metadata.stdout)
    release_dir = target_dir / PROFILE
    binary = release_dir / binary_name(system)
    say(f"target directory {target_dir}")

    state = observe(REPO, release_dir, system)
    plan = decide(state)
    if plan.fresh:
        say(f"{binary.name} is fresh (release, SPA embedded) -- nothing to build")
    else:
        say("to build: " + ", ".join(plan.reasons))
    if "--plan" in flags:
        for step, wanted in (("npm ci", plan.install), ("npm run build", plan.build_spa), ("cargo build --release --features embed", plan.build_engine)):
            say(f"  {'run ' if wanted else 'skip'} {step}")
        say("--plan: stopping here")
        return 0

    # The OCCT prebuilt: fetch_occt's warm path verifies the prefix (0.2 s);
    # the first run on a machine downloads it (minutes, printed).
    manifest = fo.load_manifest()
    layout = fo.Layout(fo.default_cache_root(environ), fo.detect_subdir(), manifest["occt_version"])
    say(f"OCCT prebuilt {manifest['occt_version']} ({layout.subdir}) in {layout.prefix}")
    try:
        fo.fetch(manifest, layout, lambda m: print(f"    {m}", flush=True))
    except fo.FetchError as error:
        raise LaunchError(f"fetch_occt: {error}") from error
    # The prebuilt Manifold (AGENTS.md iteration-speed rule 6): built ONCE per
    # machine (about a minute and a half on the dev machine, printed), then
    # every cargo build links it instead of compiling the kernel with cmake.
    fm_manifest = fm.load_manifest()
    fm_layout = fm.Layout(fm.default_cache_root(environ), fm.detect_subdir(), fm_manifest["tag"])
    say(f"Manifold prebuilt {fm_manifest['tag']} ({fm_layout.subdir}) in {fm_layout.prefix}")
    try:
        fm.ensure(fm_manifest, fm_layout, lambda m: print(f"    {m}", flush=True))
    except fm.FetchError as error:
        raise LaunchError(f"fetch_manifold: {error}") from error
    build_env = build_environment(environ, layout, system, cmake_dir, manifold=fm_layout)

    if plan.install:
        run_step("npm ci", [str(npm), "ci"], REPO / "web", build_env)
    if plan.build_spa:
        run_step("npm run build", [str(npm), "run", "build"], REPO / "web", build_env)
    if plan.build_engine:
        run_step(
            "cargo build",
            [str(cargo), "build", f"--{PROFILE}", "-p", "cicada-cli", "--features", ",".join(FEATURES)],
            REPO,
            build_env,
        )
        if not binary.is_file():
            raise LaunchError(f"cargo build succeeded but {binary} is not there")
        mark_built(binary, release_dir)
        say(f"stamped {binary.name} as {PROFILE} + {','.join(FEATURES)}")

    # The run-time libraries beside the binary (L2): idempotent, a second
    # run copies nothing.
    try:
        fo.bundle(manifest, layout, release_dir, lambda m: print(f"    {m}", flush=True))
    except fo.FetchError as error:
        raise LaunchError(f"bundle: {error}") from error

    if "--no-run" in flags:
        say(f"--no-run: {binary} is built and bundled; not starting it")
        return 0

    run_env = run_environment(environ, layout, system, python)
    command = [str(binary), "app", *app_args]
    cwd = app_cwd(app_args, Path.cwd(), REPO)
    say(f"running: {' '.join(command)}  (in {cwd}; no loader path in its environment; Ctrl-C stops the server)")
    try:
        process = subprocess.Popen(command, cwd=str(cwd), env=run_env)
    except OSError as error:
        raise LaunchError(f"could not start {binary}: {error}") from error
    try:
        return process.wait()
    except KeyboardInterrupt:
        # Ctrl-C reached the server too (same console); let it shut down.
        return process.wait()


def main(argv: list[str]) -> int:
    try:
        return launch(argv)
    except LaunchError as error:
        print(f"error: {error}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
