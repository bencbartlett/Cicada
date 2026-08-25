//! `GET /api/files` (docs/13 §HTTP surface; v0.1 wave 4 O1): ONE directory
//! of the served root per request — its directories and `*.cic` pipelines
//! — so the app's File → Open dialog can walk a root as large as the
//! user's home directory without the server ever walking it whole
//! (`/api/project`'s bounded-depth walk is the project-sized tool; this is
//! the directory-sized one). Nothing above the root is ever named: `dir`
//! is normalised lexically BEFORE the file system is touched, every escape
//! is refused as `path_not_allowed`, and the reply speaks only in
//! root-relative paths and bare names.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::{FileEntry, FileKind, FilesErrorKind, FilesResponse};

/// Directories never listed besides dot-directories and the ones the OS
/// hides: the build trees no pipeline lives in. [`skipped_directory`] is
/// the whole rule, and `/api/project`'s walk reads the same predicate — the
/// two must never disagree about what a root contains.
pub const SKIPPED_DIRECTORIES: [&str; 2] = ["node_modules", "target"];

/// Whether a directory entry is left out of listings and walks: a
/// dot-name, a name in [`SKIPPED_DIRECTORIES`], or the OS's own hidden flag
/// (read off the entry's OWN metadata — a link's, not its target's). A
/// convention about what the picker shows, not a boundary: an unlisted
/// directory under the root is still enterable by name — the root is the
/// boundary (docs/13 §HTTP surface).
pub(crate) fn skipped_directory(name: &str, own: &std::fs::Metadata) -> bool {
    is_hidden(name, own) || SKIPPED_DIRECTORIES.contains(&name)
}

/// Why a listing was refused. [`FilesError::kind`] is the wire tag, the
/// `Display` text the reason in words, and the body carries the `dir` as
/// the client sent it.
#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    /// `dir` is not a plain root-relative path (400).
    #[error("`{dir}` is not a root-relative directory path: {reason}")]
    PathNotAllowed {
        /// The request's `dir`.
        dir: String,
        /// What was wrong with it.
        reason: &'static str,
    },
    /// `dir` exists but its canonical path leaves the root — a symlink or
    /// junction pointing out (400).
    #[error("`{dir}` resolves outside the served root — refused")]
    OutsideRoot {
        /// The request's `dir`.
        dir: String,
    },
    /// No such directory under the root (404).
    #[error("no directory `{dir}` under the root")]
    NotFound {
        /// The request's `dir`.
        dir: String,
    },
    /// `dir` names a file (404).
    #[error("`{dir}` is not a directory")]
    NotADirectory {
        /// The request's `dir`.
        dir: String,
    },
    /// The directory could not be read (403).
    #[error("reading `{dir}`: {source}")]
    Io {
        /// The request's `dir`.
        dir: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
}

impl FilesError {
    /// The wire tag (docs/13 §HTTP surface has the status per tag).
    #[must_use]
    pub fn kind(&self) -> FilesErrorKind {
        match self {
            Self::PathNotAllowed { .. } | Self::OutsideRoot { .. } => {
                FilesErrorKind::PathNotAllowed
            }
            Self::NotFound { .. } | Self::NotADirectory { .. } => FilesErrorKind::NotFound,
            Self::Io { .. } => FilesErrorKind::IoError,
        }
    }

    /// The `dir` the request named, verbatim.
    #[must_use]
    pub fn dir(&self) -> &str {
        match self {
            Self::PathNotAllowed { dir, .. }
            | Self::OutsideRoot { dir }
            | Self::NotFound { dir }
            | Self::NotADirectory { dir }
            | Self::Io { dir, .. } => dir,
        }
    }

    /// The JSON refusal body: `{kind, message, path}` — the shape of every
    /// git-route refusal, so one client error type reads both.
    #[must_use]
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind(),
            "message": self.to_string(),
            "path": self.dir(),
        })
    }
}

/// Normalise `dir` lexically, before any file-system access: the path's
/// normal segments, `/`-separated on the wire. Empty and `.` segments
/// vanish (`a//b/`, `./a/b` → `a/b`); `..`, a leading `/` (absolute, or a
/// `//host/share` UNC), a backslash (a legal name character on Unix and a
/// separator on Windows — never a separator here), a `:` (a drive prefix or
/// an NTFS stream) and a NUL byte are refused. The root itself is `""`.
///
/// # Errors
///
/// [`FilesError::PathNotAllowed`].
pub fn normalize_dir(dir: &str) -> Result<Vec<String>, FilesError> {
    let refuse = |reason: &'static str| FilesError::PathNotAllowed {
        dir: dir.to_owned(),
        reason,
    };
    if dir.contains('\0') {
        return Err(refuse("a NUL byte"));
    }
    if dir.contains('\\') {
        return Err(refuse("a backslash — paths are `/`-separated"));
    }
    if dir.contains(':') {
        return Err(refuse("a `:` — no drive prefixes or streams"));
    }
    if dir.starts_with('/') {
        return Err(refuse("an absolute path"));
    }
    let mut segments = Vec::new();
    for segment in dir.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Err(refuse("a `..` segment")),
            normal => segments.push(normal.to_owned()),
        }
    }
    Ok(segments)
}

/// List `dir` under `root`: the directories (minus what
/// [`skipped_directory`] leaves out) and the `*.cic` files, directories
/// first, each group in case-insensitive name order. `dir` is normalised by
/// [`normalize_dir`], joined under the canonical root, and its canonical
/// path must still start with the root — a symlink out is refused. Not an
/// entry: a name that is not valid Unicode, a link the server cannot follow
/// to a place under the root ([`resolve_link`]), and what is neither a
/// directory nor a `.cic` file; a failure to read the directory itself, or
/// an entry of it, is the listing's failure. A skipped directory is
/// unlisted, not unenterable: named directly it lists like any other.
///
/// # Errors
///
/// [`FilesError`] — every variant.
pub fn list(root: &Path, dir: &str) -> Result<FilesResponse, FilesError> {
    let segments = normalize_dir(dir)?;
    let io = |source| FilesError::Io {
        dir: dir.to_owned(),
        source,
    };
    // The root canonical too, so the containment check compares like with
    // like (Windows: both with the `\\?\` verbatim prefix).
    let root = std::fs::canonicalize(root).map_err(io)?;
    let mut target = root.clone();
    target.extend(&segments);
    let canonical = std::fs::canonicalize(&target).map_err(|source| {
        // Nothing is there: the path names no directory (`NotFound`), runs
        // THROUGH a file (`sub/p.cic/x` — Unix says `NotADirectory`,
        // Windows says not found), or is a name this file system cannot
        // hold at all (Windows: `a?b`, `a*b` — `InvalidFilename`, os error
        // 123). None of these is "exists but unreadable", which is what
        // 403 `io_error` means.
        use std::io::ErrorKind;
        match source.kind() {
            ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidFilename => {
                FilesError::NotFound {
                    dir: dir.to_owned(),
                }
            }
            _ => io(source),
        }
    })?;
    if !canonical.starts_with(&root) {
        return Err(FilesError::OutsideRoot {
            dir: dir.to_owned(),
        });
    }
    if !canonical.is_dir() {
        return Err(FilesError::NotADirectory {
            dir: dir.to_owned(),
        });
    }
    let entries = read_entries(&root, &canonical, dir)?;
    let parent = segments.split_last().map(|(_, parents)| parents.join("/"));
    Ok(FilesResponse {
        root: root_name(&root),
        dir: segments.join("/"),
        parent,
        entries,
    })
}

/// The root's display name: its last path component, or — for a
/// file-system root, which has none — its path.
fn root_name(root: &Path) -> String {
    root.file_name().map_or_else(
        || crate::session::display_path(root),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The entries of `canonical` (a directory inside `root`, both canonical).
fn read_entries(root: &Path, canonical: &Path, dir: &str) -> Result<Vec<FileEntry>, FilesError> {
    let io = |source| FilesError::Io {
        dir: dir.to_owned(),
        source,
    };
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(canonical).map_err(io)? {
        let entry = entry.map_err(io)?;
        // A name that is not valid Unicode cannot be sent back as `dir` or
        // `?pipeline=`: it is not an entry the client could ever name.
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        // The entry's own metadata (a link's, not its target's) carries
        // the OS's hidden flag; an entry removed between the listing and
        // this read is simply gone.
        let own = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io(error)),
        };
        // A skipped directory — or a hidden link, whatever it points at —
        // is dropped BEFORE anything follows it: a home directory's hidden
        // junctions lead to places this process need not be able to stat.
        // (A hidden `.cic` FILE is still a pipeline: the rule is about
        // directories nobody can enter.)
        if !own.is_file() && skipped_directory(&name, &own) {
            continue;
        }
        // A plain entry is its own target; a link is followed, and is an
        // entry only when it leads somewhere under the root.
        let target = if own.file_type().is_symlink() {
            match resolve_link(root, &entry.path()) {
                Some(metadata) => metadata,
                None => continue,
            }
        } else {
            own
        };
        let Some(kind) = entry_kind(&name, &target) else {
            continue;
        };
        let modified_ms = modified_ms(target.modified().map_err(io)?);
        entries.push(FileEntry {
            name,
            kind,
            modified_ms,
        });
    }
    entries.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Follow a link (a symlink, or a Windows junction) at `link`: its target's
/// metadata when the canonical path stays under `root`; `None` — not an
/// entry — when it leaves the root, dangles, or cannot be resolved by this
/// process. What the list shows must be enterable: a request for a link
/// pointing out is refused, for a dangling one not found, so listing
/// either would name an entry of the root that is not one.
fn resolve_link(root: &Path, link: &Path) -> Option<std::fs::Metadata> {
    let resolved = std::fs::canonicalize(link).ok()?;
    if !resolved.starts_with(root) {
        return None;
    }
    std::fs::metadata(&resolved).ok()
}

/// What an entry (not skipped by [`skipped_directory`]) is from its name
/// and its target's metadata: a directory or a `.cic` file; `None` for
/// everything else (other files, sockets, devices).
fn entry_kind(name: &str, target: &std::fs::Metadata) -> Option<FileKind> {
    if target.is_dir() {
        Some(FileKind::Dir)
    } else if target.is_file() && is_pipeline_name(name) {
        Some(FileKind::Pipeline)
    } else {
        None
    }
}

/// `*.cic`, the extension compared case-insensitively like
/// [`crate::http::validate_pipeline_ref`] accepts it (and `/api/project`'s
/// walk collects it).
pub(crate) fn is_pipeline_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cic"))
}

/// Hidden by the platform's convention: a dot-name everywhere, plus the
/// OS's own hidden flag where it has one (read off the entry's OWN
/// metadata — a link's, not its target's).
fn is_hidden(name: &str, own: &std::fs::Metadata) -> bool {
    name.starts_with('.') || os_hidden(own)
}

/// Windows' hidden attribute — the profile's legacy junctions (`Application
/// Data`, `Cookies`, `Start Menu`, …) carry it and Explorer hides them; a
/// home-directory root would otherwise list a dozen unenterable names. The
/// attribute API exists only on Windows, so this is the one `cfg`-gated
/// body here; the other arm is the constant it reads elsewhere.
#[cfg(windows)]
fn os_hidden(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

/// No hidden attribute outside Windows — a dot-name is the convention.
#[cfg(not(windows))]
fn os_hidden(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// A file time as signed milliseconds since the Unix epoch. A file system
/// can date a file before 1970 (Windows' epoch is 1601), hence the sign;
/// the saturation only matters past year 292 million.
fn modified_ms(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[FileEntry]) -> Vec<(&str, FileKind)> {
        entries.iter().map(|e| (e.name.as_str(), e.kind)).collect()
    }

    /// A root with every kind of entry the rules speak about.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("sub").join("inner")).unwrap();
        std::fs::create_dir(root.join("Zeta")).unwrap();
        std::fs::create_dir(root.join("alpha")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir(root.join("node_modules")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join("Beta.cic"), "# cicada 1\n").unwrap();
        std::fs::write(root.join("alpha.cic"), "# cicada 1\n").unwrap();
        std::fs::write(root.join("upper.CIC"), "# cicada 1\n").unwrap();
        std::fs::write(root.join("README.md"), "not a pipeline\n").unwrap();
        std::fs::write(root.join("notes.cic.txt"), "not a pipeline\n").unwrap();
        std::fs::write(root.join("sub").join("p.cic"), "# cicada 1\n").unwrap();
        std::fs::write(root.join("sub").join("inner").join("q.cic"), "# cicada 1\n").unwrap();
        dir
    }

    #[test]
    fn normalisation_accepts_plain_relative_paths_and_drops_empty_and_dot_segments() {
        let seg = |s: &str| {
            normalize_dir(s)
                .unwrap()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("/")
        };
        assert_eq!(seg(""), "");
        assert_eq!(seg("."), "");
        assert_eq!(seg("a"), "a");
        assert_eq!(seg("a/b"), "a/b");
        assert_eq!(seg("a//b/"), "a/b");
        assert_eq!(seg("./a/./b"), "a/b");
        assert_eq!(seg("with space/ünïcode"), "with space/ünïcode");
    }

    #[test]
    fn normalisation_refuses_every_escape_before_touching_the_file_system() {
        // A root that does not exist: a refusal here proves the check is
        // lexical — the file system was never consulted.
        let root = Path::new("this-root-does-not-exist-anywhere");
        for dir in [
            "..",
            "../",
            "a/..",
            "a/../..",
            "../etc",
            "/etc",
            "/",
            "//server/share",
            "C:/Windows",
            "C:\\Windows",
            "a\\b",
            "a:b",
            "a\0b",
        ] {
            let error = list(root, dir).expect_err(dir);
            assert_eq!(
                error.kind(),
                FilesErrorKind::PathNotAllowed,
                "{dir:?}: {error}"
            );
            assert_eq!(error.dir(), dir);
            assert!(
                matches!(error, FilesError::PathNotAllowed { .. }),
                "{dir:?}: {error}"
            );
        }
    }

    #[test]
    fn the_root_lists_directories_then_pipelines_case_insensitively_and_skips_the_rest() {
        let dir = fixture();
        let listing = list(dir.path(), "").unwrap();
        assert_eq!(
            listing.root,
            dir.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(listing.dir, "");
        assert_eq!(listing.parent, None);
        assert_eq!(
            names(&listing.entries),
            vec![
                ("alpha", FileKind::Dir),
                ("sub", FileKind::Dir),
                ("Zeta", FileKind::Dir),
                ("alpha.cic", FileKind::Pipeline),
                ("Beta.cic", FileKind::Pipeline),
                ("upper.CIC", FileKind::Pipeline),
            ],
            "dot-directories, node_modules, target, README.md and notes.cic.txt are not entries"
        );
        let now = modified_ms(SystemTime::now());
        for entry in &listing.entries {
            assert!(
                entry.modified_ms > 0 && entry.modified_ms <= now + 60_000,
                "{}: modified_ms {} is not a recent time",
                entry.name,
                entry.modified_ms
            );
        }
    }

    #[test]
    fn a_nested_directory_reports_its_normalised_dir_and_parent() {
        let dir = fixture();
        let sub = list(dir.path(), "sub").unwrap();
        assert_eq!((sub.dir.as_str(), sub.parent.as_deref()), ("sub", Some("")));
        assert_eq!(
            names(&sub.entries),
            vec![("inner", FileKind::Dir), ("p.cic", FileKind::Pipeline)]
        );
        for spelling in ["sub/inner", "sub//inner/", "./sub/./inner"] {
            let inner = list(dir.path(), spelling).unwrap();
            assert_eq!(
                (inner.dir.as_str(), inner.parent.as_deref()),
                ("sub/inner", Some("sub")),
                "{spelling}"
            );
            assert_eq!(names(&inner.entries), vec![("q.cic", FileKind::Pipeline)]);
        }
    }

    /// The OS's own hidden flag (Windows' attribute) hides a directory the
    /// way a dot-name does everywhere — and a hidden `.cic` FILE is still
    /// a pipeline: the rule is about directories nobody can enter. Needs
    /// an OS with the flag; elsewhere it SKIPS LOUDLY (the dot-name arm is
    /// the test above).
    #[test]
    fn a_directory_the_os_hides_is_not_listed() {
        if !cfg!(windows) {
            eprintln!("SKIPPING: no OS hidden attribute here — the dot-name convention is tested");
            return;
        }
        let dir = fixture();
        let root = dir.path();
        std::fs::create_dir(root.join("Hidden")).unwrap();
        std::fs::write(root.join("hidden.cic"), "# cicada 1\n").unwrap();
        for target in ["Hidden", "hidden.cic"] {
            let status = std::process::Command::new("attrib")
                .arg("+H")
                .arg(root.join(target))
                .status()
                .expect("attrib runs on Windows");
            assert!(status.success(), "attrib +H {target}");
        }
        let listing = list(root, "").unwrap();
        let listed: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            !listed.contains(&"Hidden"),
            "a directory the OS hides is not listed: {listed:?}"
        );
        assert!(
            listed.contains(&"hidden.cic"),
            "a hidden pipeline FILE is still a pipeline: {listed:?}"
        );
    }

    #[test]
    fn a_missing_directory_and_a_file_are_not_found() {
        let dir = fixture();
        let missing = list(dir.path(), "nowhere/at/all").unwrap_err();
        assert_eq!(missing.kind(), FilesErrorKind::NotFound);
        assert!(matches!(missing, FilesError::NotFound { .. }), "{missing}");
        let file = list(dir.path(), "sub/p.cic").unwrap_err();
        assert_eq!(file.kind(), FilesErrorKind::NotFound);
        assert!(matches!(file, FilesError::NotADirectory { .. }), "{file}");
        assert_eq!(file.body()["kind"], "not_found");
        assert_eq!(file.body()["path"], "sub/p.cic");
        assert!(
            file.body()["message"]
                .as_str()
                .unwrap()
                .contains("not a directory")
        );
    }

    /// Nothing exists at these paths, and 403 `io_error` means "exists but
    /// unreadable" — so they are `not_found` on every OS: a path running
    /// THROUGH a file (Unix reports `NotADirectory`, Windows not found),
    /// and names Windows' file systems cannot hold (`a?b`: os error 123,
    /// `InvalidFilename` — on Unix they are legal names that simply do not
    /// exist here; either way the answer is the same).
    #[test]
    fn a_path_through_a_file_and_a_name_the_file_system_cannot_hold_are_not_found() {
        let dir = fixture();
        for path in [
            "sub/p.cic/deeper",
            "a?b",
            "a*b",
            "a<b",
            "a>b",
            "a|b",
            "a\"b",
            "sub/a?b/c",
        ] {
            let error = list(dir.path(), path).expect_err(path);
            assert_eq!(error.kind(), FilesErrorKind::NotFound, "{path:?}: {error}");
            assert!(
                matches!(error, FilesError::NotFound { .. }),
                "{path:?}: {error}"
            );
            assert_eq!(error.dir(), path);
        }
    }

    /// Skipping is a listing convention, not a boundary: a dot-directory
    /// or a build tree named directly lists like any other directory under
    /// the root (the root is the boundary — see the escape tests).
    #[test]
    fn an_unlisted_directory_is_still_enterable_by_name() {
        let dir = fixture();
        std::fs::write(dir.path().join(".git").join("h.cic"), "# cicada 1\n").unwrap();
        let hidden = list(dir.path(), ".git").unwrap();
        assert_eq!(
            (hidden.dir.as_str(), hidden.parent.as_deref()),
            (".git", Some(""))
        );
        assert_eq!(names(&hidden.entries), vec![("h.cic", FileKind::Pipeline)]);
        let skipped = list(dir.path(), "node_modules").unwrap();
        assert_eq!(skipped.dir, "node_modules");
        assert!(skipped.entries.is_empty());
    }

    #[test]
    fn a_root_that_vanished_is_an_io_failure_not_a_listing() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let error = list(&gone, "").unwrap_err();
        // The ROOT failed to resolve — that is the server's problem, never
        // a quiet empty list.
        assert_eq!(error.kind(), FilesErrorKind::IoError);
        assert!(matches!(error, FilesError::Io { .. }), "{error}");
    }

    #[test]
    fn the_refusal_body_is_kind_message_path() {
        let error = normalize_dir("../x").unwrap_err();
        let body = error.body();
        assert_eq!(body["kind"], "path_not_allowed");
        assert_eq!(body["path"], "../x");
        assert_eq!(
            body["message"],
            "`../x` is not a root-relative directory path: a `..` segment"
        );
    }

    #[test]
    fn file_times_are_signed_milliseconds() {
        assert_eq!(modified_ms(UNIX_EPOCH), 0);
        assert_eq!(
            modified_ms(UNIX_EPOCH + std::time::Duration::from_millis(1_500)),
            1_500
        );
        assert_eq!(
            modified_ms(UNIX_EPOCH - std::time::Duration::from_millis(2_500)),
            -2_500
        );
    }

    #[test]
    fn kinds_sort_directories_first() {
        assert!(FileKind::Dir < FileKind::Pipeline);
        assert_eq!(
            serde_json::to_value([FileKind::Dir, FileKind::Pipeline]).unwrap(),
            serde_json::json!(["dir", "pipeline"])
        );
    }
}
