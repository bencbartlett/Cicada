//! The build stamp's pure half (docs/17 wave 5 R1): how `build.rs` turns
//! what git and the environment say into the `CICADA_BUILD_COMMIT` and
//! `CICADA_BUILD_DATE` values every `cicada` binary carries — in
//! `--version`, `hello.version`, `GET /api/version` and About.
//!
//! The build script includes this file by path (`#[path = "src/stamp.rs"]`)
//! and the library compiles it as an ordinary module, so the rules below
//! are unit-tested here while the script stays a thin shell around process
//! calls it cannot test. Nothing in this module reads the environment or
//! runs a program.

use std::fmt;
use std::path::{Path, PathBuf};

/// The commit a build could not name: no `CICADA_GIT_SHA`, and git could
/// not answer (not installed, not a repository, a source tarball). The
/// build script says so on stderr when it stamps this — never silently.
pub const UNKNOWN: &str = "unknown";

/// The short hash's length: `git rev-parse --short=12`.
pub const SHORT_LEN: usize = 12;

/// The build inputs — the tracked paths, relative to the workspace root,
/// whose uncommitted changes make a build `-dirty` and whose tracked files
/// the build script watches so the stamp is re-taken before the next build
/// (fix round 2026-09-20, findings L2-1 / R1-C2): what the binary and the
/// SPA it embeds are built from. A docs-only or examples-only edit is not a
/// dirty BUILD, and watching it would relink `cicada-cli` on every such
/// edit. One pathspec for both questions, so `-dirty` ⇔ "the porcelain over
/// these paths lists something" holds exactly (`tests/version.rs`).
pub const BUILD_INPUTS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "crates",
    "web",
];

/// The paths in a `git ls-files -z` listing: NUL-separated, the trailing
/// terminator dropped, relative to the directory git ran in.
pub fn tracked_paths(listing: &str) -> impl Iterator<Item = &str> {
    listing.split('\0').filter(|path| !path.is_empty())
}

/// Where cargo watches one tracked build input (`rerun-if-changed`): the
/// file itself when it exists; for a file missing at stamp time — deleted,
/// so the build is `-dirty` through the porcelain — its nearest existing
/// ancestor directory BELOW `root`, which cargo scans for any modification,
/// so the file's return by any route (`git checkout`, a copy, an editor's
/// undo) re-takes the stamp. A missing FILE path would re-run the script on
/// every build until the file is back, and registering nothing left
/// `-dirty` standing after a restore that never touched the index (fix
/// round 2 2026-09-20, finding L3A-1). `None` when nothing below the root
/// exists on the way up — a file at the root itself (`Cargo.toml`), or a
/// whole build-input tree gone: the root is never registered (on CI it
/// holds the target directory), and such a restore waits for git to touch
/// the index.
pub fn watch_target(
    root: &Path,
    path: &Path,
    is_file: impl Fn(&Path) -> bool,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if is_file(path) {
        return Some(path.to_path_buf());
    }
    path.ancestors()
        .skip(1)
        .take_while(|ancestor| *ancestor != root)
        .find(|ancestor| is_dir(ancestor))
        .map(Path::to_path_buf)
}

/// A stamping input the build must refuse rather than guess around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampError {
    /// `CICADA_GIT_SHA` was set to something that is not a git object hash
    /// of at least [`SHORT_LEN`] hexadecimal digits.
    BadSha(String),
    /// `SOURCE_DATE_EPOCH` was set to something that is not a whole number
    /// of seconds since the Unix epoch.
    BadEpoch(String),
}

impl fmt::Display for StampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSha(value) => write!(
                f,
                "CICADA_GIT_SHA is {value:?} — expected a git commit hash (at least {SHORT_LEN} hexadecimal \
                 digits); unset it to let the build ask git"
            ),
            Self::BadEpoch(value) => write!(
                f,
                "SOURCE_DATE_EPOCH is {value:?} — expected whole seconds since the Unix epoch; \
                 unset it to stamp today's UTC date"
            ),
        }
    }
}

impl std::error::Error for StampError {}

fn is_hex(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// The commit from the `CICADA_GIT_SHA` override (CI checkouts, where the
/// workflow knows the SHA and no `-dirty` question arises): trimmed,
/// lower-cased, cut to [`SHORT_LEN`] digits. `Ok(None)` when the variable is
/// unset or blank — then the build asks git.
///
/// # Errors
///
/// [`StampError::BadSha`] when the value is set but is not a hash of at
/// least [`SHORT_LEN`] hexadecimal digits (a stamp is never shorter than
/// what git's `--short=12` gives).
pub fn commit_from_override(value: Option<&str>) -> Result<Option<String>, StampError> {
    let Some(value) = value else { return Ok(None) };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() < SHORT_LEN || !is_hex(trimmed) {
        return Err(StampError::BadSha(value.to_owned()));
    }
    let lower = trimmed.to_ascii_lowercase();
    Ok(Some(lower.chars().take(SHORT_LEN).collect()))
}

/// The commit from git's two answers: `git rev-parse --short=12 HEAD`
/// (`short`) plus `-dirty` when `git status --porcelain
/// --untracked-files=no` (`porcelain`) lists anything — tracked changes
/// only, as `git describe --dirty` counts them; an untracked file (a layout
/// sidecar the canvas wrote) is not a dirty build. `None` when `short` is
/// not a hash — git answered, but not with what was asked.
#[must_use]
pub fn commit_from_git(short: &str, porcelain: &str) -> Option<String> {
    let short = short.trim();
    if !is_hex(short) {
        return None;
    }
    let dirty = porcelain.lines().any(|line| !line.trim().is_empty());
    Some(if dirty {
        format!("{short}-dirty")
    } else {
        short.to_owned()
    })
}

/// Is `commit` a stamp git produced — [`SHORT_LEN`] or more hexadecimal
/// digits (`--short=12` gives more when twelve are ambiguous), optionally
/// `-dirty`? [`UNKNOWN`] is not.
#[must_use]
pub fn is_git_stamp(commit: &str) -> bool {
    let hash = commit.strip_suffix("-dirty").unwrap_or(commit);
    hash.len() >= SHORT_LEN && is_hex(hash)
}

/// The `SOURCE_DATE_EPOCH` override (reproducible builds): `Ok(None)` when
/// unset or blank — then the build stamps the current UTC date.
///
/// # Errors
///
/// [`StampError::BadEpoch`] when the value is set but is not an integer.
pub fn epoch_from_override(value: Option<&str>) -> Result<Option<i64>, StampError> {
    let Some(value) = value else { return Ok(None) };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<i64>()
        .map(Some)
        .map_err(|_| StampError::BadEpoch(value.to_owned()))
}

/// The UTC civil date (`YYYY-MM-DD`) of a Unix timestamp — the proleptic
/// Gregorian calendar, days-from-civil inverted (H. Hinnant's algorithm),
/// so the build needs no date crate.
#[must_use]
pub fn utc_date(epoch_seconds: i64) -> String {
    let days = epoch_seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, 0..=146096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era, 0..=399
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, 0..=365 (March-based)
    let mp = (5 * doy + 2) / 153; // month index, 0..=11 (March = 0)
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Is `built` a date this module stamps: `YYYY-MM-DD`, digits and dashes
/// in their places?
#[must_use]
pub fn is_date(built: &str) -> bool {
    let bytes = built.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_trimmed_lowercased_and_cut_to_twelve() {
        assert_eq!(
            commit_from_override(Some(" A82EB39D1C2E7F00112233445566778899AABBCC \n")),
            Ok(Some("a82eb39d1c2e".to_owned()))
        );
        // Exactly twelve pass through whole.
        assert_eq!(
            commit_from_override(Some("abcdef012345")),
            Ok(Some("abcdef012345".to_owned()))
        );
    }

    #[test]
    fn an_unset_or_blank_override_means_ask_git() {
        assert_eq!(commit_from_override(None), Ok(None));
        assert_eq!(commit_from_override(Some("")), Ok(None));
        assert_eq!(commit_from_override(Some("  \t")), Ok(None));
    }

    #[test]
    fn a_non_hash_override_is_refused_with_the_value_named() {
        // Eleven digits is short of a stamp; a branch name, a tag and a
        // non-hex character are not hashes at all.
        for bad in ["main", "abcdef01234", "g82eb39d1c2e", "v0.1.0-alpha.1"] {
            let error = commit_from_override(Some(bad)).unwrap_err();
            assert_eq!(error, StampError::BadSha(bad.to_owned()));
            assert!(error.to_string().contains(bad), "{error}");
        }
    }

    #[test]
    fn git_answers_become_the_hash_with_dirty_only_for_tracked_changes() {
        assert_eq!(
            commit_from_git("5c4d9e64af3a\n", ""),
            Some("5c4d9e64af3a".to_owned())
        );
        assert_eq!(
            commit_from_git("5c4d9e64af3a", " M crates/cicada-cli/src/main.rs\n"),
            Some("5c4d9e64af3a-dirty".to_owned())
        );
        // A porcelain listing of whitespace alone is no change.
        assert_eq!(
            commit_from_git("5c4d9e64af3a", "\n  \n"),
            Some("5c4d9e64af3a".to_owned())
        );
        // Not a hash: git printed something else (an error on stdout, nothing).
        assert_eq!(commit_from_git("", ""), None);
        assert_eq!(commit_from_git("fatal: not a git repository", ""), None);
    }

    #[test]
    fn a_git_stamp_is_twelve_or_more_hex_digits_optionally_dirty() {
        assert!(is_git_stamp("5c4d9e64af3a"));
        assert!(is_git_stamp("5c4d9e64af3a-dirty"));
        assert!(is_git_stamp("5c4d9e64af3a0")); // thirteen when twelve are ambiguous
        assert!(!is_git_stamp(UNKNOWN));
        assert!(!is_git_stamp("5c4d9e64af3")); // eleven
        assert!(!is_git_stamp("5c4d9e64af3a-DIRTY"));
        assert!(!is_git_stamp("5c4d9e64af3a dirty"));
    }

    #[test]
    fn source_date_epoch_is_parsed_or_refused() {
        assert_eq!(epoch_from_override(None), Ok(None));
        assert_eq!(epoch_from_override(Some("")), Ok(None));
        assert_eq!(
            epoch_from_override(Some(" 1787616000 ")),
            Ok(Some(1_787_616_000))
        );
        assert_eq!(epoch_from_override(Some("-1")), Ok(Some(-1)));
        let error = epoch_from_override(Some("2026-08-25")).unwrap_err();
        assert_eq!(error, StampError::BadEpoch("2026-08-25".to_owned()));
        assert!(error.to_string().contains("2026-08-25"), "{error}");
    }

    #[test]
    fn utc_dates_across_epoch_leap_days_and_century_rules() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(86_399), "1970-01-01");
        assert_eq!(utc_date(86_400), "1970-01-02");
        assert_eq!(utc_date(-1), "1969-12-31");
        assert_eq!(utc_date(951_782_400), "2000-02-29"); // 2000 is a leap year (÷400)
        assert_eq!(utc_date(951_868_800), "2000-03-01");
        assert_eq!(utc_date(4_107_542_400), "2100-03-01"); // 2100 is not (÷100)
        assert_eq!(utc_date(4_107_456_000), "2100-02-28");
        assert_eq!(utc_date(1_787_616_000), "2026-08-25"); // the contract's example date
        assert_eq!(utc_date(1_787_702_399), "2026-08-25");
        assert_eq!(utc_date(1_787_702_400), "2026-08-26");
        assert_eq!(utc_date(1_735_689_600), "2025-01-01");
        assert_eq!(utc_date(1_735_689_599), "2024-12-31");
    }

    #[test]
    fn tracked_paths_split_the_nul_listing_and_drop_the_terminator() {
        let listing = "Cargo.toml\0crates/cicada-cli/build.rs\0web/src/App.tsx\0";
        assert_eq!(
            tracked_paths(listing).collect::<Vec<_>>(),
            [
                "Cargo.toml",
                "crates/cicada-cli/build.rs",
                "web/src/App.tsx"
            ]
        );
        assert_eq!(tracked_paths("").count(), 0);
        // A path with a space survives whole (no quoting in `-z` output).
        assert_eq!(
            tracked_paths("web/a b.ts\0").collect::<Vec<_>>(),
            ["web/a b.ts"]
        );
    }

    #[test]
    fn a_tracked_input_is_watched_as_itself_or_by_its_nearest_existing_directory() {
        let root = Path::new("/repo");
        let present = ["/repo/web/src/App.tsx", "/repo/Cargo.toml"];
        let dirs = [
            "/repo/web/src",
            "/repo/web",
            "/repo/crates/cicada-cli",
            "/repo/crates",
        ];
        let is_file = |p: &Path| present.contains(&p.to_str().unwrap());
        let is_dir = |p: &Path| dirs.contains(&p.to_str().unwrap());
        // Present: the file itself.
        assert_eq!(
            watch_target(root, Path::new("/repo/web/src/App.tsx"), is_file, is_dir),
            Some(PathBuf::from("/repo/web/src/App.tsx"))
        );
        // Deleted: its directory, so a restore by any route is seen.
        assert_eq!(
            watch_target(root, Path::new("/repo/web/src/gone.ts"), is_file, is_dir),
            Some(PathBuf::from("/repo/web/src"))
        );
        // Its directory gone too: the nearest one that is there.
        assert_eq!(
            watch_target(
                root,
                Path::new("/repo/crates/cicada-cli/src/gone.rs"),
                is_file,
                is_dir
            ),
            Some(PathBuf::from("/repo/crates/cicada-cli"))
        );
        // Nothing below the root on the way up: the root is never registered.
        assert_eq!(
            watch_target(root, Path::new("/repo/Cargo.lock"), is_file, is_dir),
            None
        );
        assert_eq!(
            watch_target(root, Path::new("/repo/docs/gone.md"), is_file, is_dir),
            None
        );
    }

    #[test]
    fn the_build_inputs_are_the_binary_and_the_spa_sources_only() {
        // The manifests, the crates and the web app — never docs/ or
        // examples/, whose edits are not a dirty BUILD.
        assert_eq!(
            BUILD_INPUTS,
            [
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                "crates",
                "web"
            ]
        );
        assert!(
            BUILD_INPUTS
                .iter()
                .all(|p| !p.starts_with("docs") && !p.starts_with("examples"))
        );
    }

    #[test]
    fn a_date_is_ten_characters_in_the_stamped_places() {
        assert!(is_date("2026-08-25"));
        assert!(is_date(utc_date(0).as_str()));
        assert!(!is_date("2026-8-25"));
        assert!(!is_date("2026/08/25"));
        assert!(!is_date(UNKNOWN));
        assert!(!is_date("2026-08-25T00:00:00Z"));
    }
}
