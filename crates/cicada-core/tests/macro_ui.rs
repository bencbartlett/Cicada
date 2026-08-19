//! Compile-fail UX of `#[node]` / `#[derive(Ports)]` — the macros' loud
//! refusal contract. Hosted here rather than in cicada-macros because the
//! macros crate keeps zero workspace deps (DAG law) and the cases need
//! cicada-core.
//!
//! To bless updated .stderr snapshots after a rustc or message change
//! (PowerShell form; bash: `TRYBUILD=overwrite cargo test …`):
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"; cargo test -p cicada-core --test macro_ui; Remove-Item Env:\TRYBUILD
//! ```
//!
//! Review the diff, commit it with the reason. Note: trybuild builds its
//! scratch package OUTSIDE the committed Cargo.lock (fresh resolution at
//! test time) — if these tests break on a PR that touched nothing, suspect
//! a new syn/proc-macro2 registry release first, not the repo.

#[test]
fn ui() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
