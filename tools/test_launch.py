"""Unit tests for tools/launch/launch.py and tools/launch/bundle.py (docs/17
wave 4 L3) -- the pure parts: the staleness rule and its inputs, the
launcher's stamp, tool discovery tables, the build and run environments per
OS, the bundle's layout and the files it writes, `make_bundle` and
`check_bundle` over synthetic binaries and libraries (the PE / Mach-O
fixtures of test_fetch_occt; every external program through an injected
runner), the minimal environment, the smoke's URL parser. Offline,
deterministic; nothing here builds, fetches or starts a server -- the real
bundle's `--check --smoke` on a release build is the launcher work's own
evidence, recorded in docs/17."""

import contextlib
import io
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path, PurePosixPath, PureWindowsPath

import fetch_occt as fo
from test_fetch_occt import fake_macho, fake_pe

LAUNCH_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "launch")
if LAUNCH_DIR not in sys.path:
    sys.path.insert(0, LAUNCH_DIR)

import bundle  # noqa: E402
import launch  # noqa: E402

#: The SPA's markers, appended to a synthetic binary that "embeds the SPA".
SPA = b"".join(bundle.SPA_MARKERS)


def completed(argv, code=0, stdout="", stderr=""):
    return subprocess.CompletedProcess(argv, code, stdout, stderr)


# ---------------------------------------------------------------------------
# launch.py
# ---------------------------------------------------------------------------


def state(**overrides):
    """A fresh tree: binary present and stamped, dist present, everything older than the binary."""
    values = dict(
        binary_exists=True,
        binary_mtime_ns=1_000,
        stamp_valid=True,
        node_modules=True,
        dist_exists=True,
        dist_mtime_ns=900,
        web_newest_ns=800,
        engine_newest_ns=700,
    )
    values.update(overrides)
    return launch.State(**values)


class DecideTest(unittest.TestCase):
    """The staleness rule, case by case."""

    def test_fresh_tree_builds_nothing(self):
        plan = launch.decide(state())
        self.assertTrue(plan.fresh)
        self.assertEqual(plan.reasons, ())

    def test_missing_binary_builds_the_engine_only(self):
        plan = launch.decide(state(binary_exists=False, binary_mtime_ns=0, stamp_valid=False))
        self.assertEqual((plan.install, plan.build_spa, plan.build_engine), (False, False, True))
        self.assertEqual(plan.reasons, ("no release binary",))

    def test_a_binary_without_the_launchers_stamp_is_never_trusted(self):
        plan = launch.decide(state(stamp_valid=False))
        self.assertTrue(plan.build_engine)
        self.assertFalse(plan.build_spa)
        self.assertIn("no matching stamp", plan.reasons[0])

    def test_engine_sources_newer_than_the_binary(self):
        plan = launch.decide(state(engine_newest_ns=5_000))
        self.assertEqual((plan.build_spa, plan.build_engine), (False, True))
        self.assertEqual(plan.reasons, ("engine sources are newer than the binary",))

    def test_web_sources_newer_than_dist_rebuild_the_spa_and_the_engine(self):
        plan = launch.decide(state(web_newest_ns=5_000))
        self.assertEqual((plan.install, plan.build_spa, plan.build_engine), (False, True, True))
        self.assertEqual(
            plan.reasons,
            ("web sources are newer than web/dist", "the SPA will be rebuilt, so the binary embedding it must be too"),
        )

    def test_dist_newer_than_the_binary_rebuilds_the_engine(self):
        plan = launch.decide(state(dist_mtime_ns=2_000))
        self.assertEqual((plan.build_spa, plan.build_engine), (False, True))
        self.assertIn("web/dist is newer than the binary", plan.reasons[0])

    def test_fresh_checkout_does_everything(self):
        plan = launch.decide(
            state(
                binary_exists=False,
                binary_mtime_ns=0,
                stamp_valid=False,
                node_modules=False,
                dist_exists=False,
                dist_mtime_ns=0,
            )
        )
        self.assertEqual((plan.install, plan.build_spa, plan.build_engine), (True, True, True))
        self.assertEqual(plan.reasons, ("web/node_modules is missing (npm ci)", "web/dist is missing", "no release binary"))

    def test_missing_node_modules_alone_installs_only(self):
        plan = launch.decide(state(node_modules=False))
        self.assertEqual((plan.install, plan.build_spa, plan.build_engine), (True, False, False))


class TreeTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cicada-launch-"))

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def touch(self, relative, mtime_ns):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"x")
        os.utime(path, ns=(mtime_ns, mtime_ns))
        return path

    def test_newest_mtime_skips_node_modules_target_and_missing_entries(self):
        self.touch("crates/a/src/lib.rs", 1_000_000_000)
        self.touch("crates/a/target/release/x", 9_000_000_000)
        self.touch("web/src/main.tsx", 2_000_000_000)
        self.touch("web/node_modules/pkg/index.js", 9_000_000_000)
        self.touch("Cargo.lock", 3_000_000_000)
        self.assertEqual(launch.newest_mtime_ns(self.root, ("Cargo.toml", "Cargo.lock", "crates")), 3_000_000_000)
        self.assertEqual(launch.newest_mtime_ns(self.root, ("web",)), 2_000_000_000, "node_modules is never entered")
        self.assertEqual(launch.newest_mtime_ns(self.root, ("nowhere",)), 0)

    def test_stamp_pins_the_binary_and_the_build(self):
        binary = self.touch("release/cicada.exe", 5_000_000_000)
        stamp = launch.stamp_for(binary)
        self.assertEqual(stamp["features"], ["embed"])
        self.assertEqual(stamp["profile"], "release")
        self.assertTrue(launch.stamp_valid(stamp, binary))
        # A rebuilt binary (new mtime or size) invalidates it; so does a
        # stamp for another feature set, a missing stamp, a missing binary.
        # NTFS keeps 100 ns; a whole microsecond moves every file system.
        os.utime(binary, ns=(5_001_000_000, 5_001_000_000))
        self.assertFalse(launch.stamp_valid(stamp, binary))
        stamp = launch.stamp_for(binary)
        binary.write_bytes(b"xy")
        os.utime(binary, ns=(5_001_000_000, 5_001_000_000))
        self.assertFalse(launch.stamp_valid(stamp, binary))
        stamp = launch.stamp_for(binary)
        self.assertFalse(launch.stamp_valid(dict(stamp, features=[]), binary))
        self.assertFalse(launch.stamp_valid(None, binary))
        self.assertFalse(launch.stamp_valid(stamp, self.root / "gone"))

    def test_observe_reads_the_tree(self):
        binary = self.touch("target/release/cicada.exe", 5_000_000_000)
        (self.root / "target" / "release" / launch.STAMP_NAME).write_text(
            json.dumps(launch.stamp_for(binary)), encoding="utf-8"
        )
        self.touch("web/dist/index.html", 4_000_000_000)
        self.touch("web/src/main.tsx", 3_000_000_000)
        self.touch("crates/x/src/lib.rs", 2_000_000_000)
        (self.root / "web" / "node_modules").mkdir()
        observed = launch.observe(self.root, self.root / "target" / "release", "windows")
        self.assertEqual(
            observed,
            launch.State(
                binary_exists=True,
                binary_mtime_ns=5_000_000_000,
                stamp_valid=True,
                node_modules=True,
                dist_exists=True,
                dist_mtime_ns=4_000_000_000,
                web_newest_ns=3_000_000_000,
                engine_newest_ns=2_000_000_000,
            ),
        )
        self.assertTrue(launch.decide(observed).fresh)
        # The stamp wiring, negatively: a stamp for another binary (the size
        # differs) and no stamp at all both leave the binary untrusted.
        release = self.root / "target" / "release"
        (release / launch.STAMP_NAME).write_text(json.dumps(dict(launch.stamp_for(binary), size=999)), encoding="utf-8")
        observed = launch.observe(self.root, release, "windows")
        self.assertFalse(observed.stamp_valid)
        self.assertIn("no matching stamp", launch.decide(observed).reasons[0])
        (release / launch.STAMP_NAME).unlink()
        observed = launch.observe(self.root, release, "windows")
        self.assertFalse(observed.stamp_valid)
        self.assertIn("no matching stamp", launch.decide(observed).reasons[0])

    def test_a_no_op_build_still_advances_the_binary_past_the_sources(self):
        # Cargo.lock newer than the binary with its content unchanged (a
        # checkout): the rule says build; cargo finds nothing to do and leaves
        # the binary's mtime alone. mark_built touches it first, so the next
        # launch is fresh instead of "stale" for good.
        release = self.root / "target" / "release"
        binary = self.touch("target/release/cicada.exe", 5_000_000_000)
        (release / launch.STAMP_NAME).write_text(json.dumps(launch.stamp_for(binary)), encoding="utf-8")
        self.touch("web/dist/index.html", 4_000_000_000)
        (self.root / "web" / "node_modules").mkdir()
        self.touch("Cargo.lock", 6_000_000_000)
        plan = launch.decide(launch.observe(self.root, release, "windows"))
        self.assertEqual(plan.reasons, ("engine sources are newer than the binary",))
        stamp = launch.mark_built(binary, release)
        observed = launch.observe(self.root, release, "windows")
        self.assertTrue(observed.stamp_valid)
        self.assertEqual(observed.binary_mtime_ns, stamp["mtime_ns"])
        self.assertGreater(observed.binary_mtime_ns, 6_000_000_000, "touched past Cargo.lock")
        self.assertTrue(launch.decide(observed).fresh)

    def test_cargo_target_dir_is_read_from_metadata(self):
        self.assertEqual(
            launch.cargo_target_dir(json.dumps({"target_directory": "C:\\x\\cargo-target"})),
            Path("C:\\x\\cargo-target"),
        )
        with self.assertRaisesRegex(launch.LaunchError, "target_directory"):
            launch.cargo_target_dir("{}")
        with self.assertRaisesRegex(launch.LaunchError, "target_directory"):
            launch.cargo_target_dir("not json")


class ToolsTest(unittest.TestCase):
    def test_windows_cmake_dirs_use_program_files_or_their_defaults(self):
        env = {"ProgramFiles(x86)": r"D:\PF86", "ProgramFiles": r"D:\PF"}
        dirs = [PureWindowsPath(d) for d in launch.cmake_candidates("windows", env)]
        self.assertEqual(dirs[0], PureWindowsPath(r"D:\PF86\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"))
        self.assertEqual(dirs[-1], PureWindowsPath(r"D:\PF\CMake\bin"))
        # Git Bash cannot export `ProgramFiles(x86)`: the conventional location stands in.
        dirs = [str(PureWindowsPath(d)) for d in launch.cmake_candidates("windows", {"SystemDrive": "E:"})]
        self.assertTrue(dirs[0].startswith(r"E:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"), dirs[0])
        self.assertTrue(dirs[1].startswith(r"E:\Program Files\Microsoft Visual Studio\2022\Community"), dirs[1])
        self.assertEqual(launch.cmake_candidates("darwin", {}), [Path(d) for d in launch.MACOS_CMAKE_DIRS])
        self.assertEqual(launch.cmake_candidates("linux", {}), [])

    def test_find_tool_looks_on_path_then_in_the_extra_dirs(self):
        root = Path(tempfile.mkdtemp(prefix="cicada-launch-tools-"))
        try:
            on_path = root / "bin"
            extra = root / "extra"
            on_path.mkdir()
            extra.mkdir()
            name = "cmake.exe" if os.name == "nt" else "cmake"
            (extra / name).write_bytes(b"#!/bin/sh\n")
            (extra / name).chmod(0o755)
            env = {"PATH": str(on_path)}
            self.assertEqual(launch.find_tool("cmake", env, [extra]), extra / name)
            self.assertIsNone(launch.find_tool("cmake", env, []))
            self.assertIsNone(launch.find_tool("nothing-here", env, [extra]))
        finally:
            shutil.rmtree(root, ignore_errors=True)

    def test_split_args(self):
        flags, rest = launch.split_args(["--plan", "examples/02-solids.cic", "--no-run", "--help"])
        self.assertEqual(flags, {"--plan", "--no-run"})
        self.assertEqual(rest, ["examples/02-solids.cic", "--help"])

    def test_app_cwd_is_the_callers_with_arguments_and_the_repo_without(self):
        caller, repo = Path("/work/here"), Path("/repo")
        self.assertEqual(launch.app_cwd(["../examples/02-solids.cic"], caller, repo), caller)
        self.assertEqual(launch.app_cwd(["--no-browser"], caller, repo), caller)
        self.assertEqual(launch.app_cwd([], caller, repo), repo, "the double-click serves the repository")


class EnvironmentTest(unittest.TestCase):
    def layout(self, subdir):
        return fo.Layout(Path("/cache"), subdir, "7.8.1")

    def test_windows_build_env_puts_cmake_and_the_library_dir_on_path(self):
        layout = self.layout("win-64")
        env = launch.build_environment({"PATH": r"C:\bin", "HOME": "h"}, layout, "windows", PureWindowsPath(r"C:\VS\cmake\bin"))
        self.assertEqual(env["DEP_OCCT_ROOT"], str(layout.dep_occt_root))
        self.assertEqual(env["CMAKE_POLICY_VERSION_MINIMUM"], fo.CMAKE_POLICY_VERSION_MINIMUM)
        self.assertEqual(env["PATH"], rf"C:\VS\cmake\bin;{layout.library_dir};C:\bin")
        self.assertEqual(env["HOME"], "h")
        self.assertNotIn("RUSTFLAGS", env)
        # cmake already on PATH: nothing prepended for it.
        env = launch.build_environment({"PATH": r"C:\bin"}, layout, "windows", None)
        self.assertEqual(env["PATH"], rf"{layout.library_dir};C:\bin")
        # The kernel build's git needs core.longpaths per process (AGENTS.md:
        # oneTBB's doc assets exceed MAX_PATH; the clone fails without it).
        self.assertEqual(
            (env["GIT_CONFIG_COUNT"], env["GIT_CONFIG_KEY_0"], env["GIT_CONFIG_VALUE_0"]),
            ("1", "core.longpaths", "true"),
        )
        # A caller's own entries are kept; ours is appended after them.
        env = launch.build_environment(
            {"PATH": r"C:\bin", "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "user.name", "GIT_CONFIG_VALUE_0": "x"},
            layout,
            "windows",
            None,
        )
        self.assertEqual(env["GIT_CONFIG_COUNT"], "2")
        self.assertEqual((env["GIT_CONFIG_KEY_0"], env["GIT_CONFIG_VALUE_0"]), ("user.name", "x"))
        self.assertEqual((env["GIT_CONFIG_KEY_1"], env["GIT_CONFIG_VALUE_1"]), ("core.longpaths", "true"))
        with self.assertRaisesRegex(launch.LaunchError, "GIT_CONFIG_COUNT is 'many'"):
            launch.build_environment({"PATH": r"C:\bin", "GIT_CONFIG_COUNT": "many"}, layout, "windows", None)

    def test_macos_build_env_is_an_rpath_never_a_loader_variable(self):
        layout = self.layout("osx-arm64")
        env = launch.build_environment({"PATH": "/usr/bin", "RUSTFLAGS": "-C opt-level=2"}, layout, "darwin", PurePosixPath("/opt/homebrew/bin"))
        self.assertEqual(env["RUSTFLAGS"], f"-C opt-level=2 -C link-arg=-Wl,-rpath,{layout.library_dir}")
        self.assertNotIn("DYLD_LIBRARY_PATH", env)
        self.assertEqual(env["PATH"], "/opt/homebrew/bin:/usr/bin")
        self.assertEqual(env["DEP_OCCT_ROOT"], str(layout.dep_occt_root))
        self.assertNotIn("GIT_CONFIG_COUNT", env, "core.longpaths is a Windows need")

    def test_linux_build_env_uses_ld_library_path(self):
        layout = self.layout("linux-64")
        env = launch.build_environment({"PATH": "/usr/bin", "LD_LIBRARY_PATH": "/old"}, layout, "linux", None)
        self.assertEqual(env["LD_LIBRARY_PATH"], f"{layout.library_dir}:/old")
        self.assertEqual(env["PATH"], "/usr/bin")

    def test_run_env_drops_the_loader_path_and_the_build_variables(self):
        layout = self.layout("win-64")
        lib = str(layout.library_dir)
        base = {
            "PATH": rf"C:\tools;{lib.upper()};C:\Python310",
            "DEP_OCCT_ROOT": "x",
            "CMAKE_POLICY_VERSION_MINIMUM": "3.5",
            "HOME": "h",
        }
        env = launch.run_environment(base, layout, "windows", Path(r"C:\Python310\python.exe"))
        self.assertEqual(env["PATH"], r"C:\tools;C:\Python310")
        self.assertNotIn("DEP_OCCT_ROOT", env)
        self.assertNotIn("CMAKE_POLICY_VERSION_MINIMUM", env)
        self.assertEqual(env["CICADA_PYTHON"], r"C:\Python310\python.exe")
        self.assertEqual(env["HOME"], "h")
        mac = self.layout("osx-64")
        env = launch.run_environment(
            {"PATH": "/usr/bin", "DYLD_LIBRARY_PATH": str(mac.library_dir)}, mac, "darwin", Path("/usr/bin/python3")
        )
        self.assertNotIn("DYLD_LIBRARY_PATH", env, "an empty loader variable is removed, not left empty")
        env = launch.run_environment(
            {"PATH": "/usr/bin", "DYLD_LIBRARY_PATH": f"{mac.library_dir}:/keep"}, mac, "darwin", Path("/usr/bin/python3")
        )
        self.assertEqual(env["DYLD_LIBRARY_PATH"], "/keep")


# ---------------------------------------------------------------------------
# bundle.py
# ---------------------------------------------------------------------------


class PlacesTest(unittest.TestCase):
    def test_windows_layout_is_flat(self):
        spots = bundle.places(Path("dist"), "win-64")
        self.assertEqual(spots.binary, Path("dist/cicada.exe"))
        self.assertEqual(spots.launcher, Path("dist/Cicada.cmd"))
        self.assertIsNone(spots.plist)
        self.assertEqual(spots.readme, Path("dist/README.txt"))
        self.assertEqual(spots.occt_stamp, Path("dist") / fo.BUNDLE_STAMP_NAME)
        self.assertEqual(len(spots.required_files()), 5)

    def test_macos_layout_is_inside_the_app_and_never_collides_case_insensitively(self):
        spots = bundle.places(Path("dist"), "osx-arm64")
        self.assertEqual(spots.binary, Path("dist/Cicada.app/Contents/MacOS/cicada"))
        self.assertEqual(spots.launcher, Path("dist/Cicada.app/Contents/MacOS/Cicada.command"))
        self.assertEqual(spots.plist, Path("dist/Cicada.app/Contents/Info.plist"))
        self.assertEqual(spots.readme, Path("dist/README.txt"))
        names = {p.name.lower() for p in (spots.binary, spots.launcher)}
        self.assertEqual(len(names), 2, "binary and launcher must differ on a case-insensitive disk")
        self.assertEqual(len(spots.required_files()), 6)

    def test_linux_is_refused(self):
        with self.assertRaisesRegex(bundle.BundleError, "linux-64 is not one of them"):
            bundle.places(Path("dist"), "linux-64")


class FilesTest(unittest.TestCase):
    def test_windows_launcher_runs_cicada_app_and_pauses_only_on_failure(self):
        text = bundle.windows_launcher_text()
        self.assertTrue(text.endswith("\r\n"))
        self.assertNotIn("\n", text.replace("\r\n", ""), "CRLF on every line")
        self.assertIn('"%~dp0cicada.exe" app %*', text)
        self.assertIn('if not "%CODE%"=="0" (', text)
        self.assertIn("pause", text)
        self.assertLess(text.index('app %*'), text.index("pause"))
        self.assertTrue(all(ord(c) < 128 for c in text), "cmd.exe reads the OEM code page: ASCII only")

    def test_macos_launcher_reopens_in_terminal_and_runs_the_binary(self):
        text = bundle.macos_launcher_text()
        self.assertTrue(text.startswith("#!/bin/bash\n"))
        self.assertIn('if [ ! -t 1 ]; then\n  exec open -a Terminal "$0"\nfi', text)
        self.assertIn('"$here/cicada" app "$@"', text)
        self.assertIn('read -r -p "Press Return to close this window. "', text)
        self.assertNotIn("\r", text)
        for gnu_only in ("readlink -f", "mapfile", "declare -A", "${", "}"):  # bash 3.2: no associative arrays, no mapfile
            if gnu_only in ("${", "}"):
                continue
            self.assertNotIn(gnu_only, text, gnu_only)

    def test_info_plist_names_the_launcher_as_the_executable(self):
        info = plistlib.loads(bundle.info_plist_text("0.0.1").encode("utf-8"))
        self.assertEqual(info["CFBundleExecutable"], bundle.MACOS_LAUNCHER)
        self.assertEqual(info["CFBundlePackageType"], "APPL")
        self.assertEqual(info["CFBundleName"], "Cicada")
        self.assertEqual(info["CFBundleShortVersionString"], "0.0.1")

    def test_readme_states_the_requirements_per_platform(self):
        windows = bundle.readme_text("win-64", "0.0.1", "abc1234")
        self.assertIn("Cicada 0.0.1", windows)
        self.assertIn("commit abc1234", windows)
        self.assertIn("Double-click Cicada.cmd", windows)
        self.assertIn("CICADA_PYTHON", windows)
        self.assertIn("Visual C++", windows)
        self.assertIn("WORK IN PROGRESS", windows)
        self.assertIn("app built in", windows)
        self.assertNotIn("right-click", windows)
        mac = bundle.readme_text("osx-arm64", "0.0.1", None)
        self.assertIn("Double-click Cicada.app", mac)
        self.assertIn("right-click", mac)
        self.assertNotIn("Visual C++", mac)
        self.assertNotIn("commit", mac)
        # ASCII like the launchers: `type README.txt` in a cp1252 console and Notepad agree.
        for text in (windows, mac):
            self.assertTrue(all(ord(c) < 128 for c in text), sorted({c for c in text if ord(c) >= 128}))
        # The engine-only bundle says so and never claims the app.
        engine_only = bundle.readme_text("win-64", "0.0.1", "abc1234", spa=False)
        self.assertIn("ENGINE ONLY", engine_only)
        self.assertIn("cicada app has nothing to open", engine_only)
        self.assertIn("--features embed", engine_only)
        self.assertNotIn("app built in", engine_only)
        self.assertNotIn("Double-click Cicada.cmd", engine_only)
        self.assertTrue(all(ord(c) < 128 for c in engine_only))

    def test_write_if_changed_is_idempotent(self):
        root = Path(tempfile.mkdtemp(prefix="cicada-bundle-files-"))
        try:
            path = root / "a" / "f.txt"
            self.assertTrue(bundle.write_if_changed(path, "one\n"))
            before = path.stat().st_mtime_ns
            self.assertFalse(bundle.write_if_changed(path, "one\n"))
            self.assertEqual(path.stat().st_mtime_ns, before)
            self.assertTrue(bundle.write_if_changed(path, "two\n"))
            self.assertEqual(path.read_text(encoding="utf-8"), "two\n")
        finally:
            shutil.rmtree(root, ignore_errors=True)


class EnvironmentAndParsingTest(unittest.TestCase):
    def test_clean_environment_windows_is_system32_alone(self):
        env = bundle.clean_environment(
            "windows",
            {
                "SystemRoot": r"C:\Windows",
                "PATH": r"C:\cicada-occt\Library\bin;C:\Windows\System32",
                "DEP_OCCT_ROOT": "x",
                "USERPROFILE": r"D:\Profiles\u",
                "TEMP": r"C:\T",
                "CICADA_PYTHON": "py",
            },
        )
        self.assertEqual(env["PATH"], r"C:\Windows\System32;C:\Windows")
        self.assertEqual(env["SystemRoot"], r"C:\Windows")
        self.assertEqual(env["USERPROFILE"], r"D:\Profiles\u")
        self.assertEqual(env["TEMP"], r"C:\T")
        self.assertNotIn("DEP_OCCT_ROOT", env)
        self.assertNotIn("CICADA_PYTHON", env)

    def test_clean_environment_unix_has_the_system_path_and_no_loader_variable(self):
        env = bundle.clean_environment("darwin", {"PATH": "/opt/x:/usr/bin", "DYLD_LIBRARY_PATH": "/lib", "HOME": "/Volumes/Data/u"})
        self.assertEqual(env["PATH"], "/usr/bin:/bin:/usr/sbin:/sbin")
        self.assertEqual(env["HOME"], "/Volumes/Data/u")
        self.assertNotIn("DYLD_LIBRARY_PATH", env)

    def test_parse_url_line(self):
        self.assertEqual(
            bundle.parse_url_line("cicada app \u2014 http://127.0.0.1:51234/?token=t&pipeline=demo.cic"),
            "http://127.0.0.1:51234/?token=t&pipeline=demo.cic",
        )
        self.assertEqual(bundle.parse_url_line("cicada serve \u2014 http://127.0.0.1:8420/?token=t\n"), "http://127.0.0.1:8420/?token=t")
        for other in ("  Ctrl-C stops the server; the store lives in x.", "cicada app", "hello \u2014 world", ""):
            self.assertIsNone(bundle.parse_url_line(other), other)

    def test_default_binary(self):
        self.assertEqual(bundle.default_binary(Path("t"), "windows"), Path("t/release/cicada.exe"))
        self.assertEqual(bundle.default_binary(Path("t"), "darwin"), Path("t/release/cicada"))


class MakeAndCheckTest(unittest.TestCase):
    """`make_bundle` and `check_bundle` over synthetic prefixes and binaries;
    every external program answers through an injected runner."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cicada-bundle-"))
        self.manifest = {"occt_version": "7.8.1", "_sha256": "m" * 64}
        self.messages = []
        self.calls = []

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def log(self, message):
        self.messages.append(message)

    def windows_prefix(self):
        layout = fo.Layout(self.root / "cache", "win-64", "7.8.1")
        layout.library_dir.mkdir(parents=True)
        (layout.library_dir / "TKernel.dll").write_bytes(fake_pe(["KERNEL32.dll", "MSVCP140.dll"]))
        (layout.library_dir / "TKBO.dll").write_bytes(fake_pe(["TKernel.dll", "KERNEL32.dll"]) + b"\0" * 100)
        return layout

    def windows_binary(self, name="cicada.exe", extra=b"", spa=True):
        build = self.root / "target" / "release"
        build.mkdir(parents=True, exist_ok=True)
        binary = build / name
        binary.write_bytes(fake_pe(["TKernel.dll", "TKBO.dll", "KERNEL32.dll", "combase.dll"]) + (SPA if spa else b"") + extra)
        return binary

    def runner(self, help_code=0, help_stdout="Cicada: code-first parametric design\n\nCommands:\n  catalog\n  run\n  serve\n  app\n  mcp\n", version="cicada 0.0.1\n"):
        def run(argv, **kwargs):
            self.calls.append((list(argv), kwargs.get("env")))
            if argv[-1] == "--version":
                return completed(argv, 0, version)
            if argv[-1] == "--help":
                return completed(argv, help_code, help_stdout, "" if help_code == 0 else "boom")
            return completed(argv, 0, "")

        return run

    def test_windows_bundle_is_made_checked_and_idempotent(self):
        layout = self.windows_prefix()
        binary = self.windows_binary()
        out = self.root / "dist"
        environ = {"SystemRoot": r"C:\Windows", "PATH": r"C:\somewhere", "DEP_OCCT_ROOT": "x"}
        spots = bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner(), commit="abc1234")
        self.assertEqual(spots.binary, out / "cicada.exe")
        self.assertEqual((out / "cicada.exe").read_bytes(), binary.read_bytes())
        for name in ("TKernel.dll", "TKBO.dll", "Cicada.cmd", "README.txt", bundle.STAMP_NAME, fo.BUNDLE_STAMP_NAME):
            self.assertTrue((out / name).is_file(), name)
        self.assertEqual((out / "Cicada.cmd").read_bytes().decode("utf-8"), bundle.windows_launcher_text())
        readme = (out / "README.txt").read_text(encoding="utf-8")
        self.assertIn("Cicada 0.0.1", readme)
        self.assertIn("commit abc1234", readme)
        stamp = json.loads((out / bundle.STAMP_NAME).read_text(encoding="utf-8"))
        self.assertEqual(stamp["version"], "0.0.1")
        self.assertEqual(stamp["subdir"], "win-64")
        self.assertIs(stamp["spa"], True)
        self.assertEqual(stamp["binary_source"]["size"], binary.stat().st_size)
        # --version ran on the BUNDLED copy under the clean environment.
        version_calls = [(argv, env) for argv, env in self.calls if argv[-1] == "--version"]
        self.assertEqual(len(version_calls), 1)
        self.assertEqual(version_calls[0][0][0], str(out / "cicada.exe"))
        self.assertEqual(version_calls[0][1]["PATH"], r"C:\Windows\System32;C:\Windows")
        self.assertNotIn("DEP_OCCT_ROOT", version_calls[0][1])
        # The check passes, and says what it proved.
        self.messages.clear()
        self.calls.clear()
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=self.runner()), [])
        self.assertTrue(any("2 libraries present at their recorded sizes" in m for m in self.messages), self.messages)
        self.assertTrue(any("every import resolves" in m for m in self.messages), self.messages)
        self.assertTrue(any(m == "cicada.exe embeds the SPA" for m in self.messages), self.messages)
        self.assertTrue(any("--help answers from inside the bundle" in m for m in self.messages), self.messages)
        help_calls = [(argv, kw) for argv, kw in self.calls if argv[-1] == "--help"]
        self.assertEqual(help_calls[0][0], [str(out / "cicada.exe"), "--help"])
        self.assertEqual(help_calls[0][1]["PATH"], r"C:\Windows\System32;C:\Windows")
        # Idempotent: a second make changes no file (bytes AND mtimes).
        before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in out.iterdir()}
        self.messages.clear()
        bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner(), commit="abc1234")
        after = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in out.iterdir()}
        self.assertEqual(after, before)
        self.assertTrue(any("unchanged" in m for m in self.messages), self.messages)
        self.assertTrue(any("source unchanged" in m for m in self.messages), self.messages)
        # A rebuilt source binary is copied again.
        binary.write_bytes(binary.read_bytes() + b"\0" * 16)
        bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner(), commit="abc1234")
        self.assertEqual((out / "cicada.exe").read_bytes(), binary.read_bytes())

    def test_check_names_every_problem(self):
        layout = self.windows_prefix()
        binary = self.windows_binary()
        out = self.root / "dist"
        environ = {"SystemRoot": r"C:\Windows"}
        bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner())
        # A missing launcher.
        (out / "Cicada.cmd").unlink()
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=self.runner()), ["missing Cicada.cmd"])
        (out / "Cicada.cmd").write_bytes(bundle.windows_launcher_text().encode("utf-8"))
        # A truncated library, a missing one.
        (out / "TKBO.dll").write_bytes(b"x")
        (out / "TKernel.dll").unlink()
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner())
        self.assertEqual(len(problems), 2)
        self.assertTrue(problems[0].startswith("library size differs: TKBO.dll"), problems)
        self.assertEqual(problems[1], "library missing: TKernel.dll")
        bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner())
        # The binary does not answer --help under the clean environment.
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner(help_code=127))
        self.assertEqual(len(problems), 1)
        self.assertIn("--help exited 127 under a clean environment: boom", problems[0])
        # --help answers but lists no `app`: the launchers would fail.
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner(help_stdout="Commands:\n  run\n"))
        self.assertEqual(len(problems), 1)
        self.assertIn("does not list `app`", problems[0])
        # A binary importing something the bundle lacks.
        (out / "cicada.exe").write_bytes(fake_pe(["TKernel.dll", "nowhere.dll"]) + SPA)
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner())
        self.assertEqual(problems, ["cicada.exe imports nowhere.dll, which neither the bundle nor the OS provides"])
        # A binary swapped in after the bundle was made -- a plain release
        # build without the SPA, where the stamp and the README say the app is
        # in: every check above passes, the first double-click would die.
        (out / "cicada.exe").write_bytes(fake_pe(["TKernel.dll", "TKBO.dll", "KERNEL32.dll", "combase.dll"]))
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner())
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("cicada.exe embeds no SPA but .cicada-bundle.json says it does", problems[0])
        # `--out` repairs it: the bundled copy's size no longer matches the recorded source.
        self.messages.clear()
        bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner())
        self.assertTrue(any(m.startswith("copied ") for m in self.messages), self.messages)
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=self.runner()), [])
        # A stamp from before the SPA was recorded: remake, never guess.
        stamp_path = out / bundle.STAMP_NAME
        stamp = json.loads(stamp_path.read_text(encoding="utf-8"))
        del stamp["spa"]
        stamp_path.write_text(json.dumps(stamp), encoding="utf-8")
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner())
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("does not record whether the SPA is embedded", problems[0])
        # Not a bundle at all.
        with self.assertRaisesRegex(bundle.BundleError, "is not a bundle"):
            bundle.check_bundle(self.root / "empty-nowhere", self.log, environ, run=self.runner())

    def test_make_refuses_a_missing_binary_and_a_binary_that_cannot_start(self):
        layout = self.windows_prefix()
        with self.assertRaisesRegex(bundle.BundleError, r"no release binary at .*--features embed"):
            bundle.make_bundle(self.root / "nope.exe", self.root / "dist", layout, self.manifest, self.log, {}, run=self.runner())
        self.assertFalse((self.root / "dist").exists(), "a refusal before the copy leaves nothing behind")
        binary = self.windows_binary()

        def dead(argv, **kwargs):
            return completed(argv, 127, "", "STATUS_DLL_NOT_FOUND")

        with self.assertRaisesRegex(bundle.BundleError, r"--version exited 127 from inside the bundle.*STATUS_DLL_NOT_FOUND"):
            bundle.make_bundle(binary, self.root / "dist", layout, self.manifest, self.log, {"SystemRoot": "C:\\W"}, run=dead)

    def test_make_refuses_a_binary_without_the_spa_unless_allowed(self):
        # A plain `cargo build --release` (no `--features embed`): its bundle's
        # launcher would die at the first double-click.
        layout = self.windows_prefix()
        binary = self.windows_binary(spa=False)
        out = self.root / "dist"
        environ = {"SystemRoot": r"C:\Windows"}
        with self.assertRaisesRegex(bundle.BundleError, r"embeds no SPA.*Cicada\.cmd runs.*--features embed.*--allow-no-spa"):
            bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner())
        self.assertFalse(out.exists(), "a refusal before the copy leaves nothing behind")
        # Allowed: an engine-only bundle that says so everywhere.
        spots = bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=self.runner(), allow_no_spa=True)
        readme = spots.readme.read_text(encoding="utf-8")
        self.assertIn("ENGINE ONLY", readme)
        self.assertNotIn("app built in", readme)
        self.assertIs(json.loads(spots.stamp.read_text(encoding="utf-8"))["spa"], False)
        self.assertTrue(any("engine only -- no SPA" in m for m in self.messages), self.messages)
        self.messages.clear()
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=self.runner()), [])
        self.assertTrue(any("embeds no SPA (engine only, as the bundle records)" in m for m in self.messages), self.messages)

        # The smoke has nothing to open and says so before starting anything.
        def never(*args, **kwargs):
            raise AssertionError("the smoke started a process on an engine-only bundle")

        with self.assertRaisesRegex(bundle.BundleError, r"engine-only bundle .*says the SPA is not embedded"):
            bundle.smoke(out, self.log, environ, start=never, get=never)
        # The stamp that embeds the SPA but a binary that does not: caught.
        (out / "cicada.exe").write_bytes(self.windows_binary(name="other.exe").read_bytes())
        problems = bundle.check_bundle(out, self.log, environ, run=self.runner())
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("cicada.exe embeds the SPA but .cicada-bundle.json says it does not", problems[0])

    def macos_prefix(self):
        layout = fo.Layout(self.root / "cache", "osx-arm64", "7.8.1")
        layout.library_dir.mkdir(parents=True)
        (layout.library_dir / "libTKernel.7.8.dylib").write_bytes(fake_macho(["/usr/lib/libSystem.B.dylib"]))
        (layout.library_dir / "libTKBO.7.8.dylib").write_bytes(
            fake_macho(["@rpath/libTKernel.7.8.dylib", "/usr/lib/libSystem.B.dylib"])
        )
        return layout

    def test_macos_bundle_goes_inside_the_app(self):
        layout = self.macos_prefix()
        build = self.root / "target" / "release"
        build.mkdir(parents=True)
        binary = build / "cicada"
        binary.write_bytes(
            fake_macho(["@rpath/libTKernel.7.8.dylib", "/usr/lib/libSystem.B.dylib"], rpaths=[str(layout.library_dir)]) + SPA
        )
        out = self.root / "dist"

        def run(argv, **kwargs):
            self.calls.append(list(argv))
            if argv[0] == "install_name_tool":
                # The real tool rewrites the load command; the fake rewrites the file the same way.
                target = Path(argv[-1])
                if argv[1] == "-rpath":
                    target.write_bytes(
                        fake_macho(["@rpath/libTKernel.7.8.dylib", "/usr/lib/libSystem.B.dylib"], rpaths=[argv[3]]) + SPA
                    )
                return completed(argv, 0)
            if argv[-1] == "--version":
                return completed(argv, 0, "cicada 0.0.1\n")
            if argv[-1] == "--help":
                return completed(argv, 0, "Commands:\n  app\n")
            return completed(argv, 0)

        environ = {"HOME": "/Volumes/Data/u", "PATH": "/opt/x"}
        spots = bundle.make_bundle(binary, out, layout, self.manifest, self.log, environ, run=run, which=lambda tool: f"/usr/bin/{tool}")
        macos = out / "Cicada.app" / "Contents" / "MacOS"
        self.assertEqual(spots.binary, macos / "cicada")
        self.assertTrue((macos / "lib" / "libTKernel.7.8.dylib").is_file())
        self.assertTrue((macos / "Cicada.command").is_file())
        self.assertTrue((out / "Cicada.app" / "Contents" / "Info.plist").is_file())
        self.assertTrue((out / "README.txt").is_file())
        self.assertFalse((out / "cicada").exists(), "nothing of the app lives outside Cicada.app")
        self.assertEqual(
            [c for c in self.calls if c[0] in ("install_name_tool", "codesign")],
            [
                ["install_name_tool", "-rpath", str(layout.library_dir), "@executable_path/lib", str(macos / "cicada")],
                ["codesign", "--force", "--sign", "-", str(macos / "cicada")],
            ],
        )
        self.assertTrue(any(m.startswith("rpaths before: [") for m in self.messages), self.messages)
        self.assertTrue(any(m == "rpaths after: ['@executable_path/lib']" for m in self.messages), self.messages)
        if os.name != "nt":
            self.assertTrue(os.access(macos / "Cicada.command", os.X_OK))
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=run), [])
        self.assertEqual(bundle.detect_places(out).subdir, "osx-arm64")
        # Info.plist naming the binary, not the launcher, as the executable:
        # Finder would exec `cicada` with no arguments and no console. And a
        # plist that does not parse. Both are the static evidence the .app has.
        plist = out / "Cicada.app" / "Contents" / "Info.plist"
        good = plist.read_bytes()
        info = plistlib.loads(good)
        info["CFBundleExecutable"] = "Cicada"
        plist.write_bytes(plistlib.dumps(info))
        problems = bundle.check_bundle(out, self.log, environ, run=run)
        self.assertEqual(problems, ["Info.plist names 'Cicada' as the executable, not Cicada.command"])
        plist.write_bytes(b"<plist><dict><key>x</key>")
        problems = bundle.check_bundle(out, self.log, environ, run=run)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("Info.plist does not parse", problems[0])
        plist.write_bytes(good)
        self.assertEqual(bundle.check_bundle(out, self.log, environ, run=run), [])
        # A binary that kept the prefix rpath fails the check.
        (macos / "cicada").write_bytes(
            fake_macho(["@rpath/libTKernel.7.8.dylib", "/usr/lib/libSystem.B.dylib"], rpaths=[str(layout.library_dir)]) + SPA
        )
        problems = bundle.check_bundle(out, self.log, environ, run=run)
        self.assertEqual(len(problems), 2, problems)
        self.assertIn("carries no @executable_path/lib rpath", problems[0])
        self.assertIn("still carries the build prefix's rpath", problems[1])

    def test_cli_refusals_are_on_stderr(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = bundle.main(["--check", str(self.root / "nothing")])
        self.assertEqual(code, 1)
        self.assertIn("is not a bundle", stderr.getvalue())
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = bundle.main(["--out", str(self.root / "dist"), "--smoke"])
        self.assertEqual(code, 1)
        self.assertIn("--smoke goes with --check", stderr.getvalue())
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = bundle.main(["--check", str(self.root / "dist"), "--allow-no-spa"])
        self.assertEqual(code, 1)
        self.assertIn("--allow-no-spa goes with --out", stderr.getvalue())


class FakePipe:
    """A child's stderr as a pipe behaves: `read()` returns at EOF, which is
    when the process has exited -- so whoever builds a message from it must
    wait for the reader, as `Console.stop` does."""

    def __init__(self, text, exited):
        self.text = text
        self.exited = exited

    def read(self):
        self.exited.wait(timeout=bundle.LINE_TIMEOUT_SECONDS)
        return self.text


class FakeProcess:
    """What `smoke` needs of a Popen: stdout lines, stderr, the exit protocol."""

    def __init__(self, lines, stderr="", code=0):
        self.exited = threading.Event()
        self.stdout = iter(lines)
        self.stderr = FakePipe(stderr, self.exited)
        self.returncode = None
        self.exit_code = code
        self.terminated = False

    def poll(self):
        return self.returncode

    def terminate(self):
        self.terminated = True
        self.returncode = self.exit_code
        self.exited.set()

    def kill(self):
        self.returncode = self.exit_code
        self.exited.set()

    def wait(self, timeout=None):
        if self.returncode is None:
            self.returncode = self.exit_code
        self.exited.set()
        return self.returncode


class SmokeTest(unittest.TestCase):
    """The smoke's assertions over a fake server (injected `start` / `get`):
    the URL line, `/health`, `/` the SPA and never the API-only page, the
    clean environment, the stop. The real smoke on a release bundle is the
    launcher work's own evidence; this holds its ASSERTIONS to a test."""

    URL_LINE = "cicada app \u2014 http://127.0.0.1:51234/?token=t&pipeline=demo.cic"
    SPA_PAGE = '<!doctype html><html><head><title>Cicada</title></head><body><div id="root"></div></body></html>'
    API_ONLY = "<!doctype html><meta charset=utf-8><title>cicada serve</title><body><h1>cicada serve \u2014 API only</h1></body>"

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cicada-smoke-test-"))
        self.messages = []
        self.started = []
        self.requested = []
        layout = fo.Layout(self.root / "cache", "win-64", "7.8.1")
        layout.library_dir.mkdir(parents=True)
        (layout.library_dir / "TKernel.dll").write_bytes(fake_pe(["KERNEL32.dll"]))
        build = self.root / "target" / "release"
        build.mkdir(parents=True)
        binary = build / "cicada.exe"
        binary.write_bytes(fake_pe(["TKernel.dll", "KERNEL32.dll"]) + SPA)
        self.out = self.root / "dist"
        self.environ = {"SystemRoot": r"C:\Windows", "PATH": r"C:\cicada-occt\Library\bin;C:\Windows\System32"}

        def run(argv, **kwargs):
            return completed(argv, 0, "cicada 0.0.1\n")

        bundle.make_bundle(binary, self.out, layout, {"occt_version": "7.8.1", "_sha256": "m" * 64}, self.messages.append, self.environ, run=run)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def start_with(self, lines, stderr="", code=0):
        def start(argv, **kwargs):
            process = FakeProcess(lines, stderr, code)
            self.started.append((list(argv), kwargs, process))
            return process

        return start

    def get_with(self, health=(200, "ok\n"), root=None):
        root = (200, self.SPA_PAGE) if root is None else root

        def get(url):
            self.requested.append(url)
            return health if "/health" in url else root

        return get

    def test_the_smoke_passes_on_a_server_that_answers_health_and_the_spa(self):
        url = bundle.smoke(
            self.out,
            self.messages.append,
            self.environ,
            python="py.exe",
            start=self.start_with(["  Ctrl-C stops the server.", self.URL_LINE]),
            get=self.get_with(),
        )
        self.assertEqual(url, "http://127.0.0.1:51234/?token=t&pipeline=demo.cic")
        argv, kwargs, process = self.started[0]
        self.assertEqual(argv[:3], [str(self.out / "cicada.exe"), "app", "--no-browser"])
        self.assertEqual(argv[argv.index("--port") + 1], "0")
        token = argv[argv.index("--token") + 1]
        self.assertTrue(argv[-1].endswith("demo.cic"))
        self.assertEqual(kwargs["cwd"], str(self.out))
        self.assertEqual(kwargs["env"]["PATH"], r"C:\Windows\System32;C:\Windows", "the clean env: the bundle is what makes it start")
        self.assertEqual(kwargs["env"]["CICADA_PYTHON"], "py.exe")
        self.assertEqual(
            self.requested, [f"http://127.0.0.1:51234/health?token={token}", f"http://127.0.0.1:51234/?token={token}"]
        )
        self.assertTrue(process.terminated, "the server is stopped")
        self.assertTrue(any("/health -> ok" in m for m in self.messages), self.messages)
        self.assertTrue(any("/ is the SPA" in m for m in self.messages), self.messages)

    def test_the_smoke_fails_on_a_bad_health_the_api_only_page_or_an_early_exit(self):
        with self.assertRaisesRegex(bundle.BundleError, r"GET /health answered 500 'boom', not 200 ok"):
            bundle.smoke(self.out, self.messages.append, self.environ, start=self.start_with([self.URL_LINE]), get=self.get_with(health=(500, "boom")))
        self.assertTrue(self.started[-1][2].terminated, "stopped on failure too")
        for root in ((200, self.API_ONLY), (200, "ok"), (404, self.SPA_PAGE)):
            with self.assertRaisesRegex(bundle.BundleError, rf"GET / answered {root[0]} and is not the SPA"):
                bundle.smoke(self.out, self.messages.append, self.environ, start=self.start_with([self.URL_LINE]), get=self.get_with(root=root))
        self.requested.clear()
        with self.assertRaisesRegex(bundle.BundleError, r"the server exited \(1\) before the URL line; stderr:\nError: nothing to open"):
            bundle.smoke(
                self.out,
                self.messages.append,
                self.environ,
                start=self.start_with(["a line without a URL"], stderr="Error: nothing to open\n", code=1),
                get=self.get_with(),
            )
        self.assertEqual(self.requested, [], "nothing is fetched from a server that never printed its URL")


if __name__ == "__main__":
    unittest.main()
