//! Git over the `git` binary (doc 17 item 2, DECISIONS.md "Git integration
//! is first-class UI": plain git underneath, no libgit2/gix, no custom
//! store). Slice 1 answers three questions for one pipeline — where does
//! the project stand (branch, upstream, dirty scope), which NODES differ
//! from HEAD, and can the scope be committed or put back — as typed states
//! and typed refusals, never strings.
//!
//! **Markers are a rendering of `git diff`** (doc 10 §Git integration): one
//! binding per line makes line diffs node diffs, so `git diff -U0 HEAD --
//! <pipeline>` → hunks → changed NEW-side line ranges → the bindings on
//! those lines (the parser's line index IS the span). A binding on a
//! changed line is `modified` when HEAD binds the name and `added` when it
//! does not; a HEAD binding on a removed line that the working tree no
//! longer binds is `removed`; a removed + added pair with a byte-identical
//! right-hand side within one hunk is `renamed`. The sidecar never
//! produces markers — it is layout, not pipeline. Working tree vs HEAD
//! only; other refs, the graph-diff overlay and per-node history follow
//! once the markers have had weeks of use.
//!
//! **Reading never writes** (the 82df8a3 lesson: a status refresh that
//! touched the project would re-trigger the watcher). Every invocation
//! carries `--no-optional-locks` (+ `GIT_OPTIONAL_LOCKS=0`), so `status`
//! never refreshes the index on disk; `diff`/`show`/`rev-parse` are reads.
//! Commit and revert DO write — commit to `.git/` only (the watcher
//! ignores it), revert to the scope files, which the session then reloads
//! through the ordinary external-change path (barrier snapshot) — under
//! the session's write hold, so no edit can land between the checkout
//! and the reload (`http.rs`). A merge / rebase / cherry-pick / revert the
//! shell left unfinished is a state, and both writes refuse until it is
//! done: a partial commit mid-merge is what git itself refuses.
//!
//! **Scope**: this pipeline's `.cic`, its `.cic.layout.json`, and the
//! `scripts/*.py` beside it — exactly the set `apply_text` may write —
//! minus whatever `.gitignore` excludes (git does not list ignored files,
//! and `git add` refuses a whole list over one; an ignored PIPELINE is a
//! typed refusal). Never `git add -A`; `commit -- <paths>` commits only
//! those paths, so whatever else the user staged in a shell stays staged
//! and uncommitted.
//! The project directory need not be the repository root (`examples/wall`
//! is the normal case): git runs with the project dir as cwd, status
//! paths come back root-relative and are mapped through the prefix.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cicada_lang::{Document, Line};

use crate::protocol::{
    ChangeKind, CommitResponse, FileStatus, GitErrorKind, GitState, GitStatusResponse, NodeChange,
    Operation, PipelineGitStatus, ProjectGit, RemovedNode, RepoInfo, RevertResponse, ScopeFile,
    Upstream,
};

/// How long one git command may take before it is killed (a hook or a
/// signing prompt that hangs must surface as a typed timeout, not a stuck
/// request).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ------------------------------------------------------------- errors --

/// A git invocation that did not do its job — the unexpected failures.
/// The expected non-answers (not a repo, unborn, detached, locked) are
/// STATES, not errors: see [`GitState`].
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The binary is not on PATH.
    #[error("no `{program}` on PATH — install git or put it on PATH")]
    NotFound {
        /// What was looked for.
        program: String,
    },
    /// Spawning or talking to the process failed.
    #[error("running `{command}`: {source}")]
    Spawn {
        /// The command (without the global options).
        command: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// Non-zero exit the caller did not expect.
    #[error("`{command}` exited {}: {stderr}", code.map_or_else(|| "by signal".to_owned(), |c| c.to_string()))]
    Failed {
        /// The command.
        command: String,
        /// Exit code (`None` = killed by a signal).
        code: Option<i32>,
        /// Captured stderr, trimmed.
        stderr: String,
    },
    /// Killed after the timeout.
    #[error("`{command}` did not finish within {timeout_ms} ms and was killed")]
    Timeout {
        /// The command.
        command: String,
        /// The budget it blew.
        timeout_ms: u128,
    },
    /// Output git produced that this module cannot read.
    #[error("`{command}` produced unreadable output: {message}")]
    Unparsable {
        /// The command.
        command: String,
        /// What was wrong.
        message: String,
    },
    /// Reading a project file failed.
    #[error("reading {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
}

/// Why a git route refused — one enum, one `kind` per variant
/// ([`GitErrorKind`]), one HTTP status per kind (`http.rs`).
#[derive(Debug, thiserror::Error)]
pub enum GitRefusal {
    /// Malformed request.
    #[error("{0}")]
    Protocol(String),
    /// The caller does not hold the write lease.
    #[error(
        "the write lease is required: name the writer's client id (`client` in the body or \
         X-Cicada-Client)"
    )]
    Lease,
    /// Nobody holds the write lease: no client has the pipeline open.
    #[error(
        "the write lease is required and nobody holds it: no client has `{0}` open — open it in \
         the app first"
    )]
    NoWriter(String),
    /// `?pipeline=` names no pipeline of the project.
    #[error("no pipeline `{0}` in the project (see /api/project)")]
    NoSuchPipeline(String),
    /// Not inside a git work tree.
    #[error("the project directory is not inside a git repository — `git init` it first")]
    NotARepo,
    /// No git binary.
    #[error("no `git` on PATH — install git or put it on PATH")]
    GitNotFound,
    /// `index.lock` is held.
    #[error("another git process holds index.lock — retry when it has finished")]
    Locked,
    /// The scope is clean.
    #[error("nothing to commit — no file of this pipeline's scope differs from HEAD")]
    NothingToCommit,
    /// The (requested) scope is clean.
    #[error("nothing to revert — no file of the requested scope differs from HEAD")]
    NothingToRevert,
    /// A path has no HEAD version.
    #[error("`{0}` has no HEAD version to revert to (untracked or not yet committed)")]
    Untracked(String),
    /// The pipeline is matched by `.gitignore`.
    #[error(
        "`{0}` is ignored by a .gitignore rule — git refuses to add it; un-ignore it (or \
         `git add -f` it once in a shell) to commit from the app"
    )]
    Ignored(String),
    /// A multi-step git operation is in progress.
    #[error(
        "a {} is in progress in the repository — finish or abort it in your shell first \
         (committing or reverting part of it from the app would be a partial step)",
        .0.name()
    )]
    OperationInProgress(Operation),
    /// Empty commit message.
    #[error("the commit message is empty")]
    EmptyMessage,
    /// A path outside the scope.
    #[error("`{path}`: {why}")]
    PathNotAllowed {
        /// The offending path.
        path: String,
        /// The rule it broke.
        why: String,
    },
    /// An unexpected git failure.
    #[error(transparent)]
    Git(#[from] GitError),
    /// The files were restored but the session could not reload them.
    #[error("the files were restored to HEAD but the session could not reload them: {0}")]
    Reload(String),
    /// The server failed around the git call.
    #[error("internal failure around the git call: {0}")]
    Internal(String),
}

impl GitRefusal {
    /// The machine `kind`.
    #[must_use]
    pub fn kind(&self) -> GitErrorKind {
        match self {
            Self::Protocol(_) => GitErrorKind::Protocol,
            Self::NoSuchPipeline(_) => GitErrorKind::NoSuchPipeline,
            Self::Lease | Self::NoWriter(_) => GitErrorKind::Lease,
            Self::NotARepo => GitErrorKind::NotARepo,
            Self::GitNotFound | Self::Git(GitError::NotFound { .. }) => GitErrorKind::GitNotFound,
            Self::Locked => GitErrorKind::Locked,
            Self::NothingToCommit => GitErrorKind::NothingToCommit,
            Self::NothingToRevert => GitErrorKind::NothingToRevert,
            Self::Untracked(_) => GitErrorKind::Untracked,
            Self::Ignored(_) => GitErrorKind::Ignored,
            Self::OperationInProgress(_) => GitErrorKind::OperationInProgress,
            Self::EmptyMessage => GitErrorKind::EmptyMessage,
            Self::PathNotAllowed { .. } => GitErrorKind::PathNotAllowed,
            Self::Git(GitError::Timeout { .. }) => GitErrorKind::GitTimeout,
            Self::Git(GitError::Read { .. }) => GitErrorKind::IoError,
            Self::Git(_) => GitErrorKind::GitFailed,
            Self::Reload(_) => GitErrorKind::ReloadFailed,
            Self::Internal(_) => GitErrorKind::Internal,
        }
    }

    /// The `{kind, message, …details}` body: a failed command carries
    /// `command`, `code`, `stderr`; a refused path carries `path`; an
    /// operation in progress carries `operation`.
    #[must_use]
    pub fn body(&self) -> serde_json::Value {
        let mut body = serde_json::Map::new();
        body.insert(
            "kind".to_owned(),
            serde_json::to_value(self.kind()).unwrap_or_default(),
        );
        body.insert(
            "message".to_owned(),
            serde_json::Value::String(self.to_string()),
        );
        match self {
            Self::Git(GitError::Failed {
                command,
                code,
                stderr,
            }) => {
                body.insert("command".to_owned(), command.as_str().into());
                body.insert("code".to_owned(), (*code).into());
                body.insert("stderr".to_owned(), stderr.as_str().into());
            }
            Self::Git(GitError::Timeout { command, .. } | GitError::Spawn { command, .. }) => {
                body.insert("command".to_owned(), command.as_str().into());
            }
            Self::PathNotAllowed { path, .. }
            | Self::Untracked(path)
            | Self::Ignored(path)
            | Self::NoSuchPipeline(path)
            | Self::NoWriter(path) => {
                body.insert("path".to_owned(), path.as_str().into());
            }
            Self::OperationInProgress(operation) => {
                body.insert("operation".to_owned(), operation.name().into());
            }
            _ => {}
        }
        serde_json::Value::Object(body)
    }
}

/// Map an unexpected git failure to the refusal it really is: a write
/// that met `index.lock` is [`GitRefusal::Locked`] (a stale lock from a
/// crashed git looks exactly like a live one to git itself).
fn refuse(error: GitError) -> GitRefusal {
    match error {
        GitError::Failed { ref stderr, .. } if stderr.contains("index.lock") => GitRefusal::Locked,
        GitError::NotFound { .. } => GitRefusal::GitNotFound,
        other => GitRefusal::Git(other),
    }
}

// -------------------------------------------------------------- scope --

/// The commit scope of one pipeline, project-relative and `/`-separated:
/// its `.cic`, its sidecar, and the `scripts/` directory beside it — the
/// same set `apply_text` may write (`session.rs` `classify_edit_path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// `dir/p.cic`.
    pub pipeline: String,
    /// `dir/p.cic.layout.json`.
    pub sidecar: String,
    /// `dir/scripts` (no trailing slash).
    pub scripts_dir: String,
}

impl Scope {
    /// The scope of a project-relative pipeline path.
    #[must_use]
    pub fn for_pipeline(relative: &str) -> Self {
        let scripts_dir = match relative.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/scripts"),
            None => "scripts".to_owned(),
        };
        Self {
            pipeline: relative.to_owned(),
            sidecar: format!("{relative}.layout.json"),
            scripts_dir,
        }
    }

    /// Is `path` one of the scope's files? Scripts are `<scripts_dir>/
    /// <name>.py` directly in the directory — exactly what discovery picks
    /// up (`scripts.rs`) and what `apply_text` allows.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        if path == self.pipeline || path == self.sidecar {
            return true;
        }
        match path.rsplit_once('/') {
            Some((dir, name)) => {
                dir == self.scripts_dir
                    && Path::new(name).extension().is_some_and(|ext| ext == "py")
                    && name.len() > 3
            }
            None => false,
        }
    }

    /// Pathspecs for a status call with the project dir as cwd.
    fn pathspecs(&self) -> [String; 3] {
        [
            self.pipeline.clone(),
            self.sidecar.clone(),
            format!("{}/", self.scripts_dir),
        ]
    }

    /// Deterministic order for the response: pipeline, sidecar, scripts
    /// by name.
    fn rank(&self, path: &str) -> (u8, String) {
        if path == self.pipeline {
            (0, String::new())
        } else if path == self.sidecar {
            (1, String::new())
        } else {
            (2, path.to_owned())
        }
    }
}

// ------------------------------------------------------------ process --

/// Where the repository is, relative to the project dir. Cached per
/// [`Git`]: it cannot change while a project is served short of deleting
/// `.git`, which the next status notices ("not a git repository" → the
/// cache is dropped).
#[derive(Debug, Clone)]
struct Layout {
    /// `--show-toplevel`.
    root: String,
    /// `--show-prefix` without its trailing slash (`""` at the root).
    prefix: String,
    /// `--absolute-git-dir` (a worktree's private git dir when applicable).
    git_dir: PathBuf,
}

impl Layout {
    /// Root-relative path of a project-relative one.
    fn to_root(&self, project_relative: &str) -> String {
        if self.prefix.is_empty() {
            project_relative.to_owned()
        } else {
            format!("{}/{project_relative}", self.prefix)
        }
    }

    /// Project-relative path of a root-relative one (`None` = outside the
    /// project).
    fn to_project<'a>(&self, root_relative: &'a str) -> Option<&'a str> {
        if self.prefix.is_empty() {
            Some(root_relative)
        } else {
            root_relative
                .strip_prefix(self.prefix.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
        }
    }
}

/// Captured process output.
struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Output {
    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }
}

/// One project's git handle: the binary, the project dir every command
/// runs in, the timeout, and the cached layout.
pub struct Git {
    program: OsString,
    project_dir: PathBuf,
    timeout: Duration,
    layout: Mutex<Option<Layout>>,
}

impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Git")
            .field("program", &self.program)
            .field("project_dir", &self.project_dir)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Git {
    /// A handle over `project_dir` using `git` from PATH and
    /// [`DEFAULT_TIMEOUT`].
    #[must_use]
    pub fn new(project_dir: &Path) -> Self {
        Self {
            program: OsString::from("git"),
            project_dir: plain_path(project_dir),
            timeout: DEFAULT_TIMEOUT,
            layout: Mutex::new(None),
        }
    }

    /// Use another binary (tests: a name that does not exist → the
    /// `git_not_found` state).
    #[must_use]
    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }

    /// Use another timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The project directory commands run in.
    #[must_use]
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// THE command builder — every invocation goes through here: cwd = the
    /// project dir; `--no-optional-locks` and `--literal-pathspecs` before
    /// the subcommand; `GIT_OPTIONAL_LOCKS=0` as the belt; no terminal
    /// prompts; C-locale messages (this module matches on them); the
    /// ambient `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` dropped so a
    /// hook-spawned server still means THIS project.
    #[must_use]
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .current_dir(&self.project_dir)
            .arg("--no-optional-locks")
            .arg("--literal-pathspecs")
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Run with captured output and the timeout; `stdin`, when given, is
    /// written on its own thread (a pipe holds ~4 KB: a commit message
    /// longer than that against a git that exits early — a failing hook —
    /// would otherwise block the write, and a hook that never exits would
    /// block it forever, ahead of the timeout loop). A broken pipe on
    /// stdin is not an error of ours: git stopped reading, and its exit
    /// code + stderr say why. Non-zero exits are NOT errors here — callers
    /// decide (`status` exits 128 outside a repo, which is a state).
    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, GitError> {
        let command_text = format!("git {}", args.join(" "));
        let mut command = self.command(args);
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(GitError::NotFound {
                    program: self.program.to_string_lossy().into_owned(),
                });
            }
            Err(source) => {
                return Err(GitError::Spawn {
                    command: command_text,
                    source,
                });
            }
        };
        // Drain both pipes on their own threads: a process that fills one
        // while we block on the other would deadlock.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = std::thread::spawn(move || drain(stdout));
        let err_reader = std::thread::spawn(move || drain(stderr));
        let writer = match (stdin, child.stdin.take()) {
            (Some(bytes), Some(mut pipe)) => {
                let bytes = bytes.to_vec();
                Some(std::thread::spawn(move || {
                    let written = pipe.write_all(&bytes).and_then(|()| pipe.flush());
                    drop(pipe);
                    written
                }))
            }
            _ => None,
        };
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(GitError::Timeout {
                            command: command_text,
                            timeout_ms: self.timeout.as_millis(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(source) => {
                    return Err(GitError::Spawn {
                        command: command_text,
                        source,
                    });
                }
            }
        };
        let stdout = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();
        if let Some(writer) = writer {
            // The child has exited, so the write has ended one way or the
            // other: complete, or refused with a broken pipe — which git's
            // own exit status explains better than the OS error would.
            let _ = writer.join();
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Run and require exit 0.
    fn run_ok(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        let output = self.run(args, stdin)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(GitError::Failed {
                command: format!("git {}", args.join(" ")),
                code: output.status.code(),
                stderr: output.stderr_text(),
            })
        }
    }

    /// Where the repository is (`Ok(None)` = the project is not in one).
    fn layout(&self) -> Result<Option<Layout>, GitError> {
        if let Some(cached) = self
            .layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Ok(Some(cached));
        }
        let args = [
            "rev-parse",
            "--show-toplevel",
            "--show-prefix",
            "--absolute-git-dir",
        ];
        let output = self.run(&args, None)?;
        if !output.status.success() {
            let stderr = output.stderr_text();
            if stderr.contains("not a git repository") {
                return Ok(None);
            }
            return Err(GitError::Failed {
                command: format!("git {}", args.join(" ")),
                code: output.status.code(),
                stderr,
            });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = text.lines().collect();
        let [root, prefix, git_dir] = lines[..] else {
            return Err(GitError::Unparsable {
                command: format!("git {}", args.join(" ")),
                message: format!("expected 3 lines, got {}: {text:?}", lines.len()),
            });
        };
        let layout = Layout {
            root: root.to_owned(),
            prefix: prefix.trim_end_matches('/').to_owned(),
            git_dir: PathBuf::from(git_dir),
        };
        *self
            .layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(layout.clone());
        Ok(Some(layout))
    }

    fn forget_layout(&self) {
        *self
            .layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// State + the status entries under `pathspecs` (cwd-relative). One
    /// `rev-parse` (cached after the first) and one `status`; nothing that
    /// writes.
    fn survey(&self, pathspecs: &[String], all_untracked: bool) -> Result<Survey, GitError> {
        let layout = match self.layout() {
            Ok(Some(layout)) => layout,
            Ok(None) => return Ok(Survey::bare(GitState::NotARepo)),
            Err(GitError::NotFound { .. }) => return Ok(Survey::bare(GitState::GitNotFound)),
            Err(error) => return Err(error),
        };
        let mut args: Vec<&str> = vec![
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            if all_untracked {
                "--untracked-files=all"
            } else {
                "--untracked-files=normal"
            },
        ];
        if all_untracked {
            // An ignored pipeline must not pass for a clean tracked one.
            args.push("--ignored=matching");
        }
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
        let output = self.run(&args, None)?;
        if !output.status.success() {
            let stderr = output.stderr_text();
            if stderr.contains("not a git repository") {
                self.forget_layout();
                return Ok(Survey::bare(GitState::NotARepo));
            }
            return Err(GitError::Failed {
                command: format!("git {}", args.join(" ")),
                code: output.status.code(),
                stderr,
            });
        }
        let parsed =
            parse_porcelain_v2(&output.stdout).map_err(|message| GitError::Unparsable {
                command: format!("git {}", args.join(" ")),
                message,
            })?;
        let locked = layout.git_dir.join("index.lock").exists();
        let info = parsed
            .headers
            .info(&layout, operation_in_progress(&layout.git_dir));
        let state = if locked {
            GitState::Locked(info)
        } else {
            GitState::Repo(info)
        };
        Ok(Survey {
            state,
            layout: Some(layout),
            unborn: parsed.headers.unborn(),
            entries: parsed.entries,
        })
    }

    /// `GET /api/git/status`: the state, this pipeline's node markers, and
    /// the dirty files of the scope. At most one `status`, one `diff` and
    /// one `show` (plus the cached `rev-parse`); reads only.
    ///
    /// # Errors
    ///
    /// [`GitRefusal::Git`] for an unexpected git failure or an unreadable
    /// pipeline file — the expected non-answers are states in the response.
    pub fn status(&self, scope: &Scope) -> Result<GitStatusResponse, GitRefusal> {
        let pipeline_path = self.project_dir.join(&scope.pipeline);
        let bytes = std::fs::read(&pipeline_path).map_err(|source| GitError::Read {
            path: pipeline_path.clone(),
            source,
        })?;
        let text_hash = blake3::hash(&bytes).to_hex().to_string();
        let work_text = String::from_utf8_lossy(&bytes);
        let survey = self.survey(&scope.pathspecs(), true).map_err(refuse)?;
        let Some(layout) = survey.layout.as_ref() else {
            return Ok(GitStatusResponse {
                state: survey.state,
                pipeline: PipelineGitStatus {
                    path: scope.pipeline.clone(),
                    tracked: false,
                    ignored: false,
                    dirty: false,
                    nodes: Vec::new(),
                    removed: Vec::new(),
                },
                scope: Vec::new(),
                text_hash,
            });
        };
        let files = survey.scope_files(scope);
        let pipeline_entry = survey.entry_for(&layout.to_root(&scope.pipeline));
        let tracked = pipeline_entry.is_none_or(Entry::tracked);
        let ignored = pipeline_entry.is_some_and(Entry::ignored);
        let dirty = pipeline_entry.is_some();
        let in_head = survey.pipeline_in_head(scope);
        let work = Document::parse(&work_text);
        let (nodes, removed) = if !in_head {
            // No HEAD version to diff against: every node is new.
            (
                line_nodes(&work)
                    .into_iter()
                    .map(|node| NodeChange {
                        name: node.name,
                        change: ChangeKind::Added,
                        from: None,
                    })
                    .collect(),
                Vec::new(),
            )
        } else if !dirty {
            (Vec::new(), Vec::new())
        } else {
            let diff = self
                .run_ok(
                    &[
                        "diff",
                        "-U0",
                        "--no-color",
                        "--no-ext-diff",
                        "--no-textconv",
                        "HEAD",
                        "--",
                        &scope.pipeline,
                    ],
                    None,
                )
                .map_err(refuse)?;
            let hunks = parse_hunks(&String::from_utf8_lossy(&diff));
            if hunks.is_empty() {
                // Staged, then put back by hand: the index differs, the
                // working tree does not. No line differs → no marker.
                (Vec::new(), Vec::new())
            } else {
                let spec = format!("HEAD:{}", layout.to_root(&scope.pipeline));
                let head_bytes = self.run_ok(&["show", &spec], None).map_err(refuse)?;
                let head = Document::parse(&String::from_utf8_lossy(&head_bytes));
                markers(&head, &work, &hunks)
            }
        };
        Ok(GitStatusResponse {
            state: survey.state,
            pipeline: PipelineGitStatus {
                path: scope.pipeline.clone(),
                tracked,
                ignored,
                dirty,
                nodes,
                removed,
            },
            scope: files,
            text_hash,
        })
    }

    /// `POST /api/git/commit`: stage the dirty scope files and commit
    /// exactly them (`commit -- <paths>`: other staged paths stay staged),
    /// the message verbatim on stdin. Ignored files are not in the scope
    /// (`git add` would refuse the whole list over one of them); an
    /// ignored PIPELINE is a refusal of its own.
    ///
    /// # Errors
    ///
    /// [`GitRefusal::EmptyMessage`], the state refusals (`NotARepo`,
    /// `GitNotFound`, `Locked`, `OperationInProgress`),
    /// [`GitRefusal::Ignored`], [`GitRefusal::NothingToCommit`], or an
    /// unexpected git failure.
    pub fn commit(&self, scope: &Scope, message: &str) -> Result<CommitResponse, GitRefusal> {
        if message.trim().is_empty() {
            return Err(GitRefusal::EmptyMessage);
        }
        let survey = self.survey(&scope.pathspecs(), true).map_err(refuse)?;
        let layout = survey.repo_or_refuse()?;
        if survey
            .entry_for(&layout.to_root(&scope.pipeline))
            .is_some_and(Entry::ignored)
        {
            return Err(GitRefusal::Ignored(scope.pipeline.clone()));
        }
        let files = survey.scope_files(scope);
        if files.is_empty() {
            return Err(GitRefusal::NothingToCommit);
        }
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut add = vec!["add", "--"];
        add.extend(paths.iter().copied());
        self.run_ok(&add, None).map_err(refuse)?;
        let mut commit = vec!["commit", "--quiet", "--cleanup=verbatim", "--file=-", "--"];
        commit.extend(paths.iter().copied());
        self.run_ok(&commit, Some(message.as_bytes()))
            .map_err(refuse)?;
        let log_args = ["log", "-1", "--format=%H%n%h%n%s", "HEAD"];
        let log = self.run_ok(&log_args, None).map_err(refuse)?;
        let log = String::from_utf8_lossy(&log);
        let mut lines = log.lines();
        let (Some(hash), Some(short)) = (lines.next(), lines.next()) else {
            return Err(GitRefusal::Git(GitError::Unparsable {
                command: format!("git {}", log_args.join(" ")),
                message: format!("expected hash, short, subject; got {log:?}"),
            }));
        };
        let summary = lines.next().unwrap_or_default().to_owned();
        let listed = self
            .run_ok(
                &[
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--name-only",
                    "-r",
                    "-z",
                    "HEAD",
                ],
                None,
            )
            .map_err(refuse)?;
        let committed: Vec<String> = listed
            .split(|b| *b == 0)
            .filter(|part| !part.is_empty())
            .map(|part| {
                let root_relative = String::from_utf8_lossy(part);
                layout
                    .to_project(&root_relative)
                    .map_or_else(|| root_relative.clone().into_owned(), str::to_owned)
            })
            .collect();
        Ok(CommitResponse {
            hash: hash.to_owned(),
            short: short.to_owned(),
            summary,
            files: committed,
        })
    }

    /// `POST /api/git/revert`: `git checkout HEAD -- <paths>` for the dirty
    /// scope files that HAVE a HEAD version (`paths` narrows the set and is
    /// validated against the scope). Files without one — untracked,
    /// index-only — are never deleted: an explicit request for one is
    /// refused `untracked`; implicitly they are reported and left alone.
    /// The caller reloads the session afterwards ([`crate::session::
    /// Session::reload_from_disk`]) — see `http.rs`.
    ///
    /// # Errors
    ///
    /// The state refusals, [`GitRefusal::PathNotAllowed`],
    /// [`GitRefusal::Untracked`], [`GitRefusal::NothingToRevert`], or an
    /// unexpected git failure.
    pub fn revert(&self, scope: &Scope, paths: Option<&[String]>) -> Result<Reverted, GitRefusal> {
        if let Some(requested) = paths {
            for path in requested {
                if !scope.contains(path) {
                    return Err(GitRefusal::PathNotAllowed {
                        path: path.clone(),
                        why: format!(
                            "a revert may restore this pipeline (`{}`), its sidecar (`{}`), or a \
                             script beside it (`{}/<name>.py`) — nothing else",
                            scope.pipeline, scope.sidecar, scope.scripts_dir
                        ),
                    });
                }
            }
        }
        let survey = self.survey(&scope.pathspecs(), true).map_err(refuse)?;
        survey.repo_or_refuse()?;
        // The pipeline itself without a HEAD version (untracked, ignored,
        // index-only, unborn branch): there is nothing to put back —
        // refuse, whether asked explicitly or by omission.
        if !survey.pipeline_in_head(scope) {
            return Err(GitRefusal::Untracked(scope.pipeline.clone()));
        }
        // One rule for "has a HEAD version": the `in_head` the status
        // publishes on every scope file — so what a client listed from the
        // status as revertable is exactly what restores here.
        let dirty = survey.scope_files(scope);
        let wanted = |path: &str| paths.is_none_or(|set| set.iter().any(|p| p == path));
        let mut restore = Vec::new();
        let mut untracked = Vec::new();
        for file in dirty.iter().filter(|f| wanted(&f.path)) {
            if file.in_head {
                restore.push(file.path.clone());
            } else {
                untracked.push(file.path.clone());
            }
        }
        // An explicit ask for a file we would have to delete: refuse.
        if let Some(requested) = paths
            && let Some(path) = untracked.iter().find(|p| requested.contains(p))
        {
            return Err(GitRefusal::Untracked(path.clone()));
        }
        if restore.is_empty() {
            return Err(GitRefusal::NothingToRevert);
        }
        let mut checkout = vec!["checkout", "--quiet", "HEAD", "--"];
        checkout.extend(restore.iter().map(String::as_str));
        self.run_ok(&checkout, None).map_err(refuse)?;
        let touched_scripts = restore
            .iter()
            .any(|p| p != &scope.pipeline && p != &scope.sidecar);
        Ok(Reverted {
            reverted: restore,
            untracked,
            touched_scripts,
        })
    }

    /// The `GET /api/project` summary: state tag, branch, and how many
    /// entries `git status` lists under the project directory. Never
    /// fails the route: an unexpected git failure is reported as kind
    /// `error` with the message.
    #[must_use]
    pub fn summary(&self) -> ProjectGit {
        match self.survey(&[".".to_owned()], false) {
            Ok(survey) => ProjectGit {
                kind: survey.state.tag().to_owned(),
                branch: survey.state.repo().and_then(|info| info.branch.clone()),
                dirty_count: survey.entries.len(),
                error: None,
            },
            Err(error) => ProjectGit {
                kind: "error".to_owned(),
                branch: None,
                dirty_count: 0,
                error: Some(error.to_string()),
            },
        }
    }
}

/// What [`Git::revert`] did; `http.rs` turns it into the response after
/// the session reload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reverted {
    /// Restored to HEAD (project-relative).
    pub reverted: Vec<String>,
    /// Dirty, no HEAD version, left alone.
    pub untracked: Vec<String>,
    /// A `scripts/*.py` was restored — the reload must rescan scripts.
    pub touched_scripts: bool,
}

impl Reverted {
    /// The response once the session has (or has not needed to) reload.
    #[must_use]
    pub fn into_response(self, reloaded: bool) -> RevertResponse {
        RevertResponse {
            reverted: self.reverted,
            untracked: self.untracked,
            reloaded,
        }
    }
}

fn drain(pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut buffer = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buffer);
    }
    buffer
}

/// The multi-step operation the repository is in the middle of, by the
/// files git leaves in its directory while one is unfinished (the same
/// tests `git status` and shell prompts use).
fn operation_in_progress(git_dir: &Path) -> Option<Operation> {
    if git_dir.join("MERGE_HEAD").exists() {
        Some(Operation::Merge)
    } else if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        Some(Operation::Rebase)
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(Operation::CherryPick)
    } else if git_dir.join("REVERT_HEAD").exists() {
        Some(Operation::Revert)
    } else {
        None
    }
}

/// Windows' `\\?\` verbatim prefix off a canonical path: as a process cwd
/// it confuses git's own path printing, and git never needs it.
fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_owned(),
    }
}

// ------------------------------------------------------------- status --

/// One `git status --porcelain=v2` record that names a path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Root-relative path (the NEW path for renames).
    path: String,
    /// Record kind.
    kind: EntryKind,
    /// Index status letter (`.` = unchanged).
    x: char,
    /// Working-tree status letter.
    y: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// `1` — ordinary changed entry.
    Changed,
    /// `2` — renamed or copied in the index.
    Renamed,
    /// `u` — unmerged.
    Unmerged,
    /// `?` — untracked.
    Untracked,
    /// `!` — ignored.
    Ignored,
}

impl Entry {
    /// Known to git (in the index or HEAD).
    fn tracked(&self) -> bool {
        !matches!(self.kind, EntryKind::Untracked | EntryKind::Ignored)
    }

    /// Matched by `.gitignore` (and not tracked).
    fn ignored(&self) -> bool {
        self.kind == EntryKind::Ignored
    }

    /// HEAD has this path: tracked, not an index addition, not the new
    /// side of a rename.
    fn in_head(&self) -> bool {
        self.tracked() && self.kind != EntryKind::Renamed && self.x != 'A'
    }

    fn file_status(&self) -> FileStatus {
        match self.kind {
            EntryKind::Untracked | EntryKind::Ignored => FileStatus::Untracked,
            EntryKind::Renamed => FileStatus::Renamed,
            // An unfinished merge's file is, to the working tree, a
            // modification of HEAD; committing it stages the resolution.
            EntryKind::Unmerged => FileStatus::Modified,
            EntryKind::Changed => {
                if self.y == 'D' || (self.x == 'D' && self.y == '.') {
                    FileStatus::Deleted
                } else if self.x == 'A' {
                    FileStatus::Added
                } else {
                    FileStatus::Modified
                }
            }
        }
    }
}

/// The `--branch` headers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Headers {
    oid: Option<String>,
    head: Option<String>,
    upstream: Option<String>,
    ahead_behind: Option<(u32, u32)>,
}

impl Headers {
    fn unborn(&self) -> bool {
        self.oid.as_deref() == Some("(initial)")
    }

    fn info(&self, layout: &Layout, operation: Option<Operation>) -> RepoInfo {
        let unborn = self.unborn();
        let branch = match self.head.as_deref() {
            None | Some("(detached)") => None,
            Some(name) => Some(name.to_owned()),
        };
        let head_short = if unborn {
            None
        } else {
            self.oid
                .as_deref()
                .map(|oid| oid.chars().take(7).collect::<String>())
        };
        let upstream = self.upstream.as_ref().map(|name| {
            let (ahead, behind) = self.ahead_behind.unwrap_or_default();
            Upstream {
                name: name.clone(),
                ahead,
                behind,
            }
        });
        RepoInfo {
            root: layout.root.clone(),
            prefix: layout.prefix.clone(),
            branch,
            head_short,
            upstream,
            unborn,
            operation,
        }
    }
}

struct Parsed {
    headers: Headers,
    entries: Vec<Entry>,
}

/// Parse `git status --porcelain=v2 -z --branch` output: NUL-terminated
/// records; a `2` (rename) record is followed by its original path as a
/// record of its own.
fn parse_porcelain_v2(bytes: &[u8]) -> Result<Parsed, String> {
    let mut headers = Headers::default();
    let mut entries = Vec::new();
    let mut records = bytes
        .split(|b| *b == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8_lossy(record).into_owned());
    while let Some(record) = records.next() {
        let (tag, rest) = record
            .split_once(' ')
            .ok_or_else(|| format!("record without a tag: {record:?}"))?;
        match tag {
            "#" => {
                let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
                match key {
                    "branch.oid" => headers.oid = Some(value.to_owned()),
                    "branch.head" => headers.head = Some(value.to_owned()),
                    "branch.upstream" => headers.upstream = Some(value.to_owned()),
                    "branch.ab" => {
                        let mut parts = value.split(' ');
                        let ahead = parts
                            .next()
                            .and_then(|p| p.strip_prefix('+'))
                            .and_then(|p| p.parse().ok());
                        let behind = parts
                            .next()
                            .and_then(|p| p.strip_prefix('-'))
                            .and_then(|p| p.parse().ok());
                        if let (Some(ahead), Some(behind)) = (ahead, behind) {
                            headers.ahead_behind = Some((ahead, behind));
                        } else {
                            return Err(format!("unreadable branch.ab: {value:?}"));
                        }
                    }
                    _ => {}
                }
            }
            "1" => {
                // <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let [xy, _, _, _, _, _, _, path] = fields[..] else {
                    return Err(format!("short `1` record: {record:?}"));
                };
                let (x, y) = xy_of(xy, &record)?;
                entries.push(Entry {
                    path: path.to_owned(),
                    kind: EntryKind::Changed,
                    x,
                    y,
                });
            }
            "2" => {
                // <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>, then
                // the original path as the next record.
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                let [xy, _, _, _, _, _, _, _, path] = fields[..] else {
                    return Err(format!("short `2` record: {record:?}"));
                };
                let (x, y) = xy_of(xy, &record)?;
                let _original = records.next().ok_or_else(|| {
                    format!("rename record without its original path: {record:?}")
                })?;
                entries.push(Entry {
                    path: path.to_owned(),
                    kind: EntryKind::Renamed,
                    x,
                    y,
                });
            }
            "u" => {
                // <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
                let fields: Vec<&str> = rest.splitn(10, ' ').collect();
                let [xy, _, _, _, _, _, _, _, _, path] = fields[..] else {
                    return Err(format!("short `u` record: {record:?}"));
                };
                let (x, y) = xy_of(xy, &record)?;
                entries.push(Entry {
                    path: path.to_owned(),
                    kind: EntryKind::Unmerged,
                    x,
                    y,
                });
            }
            "?" => entries.push(Entry {
                path: rest.to_owned(),
                kind: EntryKind::Untracked,
                x: '?',
                y: '?',
            }),
            "!" => entries.push(Entry {
                path: rest.to_owned(),
                kind: EntryKind::Ignored,
                x: '!',
                y: '!',
            }),
            other => return Err(format!("unknown record tag {other:?}: {record:?}")),
        }
    }
    Ok(Parsed { headers, entries })
}

fn xy_of(xy: &str, record: &str) -> Result<(char, char), String> {
    let mut chars = xy.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(x), Some(y), None) => Ok((x, y)),
        _ => Err(format!("bad XY field {xy:?} in {record:?}")),
    }
}

/// A state + status snapshot.
struct Survey {
    state: GitState,
    layout: Option<Layout>,
    unborn: bool,
    entries: Vec<Entry>,
}

impl Survey {
    fn bare(state: GitState) -> Self {
        Self {
            state,
            layout: None,
            unborn: false,
            entries: Vec::new(),
        }
    }

    /// The layout, or the refusal the state means for a write: locked,
    /// not a repo, no git, or a merge/rebase/cherry-pick/revert the shell
    /// has not finished.
    fn repo_or_refuse(&self) -> Result<&Layout, GitRefusal> {
        match (&self.state, &self.layout) {
            (GitState::Repo(info), Some(layout)) => match info.operation {
                Some(operation) => Err(GitRefusal::OperationInProgress(operation)),
                None => Ok(layout),
            },
            (GitState::Locked(_), _) => Err(GitRefusal::Locked),
            (GitState::GitNotFound, _) => Err(GitRefusal::GitNotFound),
            (GitState::NotARepo, _) | (GitState::Repo(_), None) => Err(GitRefusal::NotARepo),
        }
    }

    fn entry_for(&self, root_relative: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == root_relative)
    }

    /// HEAD has a version of the pipeline: not on an unborn branch, and
    /// either clean (no entry) or an entry that is in HEAD.
    fn pipeline_in_head(&self, scope: &Scope) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        !self.unborn
            && self
                .entry_for(&layout.to_root(&scope.pipeline))
                .is_none_or(Entry::in_head)
    }

    /// The dirty files of the scope, project-relative, in scope order,
    /// each saying whether HEAD has it (`in_head` — the rule [`Git::revert`]
    /// restores by, published rather than re-derived by clients: a
    /// `deleted` status can be an `AD` entry with no HEAD version, and
    /// nothing on an unborn branch has one). Ignored files are left out:
    /// git does not list them and `git add` refuses a list that contains
    /// one — they are the user's explicit choice not to commit.
    /// (`--ignored=matching` is asked for so that the PIPELINE can be told
    /// apart from a clean tracked one.)
    fn scope_files(&self, scope: &Scope) -> Vec<ScopeFile> {
        let Some(layout) = &self.layout else {
            return Vec::new();
        };
        let mut files: Vec<ScopeFile> = self
            .entries
            .iter()
            .filter(|entry| !entry.ignored())
            .filter_map(|entry| {
                let path = layout.to_project(&entry.path)?;
                scope.contains(path).then(|| ScopeFile {
                    path: path.to_owned(),
                    status: entry.file_status(),
                    in_head: !self.unborn && entry.in_head(),
                })
            })
            .collect();
        files.sort_by_cached_key(|f| scope.rank(&f.path));
        files.dedup_by(|a, b| a.path == b.path);
        files
    }
}

// ------------------------------------------------------------ markers --

/// One hunk header of a unified diff: `@@ -old_start,old_count
/// +new_start,new_count @@`. Counts default to 1 when omitted; a count of
/// 0 is a pure insertion/deletion point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    /// First old-side line (1-based).
    pub old_start: usize,
    /// Old-side lines covered.
    pub old_count: usize,
    /// First new-side line (1-based).
    pub new_start: usize,
    /// New-side lines covered.
    pub new_count: usize,
}

/// The hunk headers of a `git diff -U0 --no-color` output (the bodies are
/// irrelevant: line ranges are all the markers need).
#[must_use]
pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    diff.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("@@ -")?;
            let (old, rest) = rest.split_once(" +")?;
            let (new, _) = rest.split_once(" @@")?;
            let (old_start, old_count) = range_of(old)?;
            let (new_start, new_count) = range_of(new)?;
            Some(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
            })
        })
        .collect()
}

fn range_of(text: &str) -> Option<(usize, usize)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// A line that names a node, as the view-model names it (`viewmodel.rs`):
/// a statement's primary target; a `#off` line's parsed name, else its
/// extracted name; a broken line's best-effort name — `line_N` as the last
/// resort for both.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineNode {
    /// 0-based line index.
    line: usize,
    /// The node name.
    name: String,
    /// Everything after the first `=`, trimmed — the rename-pairing key.
    rhs: Option<String>,
}

fn line_nodes(doc: &Document) -> Vec<LineNode> {
    doc.lines()
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let name = match line {
                Line::Statement { statement, .. }
                | Line::Disabled {
                    statement: Some(statement),
                    ..
                } => statement.name().to_owned(),
                Line::Disabled { name, .. } => name
                    .clone()
                    .unwrap_or_else(|| format!("line_{}", index + 1)),
                Line::Broken { node, .. } => node
                    .clone()
                    .unwrap_or_else(|| format!("line_{}", index + 1)),
                Line::Blank { .. } | Line::Comment { .. } | Line::Pragma { .. } => return None,
            };
            let rhs = line
                .raw()
                .split_once('=')
                .map(|(_, rhs)| rhs.trim().to_owned());
            Some(LineNode {
                line: index,
                name,
                rhs,
            })
        })
        .collect()
}

/// The markers for a working document against its HEAD version, given
/// the hunks of `git diff -U0 HEAD -- <path>`. Pure; by construction a
/// node is marked exactly when a hunk touches its line.
///
/// A rename is a removed + added pair with a byte-identical right-hand
/// side **within the same hunk**: the writer's `rename` gesture rewrites
/// the binding line in place (one line → one hunk, its old and new sides
/// together), whereas a binding deleted here and an unrelated one with
/// the same literal added elsewhere are two edits — two hunks — and are
/// reported as exactly that, `removed` + `added`.
#[must_use]
pub fn markers(
    head: &Document,
    work: &Document,
    hunks: &[Hunk],
) -> (Vec<NodeChange>, Vec<RemovedNode>) {
    // Line (1-based) → the index of the hunk that touches it.
    let mut new_changed: HashMap<usize, usize> = HashMap::new();
    let mut old_changed: HashMap<usize, usize> = HashMap::new();
    for (index, hunk) in hunks.iter().enumerate() {
        new_changed.extend((hunk.new_start..hunk.new_start + hunk.new_count).map(|l| (l, index)));
        old_changed.extend((hunk.old_start..hunk.old_start + hunk.old_count).map(|l| (l, index)));
    }
    let head_nodes = line_nodes(head);
    let work_nodes = line_nodes(work);
    let head_names: HashSet<&str> = head_nodes.iter().map(|n| n.name.as_str()).collect();
    let work_names: HashSet<&str> = work_nodes.iter().map(|n| n.name.as_str()).collect();

    // Removed candidates: HEAD bindings on removed lines whose name the
    // working tree no longer binds — with their hunk, unpaired so far.
    let mut removed: Vec<(&LineNode, usize, bool)> = head_nodes
        .iter()
        .filter_map(|node| {
            let hunk = *old_changed.get(&(node.line + 1))?;
            (!work_names.contains(node.name.as_str())).then_some((node, hunk, false))
        })
        .collect();

    let mut nodes = Vec::new();
    for node in &work_nodes {
        let Some(hunk) = new_changed.get(&(node.line + 1)).copied() else {
            continue;
        };
        if head_names.contains(node.name.as_str()) {
            nodes.push(NodeChange {
                name: node.name.clone(),
                change: ChangeKind::Modified,
                from: None,
            });
            continue;
        }
        // New name: a removed binding of the SAME hunk with the identical
        // right-hand side makes it a rename (first unpaired match, in
        // HEAD order).
        let pair = node.rhs.as_ref().and_then(|rhs| {
            removed.iter_mut().find(|(gone, gone_hunk, paired)| {
                !*paired && *gone_hunk == hunk && gone.rhs.as_ref() == Some(rhs)
            })
        });
        if let Some((gone, _, paired)) = pair {
            *paired = true;
            nodes.push(NodeChange {
                name: node.name.clone(),
                change: ChangeKind::Renamed,
                from: Some(gone.name.clone()),
            });
        } else {
            nodes.push(NodeChange {
                name: node.name.clone(),
                change: ChangeKind::Added,
                from: None,
            });
        }
    }
    let removed = removed
        .into_iter()
        .filter(|(_, _, paired)| !*paired)
        .map(|(gone, _, _)| RemovedNode {
            name: gone.name.clone(),
            line_in_head: gone.line + 1,
        })
        .collect();
    (nodes, removed)
}

/// A deterministic "which nodes does this text define" for tests and the
/// untracked case: names in line order.
#[must_use]
pub fn node_names(text: &str) -> Vec<String> {
    line_nodes(&Document::parse(text))
        .into_iter()
        .map(|n| n.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_carries_the_no_write_flags_and_the_project_cwd() {
        let project = PathBuf::from(if cfg!(windows) {
            r"C:\proj\examples\wall"
        } else {
            "/proj/examples/wall"
        });
        let git = Git::new(&project);
        for args in [
            vec!["status", "--porcelain=v2", "-z"],
            vec!["diff", "-U0", "HEAD", "--", "p.cic"],
            vec!["show", "HEAD:p.cic"],
            vec!["rev-parse", "--show-toplevel"],
            vec!["add", "--", "p.cic"],
            vec!["commit", "--file=-", "--", "p.cic"],
            vec!["checkout", "HEAD", "--", "p.cic"],
        ] {
            let command = git.command(&args);
            let argv: Vec<String> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                &argv[..2],
                ["--no-optional-locks", "--literal-pathspecs"],
                "{args:?}: the global flags precede the subcommand"
            );
            assert_eq!(&argv[2..], args.as_slice());
            assert_eq!(command.get_current_dir(), Some(project.as_path()));
            let envs: Vec<(String, Option<String>)> = command
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|v| v.to_string_lossy().into_owned()),
                    )
                })
                .collect();
            assert!(envs.contains(&("GIT_OPTIONAL_LOCKS".to_owned(), Some("0".to_owned()))));
            assert!(envs.contains(&("GIT_TERMINAL_PROMPT".to_owned(), Some("0".to_owned()))));
            assert!(
                envs.contains(&("GIT_DIR".to_owned(), None)),
                "GIT_DIR is dropped"
            );
        }
    }

    #[test]
    fn verbatim_prefix_is_stripped_for_the_cwd() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\proj")),
            PathBuf::from(r"C:\proj")
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(plain_path(Path::new("/proj")), PathBuf::from("/proj"));
    }

    #[test]
    fn scope_is_the_apply_text_set() {
        let scope = Scope::for_pipeline("examples/p.cic");
        assert_eq!(scope.sidecar, "examples/p.cic.layout.json");
        assert_eq!(scope.scripts_dir, "examples/scripts");
        assert!(scope.contains("examples/p.cic"));
        assert!(scope.contains("examples/p.cic.layout.json"));
        assert!(scope.contains("examples/scripts/a.py"));
        assert!(!scope.contains("examples/scripts/sub/a.py"));
        assert!(!scope.contains("examples/scripts/.py"));
        assert!(!scope.contains("examples/scripts/notes.txt"));
        assert!(!scope.contains("examples/q.cic"));
        assert!(!scope.contains("scripts/a.py"));
        let root = Scope::for_pipeline("p.cic");
        assert_eq!(root.scripts_dir, "scripts");
        assert!(root.contains("scripts/a.py"));
        assert_eq!(
            root.pathspecs(),
            [
                "p.cic".to_owned(),
                "p.cic.layout.json".to_owned(),
                "scripts/".to_owned()
            ]
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one record of every kind, then the shapes they become
    fn porcelain_v2_records_parse() {
        let raw = b"# branch.oid 0123456789abcdef0123456789abcdef01234567\0\
                    # branch.head main\0\
                    # branch.upstream origin/main\0\
                    # branch.ab +2 -1\0\
                    1 .M N... 100644 100644 100644 abc def examples/p.cic\0\
                    1 A. N... 000000 100644 100644 000 abc examples/scripts/new.py\0\
                    1 D. N... 100644 000000 000000 abc 000 examples/scripts/old.py\0\
                    1 AD N... 000000 100644 000000 000 abc examples/scripts/probe_ad.py\0\
                    2 R. N... 100644 100644 100644 abc abc R100 examples/b.cic\0examples/a.cic\0\
                    u UU N... 100644 100644 100644 100644 a b c examples/scripts/merge.py\0\
                    ? examples/p.cic.layout.json\0\
                    ! examples/scripts/__pycache__/x.pyc\0\
                    ! examples/scripts/local_probe.py\0";
        let parsed = parse_porcelain_v2(raw).unwrap();
        assert_eq!(parsed.headers.head.as_deref(), Some("main"));
        assert_eq!(parsed.headers.upstream.as_deref(), Some("origin/main"));
        assert_eq!(parsed.headers.ahead_behind, Some((2, 1)));
        assert!(!parsed.headers.unborn());
        let layout = Layout {
            root: "/r".into(),
            prefix: "examples".into(),
            git_dir: "/r/.git".into(),
        };
        let info = parsed.headers.info(&layout, Some(Operation::Merge));
        assert_eq!(
            info,
            RepoInfo {
                root: "/r".into(),
                prefix: "examples".into(),
                branch: Some("main".into()),
                head_short: Some("0123456".into()),
                upstream: Some(Upstream {
                    name: "origin/main".into(),
                    ahead: 2,
                    behind: 1
                }),
                unborn: false,
                operation: Some(Operation::Merge),
            }
        );
        // The wire shape: the tag beside the flattened facts, for `repo`
        // and `locked` alike (the branch chip never blanks under a lock).
        let repo = serde_json::to_value(GitState::Repo(info.clone())).unwrap();
        assert_eq!(repo["kind"], "repo");
        assert_eq!(repo["branch"], "main");
        assert_eq!(repo["operation"], "merge");
        let locked = serde_json::to_value(GitState::Locked(info.clone())).unwrap();
        assert_eq!(locked["kind"], "locked");
        assert_eq!(locked["prefix"], "examples");
        assert_eq!(locked["upstream"]["ahead"], 2);
        assert_eq!(
            serde_json::from_value::<GitState>(locked).unwrap(),
            GitState::Locked(info.clone())
        );
        assert_eq!(
            serde_json::to_value(GitState::NotARepo).unwrap(),
            serde_json::json!({"kind": "not_a_repo"})
        );
        let state = GitState::Repo(RepoInfo {
            operation: None,
            ..info
        });
        let statuses: Vec<(String, FileStatus, bool, bool)> = parsed
            .entries
            .iter()
            .map(|e| (e.path.clone(), e.file_status(), e.tracked(), e.in_head()))
            .collect();
        assert_eq!(
            statuses,
            vec![
                ("examples/p.cic".into(), FileStatus::Modified, true, true),
                (
                    "examples/scripts/new.py".into(),
                    FileStatus::Added,
                    true,
                    false
                ),
                (
                    "examples/scripts/old.py".into(),
                    FileStatus::Deleted,
                    true,
                    true
                ),
                // Added to the index, then deleted from disk: `deleted` to
                // the eye, yet HEAD never had it — the status and the
                // HEAD-version rule part ways here, which is why the scope
                // publishes `in_head` instead of letting clients infer it.
                (
                    "examples/scripts/probe_ad.py".into(),
                    FileStatus::Deleted,
                    true,
                    false
                ),
                ("examples/b.cic".into(), FileStatus::Renamed, true, false),
                (
                    "examples/scripts/merge.py".into(),
                    FileStatus::Modified,
                    true,
                    true
                ),
                (
                    "examples/p.cic.layout.json".into(),
                    FileStatus::Untracked,
                    false,
                    false
                ),
                (
                    "examples/scripts/__pycache__/x.pyc".into(),
                    FileStatus::Untracked,
                    false,
                    false
                ),
                (
                    "examples/scripts/local_probe.py".into(),
                    FileStatus::Untracked,
                    false,
                    false
                ),
            ]
        );
        // The scope filter: root-relative → project-relative, `.py` only,
        // scope order — and the IGNORED script left out (`git add` would
        // refuse the whole list over it).
        let survey = Survey {
            state,
            layout: Some(layout),
            unborn: false,
            entries: parsed.entries,
        };
        let files = survey.scope_files(&Scope::for_pipeline("p.cic"));
        let published: Vec<(&str, FileStatus, bool)> = files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.in_head))
            .collect();
        assert_eq!(
            published,
            [
                ("p.cic", FileStatus::Modified, true),
                ("p.cic.layout.json", FileStatus::Untracked, false),
                ("scripts/merge.py", FileStatus::Modified, true),
                ("scripts/new.py", FileStatus::Added, false),
                ("scripts/old.py", FileStatus::Deleted, true),
                ("scripts/probe_ad.py", FileStatus::Deleted, false),
            ],
            "`in_head` is the revert rule per file — not a function of `status`"
        );
        // On an unborn branch nothing has a HEAD version, whatever the
        // entry says.
        let unborn = Survey {
            state: survey.state.clone(),
            layout: survey.layout.clone(),
            unborn: true,
            entries: survey.entries.clone(),
        };
        assert!(
            unborn
                .scope_files(&Scope::for_pipeline("p.cic"))
                .iter()
                .all(|f| !f.in_head)
        );
        assert!(survey.pipeline_in_head(&Scope::for_pipeline("p.cic")));
        assert!(
            !survey.pipeline_in_head(&Scope::for_pipeline("b.cic")),
            "the new side of a rename has no HEAD version"
        );
        assert!(
            !survey.pipeline_in_head(&Scope::for_pipeline("scripts/new.py")),
            "index-only"
        );
        // A write under an operation in progress is refused before any
        // command runs.
        let merging = Survey {
            state: GitState::Repo(RepoInfo {
                operation: Some(Operation::CherryPick),
                ..survey.state.repo().unwrap().clone()
            }),
            layout: survey.layout.clone(),
            unborn: false,
            entries: Vec::new(),
        };
        assert!(matches!(
            merging.repo_or_refuse(),
            Err(GitRefusal::OperationInProgress(Operation::CherryPick))
        ));
        assert_eq!(
            merging.repo_or_refuse().unwrap_err().body()["operation"],
            "cherry_pick"
        );
    }

    #[test]
    fn operations_in_progress_are_read_from_the_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        assert_eq!(operation_in_progress(git_dir), None);
        std::fs::write(git_dir.join("REVERT_HEAD"), "x").unwrap();
        assert_eq!(operation_in_progress(git_dir), Some(Operation::Revert));
        std::fs::write(git_dir.join("CHERRY_PICK_HEAD"), "x").unwrap();
        assert_eq!(operation_in_progress(git_dir), Some(Operation::CherryPick));
        std::fs::create_dir(git_dir.join("rebase-apply")).unwrap();
        assert_eq!(operation_in_progress(git_dir), Some(Operation::Rebase));
        std::fs::write(git_dir.join("MERGE_HEAD"), "x").unwrap();
        assert_eq!(operation_in_progress(git_dir), Some(Operation::Merge));
    }

    #[test]
    fn porcelain_v2_unborn_and_detached() {
        let parsed = parse_porcelain_v2(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert!(parsed.headers.unborn());
        let layout = Layout {
            root: "/r".into(),
            prefix: String::new(),
            git_dir: "/r/.git".into(),
        };
        let RepoInfo {
            branch,
            head_short,
            unborn,
            ..
        } = parsed.headers.info(&layout, None);
        assert_eq!(
            (branch.as_deref(), head_short, unborn),
            (Some("main"), None, true)
        );
        let parsed =
            parse_porcelain_v2(b"# branch.oid abcdef1234567\0# branch.head (detached)\0").unwrap();
        let RepoInfo {
            branch, head_short, ..
        } = parsed.headers.info(&layout, None);
        assert_eq!((branch, head_short.as_deref()), (None, Some("abcdef1")));
        assert!(parse_porcelain_v2(b"9 what\0").is_err());
        assert!(parse_porcelain_v2(b"2 R. N... 100644 100644 100644 a b R100 new\0").is_err());
    }

    #[test]
    fn hunk_headers_parse() {
        let diff = "diff --git a/p.cic b/p.cic\n\
                    index 1..2 100644\n\
                    --- a/p.cic\n\
                    +++ b/p.cic\n\
                    @@ -2 +2 @@ size = slider(value=2.0)\n\
                    -size = slider(value=2.0)\n\
                    +size = slider(value=3.0)\n\
                    @@ -4,0 +5,2 @@\n\
                    +a = 1\n\
                    +b = 2\n\
                    @@ -7,3 +9,0 @@\n\
                    -x = 1\n\
                    -y = 2\n\
                    -z = 3\n\
                    \\ No newline at end of file\n";
        assert_eq!(
            parse_hunks(diff),
            vec![
                Hunk {
                    old_start: 2,
                    old_count: 1,
                    new_start: 2,
                    new_count: 1
                },
                Hunk {
                    old_start: 4,
                    old_count: 0,
                    new_start: 5,
                    new_count: 2
                },
                Hunk {
                    old_start: 7,
                    old_count: 3,
                    new_start: 9,
                    new_count: 0
                },
            ]
        );
        assert!(parse_hunks("").is_empty());
    }

    const HEAD: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n\
                        #off ghost = sphere(radius=size)\n\
                        extra = 4.0\n";

    /// The markers of `work` against [`HEAD`] with the hunk a minimal
    /// line diff of ONE contiguous edit produces (common prefix, common
    /// suffix, the middle is the hunk) — what `git diff -U0` emits for
    /// every edit these tests make, so the pure half is testable without
    /// git (the route tests use the real thing).
    fn diff_markers(work: &str) -> (Vec<NodeChange>, Vec<RemovedNode>) {
        let hunks = single_hunk(HEAD, work);
        markers(&Document::parse(HEAD), &Document::parse(work), &hunks)
    }

    fn single_hunk(old: &str, new: &str) -> Vec<Hunk> {
        let a: Vec<&str> = old.lines().collect();
        let b: Vec<&str> = new.lines().collect();
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suffix = a[prefix..]
            .iter()
            .rev()
            .zip(b[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let old_count = a.len() - prefix - suffix;
        let new_count = b.len() - prefix - suffix;
        if old_count == 0 && new_count == 0 {
            return Vec::new();
        }
        // `-U0` convention: a pure insertion/deletion anchors at the line
        // BEFORE it on the empty side.
        vec![Hunk {
            old_start: if old_count == 0 { prefix } else { prefix + 1 },
            old_count,
            new_start: if new_count == 0 { prefix } else { prefix + 1 },
            new_count,
        }]
    }

    #[test]
    fn modified_param_marks_that_node_only() {
        let work = HEAD.replace("value=2.0", "value=3.0");
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(
            nodes,
            vec![NodeChange {
                name: "size".into(),
                change: ChangeKind::Modified,
                from: None
            }]
        );
        assert!(removed.is_empty());
    }

    #[test]
    fn added_binding_is_added_and_deleted_is_removed_with_its_head_line() {
        let work = format!("{HEAD}ball = sphere(radius=size)\n");
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(
            nodes,
            vec![NodeChange {
                name: "ball".into(),
                change: ChangeKind::Added,
                from: None
            }]
        );
        assert!(removed.is_empty());
        let work = HEAD.replace("block = box(x=span, y=span, z=span)\n", "");
        let (nodes, removed) = diff_markers(&work);
        assert!(nodes.is_empty());
        assert_eq!(
            removed,
            vec![RemovedNode {
                name: "block".into(),
                line_in_head: 4
            }]
        );
    }

    #[test]
    fn same_rhs_under_a_new_name_is_a_rename_and_differing_rhs_is_not() {
        let work = HEAD.replace("block = box(", "cube = box(");
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(
            nodes,
            vec![NodeChange {
                name: "cube".into(),
                change: ChangeKind::Renamed,
                from: Some("block".into())
            }]
        );
        assert!(removed.is_empty(), "the pair consumed the removal");
        let work = HEAD.replace(
            "block = box(x=span, y=span, z=span)",
            "cube = box(x=span, y=span, z=size)",
        );
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(nodes[0].change, ChangeKind::Added);
        assert_eq!(removed[0].name, "block");
    }

    #[test]
    fn a_rename_pairs_within_one_hunk_only() {
        // Delete `span` at line 3 and add an unrelated `later` with the
        // same right-hand side at the end: two edits, two hunks (what
        // `git diff -U0` emits for them) — removed + added, NOT a rename.
        let work = HEAD.replace("span = construct_domain(start=0.0, end=size)\n", "")
            + "later = construct_domain(start=0.0, end=size)\n";
        let hunks = vec![
            Hunk {
                old_start: 3,
                old_count: 1,
                new_start: 2,
                new_count: 0,
            },
            Hunk {
                old_start: 6,
                old_count: 0,
                new_start: 6,
                new_count: 1,
            },
        ];
        let (nodes, removed) = markers(&Document::parse(HEAD), &Document::parse(&work), &hunks);
        assert_eq!(
            nodes,
            vec![NodeChange {
                name: "later".into(),
                change: ChangeKind::Added,
                from: None
            }]
        );
        assert_eq!(
            removed,
            vec![RemovedNode {
                name: "span".into(),
                line_in_head: 3
            }]
        );
        // The same two lines as ONE hunk (adjacent edits) pair up: the
        // rule is per hunk, not per file.
        let work = HEAD.replace(
            "span = construct_domain(start=0.0, end=size)\n",
            "later = construct_domain(start=0.0, end=size)\n",
        );
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(nodes[0].change, ChangeKind::Renamed);
        assert_eq!(nodes[0].from.as_deref(), Some("span"));
        assert!(removed.is_empty());
    }

    #[test]
    fn off_toggle_and_broken_lines_name_their_nodes() {
        // Disabling a live line: same name both sides → modified.
        let work = HEAD.replace("span = construct", "#off span = construct");
        let (nodes, _) = diff_markers(&work);
        assert_eq!(
            nodes[0],
            NodeChange {
                name: "span".into(),
                change: ChangeKind::Modified,
                from: None
            }
        );
        // Re-enabling the ghost: modified too.
        let work = HEAD.replace("#off ghost = sphere", "ghost = sphere");
        let (nodes, _) = diff_markers(&work);
        assert_eq!(nodes[0].name, "ghost");
        assert_eq!(nodes[0].change, ChangeKind::Modified);
        // A broken line is named by its first identifier.
        let work = format!("{HEAD}oops = (\n");
        let (nodes, _) = diff_markers(&work);
        assert_eq!(nodes[0].name, "oops");
        assert_eq!(nodes[0].change, ChangeKind::Added);
        assert_eq!(
            node_names(HEAD),
            ["size", "span", "block", "ghost", "extra"]
        );
    }

    #[test]
    fn unchanged_lines_never_mark_even_when_neighbours_move() {
        // Insert a line at the top: everything shifts but only the new
        // binding is marked (line numbers are new-side).
        let work = HEAD.replacen("# cicada 1\n", "# cicada 1\nfirst = 1.0\n", 1);
        let (nodes, removed) = diff_markers(&work);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "first");
        assert!(removed.is_empty());
        // A clean tree: nothing.
        let (nodes, removed) = diff_markers(HEAD);
        assert!(nodes.is_empty() && removed.is_empty());
    }

    #[test]
    fn refusal_bodies_carry_kind_and_facts() {
        let body = GitRefusal::Git(GitError::Failed {
            command: "git commit".into(),
            code: Some(128),
            stderr: "fatal: boom".into(),
        })
        .body();
        assert_eq!(body["kind"], "git_failed");
        assert_eq!(body["command"], "git commit");
        assert_eq!(body["code"], 128);
        assert_eq!(body["stderr"], "fatal: boom");
        assert_eq!(
            GitRefusal::Untracked("p.cic".into()).body()["path"],
            "p.cic"
        );
        assert_eq!(GitRefusal::Locked.body()["kind"], "locked");
        let ignored = GitRefusal::Ignored("p.cic".into()).body();
        assert_eq!(
            (ignored["kind"].as_str(), ignored["path"].as_str()),
            (Some("ignored"), Some("p.cic"))
        );
        assert_eq!(GitRefusal::NoWriter("p.cic".into()).body()["kind"], "lease");
        assert_eq!(
            GitRefusal::NoSuchPipeline("q.cic".into()).body()["kind"],
            "no_such_pipeline"
        );
        assert_eq!(
            GitRefusal::Internal("join".into()).body()["kind"],
            "internal"
        );
        assert_eq!(
            refuse(GitError::Failed {
                command: "git add".into(),
                code: Some(128),
                stderr: "fatal: Unable to create '/r/.git/index.lock': File exists.".into(),
            })
            .kind(),
            GitErrorKind::Locked
        );
        assert_eq!(
            refuse(GitError::NotFound {
                program: "git".into()
            })
            .kind(),
            GitErrorKind::GitNotFound
        );
        assert_eq!(
            GitRefusal::Git(GitError::Timeout {
                command: "git commit".into(),
                timeout_ms: 30_000
            })
            .kind(),
            GitErrorKind::GitTimeout
        );
    }
}
