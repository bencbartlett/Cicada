#!/usr/bin/env python3
"""CHANGELOG.md as the release workflow reads it (docs/17 wave 5 R1).

    python tools/changelog.py check                     # the first section names Cargo.toml's workspace version
    python tools/changelog.py check --tag v0.1.0-alpha.1  # ... and the tag names the same version
    python tools/changelog.py section 0.1.0-alpha.1     # print that version's section body (the release notes)
    python tools/changelog.py assets 0.1.0-alpha.1      # print the assets paragraph the release body ends with

A section is a `## <version> ...` heading (the version is the first
whitespace-delimited word after `## `, `[0.1.0]` as `0.1.0`; whatever
follows — a date — is ignored) and everything down to the next `## `
heading. `check` refuses a CHANGELOG whose first version section is not the
workspace version, or a tag that is not `v` + that version: the release
workflow runs it before building anything, so a tag pushed against the
wrong version stops at the first job with the reason, never as a release
whose notes describe another version. Two conventions (fix round
2026-09-20, findings F2 / L5-2 and R1-C9): a leading `## Unreleased`
section collects the next version's notes — `check` skips it, `check
--tag` refuses it, so a tag never ships unreleased notes — and a `TAG-TODO`
marker in the version's section is a decision still to make: `check`
allows it at every commit, `check --tag` refuses it, so release notes
cannot carry a stale "these land beside this entry" claim.

Exit status: 0 on success; 1 on any refusal, always through an `error:` line.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CHANGELOG = REPO / "CHANGELOG.md"
CARGO_TOML = REPO / "Cargo.toml"

HEADING = re.compile(r"^## +\[?([^\s\]]+)\]?(?:\s.*)?$")
#: The accumulator section for the next version (R1-C9): allowed at the top
#: at every commit, refused under `--tag`.
UNRELEASED = "Unreleased"
#: A decision still to make inside a section (F2 / L5-2): allowed at every
#: commit, refused under `--tag`.
TODO_MARKER = "TAG-TODO"


class ChangelogError(Exception):
    """A refusal with its reason."""


def sections(text: str) -> list[tuple[str, str]]:
    """`[(version, body)]` in file order; the body is stripped of blank edges."""
    found: list[tuple[str, list[str]]] = []
    for line in text.splitlines():
        match = HEADING.match(line)
        if match:
            found.append((match.group(1), []))
        elif found:
            found[-1][1].append(line)
    return [(version, "\n".join(body).strip("\n")) for version, body in found]


def first_version(text: str) -> str:
    """The newest VERSION section — the top of the file, past a leading
    `## Unreleased`."""
    found = [version for version, _ in sections(text) if version != UNRELEASED]
    if not found:
        raise ChangelogError("CHANGELOG.md has no `## <version>` section")
    return found[0]


def has_unreleased(text: str) -> bool:
    """Is there a `## Unreleased` section?"""
    return any(version == UNRELEASED for version, _ in sections(text))


def section(text: str, version: str) -> str:
    """The body of `version`'s section."""
    for found, body in sections(text):
        if found == version:
            if not body.strip():
                raise ChangelogError(f"CHANGELOG.md's section for {version} is empty")
            return body
    raise ChangelogError(f"CHANGELOG.md has no section for {version} (sections: {', '.join(v for v, _ in sections(text)) or 'none'})")


def workspace_version(cargo_toml: str) -> str:
    """`[workspace.package]`'s `version` in a Cargo.toml."""
    in_table = False
    for line in cargo_toml.splitlines():
        stripped = line.split("#", 1)[0].strip()
        if stripped.startswith("["):
            in_table = stripped == "[workspace.package]"
            continue
        if in_table:
            match = re.match(r'^version\s*=\s*"([^"]+)"$', stripped)
            if match:
                return match.group(1)
    raise ChangelogError("Cargo.toml has no `version` under [workspace.package]")


def check(changelog: str, cargo_toml: str, tag: str | None = None) -> str:
    """The version everything agrees on, or a refusal naming the disagreement.
    With `tag`, also what a release may not carry: an `Unreleased` section
    or a `TAG-TODO` in the version's section."""
    version = workspace_version(cargo_toml)
    newest = first_version(changelog)
    if newest != version:
        raise ChangelogError(f"CHANGELOG.md's first section is {newest}, Cargo.toml's workspace version is {version}")
    body = section(changelog, version)  # non-empty
    if tag is not None:
        if tag != f"v{version}":
            raise ChangelogError(f"tag {tag} does not name the workspace version {version} (expected v{version})")
        if has_unreleased(changelog):
            raise ChangelogError(f"CHANGELOG.md has a `## {UNRELEASED}` section — fold it into {version}'s (or remove it) before tagging")
        if TODO_MARKER in body:
            raise ChangelogError(
                f"CHANGELOG.md's section for {version} still carries a {TODO_MARKER}: decide it and delete the marker before tagging "
                "(the section is the release body verbatim)"
            )
    return version


def is_prerelease(version: str) -> bool:
    """Semver: a pre-release carries a `-` suffix (`0.1.0-alpha.1`)."""
    return "-" in version


def assets(version: str) -> str:
    """The paragraph the release body ends with: what each asset is and needs."""
    return "\n".join(
        [
            "## Assets",
            "",
            f"- `Cicada-{version}-windows-x86_64.zip` — unzip and double-click `Cicada.cmd` (the engine with the app embedded and the OpenCASCADE run-time libraries beside it; needs Python 3 and the VC++ runtime — `README.txt` inside says so).",
            f"- `Cicada-{version}-macos-arm64.zip` — Apple silicon only (no Intel build yet): unzip and open `Cicada.app` (right-click → Open the first time: not notarized; needs Python 3).",
            f"- `cicada-{version}-linux-x86_64` — the bare engine binary with the app embedded: needs the OpenCASCADE 7.8.1 run-time libraries on `LD_LIBRARY_PATH` (`python tools/fetch_occt.py --print-env bash` from a checkout) and Python 3.",
            "- `LICENSE` and `THIRD_PARTY_NOTICES.md` — Cicada's licence, and the third-party libraries the bundles carry with their licences and source; the zips hold the same two files beside their `README.txt`.",
            "",
            f"Every binary answers `cicada --version` with `cicada {version} (<commit>, <build date>)`; the app's About dialog shows the same.",
        ]
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--changelog", type=Path, default=CHANGELOG)
    parser.add_argument("--cargo-toml", type=Path, default=CARGO_TOML)
    commands = parser.add_subparsers(dest="command", required=True)
    check_cmd = commands.add_parser("check", help="the first section names Cargo.toml's workspace version")
    check_cmd.add_argument("--tag", help="a `v<version>` tag that must name the same version")
    section_cmd = commands.add_parser("section", help="print a version's section body")
    section_cmd.add_argument("version")
    assets_cmd = commands.add_parser("assets", help="print the release body's assets paragraph")
    assets_cmd.add_argument("version")
    args = parser.parse_args(argv)
    # The sections carry arrows and dashes; a cp1252 console (Windows) must
    # not turn a release body into an encode error.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    try:
        if args.command == "check":
            changelog = args.changelog.read_text(encoding="utf-8")
            cargo_toml = args.cargo_toml.read_text(encoding="utf-8")
            version = check(changelog, cargo_toml, args.tag)
            print(f"{version}: CHANGELOG.md and Cargo.toml agree" + (f"; tag {args.tag} names it" if args.tag else ""))
            return 0
        if args.command == "section":
            print(section(args.changelog.read_text(encoding="utf-8"), args.version))
            return 0
        print(assets(args.version))
        return 0
    except (ChangelogError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
