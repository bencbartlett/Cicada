"""Unit tests for tools/fetch_occt.py — the pure parts: manifest shape,
platform + layout detection, environment rendering, sha256 verification,
archive-path safety, the version-constraint matcher the manifest generator
uses, and the PE / ELF / Mach-O import readers on synthetic files. Offline,
deterministic; nothing here touches the network or the real cache dir."""

import hashlib
import shutil
import struct
import tempfile
import unittest
from pathlib import Path

import fetch_occt as fo


def minimal_manifest(**overrides):
    package = {
        "name": "occt",
        "version": "7.8.1",
        "build": "novtk_hdfbec02_103",
        "filename": "occt-7.8.1-novtk_hdfbec02_103.conda",
        "size": 24745172,
        "sha256": "28e3da6feaec750f1a7b64f0aebc7a5f209d64ce3cbc49a3d1a22d2c81ff32d1",
        "license": "LGPL-2.1-only",
    }
    manifest = {
        "format": fo.MANIFEST_FORMAT,
        "occt_version": "7.8.1",
        "occt_build_number": 103,
        "download_url": fo.DOWNLOAD_URL,
        "platforms": {"win-64": {"packages": [package]}},
    }
    manifest.update(overrides)
    return manifest


class ManifestTest(unittest.TestCase):
    def test_committed_manifest_is_valid_and_pins_every_platform(self):
        manifest = fo.load_manifest()
        self.assertEqual(set(manifest["platforms"]), set(fo.SUPPORTED_SUBDIRS))
        for subdir in fo.SUPPORTED_SUBDIRS:
            packages = fo.manifest_packages(manifest, subdir)
            occt = [p for p in packages if p["name"] == "occt"]
            self.assertEqual(len(occt), 1)
            self.assertEqual(occt[0]["version"], "7.8.1")
            self.assertTrue(occt[0]["build"].startswith("novtk_"))
            self.assertTrue(occt[0]["build"].endswith("_103"))
            # The run-time closure the memo measured must be there.
            names = {p["name"] for p in packages}
            self.assertTrue({"freetype", "freeimage"} <= names, subdir)
        # The probe's recorded sha256s (docs/probes/occt-2026-08.md §4c).
        win = {p["name"]: p for p in fo.manifest_packages(manifest, "win-64")}
        self.assertEqual(
            win["occt"]["sha256"], "28e3da6feaec750f1a7b64f0aebc7a5f209d64ce3cbc49a3d1a22d2c81ff32d1"
        )
        linux = {p["name"]: p for p in fo.manifest_packages(manifest, "linux-64")}
        self.assertEqual(
            linux["occt"]["sha256"], "801a89aad7276a56117f418ed0ebf1a94fc94b559651898bf679fadef6f98fe9"
        )
        self.assertEqual(len(manifest["_sha256"]), 64)

    def test_validate_refuses_broken_shapes(self):
        fo.validate_manifest(minimal_manifest())
        with self.assertRaises(fo.FetchError):
            fo.validate_manifest(minimal_manifest(format=99))
        with self.assertRaises(fo.FetchError):
            fo.validate_manifest(minimal_manifest(platforms={"win-64": {"packages": []}}))
        bad = minimal_manifest()
        bad["platforms"]["win-64"]["packages"][0]["sha256"] = "nope"
        with self.assertRaises(fo.FetchError):
            fo.validate_manifest(bad)
        bad = minimal_manifest()
        bad["platforms"]["win-64"]["packages"][0]["version"] = "7.9.0"
        with self.assertRaisesRegex(fo.FetchError, "7.9.0"):
            fo.validate_manifest(bad)
        bad = minimal_manifest()
        bad["platforms"]["win-64"]["packages"][0]["name"] = "not-occt"
        with self.assertRaisesRegex(fo.FetchError, "occt itself is missing"):
            fo.validate_manifest(bad)

    def test_unpinned_platform_is_loud(self):
        manifest = minimal_manifest()
        with self.assertRaisesRegex(fo.FetchError, "linux-64"):
            fo.manifest_packages(manifest, "linux-64")

    def test_package_url(self):
        manifest = minimal_manifest()
        url = fo.package_url(manifest, "win-64", manifest["platforms"]["win-64"]["packages"][0])
        self.assertEqual(
            url,
            "https://api.anaconda.org/download/conda-forge/occt/7.8.1/win-64/occt-7.8.1-novtk_hdfbec02_103.conda",
        )


class PlatformAndLayoutTest(unittest.TestCase):
    def test_detect_subdir(self):
        self.assertEqual(fo.detect_subdir("Windows", "AMD64"), "win-64")
        self.assertEqual(fo.detect_subdir("Linux", "x86_64"), "linux-64")
        self.assertEqual(fo.detect_subdir("Darwin", "x86_64"), "osx-64")
        self.assertEqual(fo.detect_subdir("Darwin", "arm64"), "osx-arm64")
        with self.assertRaisesRegex(fo.FetchError, "linux/aarch64"):
            fo.detect_subdir("Linux", "aarch64")
        with self.assertRaises(fo.FetchError):
            fo.detect_subdir("FreeBSD", "amd64")

    def test_default_cache_root(self):
        self.assertEqual(
            fo.default_cache_root({"LOCALAPPDATA": r"C:\Users\x\AppData\Local"}, "Windows"),
            Path(r"C:\Users\x\AppData\Local") / "cicada-occt",
        )
        self.assertEqual(
            fo.default_cache_root({"XDG_CACHE_HOME": "/tmp/xdg", "HOME": "/home/x"}, "Linux"),
            Path("/tmp/xdg/cicada-occt"),
        )
        self.assertEqual(
            fo.default_cache_root({"HOME": "/home/x"}, "Linux"), Path("/home/x/.cache/cicada-occt")
        )
        with self.assertRaises(fo.FetchError):
            fo.default_cache_root({}, "Windows")

    def test_windows_layout_puts_the_prefix_under_library(self):
        layout = fo.Layout(Path(r"C:\cache"), "win-64", "7.8.1")
        self.assertEqual(layout.prefix, Path(r"C:\cache\occt-7.8.1-win-64"))
        self.assertEqual(layout.dep_occt_root, Path(r"C:\cache\occt-7.8.1-win-64\Library"))
        self.assertEqual(layout.library_dir, Path(r"C:\cache\occt-7.8.1-win-64\Library\bin"))
        self.assertEqual(layout.loader_variable, "PATH")

    def test_unix_layouts(self):
        linux = fo.Layout(Path("/c"), "linux-64", "7.8.1")
        self.assertEqual(linux.dep_occt_root, Path("/c/occt-7.8.1-linux-64"))
        self.assertEqual(linux.library_dir, Path("/c/occt-7.8.1-linux-64/lib"))
        self.assertEqual(linux.loader_variable, "LD_LIBRARY_PATH")
        mac = fo.Layout(Path("/c"), "osx-arm64", "7.8.1")
        self.assertEqual(mac.loader_variable, "DYLD_LIBRARY_PATH")
        with self.assertRaises(fo.FetchError):
            fo.Layout(Path("/c"), "linux-aarch64", "7.8.1")

    def test_windows_to_msys(self):
        self.assertEqual(fo.windows_to_msys(r"C:\Users\x\bin"), "/c/Users/x/bin")
        self.assertEqual(fo.windows_to_msys("D:/a/b"), "/d/a/b")
        self.assertEqual(fo.windows_to_msys("/already/posix"), "/already/posix")


class EnvRenderingTest(unittest.TestCase):
    def test_bash_lines_windows(self):
        layout = fo.Layout(Path(r"C:\cache"), "win-64", "7.8.1")
        lines = fo.env_lines(layout, "bash")
        self.assertEqual(lines[0], "export DEP_OCCT_ROOT='C:/cache/occt-7.8.1-win-64/Library'")
        self.assertEqual(lines[1], "export PATH='/c/cache/occt-7.8.1-win-64/Library/bin'\"${PATH:+:$PATH}\"")
        self.assertEqual(lines[2], "export CMAKE_POLICY_VERSION_MINIMUM='3.5'")

    def test_bash_lines_linux(self):
        # Paths render in the HOST's form (a Unix prefix inspected on Windows
        # prints Windows paths); the structure is what is asserted here.
        root = Path("/home/x/.cache/cicada-occt")
        layout = fo.Layout(root, "linux-64", "7.8.1")
        lines = fo.env_lines(layout, "bash")
        self.assertEqual(lines[0], f"export DEP_OCCT_ROOT='{root / 'occt-7.8.1-linux-64'}'")
        self.assertIn(f"export LD_LIBRARY_PATH='{root / 'occt-7.8.1-linux-64' / 'lib'}'", lines[1])
        self.assertIn("${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}", lines[1])

    def test_powershell_lines(self):
        # Paths render in the host's form; the structure is what is asserted.
        root = Path(r"C:\cache")
        layout = fo.Layout(root, "win-64", "7.8.1")
        lines = fo.env_lines(layout, "powershell")
        self.assertEqual(lines[0], f"$env:DEP_OCCT_ROOT = '{root / 'occt-7.8.1-win-64' / 'Library'}'")
        self.assertEqual(lines[1], f"$env:Path = '{root / 'occt-7.8.1-win-64' / 'Library' / 'bin'};' + $env:Path")
        with self.assertRaises(fo.FetchError):
            fo.env_lines(layout, "fish")

    def test_github_env_entries(self):
        root = Path(r"C:\cache")
        win = fo.Layout(root, "win-64", "7.8.1")
        env, path = fo.github_env_entries(win, "")
        self.assertEqual(env[0], f"DEP_OCCT_ROOT={root / 'occt-7.8.1-win-64' / 'Library'}")
        self.assertEqual(path, [str(root / 'occt-7.8.1-win-64' / 'Library' / 'bin')])
        mac = fo.Layout(Path("/c"), "osx-64", "7.8.1")
        env, path = fo.github_env_entries(mac, "/already/there")
        self.assertIn(f"DYLD_LIBRARY_PATH={Path('/c/occt-7.8.1-osx-64/lib')}:/already/there", env)
        self.assertEqual(path, [])


class VerificationTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="cicada-occt-test-"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_sha256_and_size_are_both_checked(self):
        payload = b"not really a conda package" * 100
        path = self.dir / "pkg.conda"
        path.write_bytes(payload)
        digest = hashlib.sha256(payload).hexdigest()
        fo.verify_archive(path, len(payload), digest)
        with self.assertRaisesRegex(fo.FetchError, "size"):
            fo.verify_archive(path, len(payload) + 1, digest)
        with self.assertRaisesRegex(fo.FetchError, "sha256"):
            fo.verify_archive(path, len(payload), "0" * 64)

    def test_stamp_matching(self):
        manifest = minimal_manifest()
        manifest["_sha256"] = "m" * 64
        expected = {"occt-7.8.1-novtk_hdfbec02_103.conda": manifest["platforms"]["win-64"]["packages"][0]["sha256"]}
        self.assertTrue(fo.stamp_matches({"manifest_sha256": "m" * 64, "packages": expected}, manifest, "win-64"))
        self.assertFalse(fo.stamp_matches({"manifest_sha256": "x" * 64, "packages": expected}, manifest, "win-64"))
        self.assertFalse(fo.stamp_matches({"manifest_sha256": "m" * 64, "packages": {}}, manifest, "win-64"))
        self.assertFalse(fo.stamp_matches(None, manifest, "win-64"))
        self.assertFalse(fo.stamp_matches({}, manifest, "win-64"))

    def test_archive_member_paths_cannot_escape(self):
        prefix = self.dir / "prefix"
        self.assertEqual(fo.safe_member_path(prefix, "lib/libTKernel.so"), prefix / "lib" / "libTKernel.so")
        self.assertEqual(fo.safe_member_path(prefix, "./Library/bin/TKernel.dll"), prefix / "Library/bin/TKernel.dll")
        for evil in ("../outside", "lib/../../x", "/etc/passwd", r"C:\Windows\x"):
            with self.assertRaises(fo.FetchError, msg=evil):
                fo.safe_member_path(prefix, evil)

    def test_occt_version_is_read_from_the_header(self):
        text = "#define OCC_VERSION_MAJOR 7\n#define OCC_VERSION_MINOR 8\n#define OCC_VERSION_MAINTENANCE 1\n"
        self.assertEqual(fo.parse_occt_version(text), "7.8.1")
        header = self.dir / "Standard_Version.hxx"
        header.write_text(text.replace("MAINTENANCE 1", "MAINTENANCE 2"), encoding="utf-8")
        with self.assertRaisesRegex(fo.FetchError, "7.8.2"):
            fo.check_occt_version(header, "7.8.1")
        with self.assertRaises(fo.FetchError):
            fo.parse_occt_version("nothing here")


class VersionSpecTest(unittest.TestCase):
    def test_ordering(self):
        self.assertLess(fo.compare_versions("2.13.3", "3.0a0"), 0)
        self.assertLess(fo.compare_versions("3.0a0", "3.0"), 0)  # pre-release below the release
        self.assertGreater(fo.compare_versions("3.0", "3.0a0"), 0)
        self.assertEqual(fo.compare_versions("1.6.44", "1.6.44"), 0)
        self.assertGreater(fo.compare_versions("1.6.44", "1.6.9"), 0)
        self.assertGreater(fo.compare_versions("1.1.0.post20250205", "1.1.0"), 0)

    def test_constraints(self):
        self.assertTrue(fo.version_satisfies("2.13.3", ">=2.12.1,<3.0a0"))
        self.assertFalse(fo.version_satisfies("3.0", ">=2.12.1,<3.0a0"))
        self.assertFalse(fo.version_satisfies("3.0a0", ">=2.12.1,<3.0a0"))
        self.assertTrue(fo.version_satisfies("3.18.0", ">=3.18.0,<3.19.0a0"))
        self.assertTrue(fo.version_satisfies("1.6.44", "1.6.*"))
        self.assertFalse(fo.version_satisfies("1.7.0", "1.6.*"))
        self.assertTrue(fo.version_satisfies("1.2.3", "1.2.3"))
        self.assertTrue(fo.version_satisfies("4.5", ""))
        self.assertTrue(fo.version_satisfies("2.0", ">=1,<2.0a0|>=2,<3.0a0"))
        with self.assertRaises(fo.FetchError):
            fo.version_satisfies("1.0", "~=1.0")

    def test_parse_depends_and_ignores(self):
        self.assertEqual(fo.parse_depends("freetype >=2.12.1,<3.0a0"), ("freetype", ">=2.12.1,<3.0a0", "*"))
        self.assertEqual(fo.parse_depends("_openmp_mutex >=4.5 *_gnu"), ("_openmp_mutex", ">=4.5", "*_gnu"))
        self.assertEqual(fo.parse_depends("rapidjson"), ("rapidjson", "", "*"))
        self.assertTrue(fo.is_ignored_dependency("__glibc"))
        self.assertTrue(fo.is_ignored_dependency("vc14_runtime"))
        self.assertTrue(fo.is_ignored_dependency("font-ttf-dejavu-sans-mono"))
        self.assertFalse(fo.is_ignored_dependency("freetype"))
        self.assertFalse(fo.is_ignored_dependency("libstdcxx"))


def fake_pe(import_names):
    """A PE32+ image with one section holding an import directory."""
    section_rva = 0x1000
    section_raw = 0x400
    blob = bytearray()
    descriptors_size = 20 * (len(import_names) + 1)
    names_offset = descriptors_size
    name_offsets = []
    names_blob = bytearray()
    for name in import_names:
        name_offsets.append(names_offset + len(names_blob))
        names_blob += name.encode() + b"\0"
    for offset in name_offsets:
        blob += struct.pack("<IIIII", 0, 0, 0, section_rva + offset, 0)
    blob += b"\0" * 20
    blob += names_blob
    optional = bytearray(240)
    struct.pack_into("<H", optional, 0, 0x20B)
    struct.pack_into("<II", optional, 112 + 8, section_rva, len(blob))
    header = bytearray(b"MZ" + b"\0" * 62)
    struct.pack_into("<I", header, 0x3C, 64)
    coff = b"PE\0\0" + struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, len(optional), 0)
    section = b".idata\0\0" + struct.pack("<IIII", len(blob), section_rva, len(blob), section_raw) + b"\0" * 16
    image = bytearray(header + coff + optional + section)
    image += b"\0" * (section_raw - len(image))
    image += blob
    return bytes(image)


def fake_elf(needed_names):
    """A 64-bit little-endian ELF with a .dynstr and a .dynamic of DT_NEEDED."""
    strtab = bytearray(b"\0")
    offsets = []
    for name in needed_names:
        offsets.append(len(strtab))
        strtab += name.encode() + b"\0"
    dynamic = bytearray()
    for offset in offsets:
        dynamic += struct.pack("<qQ", 1, offset)
    dynamic += struct.pack("<qQ", 0, 0)
    header_size = 64
    strtab_offset = header_size
    dynamic_offset = strtab_offset + len(strtab)
    shoff = dynamic_offset + len(dynamic)
    ident = b"\x7fELF" + bytes([2, 1, 1, 0]) + b"\0" * 8
    header = ident + struct.pack("<HHIQQQIHHHHHH", 3, 62, 1, 0, 0, shoff, 0, 64, 0, 0, 64, 3, 0)
    null_section = b"\0" * 64
    strtab_section = struct.pack("<IIQQQQIIQQ", 1, 3, 0, 0, strtab_offset, len(strtab), 0, 0, 1, 0)
    dynamic_section = struct.pack("<IIQQQQIIQQ", 2, 6, 0, 0, dynamic_offset, len(dynamic), 1, 0, 8, 16)
    return bytes(header + strtab + dynamic + null_section + strtab_section + dynamic_section)


def fake_macho(dylibs):
    """A 64-bit little-endian Mach-O with LC_LOAD_DYLIB commands."""
    commands = bytearray()
    for name in dylibs:
        name_bytes = name.encode() + b"\0"
        size = 24 + len(name_bytes)
        size += (-size) % 8
        commands += struct.pack("<IIIIII", 0xC, size, 24, 0, 0, 0) + name_bytes.ljust(size - 24, b"\0")
    header = struct.pack("<IIIIIIII", 0xFEEDFACF, 0x0100000C, 0, 6, len(dylibs), len(commands), 0, 0)
    return bytes(header + commands)


class ImportReaderTest(unittest.TestCase):
    def test_pe_imports(self):
        self.assertEqual(fo.pe_imports(fake_pe(["TKernel.dll", "KERNEL32.dll"])), ["TKernel.dll", "KERNEL32.dll"])
        with self.assertRaises(fo.FetchError):
            fo.pe_imports(b"not a pe")

    def test_elf_needed(self):
        self.assertEqual(fo.elf_needed(fake_elf(["libTKernel.so.7.8", "libc.so.6"])), ["libTKernel.so.7.8", "libc.so.6"])
        with self.assertRaises(fo.FetchError):
            fo.elf_needed(b"ELF but not")

    def test_macho_dylibs(self):
        names = ["@rpath/libTKernel.7.8.dylib", "/usr/lib/libSystem.B.dylib"]
        self.assertEqual(fo.macho_dylibs(fake_macho(names)), names)
        with self.assertRaises(fo.FetchError):
            fo.macho_dylibs(b"\0" * 32)

    def test_closure_check_on_a_synthetic_prefix(self):
        root = Path(tempfile.mkdtemp(prefix="cicada-occt-closure-"))
        try:
            layout = fo.Layout(root, "linux-64", "7.8.1")
            layout.library_dir.mkdir(parents=True)
            (layout.library_dir / "libTKernel.so.7.8").write_bytes(fake_elf(["libc.so.6", "libstdc++.so.6"]))
            (layout.library_dir / "libTKService.so.7.8").write_bytes(fake_elf(["libTKernel.so.7.8", "libfreetype.so.6"]))
            (layout.library_dir / "libgcc_s.so").write_bytes(b"/* GNU ld script */\n")
            messages = []
            unresolved = fo.check_closure(layout, messages.append)
            self.assertEqual(unresolved, ["libfreetype.so.6"])
            (layout.library_dir / "libfreetype.so.6").write_bytes(fake_elf(["libc.so.6"]))
            self.assertEqual(fo.check_closure(layout, messages.append), [])
            self.assertTrue(any("skipping libgcc_s.so" in m for m in messages))
        finally:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
