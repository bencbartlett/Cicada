//! Stamps the build (docs/17 wave 5 R1): `CICADA_BUILD_COMMIT` — git's
//! 12-digit short hash of HEAD, `-dirty` when the build inputs carry
//! uncommitted tracked changes; the `CICADA_GIT_SHA` environment variable
//! wins when set (CI checkouts); `unknown`, SAID on stderr as a cargo
//! warning, when neither can answer — and `CICADA_BUILD_DATE`, the UTC date
//! (`SOURCE_DATE_EPOCH` when set). `cicada --version`, `hello.version`,
//! `GET /api/version` and the About dialog read them through
//! `cicada_cli::version`. The rules are `src/stamp.rs` — included here by
//! path and compiled into the library too, where they are unit-tested; this
//! file is the process calls around them.
//!
//! What `-dirty` means and when the stamp is re-taken (fix round
//! 2026-09-20, findings L2-1 / R1-C2): the build is dirty when `git status
//! --porcelain --untracked-files=no -- <stamp::BUILD_INPUTS>` lists anything
//! — an uncommitted change to a TRACKED file under `crates/`, `web/`,
//! `Cargo.toml`, `Cargo.lock` or `rust-toolchain.toml`, what the binary and
//! the SPA it embeds are built from (a docs-only or examples-only edit is
//! not a dirty build). The script registers every tracked file under those
//! paths (`git ls-files`) plus HEAD, HEAD's reflog and the index as
//! `rerun-if-changed`, so a change to any of them re-takes the stamp before
//! the next build and `-dirty` follows the tree exactly
//! (`tests/version.rs` holds the equivalence) — at the price of one relink
//! of `cicada-cli` per such change, and per external index write (`git
//! add`, an IDE's status refresh after an edit — finding L2-6, accepted:
//! the index is watched because staging a NEW file changes nothing else
//! this script could see). The script's own git calls run with
//! `GIT_OPTIONAL_LOCKS=0`, so its `status` never rewrites the index behind
//! cargo's back (that re-ran the script on the very next build — finding
//! L3-1). The repository must be THIS workspace: a source tree unpacked
//! inside another repository stamps `unknown` with the reason, never that
//! repository's HEAD (finding L3-3). A bad override fails the build loudly;
//! a missing git does not — then only the two variables and `PATH` re-run
//! the script, and the warning says how to re-stamp once git can answer.

#[path = "src/stamp.rs"]
mod stamp;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    // `crates/cicada-cli` → the workspace root.
    let Some(root) = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
    else {
        fail(&format!(
            "{} is not <workspace>/crates/cicada-cli",
            manifest_dir.display()
        ))
    };
    println!("cargo:rerun-if-env-changed=CICADA_GIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let repository = this_repository(&root);
    match &repository {
        Ok(()) => {
            for path in watched_paths(&root) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        // No repository to watch: git appearing on PATH is the one event
        // that could change the answer (a `git init` under the tree is not
        // observable — cargo re-runs on a MISSING path every build, so the
        // future `.git` cannot be registered; the warning says the way).
        Err(_) => println!("cargo:rerun-if-env-changed=PATH"),
    }

    let commit = match stamp::commit_from_override(env_var("CICADA_GIT_SHA").as_deref()) {
        Ok(Some(commit)) => commit,
        Ok(None) => match repository.and_then(|()| git_commit(&root)) {
            Ok(commit) => commit,
            Err(why) => {
                println!(
                    "cargo:warning=cicada-cli: the build commit is `{}` — {why}; set CICADA_GIT_SHA to stamp one, or \
                     `cargo clean -p cicada-cli` to re-stamp once git can answer here",
                    stamp::UNKNOWN
                );
                stamp::UNKNOWN.to_owned()
            }
        },
        Err(error) => fail(&error.to_string()),
    };
    let built = match stamp::epoch_from_override(env_var("SOURCE_DATE_EPOCH").as_deref()) {
        Ok(Some(epoch)) => stamp::utc_date(epoch),
        Ok(None) => match now_epoch_seconds() {
            Ok(now) => stamp::utc_date(now),
            Err(why) => fail(&why),
        },
        Err(error) => fail(&error.to_string()),
    };
    // The shape every reader relies on (the tests, About): refuse to stamp
    // anything else rather than let a drifted rule through.
    if !(stamp::is_git_stamp(&commit) || commit == stamp::UNKNOWN) {
        fail(&format!(
            "the commit stamp {commit:?} is neither a git hash nor `unknown`"
        ));
    }
    if !stamp::is_date(&built) {
        fail(&format!("the date stamp {built:?} is not YYYY-MM-DD"));
    }
    println!("cargo:rustc-env=CICADA_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=CICADA_BUILD_DATE={built}");
}

/// A build-time variable as text; `None` when unset or not Unicode.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Stop the build with the reason on stderr.
fn fail(message: &str) -> ! {
    eprintln!("error: cicada-cli build stamp: {message}");
    std::process::exit(1)
}

/// Now, as whole seconds since the Unix epoch — or why the clock could not
/// say (a build date is stamped from a real clock or not at all; the
/// silent `1970-01-01` this once fell back to was finding L2-2).
fn now_epoch_seconds() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("the system clock is before the Unix epoch ({error})"))?;
    i64::try_from(elapsed.as_secs()).map_err(|_| {
        format!(
            "the system clock is beyond i64 seconds ({})",
            elapsed.as_secs()
        )
    })
}

/// Run git in `dir`; its trimmed stdout, or why it could not answer.
/// `GIT_OPTIONAL_LOCKS=0`: no command here may rewrite the index (a
/// `status` refresh would, and the index is a `rerun-if-changed` path).
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(dir)
        .output()
        .map_err(|error| format!("git is not runnable ({error})"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            stderr.trim().lines().next().unwrap_or("no message")
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Is `root` the top level of the repository git finds from it? Git walks
/// up to the nearest enclosing repository, so a source tarball unpacked
/// inside another project's checkout would otherwise be stamped with THAT
/// project's HEAD and dirt (finding L3-3).
fn this_repository(root: &Path) -> Result<(), String> {
    let toplevel = PathBuf::from(git(root, &["rev-parse", "--show-toplevel"])?);
    let same = match (
        std::fs::canonicalize(&toplevel),
        std::fs::canonicalize(root),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        Ok(())
    } else {
        Err(format!(
            "the enclosing repository is {}, not this workspace ({})",
            toplevel.display(),
            root.display()
        ))
    }
}

/// The commit as git sees the workspace: HEAD's short hash, `-dirty` when
/// the build inputs carry uncommitted tracked changes.
fn git_commit(root: &Path) -> Result<String, String> {
    let short = git(root, &["rev-parse", "--short=12", "HEAD"])?;
    let mut status = vec!["status", "--porcelain", "--untracked-files=no", "--"];
    status.extend_from_slice(stamp::BUILD_INPUTS);
    let porcelain = git(root, &status)?;
    stamp::commit_from_git(&short, &porcelain)
        .ok_or_else(|| format!("git answered {short:?} for HEAD, which is not a hash"))
}

/// The paths whose change re-takes the stamp: HEAD, its reflog and the
/// index (through `git rev-parse --git-path`, so a worktree's own files are
/// watched, not the main checkout's) and every tracked file under the build
/// inputs — those that exist (a deleted tracked file is `-dirty` through
/// the porcelain; registering a missing path would re-run this script on
/// every build until it is back, and its return rewrites the index anyway).
fn watched_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(listing) = git(
        root,
        &[
            "rev-parse",
            "--git-path",
            "HEAD",
            "--git-path",
            "logs/HEAD",
            "--git-path",
            "index",
        ],
    ) {
        paths.extend(
            listing
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| root.join(line)),
        );
    }
    let mut ls_files = vec!["ls-files", "-z", "--"];
    ls_files.extend_from_slice(stamp::BUILD_INPUTS);
    if let Ok(listing) = git(root, &ls_files) {
        paths.extend(stamp::tracked_paths(&listing).map(|path| root.join(path)));
    }
    paths.retain(|path| path.is_file());
    paths
}
