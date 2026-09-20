//! `cicada --version` at the process level (docs/17 wave 5 R1): the line
//! has the stamped shape — `cicada <semver> (<commit>, <YYYY-MM-DD>)` with
//! the workspace version, a 12-digit git hash (`-dirty` allowed) or
//! `unknown`, and a date — and, where git can be asked, the hash is HEAD's.
//! The rules themselves are unit-tested in `cicada_cli::stamp`; this proves
//! the build script ran and clap prints what it stamped.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

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
/// is HEAD's (the build script re-runs when HEAD moves). The `-dirty`
/// suffix is not compared: the tree's state at build time is not its state
/// now.
#[test]
fn the_commit_is_head_when_git_can_say() {
    if std::env::var_os("CICADA_GIT_SHA").is_some_and(|v| !v.is_empty()) {
        return; // the override wins by contract; HEAD is not the reference
    }
    let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
    else {
        return; // no git: the stamp is `unknown` by contract, checked above
    };
    if !output.status.success() {
        return;
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let (_, commit, _) = parse(&version_line());
    assert_eq!(
        commit.strip_suffix("-dirty").unwrap_or(&commit),
        head,
        "the stamped commit is not HEAD's short hash"
    );
}
