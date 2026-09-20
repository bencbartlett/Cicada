"""Unit tests for tools/changelog.py (docs/17 wave 5 R1): the section
parser, the workspace-version reader, the agreement check the release
workflow runs first, the pre-release rule — over fixtures and over the
repository's own CHANGELOG.md and Cargo.toml, which must agree at every
commit so a tag never ships notes for another version."""

import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path

import changelog as cl

FIXTURE = """# Changelog

Preamble that is not a section.

## 0.2.0 — 2027-01-01

Second release.

- a thing

## 0.1.0-alpha.1 — 2026-09-19

First pre-release.

### Added

- everything

## 0.0.9

"""

CARGO = """[workspace]
members = ["crates/*"]

[workspace.package]
# a comment
version = "0.2.0"
edition = "2024"

[workspace.dependencies]
version = "not-this-one"
"""


class SectionsTest(unittest.TestCase):
    def test_sections_are_split_on_version_headings_and_bodies_stripped(self):
        found = cl.sections(FIXTURE)
        self.assertEqual([v for v, _ in found], ["0.2.0", "0.1.0-alpha.1", "0.0.9"])
        self.assertEqual(found[0][1], "Second release.\n\n- a thing")
        self.assertEqual(found[1][1], "First pre-release.\n\n### Added\n\n- everything")
        self.assertEqual(found[2][1], "")

    def test_section_returns_the_body_and_refuses_missing_or_empty(self):
        self.assertEqual(cl.section(FIXTURE, "0.1.0-alpha.1"), "First pre-release.\n\n### Added\n\n- everything")
        with self.assertRaises(cl.ChangelogError) as refused:
            cl.section(FIXTURE, "9.9.9")
        self.assertIn("no section for 9.9.9", str(refused.exception))
        with self.assertRaises(cl.ChangelogError) as refused:
            cl.section(FIXTURE, "0.0.9")
        self.assertIn("empty", str(refused.exception))

    def test_first_version_is_the_top_section(self):
        self.assertEqual(cl.first_version(FIXTURE), "0.2.0")
        with self.assertRaises(cl.ChangelogError):
            cl.first_version("# Changelog\n\nnothing yet\n")


class WorkspaceVersionTest(unittest.TestCase):
    def test_reads_the_workspace_package_table_only(self):
        self.assertEqual(cl.workspace_version(CARGO), "0.2.0")
        with self.assertRaises(cl.ChangelogError):
            cl.workspace_version("[package]\nversion = \"1.0.0\"\n")


class CheckTest(unittest.TestCase):
    def test_agreement_and_the_tag(self):
        self.assertEqual(cl.check(FIXTURE, CARGO), "0.2.0")
        self.assertEqual(cl.check(FIXTURE, CARGO, tag="v0.2.0"), "0.2.0")
        with self.assertRaises(cl.ChangelogError) as refused:
            cl.check(FIXTURE, CARGO, tag="v0.1.0-alpha.1")
        self.assertIn("tag v0.1.0-alpha.1 does not name the workspace version 0.2.0", str(refused.exception))
        with self.assertRaises(cl.ChangelogError) as refused:
            cl.check(FIXTURE, CARGO.replace('"0.2.0"', '"0.3.0"'))
        self.assertIn("first section is 0.2.0, Cargo.toml's workspace version is 0.3.0", str(refused.exception))

    def test_prerelease_is_a_dash_suffix(self):
        self.assertTrue(cl.is_prerelease("0.1.0-alpha.1"))
        self.assertTrue(cl.is_prerelease("1.0.0-rc.1"))
        self.assertFalse(cl.is_prerelease("0.1.0"))

    def test_assets_name_the_three_files_for_the_version(self):
        text = cl.assets("0.1.0-alpha.1")
        # Every asset names its architecture (L3-6 / R1-C7): the macOS zip is Apple silicon and says so.
        for name in ["Cicada-0.1.0-alpha.1-windows-x86_64.zip", "Cicada-0.1.0-alpha.1-macos-arm64.zip", "cicada-0.1.0-alpha.1-linux-x86_64"]:
            self.assertIn(name, text)
        self.assertIn("Apple silicon only", text)
        self.assertNotIn("-macos.zip", text)
        self.assertIn("cicada 0.1.0-alpha.1 (<commit>, <build date>)", text)
        # The licensing files ride as assets (R1-C3): the release job attaches them.
        self.assertIn("`LICENSE` and `THIRD_PARTY_NOTICES.md`", text)


class RepositoryTest(unittest.TestCase):
    """The committed files agree: the release workflow's first check, run here at every CI run."""

    def test_the_committed_changelog_names_the_workspace_version(self):
        changelog = cl.CHANGELOG.read_text(encoding="utf-8")
        cargo_toml = cl.CARGO_TOML.read_text(encoding="utf-8")
        version = cl.check(changelog, cargo_toml, tag=f"v{cl.workspace_version(cargo_toml)}")
        self.assertRegex(version, r"^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$")
        self.assertTrue(cl.section(changelog, version).strip())


class CliTest(unittest.TestCase):
    def run_cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = cl.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_check_section_and_assets_over_fixture_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            changelog = Path(tmp) / "CHANGELOG.md"
            cargo = Path(tmp) / "Cargo.toml"
            changelog.write_text(FIXTURE, encoding="utf-8")
            cargo.write_text(CARGO, encoding="utf-8")
            base = ["--changelog", str(changelog), "--cargo-toml", str(cargo)]
            code, out, _ = self.run_cli(*base, "check", "--tag", "v0.2.0")
            self.assertEqual(code, 0, out)
            self.assertIn("0.2.0: CHANGELOG.md and Cargo.toml agree; tag v0.2.0 names it", out)
            code, _, err = self.run_cli(*base, "check", "--tag", "v0.2.1")
            self.assertEqual(code, 1)
            self.assertTrue(err.startswith("error: tag v0.2.1"), err)
            code, out, _ = self.run_cli(*base, "section", "0.1.0-alpha.1")
            self.assertEqual(code, 0)
            self.assertEqual(out, "First pre-release.\n\n### Added\n\n- everything\n")
            code, _, err = self.run_cli(*base, "section", "0.0.9")
            self.assertEqual(code, 1)
            self.assertIn("empty", err)
            code, out, _ = self.run_cli("assets", "0.2.0")
            self.assertEqual(code, 0)
            self.assertIn("Cicada-0.2.0-windows-x86_64.zip", out)

    def test_a_missing_changelog_is_an_error_line(self):
        missing = os.path.join(tempfile.gettempdir(), "cicada-no-such-changelog.md")
        code, _, err = self.run_cli("--changelog", missing, "check")
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("error: "), err)


if __name__ == "__main__":
    sys.exit(unittest.main())
