//! Stamps the build (docs/17 wave 5 R1): `CICADA_BUILD_COMMIT` — git's
//! 12-digit short hash of HEAD, `-dirty` when the tree carries uncommitted
//! tracked changes; the `CICADA_GIT_SHA` environment variable wins when set
//! (CI checkouts); `unknown`, SAID on stderr as a cargo warning, when
//! neither can answer — and `CICADA_BUILD_DATE`, the UTC date
//! (`SOURCE_DATE_EPOCH` when set). `cicada --version`, `hello.version`,
//! `GET /api/version` and the About dialog read them through
//! `cicada_cli::version`. The rules are `src/stamp.rs` — included here by
//! path and compiled into the library too, where they are unit-tested; this
//! file is the process calls around them.
//!
//! When it re-runs: on a change of the two variables, of HEAD (a checkout),
//! of HEAD's reflog (every commit, reset or merge moves it) and of the index
//! (staging — and most `git status` refreshes write it), found through
//! `git rev-parse --git-path` so a worktree's files are watched, not the
//! main checkout's. NOT on every source edit — the stamp is HEAD's state as
//! of the last of those events, so a dev build's `-dirty` and date can lag
//! by an edit or a day; a release build (a fresh CI checkout) is exact. A
//! bad override fails the build loudly; a missing git does not.

#[path = "src/stamp.rs"]
mod stamp;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    println!("cargo:rerun-if-env-changed=CICADA_GIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    for path in git_paths(&manifest_dir) {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let commit = match stamp::commit_from_override(env_var("CICADA_GIT_SHA").as_deref()) {
        Ok(Some(commit)) => commit,
        Ok(None) => match git_commit(&manifest_dir) {
            Ok(commit) => commit,
            Err(why) => {
                println!(
                    "cargo:warning=cicada-cli: the build commit is `{}` — {why}; set CICADA_GIT_SHA to stamp one",
                    stamp::UNKNOWN
                );
                stamp::UNKNOWN.to_owned()
            }
        },
        Err(error) => fail(&error.to_string()),
    };
    let built = match stamp::epoch_from_override(env_var("SOURCE_DATE_EPOCH").as_deref()) {
        Ok(Some(epoch)) => stamp::utc_date(epoch),
        Ok(None) => stamp::utc_date(now_epoch_seconds()),
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

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0)
}

/// Run git in `dir`; its trimmed stdout, or why it could not answer.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
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

/// The commit as git sees the tree the manifest lives in.
fn git_commit(dir: &Path) -> Result<String, String> {
    let short = git(dir, &["rev-parse", "--short=12", "HEAD"])?;
    let porcelain = git(dir, &["status", "--porcelain", "--untracked-files=no"])?;
    stamp::commit_from_git(&short, &porcelain)
        .ok_or_else(|| format!("git answered {short:?} for HEAD, which is not a hash"))
}

/// The repository files whose change re-stamps the build — those that
/// exist; empty when git cannot say (then nothing but the variables
/// re-runs this script, and the stamp stays `unknown`).
fn git_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(listing) = git(
        dir,
        &[
            "rev-parse",
            "--git-path",
            "HEAD",
            "--git-path",
            "logs/HEAD",
            "--git-path",
            "index",
        ],
    ) else {
        return Vec::new();
    };
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| dir.join(line))
        .filter(|path| path.is_file())
        .collect()
}
