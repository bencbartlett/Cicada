"""Offline tests for tools/fetch_manifold.py: the pin is held to Cargo.lock and to the
-sys crate's own build script, the env is spelled right per shell, and the stamp
check refuses every stale or partial prefix. No network, no cmake."""

from __future__ import annotations

import json
import os
import re
import sys
import tempfile
import unittest
from pathlib import Path, PurePosixPath, PureWindowsPath

TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_DIR = os.path.dirname(TOOLS_DIR)
sys.path.insert(0, TOOLS_DIR)

import fetch_manifold as fm  # noqa: E402


def cargo_lock_version(crate: str) -> str:
    lock = Path(REPO_DIR, "Cargo.lock").read_text(encoding="utf-8")
    match = re.search(r'\[\[package\]\]\nname = "%s"\nversion = "([^"]+)"' % re.escape(crate), lock)
    assert match, f"{crate} is not in Cargo.lock"
    return match.group(1)


def registry_sys_crate(version: str) -> Path | None:
    """The -sys crate's source in the local cargo registry, when it has been fetched."""
    registry = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry" / "src"
    for candidate in registry.glob(f"*/manifold-csg-sys-{version}"):
        if (candidate / "build.rs").is_file():
            return candidate
    return None


class ManifestPins(unittest.TestCase):
    def setUp(self):
        self.manifest = fm.load_manifest()

    def test_manifest_matches_the_lock(self):
        self.assertEqual(self.manifest["manifold_csg_sys"], cargo_lock_version("manifold-csg-sys"))

    def test_parallel_matches_our_feature_set(self):
        # cicada-geom takes manifold-csg with default features -> `parallel` -> TBB is linked,
        # so the prebuilt must be a parallel build or the link fails on `tbb`.
        csg_toml = Path(REPO_DIR, "crates", "cicada-geom", "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn("manifold-csg", csg_toml)
        self.assertNotIn("default-features = false", csg_toml.split("manifold-csg")[1].split("\n")[0])
        self.assertTrue(self.manifest["parallel"])

    def test_tag_and_flags_match_the_sys_crate_when_its_source_is_on_disk(self):
        crate = registry_sys_crate(self.manifest["manifold_csg_sys"])
        if crate is None:
            # CI's `rust` job sets CICADA_REQUIRE_SYS_CRATE=1 after `cargo build` (the
            # registry is warm there): a skip would hide a drifted pin, so it FAILS.
            if os.environ.get("CICADA_REQUIRE_SYS_CRATE"):
                self.fail("CICADA_REQUIRE_SYS_CRATE is set but manifold-csg-sys's source is not in the cargo registry")
            self.skipTest("manifold-csg-sys source is not in the local cargo registry")
        build_rs = (crate / "build.rs").read_text(encoding="utf-8")
        tag = re.search(r'MANIFOLD_VERSION:\s*&str\s*=\s*"([^"]+)"', build_rs)
        self.assertIsNotNone(tag, "the -sys crate no longer pins MANIFOLD_VERSION where this test looks")
        self.assertEqual(tag.group(1), self.manifest["tag"])
        inner = (crate / "build" / "build.rs").read_text(encoding="utf-8")
        theirs = set(re.findall(r'"(-D[A-Z0-9_]+=[A-Za-z0-9_]+)"', inner))
        ours = set(self.manifest["cmake_flags"])
        # Every flag we pass is one the crate passes on its host path …
        self.assertTrue(ours <= theirs, f"flags not in the -sys crate's build: {sorted(ours - theirs)}")
        # … and every host flag of theirs is ours, except the alternatives of the branches we do not take.
        alternatives = {"-DMANIFOLD_PAR=OFF"}
        wasm_only = {f for f in theirs if "wasm" in f.lower()}
        self.assertEqual(theirs - alternatives - wasm_only, ours, "the -sys crate's cmake flags changed; re-pin the manifest")
        # No carry patches: a plain upstream checkout matches the crate's build.
        self.assertFalse((crate / "patches").exists() and any((crate / "patches").iterdir()), "the -sys crate carries patches this script does not apply")

    def test_validate_refuses_a_parallel_lie(self):
        manifest = dict(self.manifest)
        manifest["parallel"] = False
        with self.assertRaises(fm.FetchError):
            fm.validate_manifest(manifest)

    def test_manifest_hash_is_of_the_bytes(self):
        import hashlib

        self.assertEqual(self.manifest["_sha256"], hashlib.sha256(fm.MANIFEST_PATH.read_bytes()).hexdigest())


class Environment(unittest.TestCase):
    def layout(self, subdir: str) -> fm.Layout:
        # Pure paths: a Windows layout spells itself with backslashes and a POSIX one
        # with slashes on EVERY test host (a `Path(r"C:\…")` on Linux is a POSIX path
        # with a literal backslash in its name — the first CI run caught that).
        root = PureWindowsPath(r"C:\cache\cicada-manifold") if subdir == "win-64" else PurePosixPath("/home/x/.cache/cicada-manifold")
        return fm.Layout(root, subdir, "v3.5.2")  # type: ignore[arg-type]

    def test_bash_lines_on_windows_use_forward_slashes(self):
        lines = fm.env_lines(self.layout("win-64"), "bash")
        self.assertEqual(lines, ["export MANIFOLD_CSG_LIB_DIR='C:/cache/cicada-manifold/manifold-v3.5.2-win-64/lib'", "export MANIFOLD_CSG_LIB_KIND='static'"])

    def test_powershell_and_cmd_keep_the_native_path(self):
        layout = self.layout("win-64")
        self.assertEqual(fm.env_lines(layout, "powershell")[0], r"$env:MANIFOLD_CSG_LIB_DIR = 'C:\cache\cicada-manifold\manifold-v3.5.2-win-64\lib'")
        self.assertEqual(fm.env_lines(layout, "cmd"), [r"set MANIFOLD_CSG_LIB_DIR=C:\cache\cicada-manifold\manifold-v3.5.2-win-64\lib", "set MANIFOLD_CSG_LIB_KIND=static"])

    def test_unix_lines(self):
        self.assertEqual(
            fm.env_lines(self.layout("linux-64"), "bash"),
            ["export MANIFOLD_CSG_LIB_DIR='/home/x/.cache/cicada-manifold/manifold-v3.5.2-linux-64/lib'", "export MANIFOLD_CSG_LIB_KIND='static'"],
        )

    def test_unknown_shell_is_refused(self):
        with self.assertRaises(fm.FetchError):
            fm.env_lines(self.layout("linux-64"), "fish")

    def test_github_env_entries(self):
        self.assertEqual(
            fm.github_env_entries(self.layout("linux-64")),
            ["MANIFOLD_CSG_LIB_DIR=/home/x/.cache/cicada-manifold/manifold-v3.5.2-linux-64/lib", "MANIFOLD_CSG_LIB_KIND=static"],
        )

    def test_static_filenames_follow_the_sys_crate_table(self):
        self.assertEqual(fm.static_filename("manifold", "win-64"), "manifold.lib")
        self.assertEqual(fm.static_filename("Clipper2", "linux-64"), "libClipper2.a")
        self.assertEqual(fm.static_filename("tbb12", "osx-arm64"), "libtbb12.a")

    def test_subdir_detection_matches_conda_names(self):
        self.assertEqual(fm.detect_subdir("Windows", "AMD64"), "win-64")
        self.assertEqual(fm.detect_subdir("Darwin", "arm64"), "osx-arm64")
        self.assertEqual(fm.detect_subdir("Darwin", "x86_64"), "osx-64")
        self.assertEqual(fm.detect_subdir("Linux", "x86_64"), "linux-64")
        with self.assertRaises(fm.FetchError):
            fm.detect_subdir("Linux", "aarch64")

    def test_default_cache_root(self):
        self.assertEqual(fm.default_cache_root({"LOCALAPPDATA": r"C:\L"}, "Windows"), Path(r"C:\L") / "cicada-manifold")
        self.assertEqual(fm.default_cache_root({"XDG_CACHE_HOME": "/x"}, "Linux"), Path("/x/cicada-manifold"))
        self.assertEqual(fm.default_cache_root({"HOME": "/home/u"}, "Darwin"), Path("/home/u/.cache/cicada-manifold"))
        with self.assertRaises(fm.FetchError):
            fm.default_cache_root({}, "Windows")


class Cmake(unittest.TestCase):
    def test_path_wins(self):
        self.assertEqual(fm.find_cmake({"PATH": "/bin"}, which=lambda name, path=None: "/bin/cmake"), "/bin/cmake")

    def test_visual_studio_fallback(self):
        env = {"PATH": "", "ProgramFiles(x86)": r"C:\PF86"}
        expected = Path(r"C:\PF86") / "Microsoft Visual Studio" / "2022" / "BuildTools" / fm.VS_CMAKE_SUFFIX
        found = fm.find_cmake(env, which=lambda name, path=None: None, exists=lambda p: Path(p) == expected)
        self.assertEqual(found, str(expected))

    def test_missing_cmake_is_a_refusal_that_says_where_it_looked(self):
        with self.assertRaises(fm.FetchError) as caught:
            fm.find_cmake({"PATH": "", "ProgramFiles(x86)": r"C:\PF86"}, which=lambda name, path=None: None, exists=lambda p: False)
        self.assertIn("BuildTools", str(caught.exception))

    def test_git_env_appends_longpaths_without_clobbering(self):
        env = fm.git_env({"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "a", "GIT_CONFIG_VALUE_0": "b"})
        self.assertEqual(env["GIT_CONFIG_COUNT"], "2")
        self.assertEqual((env["GIT_CONFIG_KEY_0"], env["GIT_CONFIG_VALUE_0"]), ("a", "b"))
        self.assertEqual((env["GIT_CONFIG_KEY_1"], env["GIT_CONFIG_VALUE_1"]), ("core.longpaths", "true"))


class Stamp(unittest.TestCase):
    def setUp(self):
        self.manifest = fm.load_manifest()
        self.tmp = tempfile.TemporaryDirectory()
        self.layout = fm.Layout(Path(self.tmp.name), "linux-64", self.manifest["tag"])
        self.layout.lib_dir.mkdir(parents=True)
        self.libraries = {}
        for name in ("libmanifold.a", "libmanifoldc.a", "libClipper2.a", "libtbb12.a"):
            (self.layout.lib_dir / name).write_bytes(b"x" * (100 + len(name)))
            self.libraries[name] = 100 + len(name)
        fm.write_stamp(self.manifest, self.layout, self.libraries, "/usr/bin/cmake")

    def tearDown(self):
        self.tmp.cleanup()

    def test_warm_prefix_has_no_problems(self):
        self.assertEqual(fm.prefix_problems(self.manifest, self.layout), [])

    def test_missing_stamp(self):
        self.layout.stamp.unlink()
        self.assertIn("no stamp", fm.prefix_problems(self.manifest, self.layout)[0])

    def test_resized_archive(self):
        (self.layout.lib_dir / "libmanifold.a").write_bytes(b"y")
        problems = fm.prefix_problems(self.manifest, self.layout)
        self.assertTrue(any("libmanifold.a is 1 bytes" in p for p in problems), problems)

    def test_missing_archive(self):
        (self.layout.lib_dir / "libClipper2.a").unlink()
        self.assertTrue(any("missing" in p and "libClipper2.a" in p for p in fm.prefix_problems(self.manifest, self.layout)))

    def test_stale_pin(self):
        stamp = json.loads(self.layout.stamp.read_text(encoding="utf-8"))
        stamp["commit"] = "0" * 40
        self.layout.stamp.write_text(json.dumps(stamp), encoding="utf-8")
        self.assertTrue(any("commit" in p for p in fm.prefix_problems(self.manifest, self.layout)))

    def test_parallel_build_without_tbb_is_stale(self):
        stamp = json.loads(self.layout.stamp.read_text(encoding="utf-8"))
        del stamp["libraries"]["libtbb12.a"]
        self.layout.stamp.write_text(json.dumps(stamp), encoding="utf-8")
        self.assertTrue(any("TBB" in p for p in fm.prefix_problems(self.manifest, self.layout)))

    def test_env_lines_if_present_are_silent_when_warm_and_a_note_when_not(self):
        notes = []
        lines = fm.env_lines_if_present("bash", notes.append, cache_root=Path(self.tmp.name), subdir="linux-64")
        self.assertEqual(len(lines), 2)
        self.assertEqual(notes, [])
        empty_root = Path(self.tmp.name) / "empty"
        self.assertEqual(fm.env_lines_if_present("bash", notes.append, cache_root=empty_root, subdir="linux-64"), [])
        self.assertEqual(len(notes), 1)
        self.assertIn("fetch_manifold.py", notes[0])

    def test_every_pinned_key_of_the_stamp_is_compared(self):
        for key, stale in (("tag", "v0.0.0"), ("commit", "0" * 40), ("manifold_csg_sys", "0.0.0"), ("cmake_flags", ["-DX=Y"]), ("parallel", False)):
            stamp = json.loads(self.layout.stamp.read_text(encoding="utf-8"))
            stamp[key] = stale
            self.layout.stamp.write_text(json.dumps(stamp), encoding="utf-8")
            problems = fm.prefix_problems(self.manifest, self.layout)
            self.assertTrue(any(key in p for p in problems), f"{key}: {problems}")
            fm.write_stamp(self.manifest, self.layout, self.libraries, "/usr/bin/cmake")

    def test_validate_refuses_a_wrong_required_set(self):
        manifest = json.loads(json.dumps(self.manifest))
        manifest["libraries"]["required"] = ["manifold", "manifoldc"]
        with self.assertRaises(fm.FetchError):
            fm.validate_manifest(manifest)

    def test_malformed_manifest_is_a_refusal(self):
        bad = Path(self.tmp.name) / "bad.json"
        bad.write_text("{not json", encoding="utf-8")
        with self.assertRaises(fm.FetchError) as caught:
            fm.load_manifest(bad)
        self.assertIn("not valid JSON", str(caught.exception))


class Ensure(unittest.TestCase):
    """The decision: a warm prefix builds nothing; a stale one runs clone -> configure ->
    harvest -> stamp and re-verifies; a harvest that lies about sizes is refused."""

    def setUp(self):
        self.manifest = fm.load_manifest()
        self.tmp = tempfile.TemporaryDirectory()
        self.layout = fm.Layout(Path(self.tmp.name), "linux-64", self.manifest["tag"])
        self.calls: list[str] = []
        self.saved = {name: getattr(fm, name) for name in ("find_cmake", "clone", "configure_and_build", "harvest")}
        fm.find_cmake = lambda *a, **k: (self.calls.append("cmake"), "/usr/bin/cmake")[1]
        fm.clone = lambda manifest, layout, log, **k: self.calls.append("clone")
        fm.configure_and_build = lambda manifest, layout, cmake, jobs, log, **k: self.calls.append("build")

        def harvest(manifest, layout, log):
            self.calls.append("harvest")
            layout.lib_dir.mkdir(parents=True, exist_ok=True)
            sizes = {}
            for name in ("libmanifold.a", "libmanifoldc.a", "libClipper2.a", "libtbb12.a"):
                (layout.lib_dir / name).write_bytes(b"x" * 10)
                sizes[name] = 10 if not self.lie else 11
            return sizes

        self.lie = False
        fm.harvest = harvest

    def tearDown(self):
        for name, fn in self.saved.items():
            setattr(fm, name, fn)
        self.tmp.cleanup()

    def quiet(self, message):
        pass

    def test_a_stale_prefix_builds_in_order_and_becomes_warm(self):
        self.assertTrue(fm.ensure(self.manifest, self.layout, self.quiet, keep_work=True))
        self.assertEqual(self.calls, ["cmake", "clone", "build", "harvest"])
        self.assertEqual(fm.prefix_problems(self.manifest, self.layout), [])

    def test_a_warm_prefix_builds_nothing(self):
        fm.ensure(self.manifest, self.layout, self.quiet, keep_work=True)
        self.calls.clear()
        self.assertFalse(fm.ensure(self.manifest, self.layout, self.quiet, keep_work=True))
        self.assertEqual(self.calls, [])

    def test_a_harvest_that_lies_about_sizes_is_refused(self):
        self.lie = True
        with self.assertRaises(fm.FetchError) as caught:
            fm.ensure(self.manifest, self.layout, self.quiet, keep_work=True)
        self.assertIn("not warm after its own build", str(caught.exception))

    def test_the_work_tree_is_removed_unless_kept(self):
        fm.ensure(self.manifest, self.layout, self.quiet, keep_work=True)
        self.layout.work.mkdir(parents=True, exist_ok=True)
        (self.layout.work / "x").write_bytes(b"x")
        fm.prefix_problems(self.manifest, self.layout)
        # Force a rebuild by staling the stamp, then let ensure() remove the work tree.
        stamp = json.loads(self.layout.stamp.read_text(encoding="utf-8"))
        stamp["commit"] = "0" * 40
        self.layout.stamp.write_text(json.dumps(stamp), encoding="utf-8")
        fm.ensure(self.manifest, self.layout, self.quiet, keep_work=False)
        self.assertFalse(self.layout.work.exists())


class CommandLines(unittest.TestCase):
    def setUp(self):
        self.manifest = fm.load_manifest()
        self.layout = fm.Layout(Path("/cache"), "linux-64", self.manifest["tag"])

    def test_configure_passes_the_manifest_flags_verbatim_plus_the_policy_floor(self):
        argv = fm.configure_command(self.manifest, self.layout, "/usr/bin/cmake")
        self.assertEqual(argv[:5], ["/usr/bin/cmake", "-S", str(self.layout.source), "-B", str(self.layout.build)])
        for flag in self.manifest["cmake_flags"]:
            self.assertIn(flag, argv)
        self.assertEqual(argv[-1], "-DCMAKE_POLICY_VERSION_MINIMUM=3.5")
        self.assertEqual(len(argv), 5 + len(self.manifest["cmake_flags"]) + 1)

    def test_configure_and_build_runs_configure_then_a_release_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            layout = fm.Layout(Path(tmp), "linux-64", self.manifest["tag"])
            runs = []
            fm.configure_and_build(self.manifest, layout, "/usr/bin/cmake", 4, lambda m: None, run=lambda argv, log, **k: runs.append(argv))
            self.assertEqual(len(runs), 2)
            self.assertEqual(runs[0], fm.configure_command(self.manifest, layout, "/usr/bin/cmake"))
            self.assertEqual(runs[1], ["/usr/bin/cmake", "--build", str(layout.build), "--config", "Release", "--parallel", "4"])

    def test_clone_is_shallow_at_the_tag_and_refuses_the_wrong_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            layout = fm.Layout(Path(tmp), "linux-64", self.manifest["tag"])
            runs = []
            heads = iter([self.manifest["commit"]])
            fm.clone(self.manifest, layout, lambda m: None, run=lambda argv, log, **k: runs.append(argv), git_head=lambda source, env: next(heads), which=lambda name: "/usr/bin/git")
            self.assertEqual(runs, [["git", "clone", "--depth", "1", "--branch", self.manifest["tag"], self.manifest["repository"], str(layout.source)]])
            wrong = iter(["f" * 40])
            with self.assertRaises(fm.FetchError) as caught:
                fm.clone(self.manifest, layout, lambda m: None, run=lambda argv, log, **k: None, git_head=lambda source, env: next(wrong), which=lambda name: "/usr/bin/git")
            self.assertIn("refusing", str(caught.exception))
            with self.assertRaises(fm.FetchError):
                fm.clone(self.manifest, layout, lambda m: None, run=lambda argv, log, **k: None, git_head=lambda source, env: None, which=lambda name: None)


class Harvest(unittest.TestCase):
    def test_release_archive_wins_over_debug_and_tbb_prefers_tbb12(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = fm.load_manifest()
            layout = fm.Layout(Path(tmp), "win-64", manifest["tag"])
            for rel in ("Debug/manifold.lib", "Release/manifold.lib", "src/Release/manifoldc.lib", "_deps/clipper2-build/Release/Clipper2.lib", "_deps/tbb-build/Release/tbb12.lib", "_deps/tbb-build/Release/tbb.lib"):
                path = layout.build / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(rel.encode())
            sizes = fm.harvest(manifest, layout, lambda m: None)
            self.assertEqual(set(sizes), {"manifold.lib", "manifoldc.lib", "Clipper2.lib", "tbb12.lib"})
            self.assertEqual((layout.lib_dir / "manifold.lib").read_bytes(), b"Release/manifold.lib")

    def test_partial_build_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = fm.load_manifest()
            layout = fm.Layout(Path(tmp), "linux-64", manifest["tag"])
            (layout.build / "libmanifold.a").parent.mkdir(parents=True)
            (layout.build / "libmanifold.a").write_bytes(b"m")
            with self.assertRaises(fm.FetchError) as caught:
                fm.harvest(manifest, layout, lambda m: None)
            self.assertIn("libmanifoldc.a", str(caught.exception))
            self.assertIn("TBB", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
