#!/usr/bin/env python3
"""Build the pinned Manifold kernel ONCE per machine and link it prebuilt.

`cicada-geom` takes Manifold through `manifold-csg` -> `manifold-csg-sys`, whose
build script clones upstream and compiles it with cmake INSIDE every cargo target
dir -- once per worktree, and once more per feature-unification context (debug,
release, `--features embed`), about five minutes each: the single largest cost of
a cold build (AGENTS.md iteration-speed rule 6, decided with Ben 2026-08-24). The
same build script links an EXTERNAL Manifold instead when `MANIFOLD_CSG_LIB_DIR`
names a flat directory holding the static archives and `MANIFOLD_CSG_LIB_KIND` is
`static`: no clone, no cmake, no C++ compile. This script produces that directory.

It clones upstream at the pinned tag, checks the commit, configures cmake with
EXACTLY the flags the -sys crate's own build uses (pinned in
`tools/fetch_manifold_manifest.json`; `tools/test_fetch_manifold.py` holds them to
the crate's source when it is on disk), builds Release, harvests the static
archives into `<prefix>/lib` -- `manifold`, `manifoldc`, `Clipper2` and the TBB
the `parallel` feature links -- and writes a stamp. Idempotent: a prefix whose
stamp matches the manifest and whose archives are present at their recorded
sizes is left alone (the warm path is a handful of stats).

    python tools/fetch_manifold.py                   # build for this platform if needed; print the env
    python tools/fetch_manifold.py --print-env bash  # `export ...` lines (also: powershell, cmd)
    python tools/fetch_manifold.py --github-env      # append to $GITHUB_ENV
    python tools/fetch_manifold.py --dest DIR        # cache root (default below)
    python tools/fetch_manifold.py --check           # verify only: exit 1 when the prefix is missing or stale
    python tools/fetch_manifold.py --manifest-hash   # the CI cache key
    python tools/fetch_manifold.py --keep-work       # keep the clone and the cmake tree after the harvest

`tools/fetch_occt.py --print-env` / `--github-env` also emit the two variables
when this prefix exists, so the one incantation every shell already runs covers
both kernels; without the prefix they say so on stderr and cargo compiles
Manifold from source as before -- slower, never wrong.

Default cache root: `%LOCALAPPDATA%\\cicada-manifold` on Windows, else
`$XDG_CACHE_HOME/cicada-manifold` (`~/.cache/cicada-manifold`). The prefix is
`<root>/manifold-<tag>-<subdir>` (the subdir names are conda's, shared with
`fetch_occt.py`: win-64, linux-64, osx-64, osx-arm64); the clone and the cmake
tree live under `<root>/work/<tag>-<subdir>` and are removed after a successful
harvest unless `--keep-work`.

Needs: git, cmake (on PATH, or the Visual Studio Build Tools copy on Windows --
the same one AGENTS.md prepends for the in-tree kernel build), a C++ toolchain,
and network ONCE: the clone, plus Manifold's own FetchContent of Clipper2 and
oneTBB at the versions its CMakeLists pins. Windows long paths: oneTBB's
documentation assets exceed MAX_PATH, so every git this script (or cmake on its
behalf) runs gets `core.longpaths=true` through GIT_CONFIG_COUNT -- the fix
AGENTS.md records for the in-tree build.

Exit status: 0 on success; 1 on any mismatch (commit, missing archive, stale
stamp), a failed tool, or a missing tool -- always through the `error:` line,
never a silent fallback.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST_PATH = HERE / "fetch_manifold_manifest.json"
CMAKE_POLICY_VERSION_MINIMUM = "3.5"
LIB_DIR_VARIABLE = "MANIFOLD_CSG_LIB_DIR"
LIB_KIND_VARIABLE = "MANIFOLD_CSG_LIB_KIND"
LIB_KIND = "static"
SUPPORTED_SUBDIRS = ("win-64", "linux-64", "osx-64", "osx-arm64")

# Where Visual Studio's bundled cmake lives when it is not on PATH (AGENTS.md dev notes).
VS_CMAKE_SUFFIX = Path("Common7") / "IDE" / "CommonExtensions" / "Microsoft" / "CMake" / "CMake" / "bin" / "cmake.exe"
VS_EDITIONS = ("BuildTools", "Community", "Professional", "Enterprise")


class FetchError(Exception):
    """A refusal: printed as `error: ...`, exit 1, never a fallback."""


# ---------------------------------------------------------------------------
# Platform and layout
# ---------------------------------------------------------------------------


def detect_subdir(system: str | None = None, machine: str | None = None) -> str:
    """conda's platform names, the same ones `fetch_occt.py` uses."""
    system = (system or platform.system()).lower()
    machine = (machine or platform.machine()).lower()
    arm = machine in ("arm64", "aarch64")
    if system == "windows":
        return "win-64"
    if system == "darwin":
        return "osx-arm64" if arm else "osx-64"
    if system == "linux":
        if arm:
            raise FetchError("linux-aarch64 is not a supported platform for the prebuilt Manifold (no CI job builds it)")
        return "linux-64"
    raise FetchError(f"unsupported platform {system}/{machine}")


def default_cache_root(environ: dict[str, str] | None = None, system: str | None = None) -> Path:
    env = os.environ if environ is None else environ
    system = (system or platform.system()).lower()
    if system == "windows":
        local = env.get("LOCALAPPDATA")
        if not local:
            raise FetchError("LOCALAPPDATA is not set; pass --dest")
        return Path(local) / "cicada-manifold"
    xdg = env.get("XDG_CACHE_HOME")
    base = Path(xdg) if xdg else Path(env.get("HOME", str(Path.home()))) / ".cache"
    return base / "cicada-manifold"


class Layout:
    """Where a platform's prefix, stamp, and work tree live under the cache root."""

    def __init__(self, cache_root: Path, subdir: str, tag: str):
        if subdir not in SUPPORTED_SUBDIRS:
            raise FetchError(f"unsupported subdir {subdir!r}; one of {', '.join(SUPPORTED_SUBDIRS)}")
        self.cache_root = cache_root
        self.subdir = subdir
        self.tag = tag
        self.prefix = cache_root / f"manifold-{tag}-{subdir}"
        self.lib_dir = self.prefix / "lib"
        self.stamp = self.prefix / "stamp.json"
        self.work = cache_root / "work" / f"{tag}-{subdir}"
        self.source = self.work / "src"
        self.build = self.work / "build"

    @property
    def is_windows(self) -> bool:
        return self.subdir == "win-64"

    @property
    def is_macos(self) -> bool:
        return self.subdir.startswith("osx-")


def static_filename(name: str, subdir: str) -> str:
    """The archive name the platform linker resolves for `-l{name}` of kind static --
    the same table `manifold-csg-sys` probes (`external_lib_filenames`)."""
    return f"{name}.lib" if subdir == "win-64" else f"lib{name}.a"


# ---------------------------------------------------------------------------
# Manifest and stamp
# ---------------------------------------------------------------------------


def load_manifest(path: Path = MANIFEST_PATH) -> dict:
    try:
        data = path.read_bytes()
        manifest = json.loads(data)
    except OSError as error:
        raise FetchError(f"cannot read {path}: {error}") from error
    except ValueError as error:
        raise FetchError(f"{path} is not valid JSON: {error}") from error
    validate_manifest(manifest)
    manifest["_sha256"] = hashlib.sha256(data).hexdigest()
    return manifest


def validate_manifest(manifest: dict) -> None:
    for key in ("manifold_csg_sys", "repository", "tag", "commit", "parallel", "cmake_flags", "libraries"):
        if key not in manifest:
            raise FetchError(f"manifest is missing {key!r}")
    commit = manifest["commit"]
    if not (isinstance(commit, str) and len(commit) == 40 and all(c in "0123456789abcdef" for c in commit)):
        raise FetchError("manifest commit must be a 40-hex sha")
    if not manifest["tag"].startswith("v"):
        raise FetchError("manifest tag must be upstream's v-prefixed tag")
    flags = manifest["cmake_flags"]
    if not isinstance(flags, list) or not all(isinstance(f, str) and f.startswith("-D") for f in flags):
        raise FetchError("manifest cmake_flags must be a list of -D… strings")
    has_par_on = "-DMANIFOLD_PAR=ON" in flags
    if bool(manifest["parallel"]) != has_par_on:
        raise FetchError("manifest `parallel` must agree with -DMANIFOLD_PAR=ON in cmake_flags")
    libs = manifest["libraries"]
    if set(libs.get("required", [])) != {"manifold", "manifoldc", "Clipper2"}:
        raise FetchError("manifest libraries.required must be manifold, manifoldc, Clipper2 (what the -sys crate links)")
    if manifest["parallel"] and not libs.get("tbb_candidates"):
        raise FetchError("a parallel build must name its tbb_candidates")


def expected_archives(manifest: dict, subdir: str) -> list[str]:
    """The required archive filenames (TBB's name is whatever the build produced; see the stamp)."""
    return [static_filename(name, subdir) for name in manifest["libraries"]["required"]]


def read_stamp(layout: Layout) -> dict | None:
    try:
        return json.loads(layout.stamp.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def prefix_problems(manifest: dict, layout: Layout) -> list[str]:
    """Everything that makes the prefix unusable or stale; empty means warm."""
    stamp = read_stamp(layout)
    if stamp is None:
        return [f"no stamp at {layout.stamp}"]
    problems: list[str] = []
    for key in ("tag", "commit", "manifold_csg_sys", "cmake_flags", "parallel"):
        if stamp.get(key) != manifest[key]:
            problems.append(f"stamp {key} = {stamp.get(key)!r}, manifest says {manifest[key]!r}")
    libraries = stamp.get("libraries")
    if not isinstance(libraries, dict) or not libraries:
        return problems + ["stamp records no libraries"]
    for filename in expected_archives(manifest, layout.subdir):
        if filename not in libraries:
            problems.append(f"stamp does not record {filename}")
    if manifest["parallel"] and not any(name.startswith(("tbb", "libtbb")) for name in libraries):
        problems.append("stamp records no TBB archive for a parallel build")
    for filename, size in libraries.items():
        path = layout.lib_dir / filename
        try:
            actual = path.stat().st_size
        except OSError:
            problems.append(f"missing {path}")
            continue
        if actual != size:
            problems.append(f"{path} is {actual} bytes, the stamp says {size}")
    return problems


# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------


def find_cmake(environ: dict[str, str] | None = None, which=shutil.which, exists=None) -> str:
    """cmake on PATH, else Visual Studio's bundled copy on Windows; a refusal otherwise."""
    env = os.environ if environ is None else environ
    exists = exists or (lambda p: Path(p).is_file())
    found = which("cmake", path=env.get("PATH"))
    if found:
        return found
    candidates: list[Path] = []
    for var in ("ProgramFiles(x86)", "ProgramFiles"):
        base = env.get(var)
        if base:
            for edition in VS_EDITIONS:
                candidates.append(Path(base) / "Microsoft Visual Studio" / "2022" / edition / VS_CMAKE_SUFFIX)
    for candidate in candidates:
        if exists(candidate):
            return str(candidate)
    raise FetchError(
        "cmake not found on PATH"
        + (f" nor at {candidates[0]}" if candidates else "")
        + "; install cmake or prepend the Visual Studio Build Tools copy (AGENTS.md dev notes)"
    )


def git_env(environ: dict[str, str] | None = None) -> dict[str, str]:
    """The environment for every git this script runs, and for cmake's FetchContent
    clones: `core.longpaths=true` (oneTBB's doc assets exceed MAX_PATH on Windows)."""
    env = dict(os.environ if environ is None else environ)
    count = int(env.get("GIT_CONFIG_COUNT", "0") or 0)
    env[f"GIT_CONFIG_KEY_{count}"] = "core.longpaths"
    env[f"GIT_CONFIG_VALUE_{count}"] = "true"
    env["GIT_CONFIG_COUNT"] = str(count + 1)
    return env


def run(command: list[str], log, cwd: Path | None = None, env: dict[str, str] | None = None, log_file: Path | None = None) -> None:
    log("$ " + " ".join(command))
    if log_file is not None:
        log_file.parent.mkdir(parents=True, exist_ok=True)
        with open(log_file, "w", encoding="utf-8", errors="replace") as handle:
            completed = subprocess.run(command, cwd=cwd, env=env, stdout=handle, stderr=subprocess.STDOUT, check=False)
        if completed.returncode != 0:
            tail = log_file.read_text(encoding="utf-8", errors="replace").splitlines()[-40:]
            raise FetchError(f"`{command[0]}` exited {completed.returncode}; the log is {log_file}; its tail:\n" + "\n".join(tail))
        return
    completed = subprocess.run(command, cwd=cwd, env=env, check=False)
    if completed.returncode != 0:
        raise FetchError(f"`{' '.join(command)}` exited {completed.returncode}")


def git_head(source: Path, env: dict[str, str]) -> str | None:
    try:
        completed = subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"], env=env, capture_output=True, text=True, check=False)
    except OSError:
        return None
    return completed.stdout.strip() if completed.returncode == 0 else None


# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------


def clone(manifest: dict, layout: Layout, log, run=run, git_head=git_head, which=shutil.which) -> None:
    """A shallow clone of the pinned tag, refused unless its HEAD is the pinned commit
    (`run`, `git_head` and `which` are injectable so the decision is unit-tested)."""
    env = git_env()
    if layout.source.exists():
        if git_head(layout.source, env) == manifest["commit"]:
            log(f"source at {layout.source} is already at {manifest['commit'][:12]}")
            return
        log(f"source at {layout.source} is not at the pinned commit; re-cloning")
        shutil.rmtree(layout.source)
    layout.work.mkdir(parents=True, exist_ok=True)
    if which("git") is None:
        raise FetchError("git not found on PATH")
    run(["git", "clone", "--depth", "1", "--branch", manifest["tag"], manifest["repository"], str(layout.source)], log, env=env)
    head = git_head(layout.source, env)
    if head != manifest["commit"]:
        raise FetchError(f"the clone of {manifest['tag']} is at {head}, the manifest pins {manifest['commit']} -- refusing to build an unpinned tree")


def configure_command(manifest: dict, layout: Layout, cmake: str) -> list[str]:
    """The configure line: the manifest's flags verbatim, plus the policy floor CI sets."""
    return [cmake, "-S", str(layout.source), "-B", str(layout.build), *manifest["cmake_flags"], f"-DCMAKE_POLICY_VERSION_MINIMUM={CMAKE_POLICY_VERSION_MINIMUM}"]


def build_command(layout: Layout, cmake: str, jobs: int | None) -> list[str]:
    build = [cmake, "--build", str(layout.build), "--config", "Release", "--parallel"]
    if jobs:
        build.append(str(jobs))
    return build


def configure_and_build(manifest: dict, layout: Layout, cmake: str, jobs: int | None, log, run=run) -> None:
    env = git_env()
    env.setdefault("CMAKE_POLICY_VERSION_MINIMUM", CMAKE_POLICY_VERSION_MINIMUM)
    if layout.build.exists():
        shutil.rmtree(layout.build)
    run(configure_command(manifest, layout, cmake), log, env=env, log_file=layout.work / "cmake-configure.log")
    run(build_command(layout, cmake, jobs), log, env=env, log_file=layout.work / "cmake-build.log")


def find_archive(root: Path, filename: str) -> Path | None:
    """The first archive with this exact name under the build tree (the -sys crate's
    `find_lib_recursive`); Release directories win over Debug ones on multi-config generators."""
    matches = sorted(root.rglob(filename), key=lambda p: (0 if "Release" in p.parts else 1, len(p.parts), str(p)))
    return matches[0] if matches else None


def harvest(manifest: dict, layout: Layout, log) -> dict[str, int]:
    """Copy the static archives into the flat lib dir; return {filename: size}."""
    wanted = list(expected_archives(manifest, layout.subdir))
    found: dict[str, Path] = {}
    missing: list[str] = []
    for filename in wanted:
        path = find_archive(layout.build, filename)
        if path is None:
            missing.append(filename)
        else:
            found[filename] = path
    if manifest["parallel"]:
        tbb = None
        for candidate in manifest["libraries"]["tbb_candidates"]:
            path = find_archive(layout.build, static_filename(candidate, layout.subdir))
            if path is not None:
                tbb = path
                break
        if tbb is None:
            missing.append("a TBB archive (" + ", ".join(static_filename(c, layout.subdir) for c in manifest["libraries"]["tbb_candidates"]) + ")")
        else:
            found[tbb.name] = tbb
    if missing:
        raise FetchError(f"the build tree {layout.build} has no {', '.join(missing)} -- the harvest refuses a partial prefix")
    if layout.lib_dir.exists():
        shutil.rmtree(layout.lib_dir)
    layout.lib_dir.mkdir(parents=True)
    sizes: dict[str, int] = {}
    for filename, path in found.items():
        shutil.copy2(path, layout.lib_dir / filename)
        sizes[filename] = (layout.lib_dir / filename).stat().st_size
        log(f"harvested {filename} ({sizes[filename]} bytes) from {path.relative_to(layout.build)}")
    return sizes


def write_stamp(manifest: dict, layout: Layout, libraries: dict[str, int], cmake: str) -> None:
    stamp = {
        "tag": manifest["tag"],
        "commit": manifest["commit"],
        "manifold_csg_sys": manifest["manifold_csg_sys"],
        "parallel": manifest["parallel"],
        "cmake_flags": manifest["cmake_flags"],
        "subdir": layout.subdir,
        "libraries": libraries,
        "cmake": cmake,
        "built_at": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
    }
    layout.prefix.mkdir(parents=True, exist_ok=True)
    layout.stamp.write_text(json.dumps(stamp, indent=2) + "\n", encoding="utf-8")


def ensure(manifest: dict, layout: Layout, log, jobs: int | None = None, keep_work: bool = False) -> bool:
    """Make the prefix warm; True when a build ran."""
    problems = prefix_problems(manifest, layout)
    if not problems:
        log(f"prefix {layout.prefix} is present and verified ({len(read_stamp(layout)['libraries'])} archives at their sizes)")
        return False
    log(f"prefix {layout.prefix} needs a build: " + "; ".join(problems))
    cmake = find_cmake()
    clone(manifest, layout, log)
    configure_and_build(manifest, layout, cmake, jobs, log)
    libraries = harvest(manifest, layout, log)
    write_stamp(manifest, layout, libraries, cmake)
    remaining = prefix_problems(manifest, layout)
    if remaining:
        raise FetchError("the prefix is not warm after its own build: " + "; ".join(remaining))
    if not keep_work:
        remove_work_tree(layout, log)
    return True


def remove_work_tree(layout: Layout, log) -> bool:
    """Remove the clone and the cmake tree; True when gone. A leftover is a note, not a
    refusal (the next run re-checks the clone's HEAD and wipes `build/` itself) — but it is
    said, never logged as removed: on Windows a compiler helper can still hold a handle."""
    failures: list[str] = []

    def on_error(function, path, exc_info):  # noqa: ARG001 — shutil's onerror signature
        failures.append(f"{path}: {exc_info[1]}")

    shutil.rmtree(layout.work, onerror=on_error)
    if layout.work.exists():
        log(f"note: the work tree {layout.work} could not be removed completely ({failures[0] if failures else 'unknown reason'}); harmless, delete it by hand or pass --keep-work to keep it on purpose")
        return False
    log(f"removed the work tree {layout.work} (--keep-work keeps it)")
    return True


# ---------------------------------------------------------------------------
# Environment
# ---------------------------------------------------------------------------


def env_lines(layout: Layout, shell: str) -> list[str]:
    """The two variables `manifold-csg-sys` reads. The dir is read by a Rust build
    script (`Path::is_dir`), so a Windows path works with either slash; bash gets
    forward slashes so nothing in the shell interprets a backslash."""
    lib = str(layout.lib_dir)
    if shell == "bash":
        if layout.is_windows:
            lib = lib.replace("\\", "/")
        return [f"export {LIB_DIR_VARIABLE}='{lib}'", f"export {LIB_KIND_VARIABLE}='{LIB_KIND}'"]
    if shell == "powershell":
        return [f"$env:{LIB_DIR_VARIABLE} = '{lib}'", f"$env:{LIB_KIND_VARIABLE} = '{LIB_KIND}'"]
    if shell == "cmd":
        return [f"set {LIB_DIR_VARIABLE}={lib}", f"set {LIB_KIND_VARIABLE}={LIB_KIND}"]
    raise FetchError(f"unknown shell {shell!r}; use bash, powershell or cmd")


def github_env_entries(layout: Layout) -> list[str]:
    return [f"{LIB_DIR_VARIABLE}={layout.lib_dir}", f"{LIB_KIND_VARIABLE}={LIB_KIND}"]


def default_layout(cache_root: Path | None = None, subdir: str | None = None, manifest: dict | None = None) -> Layout:
    manifest = manifest or load_manifest()
    return Layout(cache_root or default_cache_root(), subdir or detect_subdir(), manifest["tag"])


def env_lines_if_present(shell: str, note, cache_root: Path | None = None, subdir: str | None = None) -> list[str]:
    """For `fetch_occt.py`'s dev-shell forms: the lines when the prefix is warm, else a
    note and nothing -- cargo then compiles Manifold from source, slower, never wrong.
    (`--github-env` has no rider: CI runs this script's own `--github-env` step.)"""
    manifest = load_manifest()
    layout = default_layout(cache_root, subdir, manifest=manifest)
    problems = prefix_problems(manifest, layout)
    if problems:
        note(
            f"note: no prebuilt Manifold at {layout.prefix} ({problems[0]}); cargo will compile it from source "
            f"in every target dir (~5 min each) -- `python tools/fetch_manifold.py` builds it once"
        )
        return []
    return env_lines(layout, shell)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dest", type=Path, help="cache root (default: the user cache dir, see above)")
    parser.add_argument("--subdir", choices=SUPPORTED_SUBDIRS, help="platform; default: this one")
    parser.add_argument("--print-env", choices=["bash", "powershell", "cmd"], help="emit shell lines instead of the summary")
    parser.add_argument("--github-env", action="store_true", help="append to $GITHUB_ENV")
    parser.add_argument("--check", action="store_true", help="verify the prefix; never build")
    parser.add_argument("--manifest-hash", action="store_true", help="print the manifest sha256 and exit")
    parser.add_argument("--jobs", type=int, help="cmake --parallel N (default: cmake's choice)")
    parser.add_argument("--keep-work", action="store_true", help="keep the clone and the cmake tree after the harvest")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    def log(message: str) -> None:
        if not args.quiet:
            print(message, file=sys.stderr)

    try:
        manifest = load_manifest()
        if args.manifest_hash:
            print(manifest["_sha256"])
            return 0
        layout = Layout(args.dest or default_cache_root(), args.subdir or detect_subdir(), manifest["tag"])
        if args.check:
            problems = prefix_problems(manifest, layout)
            if problems:
                raise FetchError(f"prefix {layout.prefix} is not usable: " + "; ".join(problems))
            log(f"prefix {layout.prefix} is present and verified")
        else:
            ensure(manifest, layout, log, jobs=args.jobs, keep_work=args.keep_work)
        if args.print_env:
            print("\n".join(env_lines(layout, args.print_env)))
        elif args.github_env:
            env_file = os.environ.get("GITHUB_ENV")
            if not env_file:
                raise FetchError("--github-env needs GITHUB_ENV in the environment")
            entries = github_env_entries(layout)
            with open(env_file, "a", encoding="utf-8") as handle:
                handle.write("".join(line + "\n" for line in entries))
            log("wrote " + ", ".join(entries))
        else:
            for line in github_env_entries(layout):
                print(line)
        return 0
    except FetchError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
