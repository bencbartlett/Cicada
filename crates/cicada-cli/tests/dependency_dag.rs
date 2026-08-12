//! Dependency direction is law (doc 14 §Workspace layout):
//! `core ← {geom, lang, stdlib, sched, script} ← server ← cli`.
//! This test asserts the workspace dependency DAG matches that layering, so a
//! forbidden edge fails `cargo test` everywhere — not just in CI.

// Tests are exempt from the expect/unwrap denial (clippy.toml), but that
// exemption only recognizes #[test] fns — not helpers in integration tests.
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::{DependencyKind, MetadataCommand};

const MID_LAYER: &[&str] = &[
    "cicada-geom",
    "cicada-lang",
    "cicada-stdlib",
    "cicada-sched",
    "cicada-script",
];

/// Direct in-workspace dependency edges, `crate → {deps}`, all kinds
/// (normal, dev, build) — the layering law has no dev-dependency exemption.
fn workspace_edges() -> BTreeMap<String, BTreeSet<String>> {
    let metadata = MetadataCommand::new()
        .exec()
        .expect("cargo metadata must run");
    let workspace: BTreeSet<String> = metadata
        .workspace_packages()
        .iter()
        .map(|package| package.name.clone())
        .collect();
    metadata
        .workspace_packages()
        .iter()
        .map(|package| {
            let deps = package
                .dependencies
                .iter()
                .filter(|dep| {
                    workspace.contains(&dep.name)
                        && matches!(
                            dep.kind,
                            DependencyKind::Normal
                                | DependencyKind::Development
                                | DependencyKind::Build
                        )
                })
                .map(|dep| dep.name.clone())
                .collect();
            (package.name.clone(), deps)
        })
        .collect()
}

fn transitive_deps(edges: &BTreeMap<String, BTreeSet<String>>, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_owned()];
    while let Some(name) = stack.pop() {
        if let Some(deps) = edges.get(&name) {
            for dep in deps {
                if seen.insert(dep.clone()) {
                    stack.push(dep.clone());
                }
            }
        }
    }
    seen
}

#[test]
fn all_nine_crates_exist() {
    let edges = workspace_edges();
    for name in [
        "cicada-core",
        "cicada-macros",
        "cicada-geom",
        "cicada-lang",
        "cicada-stdlib",
        "cicada-sched",
        "cicada-script",
        "cicada-server",
        "cicada-cli",
    ] {
        assert!(edges.contains_key(name), "workspace is missing {name}");
    }
}

#[test]
fn core_depends_only_on_macros() {
    let edges = workspace_edges();
    let allowed = BTreeSet::from(["cicada-macros".to_owned()]);
    let extra: Vec<_> = edges["cicada-core"].difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "cicada-core must stay tiny (doc 14) — forbidden deps: {extra:?}"
    );
}

#[test]
fn macros_depends_on_no_workspace_crate() {
    let edges = workspace_edges();
    assert!(
        edges["cicada-macros"].is_empty(),
        "cicada-macros must not depend on workspace crates: {:?}",
        edges["cicada-macros"]
    );
}

#[test]
fn only_cli_depends_on_server() {
    let edges = workspace_edges();
    for (name, deps) in &edges {
        if name != "cicada-cli" {
            assert!(
                !deps.contains("cicada-server"),
                "{name} must not depend on cicada-server (only cicada-cli may, doc 14)"
            );
        }
    }
}

#[test]
fn nothing_depends_on_cli() {
    let edges = workspace_edges();
    for (name, deps) in &edges {
        assert!(
            !deps.contains("cicada-cli"),
            "{name} must not depend on cicada-cli"
        );
    }
}

#[test]
fn mid_layer_never_depends_on_server_or_cli() {
    let edges = workspace_edges();
    for name in MID_LAYER {
        let transitive = transitive_deps(&edges, name);
        assert!(
            !transitive.contains("cicada-server") && !transitive.contains("cicada-cli"),
            "{name} must not reach server/cli (doc 14 layering): {transitive:?}"
        );
    }
}

#[test]
fn stdlib_never_depends_on_sched() {
    // Nodes are pure functions; the scheduler calls THEM (doc 14).
    let edges = workspace_edges();
    let transitive = transitive_deps(&edges, "cicada-stdlib");
    assert!(
        !transitive.contains("cicada-sched"),
        "cicada-stdlib must never depend on cicada-sched, even transitively"
    );
}
