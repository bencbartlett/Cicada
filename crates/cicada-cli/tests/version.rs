//! `cicada --version` at the process level (docs/17 wave 5 R1): the line
//! has the stamped shape — `cicada <semver> (<commit>, <YYYY-MM-DD>)` with
//! the workspace version, a 12-digit git hash (`-dirty` allowed) or
//! `unknown`, and a date — and, where git can be asked, the hash is HEAD's,
//! `-dirty` is exactly the build inputs' state and the date is not before
//! HEAD's commit. The rules themselves are unit-tested in
//! `cicada_cli::stamp`; this proves the build script ran, re-ran when it had
//! to, and clap prints what it stamped.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use cicada_cli::stamp::{BUILD_INPUTS, utc_date};

fn version_line() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cicada"))
        .arg("--version")
        .output()
        .expect("cicada binary runs");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// `cicada <semver> (<commit>, <built>)` → its three parts.
fn parse(line: &str) -> (String, String, String) {
    let rest = line
        .strip_prefix("cicada ")
        .unwrap_or_else(|| panic!("no `cicada ` prefix: {line:?}"));
    let (semver, tail) = rest
        .split_once(" (")
        .unwrap_or_else(|| panic!("no ` (` in {line:?}"));
    let inner = tail
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("no closing paren in {line:?}"));
    let (commit, built) = inner
        .split_once(", ")
        .unwrap_or_else(|| panic!("no `, ` between commit and date in {line:?}"));
    (semver.to_owned(), commit.to_owned(), built.to_owned())
}

/// The workspace root: `crates/cicada-cli` → two up.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// Git in the workspace root, as the build script runs it (no index
/// rewrite): its trimmed stdout, or `None` when git cannot answer — no git,
/// not a repository.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(workspace_root())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `CICADA_GIT_SHA` set and non-blank: the override wins by contract, and
/// HEAD is not the reference for the commit.
fn overridden() -> bool {
    std::env::var_os("CICADA_GIT_SHA").is_some_and(|v| !v.is_empty())
}

#[test]
fn version_prints_the_stamped_shape() {
    let line = version_line();
    let (semver, commit, built) = parse(&line);
    // The test binary and `cicada` are one package: the same version.
    assert_eq!(semver, env!("CARGO_PKG_VERSION"), "{line}");
    assert!(
        cicada_cli::stamp::is_git_stamp(&commit) || commit == cicada_cli::stamp::UNKNOWN,
        "commit {commit:?} is neither a 12-digit git hash (optionally -dirty) nor `unknown`: {line}"
    );
    assert!(
        cicada_cli::stamp::is_date(&built),
        "built {built:?} is not YYYY-MM-DD: {line}"
    );
    assert_eq!(line, format!("cicada {}", cicada_cli::version::LINE));
}

/// Where git can say what HEAD is and no override was set, the stamped hash
/// is HEAD's (the build script re-runs when HEAD moves).
#[test]
fn the_commit_is_head_when_git_can_say() {
    if overridden() {
        return;
    }
    let Some(head) = git(&["rev-parse", "--short=12", "HEAD"]) else {
        return; // no git: the stamp is `unknown` by contract, checked above
    };
    let (_, commit, _) = parse(&version_line());
    assert_eq!(
        commit.strip_suffix("-dirty").unwrap_or(&commit),
        head,
        "the stamped commit is not HEAD's short hash"
    );
}

/// `-dirty` follows the build inputs exactly (fix round 2026-09-20,
/// findings L2-1 / R1-C2): the build script registers every tracked file
/// under `stamp::BUILD_INPUTS`, HEAD and the index as `rerun-if-changed`,
/// so the stamp this binary carries was taken after the last change to
/// them — and it says `-dirty` iff `git status --porcelain
/// --untracked-files=no -- <BUILD_INPUTS>` lists anything now. A script
/// that never answers `-dirty` (the review's mutation) is red here on any
/// dirty tree — and the mutation itself dirties `build.rs`.
#[test]
fn dirty_follows_the_build_inputs_when_git_can_say() {
    if overridden() {
        return;
    }
    let mut args = vec!["status", "--porcelain", "--untracked-files=no", "--"];
    args.extend_from_slice(BUILD_INPUTS);
    let Some(porcelain) = git(&args) else {
        return;
    };
    let dirty = porcelain.lines().any(|line| !line.trim().is_empty());
    let (_, commit, _) = parse(&version_line());
    assert_eq!(
        commit.ends_with("-dirty"),
        dirty,
        "the stamp {commit:?} disagrees with the build inputs' porcelain:\n{porcelain}"
    );
}

/// The build date is a real date (finding L2-2): never before HEAD's
/// commit date — the stamp is re-taken when HEAD moves, so a binary cannot
/// predate its own commit — which a pinned or fallen-back date
/// (`1970-01-01`) fails. `SOURCE_DATE_EPOCH` names the date by contract;
/// `YYYY-MM-DD` compares chronologically as text.
#[test]
fn the_build_date_is_not_before_heads_commit_date() {
    if std::env::var_os("SOURCE_DATE_EPOCH").is_some_and(|v| !v.is_empty()) {
        return;
    }
    let Some(committed) = git(&["log", "-1", "--format=%ct", "HEAD"]) else {
        return;
    };
    let committed: i64 = committed.parse().expect("git's %ct is an integer");
    let head_date = utc_date(committed);
    let (_, _, built) = parse(&version_line());
    assert!(
        built.as_str() >= head_date.as_str(),
        "built {built} predates HEAD's commit date {head_date}"
    );
}
