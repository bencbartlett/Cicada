//! The build behind this binary (docs/17 wave 5 R1): the workspace version
//! plus the commit and the UTC date `build.rs` stamped (`src/stamp.rs` has
//! the rules). `cicada --version` prints [`LINE`]; `cicada serve` / `app`
//! hand the same three values to the server, which answers them on `hello`
//! as `version`, on `GET /api/version`, and — through the app — in About.

/// The workspace version (`CARGO_PKG_VERSION`), e.g. `0.1.0-alpha.1`.
pub const SEMVER: &str = env!("CARGO_PKG_VERSION");

/// The commit the binary was built from: git's 12-digit short hash,
/// `-dirty` when the tree had uncommitted tracked changes, or `unknown`
/// when the build could not tell (said at build time; `stamp::UNKNOWN`).
pub const COMMIT: &str = env!("CICADA_BUILD_COMMIT");

/// The UTC date of the build, `YYYY-MM-DD` (`SOURCE_DATE_EPOCH` when set).
pub const BUILT: &str = env!("CICADA_BUILD_DATE");

/// What `cicada --version` prints after the binary's name:
/// `0.1.0-alpha.1 (a82eb39d1c2e, 2026-08-25)`.
pub const LINE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("CICADA_BUILD_COMMIT"),
    ", ",
    env!("CICADA_BUILD_DATE"),
    ")"
);

/// The same three values as the server reports them — `serve` and `app`
/// put this in [`cicada_server::ServeConfig::version`], so `hello.version`,
/// `GET /api/version` and About say exactly what `--version` says.
#[must_use]
pub fn info() -> cicada_server::protocol::VersionInfo {
    cicada_server::protocol::VersionInfo {
        semver: SEMVER.to_owned(),
        commit: COMMIT.to_owned(),
        built: BUILT.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stamp;

    #[test]
    fn the_stamp_has_the_shape_the_build_script_promises() {
        assert!(!SEMVER.is_empty());
        assert!(
            stamp::is_git_stamp(COMMIT) || COMMIT == stamp::UNKNOWN,
            "commit {COMMIT:?} is neither a git stamp nor `unknown`"
        );
        assert!(stamp::is_date(BUILT), "built {BUILT:?} is not YYYY-MM-DD");
        assert_eq!(LINE, format!("{SEMVER} ({COMMIT}, {BUILT})"));
        let info = info();
        assert_eq!(
            (
                info.semver.as_str(),
                info.commit.as_str(),
                info.built.as_str()
            ),
            (SEMVER, COMMIT, BUILT)
        );
    }
}
