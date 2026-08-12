//! The `cicada` binary (doc 14): `serve`, `run`, `fmt`, `docs`, `catalog`,
//! `cache`. Subcommands appear as their stages land (doc 15); stage 0 ships
//! `catalog` only — inventing stubs for the rest would lie to `--help`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cicada",
    version,
    about = "Cicada: code-first parametric design"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate docs/generated/CATALOG.md from the node registry (doc 14).
    Catalog {
        /// Verify the committed catalog is fresh instead of writing (CI mode);
        /// exits non-zero when stale.
        #[arg(long)]
        check: bool,
        /// Output path; defaults to docs/generated/CATALOG.md at the repo root.
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Catalog { check, path } => catalog(check, path),
    }
}

/// Repo-root catalog location, resolved from this crate's manifest directory
/// so `cargo run -p cicada-cli` works from any working directory.
fn default_catalog_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/generated/CATALOG.md")
}

fn catalog(check: bool, path: Option<PathBuf>) -> anyhow::Result<()> {
    let path = path.unwrap_or_else(default_catalog_path);
    let rendered = cicada_core::catalog::render_markdown(&cicada_stdlib::registry());

    if check {
        // Byte-exact on purpose: .gitattributes pins LF everywhere, so any
        // difference — including line endings — is real staleness.
        let committed =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        if committed == rendered {
            println!("catalog fresh: {}", path.display());
        } else {
            bail!(
                "{} is stale — regenerate with `cargo run -p cicada-cli -- catalog` \
                 and commit the diff",
                path.display()
            );
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, &rendered).with_context(|| format!("writing {}", path.display()))?;
        println!("catalog written: {}", path.display());
    }
    Ok(())
}
