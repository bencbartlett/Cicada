//! The git panel's server half against a REAL `git` (doc 17 item 2):
//! fixture repositories in temp dirs, the project dir a subdirectory of
//! the repo (`examples/wall` — the normal case), then the direct
//! [`cicada_server::git::Git`] API for every state and marker kind, and
//! the HTTP routes over a served project with the watcher running — the
//! self-retrigger guard (a status refresh never reloads) and the revert
//! barrier (exactly one snapshot). No network; every git command is local;
//! the store lives in the temp dir.
//!
//! Without `git` on PATH the tests SKIP LOUDLY (a printed reason, an early
//! return) — never `#[ignore]`, never a silent pass of nothing.

// Tests are exempt from the expect/unwrap denial (clippy.toml), but the
// exemption recognizes #[test] fns only — not helpers in integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cicada_server::git::{Git, GitError, GitRefusal, Scope, node_names};
use cicada_server::protocol::{
    ChangeKind, FileStatus, GitState, Operation, PROTOCOL_VERSION, RepoInfo,
};
use cicada_server::sidecar::Sidecar;
use cicada_server::{ServeConfig, serve};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

const PIPELINE: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n\
                        extra = 4.0\n";

const SCRIPT: &str = "# a helper with no nodes — in the commit scope, never loaded by these tests\n\
                      def helper():\n    return 1\n";

/// `git` is on PATH? Otherwise the caller prints why it skips and returns.
fn git_available() -> bool {
    match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => true,
        _ => {
            eprintln!(
                "SKIPPING: no `git` on PATH — the git panel tests need the git binary \
                 (install git or put it on PATH)"
            );
            false
        }
    }
}

/// Run git in `dir` with a hygienic environment (no global/system config,
/// no prompts), panicking on failure — the fixture's plumbing.
fn sh_git(dir: &Path, args: &[&str]) -> String {
    let (success, stdout, stderr) = sh_git_raw(dir, args);
    assert!(
        success,
        "git {args:?} in {} failed: {stderr}",
        dir.display()
    );
    stdout
}

/// `sh_git` that reports instead of panicking: `(success, stdout, stderr)`
/// — for the commands the fixture EXPECTS to fail (a conflicting merge).
fn sh_git_raw(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join(".no-global-gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("spawn git");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A repository whose project directory is `examples/wall`, holding a
/// pipeline, its sidecar, and `scripts/helper.py`, committed (unless
/// `commit` is false — the unborn case). Local config pins what global
/// config could otherwise vary: identity, no signing, no hooks, no CRLF
/// conversion, no fsmonitor.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    /// `core.hooksPath` — empty, so a test that wants a hook writes it here.
    hooks: PathBuf,
}

impl Fixture {
    fn new(commit: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        sh_git(&root, &["init", "-q"]);
        sh_git(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        let hooks = root.join(".no-hooks");
        std::fs::create_dir(&hooks).unwrap();
        for (key, value) in [
            ("user.name", "Cicada Test"),
            ("user.email", "test@cicada.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.fsmonitor", "false"),
            ("core.hooksPath", hooks.to_string_lossy().as_ref()),
        ] {
            sh_git(&root, &["config", key, value]);
        }
        let project = root.join("examples").join("wall");
        std::fs::create_dir_all(project.join("scripts")).unwrap();
        std::fs::write(project.join("p.cic"), PIPELINE).unwrap();
        std::fs::write(
            project.join("p.cic.layout.json"),
            Sidecar::default().render(),
        )
        .unwrap();
        std::fs::write(project.join("scripts").join("helper.py"), SCRIPT).unwrap();
        std::fs::write(root.join("other.txt"), "outside the scope\n").unwrap();
        if commit {
            sh_git(&root, &["add", "-A"]);
            sh_git(&root, &["commit", "-q", "-m", "fixture"]);
        }
        Self {
            _dir: dir,
            root,
            project,
            hooks,
        }
    }

    fn git(&self) -> Git {
        Git::new(&self.project)
    }

    /// Install a hook (`pre-commit`, …) with the given shell body.
    fn hook(&self, name: &str, body: &str) {
        let path = self.hooks.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// The repository facts of a `repo` state (panics on any other).
    fn repo_info(state: &GitState) -> &RepoInfo {
        match state {
            GitState::Repo(info) => info,
            other => panic!("expected a repo state, got {other:?}"),
        }
    }

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.project.join(relative), text).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.project.join(relative)).unwrap()
    }

    fn head_text(&self, root_relative: &str) -> String {
        sh_git(&self.root, &["show", &format!("HEAD:{root_relative}")])
    }
}

fn scope() -> Scope {
    Scope::for_pipeline("p.cic")
}

fn node_changes(
    status: &cicada_server::protocol::GitStatusResponse,
) -> Vec<(String, ChangeKind, Option<String>)> {
    status
        .pipeline
        .nodes
        .iter()
        .map(|n| (n.name.clone(), n.change, n.from.clone()))
        .collect()
}

fn scope_paths(status: &cicada_server::protocol::GitStatusResponse) -> Vec<(String, FileStatus)> {
    status
        .scope
        .iter()
        .map(|f| (f.path.clone(), f.status))
        .collect()
}

// ------------------------------------------------------- direct API --

#[test]
fn status_reports_state_and_every_marker_kind() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();

    // Clean: a repo with a prefix, on main, tracked, no markers, no scope.
    let status = git.status(&scope()).unwrap();
    let RepoInfo {
        prefix,
        branch,
        head_short,
        upstream,
        unborn,
        root,
        operation,
    } = Fixture::repo_info(&status.state);
    assert_eq!(prefix, "examples/wall");
    assert_eq!(branch.as_deref(), Some("main"));
    assert_eq!(head_short.as_deref().map(str::len), Some(7));
    assert!(upstream.is_none());
    assert!(!unborn);
    assert!(operation.is_none());
    assert!(
        Path::new(root).join("examples/wall/p.cic").is_file(),
        "root is the repository root: {root}"
    );
    assert_eq!(status.pipeline.path, "p.cic");
    assert!(status.pipeline.tracked);
    assert!(!status.pipeline.ignored);
    assert!(!status.pipeline.dirty);
    assert!(status.pipeline.nodes.is_empty());
    assert!(status.pipeline.removed.is_empty());
    assert!(status.scope.is_empty());
    assert_eq!(
        status.text_hash,
        blake3::hash(PIPELINE.as_bytes()).to_hex().to_string()
    );

    // A param change: that node only, the file modified.
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        node_changes(&status),
        vec![("size".to_owned(), ChangeKind::Modified, None)]
    );
    assert!(status.pipeline.dirty && status.pipeline.tracked);
    assert_eq!(
        scope_paths(&status),
        vec![("p.cic".to_owned(), FileStatus::Modified)]
    );
    assert_eq!(
        status.text_hash,
        blake3::hash(fx.read("p.cic").as_bytes())
            .to_hex()
            .to_string()
    );

    // A new binding: added.
    fx.write("p.cic", &format!("{PIPELINE}ball = sphere(radius=size)\n"));
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        node_changes(&status),
        vec![("ball".to_owned(), ChangeKind::Added, None)]
    );

    // A deleted binding: removed, with its HEAD line.
    fx.write(
        "p.cic",
        &PIPELINE.replace("block = box(x=span, y=span, z=span)\n", ""),
    );
    let status = git.status(&scope()).unwrap();
    assert!(status.pipeline.nodes.is_empty());
    assert_eq!(status.pipeline.removed.len(), 1);
    assert_eq!(status.pipeline.removed[0].name, "block");
    assert_eq!(status.pipeline.removed[0].line_in_head, 4);

    // A renamed binding (same right-hand side): renamed {from}.
    fx.write("p.cic", &PIPELINE.replace("block = box(", "cube = box("));
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        node_changes(&status),
        vec![(
            "cube".to_owned(),
            ChangeKind::Renamed,
            Some("block".to_owned())
        )]
    );
    assert!(status.pipeline.removed.is_empty());

    // Two edits apart: two hunks, two markers, nothing else.
    fx.write(
        "p.cic",
        &PIPELINE
            .replace("value=2.0", "value=9.0")
            .replace("extra = 4.0", "extra = 5.0"),
    );
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        node_changes(&status),
        vec![
            ("size".to_owned(), ChangeKind::Modified, None),
            ("extra".to_owned(), ChangeKind::Modified, None)
        ]
    );

    // Sidecar-only change: dirty scope, zero node markers, pipeline clean.
    fx.write("p.cic", PIPELINE);
    let mut sidecar = Sidecar::default();
    sidecar.overrides.insert(
        "size".to_owned(),
        cicada_server::sidecar::Override {
            cell: Some([4, 2]),
            ..Default::default()
        },
    );
    fx.write("p.cic.layout.json", &sidecar.render());
    let status = git.status(&scope()).unwrap();
    assert!(status.pipeline.nodes.is_empty());
    assert!(!status.pipeline.dirty);
    assert_eq!(
        scope_paths(&status),
        vec![("p.cic.layout.json".to_owned(), FileStatus::Modified)]
    );

    // Scripts: a modified one and a new one are in the scope; a non-.py
    // and a file elsewhere in the project are not.
    fx.write("scripts/helper.py", "def helper():\n    return 2\n");
    fx.write("scripts/new.py", "def new():\n    return 0\n");
    fx.write("scripts/notes.txt", "not a script\n");
    fx.write("stray.cic", PIPELINE);
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        scope_paths(&status),
        vec![
            ("p.cic.layout.json".to_owned(), FileStatus::Modified),
            ("scripts/helper.py".to_owned(), FileStatus::Modified),
            ("scripts/new.py".to_owned(), FileStatus::Untracked),
        ]
    );
}

/// The docs/17 item-2 "done when", against a REAL diff: for a working tree
/// with many edits apart (a line inserted at the top so every later line
/// shifts, a param change, an in-place rename, a deletion, a deletion and
/// an unrelated same-literal addition far apart, a `#off` toggle, a new
/// comment, an appended binding), the set of marked nodes IS the set of
/// bindings on the `+` lines of `git diff -U0 HEAD`, and every binding on
/// a `-` line is accounted for — still bound (modified), `removed` with
/// its HEAD line, or a rename's `from`. No expected list is written by
/// hand: both sides come from git's own output.
#[test]
fn markers_are_exactly_the_bindings_on_the_diffs_changed_lines() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    let head = "# cicada 1\n\
                size = slider(value=2.0, min=0.5, max=5.0)\n\
                span = construct_domain(start=0.0, end=size)\n\
                block = box(x=span, y=span, z=span)\n\
                a = 4.0\n\
                b = 5.0\n\
                c = 6.0\n\
                d = 7.0\n\
                e = 8.0\n\
                f = 9.0\n\
                ball = sphere(radius=size)\n\
                g = 10.0\n";
    fx.write("p.cic", head);
    sh_git(&fx.root, &["commit", "-q", "-am", "twelve bindings"]);
    let work = "# cicada 1\n\
                # a comment on a new line\n\
                first = 0.5\n\
                size = slider(value=3.0, min=0.5, max=5.0)\n\
                span = construct_domain(start=0.0, end=size)\n\
                cube = box(x=span, y=span, z=span)\n\
                b = 5.0\n\
                c = 6.0\n\
                #off d = 7.0\n\
                e = 8.0\n\
                f = 9.0\n\
                ball = sphere(radius=size)\n\
                g = 10.0\n\
                later = 4.0\n\
                tail = 11.0\n";
    fx.write("p.cic", work);

    let status = git.status(&scope()).unwrap();
    let diff = sh_git(
        &fx.root,
        &[
            "diff",
            "-U0",
            "--no-color",
            "HEAD",
            "--",
            "examples/wall/p.cic",
        ],
    );
    let names_on = |sign: char| -> BTreeSet<String> {
        diff.lines()
            .filter(|line| {
                line.starts_with(sign) && !line.starts_with("+++") && !line.starts_with("---")
            })
            .flat_map(|line| node_names(&line[1..]))
            .collect()
    };
    let plus = names_on('+');
    let minus = names_on('-');
    assert!(plus.len() >= 6 && minus.len() >= 4, "{diff}");
    assert!(
        diff.matches("\n@@ ").count() >= 3,
        "the edits must be apart (several hunks): {diff}"
    );

    let marked: BTreeSet<String> = status
        .pipeline
        .nodes
        .iter()
        .map(|n| n.name.clone())
        .collect();
    assert_eq!(
        marked, plus,
        "the marked nodes are exactly the bindings on the diff's `+` lines:\n{diff}"
    );
    assert_eq!(
        marked.len(),
        status.pipeline.nodes.len(),
        "one marker per node"
    );
    let work_names: BTreeSet<String> = node_names(work).into_iter().collect();
    let removed: BTreeSet<String> = status
        .pipeline
        .removed
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let froms: BTreeSet<String> = status
        .pipeline
        .nodes
        .iter()
        .filter_map(|n| n.from.clone())
        .collect();
    for name in &minus {
        assert!(
            work_names.contains(name) || removed.contains(name) || froms.contains(name),
            "`{name}` is on a `-` line but neither still bound, removed, nor a rename source: \
             {status:?}"
        );
    }
    assert!(removed.is_subset(&minus) && froms.is_subset(&minus));
    assert!(
        removed.is_disjoint(&work_names),
        "removed means no longer bound"
    );
    // Each removed node's HEAD line names it in HEAD's text.
    for gone in &status.pipeline.removed {
        let line = head.lines().nth(gone.line_in_head - 1).unwrap();
        assert_eq!(
            node_names(line),
            std::slice::from_ref(&gone.name),
            "{gone:?}"
        );
    }
    // The kinds, by the rules: a name HEAD binds → modified; a new name
    // paired in its hunk → renamed; the rest → added. `later = 4.0` shares
    // `a`'s literal but sits in another hunk: added, and `a` removed.
    let kinds: Vec<(String, ChangeKind, Option<String>)> = node_changes(&status);
    assert_eq!(
        kinds,
        vec![
            ("first".to_owned(), ChangeKind::Added, None),
            ("size".to_owned(), ChangeKind::Modified, None),
            (
                "cube".to_owned(),
                ChangeKind::Renamed,
                Some("block".to_owned())
            ),
            ("d".to_owned(), ChangeKind::Modified, None),
            ("later".to_owned(), ChangeKind::Added, None),
            ("tail".to_owned(), ChangeKind::Added, None),
        ]
    );
    assert_eq!(removed, BTreeSet::from(["a".to_owned()]));
}

#[test]
fn untracked_pipeline_is_all_added_and_cannot_be_reverted() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    fx.write("q.cic", PIPELINE);
    let q = Scope::for_pipeline("q.cic");
    let status = git.status(&q).unwrap();
    assert!(!status.pipeline.tracked);
    assert!(status.pipeline.dirty);
    assert_eq!(
        node_changes(&status),
        ["size", "span", "block", "extra"]
            .into_iter()
            .map(|n| (n.to_owned(), ChangeKind::Added, None))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        scope_paths(&status),
        vec![("q.cic".to_owned(), FileStatus::Untracked)]
    );
    let refused = git.revert(&q, None).unwrap_err();
    assert!(
        matches!(refused, GitRefusal::Untracked(ref p) if p == "q.cic"),
        "{refused:?}"
    );
    // Staged but not committed: known to git, still no HEAD version.
    sh_git(&fx.root, &["add", "examples/wall/q.cic"]);
    let status = git.status(&q).unwrap();
    assert!(status.pipeline.tracked);
    assert_eq!(status.pipeline.nodes.len(), 4);
    assert_eq!(
        scope_paths(&status),
        vec![("q.cic".to_owned(), FileStatus::Added)]
    );
    assert!(matches!(
        git.revert(&q, None),
        Err(GitRefusal::Untracked(_))
    ));
    // Committing it works; then it is clean.
    let committed = git.commit(&q, "add q").unwrap();
    assert_eq!(committed.files, ["q.cic"]);
    let status = git.status(&q).unwrap();
    assert!(status.pipeline.tracked && !status.pipeline.dirty);
    assert!(status.pipeline.nodes.is_empty());
}

#[test]
fn commit_stages_exactly_the_scope_with_the_message_verbatim() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();

    // Nothing dirty → refused; empty message → refused before anything.
    assert!(matches!(
        git.commit(&scope(), "x"),
        Err(GitRefusal::NothingToCommit)
    ));
    assert!(matches!(
        git.commit(&scope(), "  \n"),
        Err(GitRefusal::EmptyMessage)
    ));

    // Dirty: the whole scope, plus an out-of-scope untracked file in the
    // project, plus a file the user STAGED in a shell outside the project.
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    let mut sidecar = Sidecar::default();
    sidecar.overrides.insert(
        "size".to_owned(),
        cicada_server::sidecar::Override {
            cell: Some([4, 2]),
            ..Default::default()
        },
    );
    fx.write("p.cic.layout.json", &sidecar.render());
    fx.write("scripts/helper.py", "def helper():\n    return 2\n");
    fx.write("notes.txt", "not in scope\n");
    std::fs::write(fx.root.join("other.txt"), "changed outside\n").unwrap();
    sh_git(&fx.root, &["add", "other.txt"]);

    // A message git's default cleanup (`whitespace` for a piped message)
    // WOULD alter: trailing spaces on the subject, three blank lines run
    // together, trailing blank lines — so this test fails if
    // `--cleanup=verbatim` is ever dropped.
    let message =
        "Größe auf 3.0 — 尺寸  \n\n\n\nSecond paragraph, then trailing blank lines.  \n\n\n";
    let committed = git.commit(&scope(), message).unwrap();
    assert_eq!(committed.hash.len(), 40);
    assert!(committed.hash.starts_with(&committed.short));
    assert_eq!(committed.summary.trim_end(), "Größe auf 3.0 — 尺寸");
    assert_eq!(
        committed.files,
        ["p.cic", "p.cic.layout.json", "scripts/helper.py"]
    );
    // git's own view: exactly the scoped paths, the message byte-exact
    // (the commit OBJECT — `log --format=%B` appends its own newline).
    assert_eq!(
        stored_message(&fx.root),
        message.as_bytes(),
        "the message is stored verbatim"
    );
    let files = sh_git(
        &fx.root,
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    );
    let mut listed: Vec<&str> = files.lines().collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            "examples/wall/p.cic",
            "examples/wall/p.cic.layout.json",
            "examples/wall/scripts/helper.py"
        ]
    );
    assert_eq!(
        sh_git(&fx.root, &["rev-parse", "HEAD"]).trim(),
        committed.hash
    );
    // Outside the scope: untouched — the shell-staged file is still
    // staged and uncommitted, the stray file still untracked.
    assert_eq!(
        sh_git(&fx.root, &["diff", "--cached", "--name-only"]).trim(),
        "other.txt"
    );
    assert!(
        sh_git(&fx.root, &["status", "--porcelain"]).contains("?? examples/wall/notes.txt"),
        "notes.txt stays untracked"
    );
    // The scope is clean now.
    let status = git.status(&scope()).unwrap();
    assert!(status.scope.is_empty() && status.pipeline.nodes.is_empty());
    assert!(matches!(
        git.commit(&scope(), "again"),
        Err(GitRefusal::NothingToCommit)
    ));
}

/// The message bytes of HEAD's commit object: everything after the header
/// block — what git stored, nothing added.
fn stored_message(root: &Path) -> Vec<u8> {
    let object = git_bytes(root, &["cat-file", "commit", "HEAD"]);
    let split = object
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("a commit object has a blank line after its headers");
    object[split + 2..].to_vec()
}

/// Raw stdout of a git command (for byte-exact checks).
fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join(".no-global-gitconfig"))
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn revert_restores_head_bytes_for_the_scope_and_validates_paths() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    assert!(matches!(
        git.revert(&scope(), None),
        Err(GitRefusal::NothingToRevert)
    ));
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    fx.write("scripts/helper.py", "def helper():\n    return 2\n");
    fx.write("scripts/new.py", "def new():\n    return 0\n");
    fx.write("notes.txt", "not in scope\n");

    // A path outside the scope is refused before anything moves.
    let refused = git
        .revert(&scope(), Some(&["notes.txt".to_owned()]))
        .unwrap_err();
    assert!(matches!(refused, GitRefusal::PathNotAllowed { ref path, .. } if path == "notes.txt"));
    assert!(fx.read("p.cic").contains("value=3.0"), "nothing reverted");
    // An explicit ask for the untracked script is refused (we never delete).
    let refused = git
        .revert(&scope(), Some(&["scripts/new.py".to_owned()]))
        .unwrap_err();
    assert!(matches!(refused, GitRefusal::Untracked(ref p) if p == "scripts/new.py"));

    // A subset: the pipeline only.
    let done = git.revert(&scope(), Some(&["p.cic".to_owned()])).unwrap();
    assert_eq!(done.reverted, ["p.cic"]);
    assert!(!done.touched_scripts);
    assert_eq!(fx.read("p.cic"), fx.head_text("examples/wall/p.cic"));
    assert!(
        fx.read("scripts/helper.py").contains("return 2"),
        "not asked"
    );

    // The rest: the script goes back, the untracked one is reported and
    // left alone.
    let done = git.revert(&scope(), None).unwrap();
    assert_eq!(done.reverted, ["scripts/helper.py"]);
    assert_eq!(done.untracked, ["scripts/new.py"]);
    assert!(done.touched_scripts);
    assert_eq!(
        fx.read("scripts/helper.py"),
        fx.head_text("examples/wall/scripts/helper.py")
    );
    assert!(fx.project.join("scripts/new.py").is_file());
    assert_eq!(fx.read("notes.txt"), "not in scope\n");
    // Staged ≠ HEAD ≠ working tree: the revert restores HEAD's bytes, not
    // the index's (`checkout HEAD --`, not `checkout --`), and the index
    // follows HEAD for the path.
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=7.0"));
    sh_git(&fx.root, &["add", "examples/wall/p.cic"]);
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=8.0"));
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        node_changes(&status),
        vec![("size".to_owned(), ChangeKind::Modified, None)],
        "markers are working tree vs HEAD, whatever the index holds"
    );
    let done = git.revert(&scope(), Some(&["p.cic".to_owned()])).unwrap();
    assert_eq!(done.reverted, ["p.cic"]);
    assert_eq!(fx.read("p.cic"), fx.head_text("examples/wall/p.cic"));
    assert_eq!(
        sh_git(
            &fx.root,
            &[
                "diff",
                "--cached",
                "--name-only",
                "--",
                "examples/wall/p.cic"
            ]
        )
        .trim(),
        "",
        "the staged version is gone too"
    );
    // A deleted tracked file comes back too.
    std::fs::remove_file(fx.project.join("p.cic.layout.json")).unwrap();
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        scope_paths(&status),
        vec![
            ("p.cic.layout.json".to_owned(), FileStatus::Deleted),
            ("scripts/new.py".to_owned(), FileStatus::Untracked)
        ]
    );
    let done = git.revert(&scope(), None).unwrap();
    assert_eq!(done.reverted, ["p.cic.layout.json"]);
    assert_eq!(fx.read("p.cic.layout.json"), Sidecar::default().render());
}

#[test]
fn index_lock_is_the_locked_state_and_refuses_writes() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    let lock = fx.root.join(".git").join("index.lock");
    std::fs::write(&lock, "").unwrap();
    let status = git.status(&scope()).unwrap();
    // `locked` carries the same facts as `repo`: the branch chip must not
    // blank out while another git (or our own commit) holds the index.
    let GitState::Locked(info) = &status.state else {
        panic!("{:?}", status.state)
    };
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert_eq!(info.prefix, "examples/wall");
    assert_eq!(info.head_short.as_deref().map(str::len), Some(7));
    let summary = git.summary();
    assert_eq!(
        (summary.kind.as_str(), summary.branch.as_deref()),
        ("locked", Some("main"))
    );
    // Reads still answer under the lock: the markers are there.
    assert_eq!(
        node_changes(&status),
        vec![("size".to_owned(), ChangeKind::Modified, None)]
    );
    assert!(matches!(git.commit(&scope(), "x"), Err(GitRefusal::Locked)));
    assert!(matches!(
        git.revert(&scope(), None),
        Err(GitRefusal::Locked)
    ));
    assert!(fx.read("p.cic").contains("value=3.0"), "nothing moved");
    std::fs::remove_file(&lock).unwrap();
    assert!(matches!(
        git.status(&scope()).unwrap().state,
        GitState::Repo(_)
    ));
}

#[test]
fn ignored_files_stay_out_of_the_scope_and_an_ignored_pipeline_is_refused() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    // The user's .gitignore: local probe scripts and a scratch pipeline.
    std::fs::write(fx.root.join(".gitignore"), "local_*.py\nscratch.cic\n").unwrap();
    sh_git(&fx.root, &["add", ".gitignore"]);
    sh_git(&fx.root, &["commit", "-q", "-m", "ignore local probes"]);
    fx.write("scripts/local_probe.py", "print('probe')\n");
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    // The ignored script is not in the scope (git itself does not list
    // it, and `git add` would refuse the whole list over it).
    let status = git.status(&scope()).unwrap();
    assert_eq!(
        scope_paths(&status),
        vec![("p.cic".to_owned(), FileStatus::Modified)]
    );
    // Commit succeeds with exactly the pipeline — the case that used to
    // fail the WHOLE commit with `git_failed` ("The following paths are
    // ignored by one of your .gitignore files").
    let committed = git.commit(&scope(), "size 3, probe left alone").unwrap();
    assert_eq!(committed.files, ["p.cic"]);
    assert!(fx.project.join("scripts/local_probe.py").is_file());
    assert!(
        !sh_git(&fx.root, &["ls-files", "examples/wall/scripts"]).contains("local_probe"),
        "the ignored script was not added"
    );
    // An ignored PIPELINE: reported as such, every node `added` (no HEAD
    // version), nothing in the scope, and a typed refusal to commit or
    // revert it — never a 500 from `git add`.
    fx.write("scratch.cic", PIPELINE);
    let scratch = Scope::for_pipeline("scratch.cic");
    let status = git.status(&scratch).unwrap();
    assert!(status.pipeline.ignored);
    assert!(!status.pipeline.tracked);
    assert!(status.pipeline.dirty);
    assert_eq!(status.pipeline.nodes.len(), 4);
    assert!(
        status
            .pipeline
            .nodes
            .iter()
            .all(|n| n.change == ChangeKind::Added)
    );
    assert!(status.scope.is_empty());
    let refused = git.commit(&scratch, "scratch").unwrap_err();
    assert!(
        matches!(refused, GitRefusal::Ignored(ref p) if p == "scratch.cic"),
        "{refused:?}"
    );
    assert_eq!(refused.body()["kind"], "ignored");
    assert!(matches!(
        git.revert(&scratch, None),
        Err(GitRefusal::Untracked(_))
    ));
    assert_eq!(
        sh_git(&fx.root, &["rev-parse", "HEAD"]).trim(),
        committed.hash,
        "nothing else was committed"
    );
}

#[test]
fn a_failing_hook_reports_its_exit_code_and_stderr_even_for_a_long_message() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    fx.hook("pre-commit", "echo HOOK SAYS NO >&2\nexit 1\n");
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    // A message far beyond a pipe's buffer (~4 KB): git exits before it
    // reads stdin. The refusal must still be git's — exit code + stderr —
    // not our broken-pipe error (which would also have hidden an
    // `index.lock` message behind it).
    let long = format!("long\n\n{}", "x".repeat(100 * 1024));
    for message in ["short", long.as_str()] {
        let refused = git.commit(&scope(), message).unwrap_err();
        let GitRefusal::Git(GitError::Failed {
            command,
            code,
            stderr,
        }) = &refused
        else {
            panic!("expected the hook's failure, got {refused:?}");
        };
        assert!(command.starts_with("git commit"), "{command}");
        assert_eq!(*code, Some(1), "{refused:?}");
        assert!(stderr.contains("HOOK SAYS NO"), "{stderr:?}");
        let body = refused.body();
        assert_eq!(body["kind"], "git_failed");
        assert_eq!(body["code"], 1);
    }
    // Nothing was committed; the file is staged (the `add` ran) and the
    // working tree untouched.
    assert_eq!(sh_git(&fx.root, &["log", "--oneline"]).lines().count(), 1);
    assert!(fx.read("p.cic").contains("value=3.0"));
}

#[test]
fn an_unfinished_merge_is_a_state_and_refuses_writes() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    // A branch with a conflicting edit, and a different edit on main.
    sh_git(&fx.root, &["checkout", "-q", "-b", "b"]);
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=9.0"));
    sh_git(&fx.root, &["commit", "-q", "-am", "nine"]);
    sh_git(&fx.root, &["checkout", "-q", "main"]);
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    sh_git(&fx.root, &["commit", "-q", "-am", "three"]);
    let (merged, _, _) = sh_git_raw(&fx.root, &["merge", "b"]);
    assert!(!merged, "the merge must conflict");
    assert!(fx.read("p.cic").contains("<<<<<<<"));

    let status = git.status(&scope()).unwrap();
    let info = Fixture::repo_info(&status.state);
    assert_eq!(info.operation, Some(Operation::Merge));
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert_eq!(
        scope_paths(&status),
        vec![("p.cic".to_owned(), FileStatus::Modified)]
    );
    // Writes: a typed refusal naming the operation, not `git_failed` 500
    // ("cannot do a partial commit during a merge") — and the revert does
    // not quietly resolve the conflict to HEAD behind the merge's back.
    let refused = git.commit(&scope(), "resolve?").unwrap_err();
    assert!(
        matches!(refused, GitRefusal::OperationInProgress(Operation::Merge)),
        "{refused:?}"
    );
    assert_eq!(refused.body()["operation"], "merge");
    assert!(matches!(
        git.revert(&scope(), None),
        Err(GitRefusal::OperationInProgress(Operation::Merge))
    ));
    assert!(fx.read("p.cic").contains("<<<<<<<"), "untouched");
    // The shell finishes (here: aborts) the merge; the state clears.
    sh_git(&fx.root, &["merge", "--abort"]);
    let status = git.status(&scope()).unwrap();
    assert!(Fixture::repo_info(&status.state).operation.is_none());
    assert!(status.scope.is_empty());
}

#[test]
fn detached_head_has_no_branch_and_upstream_counts_ahead() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    // An upstream: a bare clone as `origin`, pushed, then one commit ahead.
    let remote = fx.root.join(".remote.git");
    sh_git(
        &fx.root,
        &[
            "clone",
            "-q",
            "--bare",
            ".",
            remote.to_string_lossy().as_ref(),
        ],
    );
    sh_git(
        &fx.root,
        &["remote", "add", "origin", remote.to_string_lossy().as_ref()],
    );
    sh_git(&fx.root, &["fetch", "-q", "origin"]);
    sh_git(&fx.root, &["branch", "-u", "origin/main", "main"]);
    let upstream = Fixture::repo_info(&git.status(&scope()).unwrap().state)
        .upstream
        .clone()
        .expect("upstream set");
    assert_eq!(upstream.name, "origin/main");
    assert_eq!((upstream.ahead, upstream.behind), (0, 0));
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    git.commit(&scope(), "ahead by one").unwrap();
    let status = git.status(&scope()).unwrap();
    let upstream = Fixture::repo_info(&status.state).upstream.as_ref();
    assert_eq!(upstream.map(|u| (u.ahead, u.behind)), Some((1, 0)));

    // Detached: no branch, the short hash to show instead.
    sh_git(&fx.root, &["checkout", "-q", "--detach"]);
    let status = git.status(&scope()).unwrap();
    let RepoInfo {
        branch, head_short, ..
    } = Fixture::repo_info(&status.state);
    assert!(branch.is_none());
    assert_eq!(
        head_short.as_deref(),
        Some(&sh_git(&fx.root, &["rev-parse", "HEAD"])[..7])
    );
}

#[test]
fn unborn_repo_is_all_added_and_the_first_commit_works() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(false);
    let git = fx.git();
    let status = git.status(&scope()).unwrap();
    let RepoInfo {
        unborn,
        branch,
        head_short,
        ..
    } = Fixture::repo_info(&status.state);
    assert!(unborn);
    assert_eq!(branch.as_deref(), Some("main"));
    assert!(head_short.is_none());
    assert!(!status.pipeline.tracked);
    assert_eq!(status.pipeline.nodes.len(), 4);
    assert!(
        status
            .pipeline
            .nodes
            .iter()
            .all(|n| n.change == ChangeKind::Added)
    );
    assert_eq!(
        scope_paths(&status),
        vec![
            ("p.cic".to_owned(), FileStatus::Untracked),
            ("p.cic.layout.json".to_owned(), FileStatus::Untracked),
            ("scripts/helper.py".to_owned(), FileStatus::Untracked),
        ]
    );
    assert!(matches!(
        git.revert(&scope(), None),
        Err(GitRefusal::Untracked(_))
    ));
    // Staged in the unborn index: tracked, still all added.
    sh_git(&fx.root, &["add", "examples/wall/p.cic"]);
    let status = git.status(&scope()).unwrap();
    assert!(status.pipeline.tracked);
    assert_eq!(status.pipeline.nodes.len(), 4);
    assert_eq!(
        status.scope[0],
        cicada_server::protocol::ScopeFile {
            path: "p.cic".into(),
            status: FileStatus::Added
        }
    );
    let committed = git.commit(&scope(), "first\n").unwrap();
    assert_eq!(
        committed.files,
        ["p.cic", "p.cic.layout.json", "scripts/helper.py"]
    );
    let status = git.status(&scope()).unwrap();
    assert!(!Fixture::repo_info(&status.state).unborn);
    assert!(status.pipeline.nodes.is_empty() && status.scope.is_empty());
    assert!(
        !fx.root.join("other.txt").exists()
            || sh_git(&fx.root, &["status", "--porcelain"]).contains("?? other.txt"),
        "the root file outside the scope was not committed"
    );
}

#[test]
fn outside_a_repo_and_without_git_are_states_not_errors() {
    if !git_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Not inside any repository: the temp dir's ancestors are not repos
    // on a sane machine; guard the assumption loudly rather than assume.
    let probe = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    if probe.status.success() {
        eprintln!(
            "SKIPPING the not-a-repo case: the temp dir is inside a repository ({})",
            String::from_utf8_lossy(&probe.stdout).trim()
        );
    } else {
        std::fs::write(dir.path().join("p.cic"), PIPELINE).unwrap();
        let git = Git::new(dir.path());
        let status = git.status(&scope()).unwrap();
        assert_eq!(status.state, GitState::NotARepo);
        assert!(!status.pipeline.tracked && !status.pipeline.dirty);
        assert!(status.pipeline.nodes.is_empty() && status.scope.is_empty());
        assert!(matches!(
            git.commit(&scope(), "x"),
            Err(GitRefusal::NotARepo)
        ));
        assert!(matches!(
            git.revert(&scope(), None),
            Err(GitRefusal::NotARepo)
        ));
        assert_eq!(git.summary().kind, "not_a_repo");
    }
    // No binary: a typed state, not a crash.
    std::fs::write(dir.path().join("p.cic"), PIPELINE).unwrap();
    let missing = Git::new(dir.path()).with_program("cicada-no-such-git-binary");
    let status = missing.status(&scope()).unwrap();
    assert_eq!(status.state, GitState::GitNotFound);
    assert!(matches!(
        missing.commit(&scope(), "x"),
        Err(GitRefusal::GitNotFound)
    ));
    assert_eq!(missing.summary().kind, "git_not_found");
}

#[test]
fn project_summary_counts_dirty_entries_under_the_project() {
    if !git_available() {
        return;
    }
    let fx = Fixture::new(true);
    let git = fx.git();
    let summary = git.summary();
    assert_eq!((summary.kind.as_str(), summary.dirty_count), ("repo", 0));
    assert_eq!(summary.branch.as_deref(), Some("main"));
    fx.write("p.cic", &PIPELINE.replace("value=2.0", "value=3.0"));
    fx.write("notes.txt", "x\n");
    std::fs::write(fx.root.join("other.txt"), "changed outside the project\n").unwrap();
    let summary = git.summary();
    assert_eq!(
        summary.dirty_count, 2,
        "the project's entries, not the repo's"
    );
}

// ------------------------------------------------------------ routes --

/// Minimal HTTP/1.1 request (loopback only): `(status, body)`.
fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut headers = String::new();
    for (name, value) in extra_headers {
        use std::fmt::Write as _;
        let _ = write!(headers, "{name}: {value}\r\n");
    }
    let body = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nX-Cicada-Token: t\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    assert!(
        !head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "unexpected chunked body:\n{head}"
    );
    (status, body.to_owned())
}

async fn get(addr: SocketAddr, path: &str) -> (u16, serde_json::Value) {
    let path = path.to_owned();
    let (status, body) = tokio::task::spawn_blocking(move || http(addr, "GET", &path, None, &[]))
        .await
        .unwrap();
    let value = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, value)
}

async fn post(
    addr: SocketAddr,
    path: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> (u16, serde_json::Value) {
    let path = path.to_owned();
    let body = body.to_owned();
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let (status, body) = tokio::task::spawn_blocking(move || {
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        http(addr, "POST", &path, Some(&body), &borrowed)
    })
    .await
    .unwrap();
    let value = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, value)
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Read text messages until `quiet` passes with nothing arriving (or the
/// overall deadline), returning their `type`s + payloads.
async fn drain(socket: &mut Socket, quiet: Duration) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(quiet, socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                out.push(serde_json::from_str(&text).unwrap());
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_)) | None) | Err(_) => break,
        }
    }
    out
}

fn count(messages: &[serde_json::Value], kind: &str) -> usize {
    messages.iter().filter(|m| m["type"] == kind).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn git_routes_over_a_served_project_under_the_watcher() {
    if !git_available() {
        return;
    }
    // The served fixture has no scripts (a `scripts/*.py` beside the
    // pipeline would start the Python host — the scope's script half is
    // covered by the direct tests above).
    let fx = Fixture::new(true);
    std::fs::remove_dir_all(fx.project.join("scripts")).unwrap();
    // A sidecar with an override: the session DELETES a default-shaped
    // sidecar on every persist (`Sidecar::save`), which would show as a
    // deletion in the scope — true, but not what this test is about.
    let mut sidecar = Sidecar::default();
    sidecar.overrides.insert(
        "size".to_owned(),
        cicada_server::sidecar::Override {
            cell: Some([4, 2]),
            ..Default::default()
        },
    );
    fx.write("p.cic.layout.json", &sidecar.render());
    // A second pipeline nobody opens: status about it must not open it.
    fx.write("q.cic", PIPELINE);
    sh_git(&fx.root, &["add", "-A"]);
    sh_git(
        &fx.root,
        &["commit", "-q", "-m", "no scripts for the served fixture"],
    );
    let head_text = fx.head_text("examples/wall/p.cic");
    let index_file = fx.root.join(".git").join("index");
    let index_meta = |path: &Path| {
        let meta = std::fs::metadata(path).unwrap();
        (meta.len(), meta.modified().unwrap())
    };

    let mut config = ServeConfig::new(fx.project.clone());
    config.pipeline = Some("p.cic".to_owned());
    config.port = 0;
    config.token = Some("t".to_owned());
    config.cache_dir = Some(fx.root.join(".cache"));
    config.threads = 2;
    let handle = serve(config).await.expect("serve");
    let addr = handle.addr;

    // /api/project carries the git summary (additive) and the scripts list.
    let (status, project) = get(addr, "/api/project").await;
    assert_eq!(status, 200);
    assert_eq!(project["git"]["kind"], "repo");
    assert_eq!(project["git"]["branch"], "main");
    assert_eq!(project["scripts"], serde_json::json!([]));
    assert_eq!(project["pipelines"], serde_json::json!(["p.cic", "q.cic"]));

    // A client: the writer.
    let url = format!("ws://{addr}/ws?token=t&pipeline=p.cic");
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    socket
        .send(Message::Text(
            format!(
                r#"{{"v":{PROTOCOL_VERSION},"type":"hello","payload":{{"v":{PROTOCOL_VERSION}}}}}"#
            )
            .into(),
        ))
        .await
        .unwrap();
    let _ = get(addr, "/debug/state?wait=true").await;
    let hydration = drain(&mut socket, Duration::from_millis(400)).await;
    assert_eq!(hydration[0]["type"], "hello");
    let client_id = hydration[0]["payload"]["client_id"].as_u64().unwrap();
    assert_eq!(count(&hydration, "snapshot"), 1, "the initial snapshot");

    // ---- Clean status; and the self-retrigger guard: refreshing status
    // (status + diff + show are reads under --no-optional-locks) never
    // reloads the session — seq unchanged, nothing on the socket.
    let (_, before) = get(addr, "/debug/state?wait=true").await;
    let seq_before = before["seq"].as_u64().unwrap();
    for _ in 0..3 {
        let (status, body) = get(addr, "/api/git/status").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["state"]["kind"], "repo");
        assert_eq!(body["state"]["prefix"], "examples/wall");
        assert_eq!(body["pipeline"]["tracked"], true);
        assert_eq!(body["pipeline"]["dirty"], false);
        assert_eq!(body["pipeline"]["nodes"], serde_json::json!([]));
        assert_eq!(body["scope"], serde_json::json!([]));
        assert_eq!(body["text_hash"], before["text_hash"]);
    }
    let (_, after) = get(addr, "/debug/state?wait=true").await;
    assert_eq!(
        after["seq"].as_u64().unwrap(),
        seq_before,
        "no reload happened"
    );
    let echoes = drain(&mut socket, Duration::from_millis(500)).await;
    assert_eq!(
        (count(&echoes, "snapshot"), count(&echoes, "delta")),
        (0, 0),
        "a status refresh must not wake the watcher: {echoes:?}"
    );

    // ---- An edit through the canvas: the marker follows the delta.
    socket
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":"s1","type":"set_param","payload":{{"node":"size","port":"value","value":"3.0"}}}}"#).into(),
        ))
        .await
        .unwrap();
    let _ = get(addr, "/debug/state?wait=true").await;
    let messages = drain(&mut socket, Duration::from_millis(300)).await;
    let delta = messages
        .iter()
        .find(|m| m["type"] == "delta" && m["payload"]["source"]["intent_id"] == "s1")
        .expect("the delta");
    let (status, body) = get(addr, "/api/git/status").await;
    assert_eq!(status, 200);
    assert_eq!(
        body["pipeline"]["nodes"],
        serde_json::json!([{"name": "size", "change": "modified"}])
    );
    assert_eq!(
        body["scope"],
        serde_json::json!([{"path": "p.cic", "status": "modified"}])
    );
    let delta_text = delta["payload"]["text"].as_str().unwrap();
    assert_eq!(
        body["text_hash"],
        blake3::hash(delta_text.as_bytes()).to_hex().to_string(),
        "the hash the client dedupes on IS the delta's text"
    );
    let (_, project) = get(addr, "/api/project").await;
    assert_eq!(project["git"]["dirty_count"], 1);
    // What `--no-optional-locks` buys, observably: the working file's
    // stat now differs from the index, so a status WITHOUT the flag would
    // refresh and rewrite `.git/index`. Three refreshes: untouched.
    let index_before = index_meta(&index_file);
    for _ in 0..3 {
        let (status, _) = get(addr, "/api/git/status").await;
        assert_eq!(status, 200);
    }
    assert_eq!(
        index_meta(&index_file),
        index_before,
        ".git/index was rewritten by a status refresh"
    );

    // ---- Status about a pipeline nobody has open: a read about a file,
    // no session hydrated (and no solve started) for it; the writer-gated
    // routes refuse it `lease` — nobody holds one — rather than open it.
    let (status, body) = get(addr, "/api/git/status?pipeline=q.cic").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pipeline"]["path"], "q.cic");
    assert_eq!(body["pipeline"]["dirty"], false);
    let (_, project) = get(addr, "/api/project").await;
    assert_eq!(project["open"], serde_json::json!(["p.cic"]));
    let (status, body) = post(
        addr,
        "/api/git/commit?pipeline=q.cic",
        &format!(r#"{{"message":"q","client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (403, Some("lease")),
        "{body}"
    );
    assert_eq!(body["path"], "q.cic");
    let (_, project) = get(addr, "/api/project").await;
    assert_eq!(project["open"], serde_json::json!(["p.cic"]));

    // ---- Commit: writer-gated, message verbatim, exactly the scope.
    let (status, body) = post(addr, "/api/git/commit", r#"{"message":"size 3"}"#, &[]).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["kind"], "lease");
    let (status, body) = post(
        addr,
        "/api/git/commit",
        &format!(r#"{{"message":"size 3","client":{}}}"#, client_id + 7),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (403, Some("lease")),
        "a non-writer id"
    );
    let (status, body) = post(
        addr,
        "/api/git/commit",
        &format!(r#"{{"message":"","client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (422, Some("empty_message"))
    );
    let (status, body) = post(addr, "/api/git/commit", "{not json", &[]).await;
    assert_eq!((status, body["kind"].as_str()), (400, Some("protocol")));
    let (status, _) = tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "POST /api/git/commit HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let status: u16 = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, raw)
    })
    .await
    .unwrap();
    assert_eq!(status, 401, "no token");
    // The header form of the writer id (the /api/run precedent).
    let message = "Größe 3.0 from the canvas\n";
    let (status, body) = post(
        addr,
        "/api/git/commit",
        &serde_json::json!({ "message": message }).to_string(),
        &[("X-Cicada-Client", &client_id.to_string())],
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["files"], serde_json::json!(["p.cic"]));
    assert_eq!(body["summary"], "Größe 3.0 from the canvas");
    assert_eq!(
        sh_git(&fx.root, &["rev-parse", "HEAD"]).trim(),
        body["hash"].as_str().unwrap()
    );
    assert_eq!(stored_message(&fx.root), message.as_bytes());
    assert_eq!(
        sh_git(
            &fx.root,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        )
        .trim(),
        "examples/wall/p.cic"
    );
    let (status, body) = get(addr, "/api/git/status").await;
    assert_eq!(status, 200);
    assert_eq!(body["pipeline"]["dirty"], false);
    assert_eq!(body["scope"], serde_json::json!([]));
    let (status, body) = post(
        addr,
        "/api/git/commit",
        &format!(r#"{{"message":"again","client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (409, Some("nothing_to_commit"))
    );
    // Committing touched only .git/: no reload, nothing on the socket.
    let (_, state) = get(addr, "/debug/state?wait=true").await;
    let seq_after_commit = state["seq"].as_u64().unwrap();
    let echoes = drain(&mut socket, Duration::from_millis(400)).await;
    assert_eq!(count(&echoes, "snapshot"), 0, "{echoes:?}");
    assert_eq!(state["history"]["depth"], 1, "the op log survives a commit");

    // ---- Revert: back to HEAD through the barrier — exactly one barrier
    // snapshot, the op log cleared, the file byte-equal to HEAD.
    let new_head = fx.read("p.cic");
    assert!(new_head.contains("value=3.0"));
    let (status, body) = post(
        addr,
        "/api/git/revert",
        &format!(r#"{{"client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (409, Some("nothing_to_revert"))
    );
    socket
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":"s2","type":"set_param","payload":{{"node":"size","port":"value","value":"4.0"}}}}"#).into(),
        ))
        .await
        .unwrap();
    let _ = get(addr, "/debug/state?wait=true").await;
    let _ = drain(&mut socket, Duration::from_millis(300)).await;
    assert!(fx.read("p.cic").contains("value=4.0"));
    let (status, body) = post(addr, "/api/git/revert", "{}", &[]).await;
    assert_eq!((status, body["kind"].as_str()), (403, Some("lease")));
    let (status, body) = post(
        addr,
        "/api/git/revert",
        &format!(r#"{{"paths":["../p.cic"],"client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (422, Some("path_not_allowed"))
    );
    assert!(
        fx.read("p.cic").contains("value=4.0"),
        "refused before moving"
    );
    let revert_started = std::time::Instant::now();
    let (status, body) = post(
        addr,
        "/api/git/revert",
        &format!(r#"{{"client":{client_id}}}"#),
        &[],
    )
    .await;
    let revert_answered = revert_started.elapsed();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reverted"], serde_json::json!(["p.cic"]));
    assert_eq!(body["untracked"], serde_json::json!([]));
    assert_eq!(
        body["reloaded"], true,
        "the checkout and the reload run under the session's write hold, so the watcher \
         cannot win the race and this call always is the reload: {body}"
    );
    assert_eq!(fx.read("p.cic"), new_head, "bytes == HEAD");
    let (_, state) = get(addr, "/debug/state?wait=true").await;
    assert_eq!(state["text"], new_head);
    assert_eq!(state["history"]["depth"], 0, "the barrier cleared the log");
    assert!(state["seq"].as_u64().unwrap() > seq_after_commit);
    // Let the watcher's echo window pass (its 80 ms coalesce + reload):
    // the one barrier snapshot is the only one.
    let messages = drain(&mut socket, Duration::from_millis(600)).await;
    let barriers: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|m| m["type"] == "snapshot" && m["payload"]["barrier"] == true)
        .collect();
    assert_eq!(
        barriers.len(),
        1,
        "exactly one barrier snapshot: {messages:?}"
    );
    // Deterministic under the hold (see above): the barrier is ours.
    assert_eq!(barriers[0]["payload"]["reason"], "git revert");
    assert_eq!(barriers[0]["payload"]["text"], new_head);
    // The docs/17 item-2 number: the barrier snapshot had been sent to
    // the socket before the POST answered (the reload broadcasts, then
    // returns), so POST → answer bounds POST → barrier on the wire.
    eprintln!(
        "MEASURED revert: POST /api/git/revert → barrier snapshot sent ≤ {:.1} ms",
        revert_answered.as_secs_f64() * 1e3
    );
    assert_eq!(
        count(&messages, "snapshot"),
        1,
        "and no non-barrier one either"
    );
    let (status, body) = get(addr, "/api/git/status").await;
    assert_eq!(status, 200);
    assert_eq!(body["pipeline"]["dirty"], false);

    // ---- index.lock: 423 for writes, status still answers as `locked`.
    let lock = fx.root.join(".git").join("index.lock");
    std::fs::write(&lock, "").unwrap();
    let (status, body) = get(addr, "/api/git/status").await;
    assert_eq!(status, 200);
    assert_eq!(body["state"]["kind"], "locked");
    assert_eq!(
        body["state"]["branch"], "main",
        "locked keeps the facts: {}",
        body["state"]
    );
    assert_eq!(body["state"]["prefix"], "examples/wall");
    let (_, project) = get(addr, "/api/project").await;
    assert_eq!(project["git"]["kind"], "locked");
    assert_eq!(project["git"]["branch"], "main");
    let (status, body) = post(
        addr,
        "/api/git/commit",
        &format!(r#"{{"message":"x","client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!((status, body["kind"].as_str()), (423, Some("locked")));
    let (status, body) = post(
        addr,
        "/api/git/revert",
        &format!(r#"{{"client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!((status, body["kind"].as_str()), (423, Some("locked")));
    std::fs::remove_file(&lock).unwrap();
    let (_, project) = get(addr, "/api/project").await;
    assert_eq!(project["git"]["kind"], "repo");

    // Unknown pipeline and traversal: the same validation as every route,
    // answered as git-route JSON (`{kind, message, path}`), not text.
    let (status, body) = get(addr, "/api/git/status?pipeline=nope.cic").await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (404, Some("no_such_pipeline")),
        "{body}"
    );
    assert_eq!(body["path"], "nope.cic");
    assert!(body["message"].is_string());
    let (status, body) = get(addr, "/api/git/status?pipeline=../p.cic").await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (400, Some("protocol")),
        "{body}"
    );
    let (status, body) = post(
        addr,
        "/api/git/revert?pipeline=nope.cic",
        &format!(r#"{{"client":{client_id}}}"#),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["kind"].as_str()),
        (404, Some("no_such_pipeline"))
    );

    socket.close(None).await.ok();
    handle.shutdown().await;
    let _ = head_text;
}
