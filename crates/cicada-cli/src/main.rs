//! The `cicada` binary (doc 14): `serve`, `run`, `fmt`, `docs`, `catalog`,
//! `cache`. Subcommands appear as their stages land (doc 15); stage 3
//! ships `catalog` and `run` — inventing stubs for the rest would lie to
//! `--help`. Logic lives in the library ([`cicada_cli`]); this file only
//! parses arguments.

use std::path::PathBuf;

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
    /// Regenerate docs/generated/CATALOG.md + catalog.json from the node
    /// registry (doc 14 §Documentation pipeline).
    Catalog {
        /// Verify the committed catalog files are fresh instead of writing
        /// (CI mode); exits non-zero when stale.
        #[arg(long)]
        check: bool,
        /// Output directory; defaults to docs/generated/ at the repo root.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Solve a pipeline headlessly and print the requested outputs
    /// (doc 15 stage 3; the agent verification loop's first stop).
    Run {
        /// The .cic pipeline file.
        pipeline: PathBuf,
        /// Binding to compute (repeatable); defaults to every leaf.
        #[arg(long = "node")]
        nodes: Vec<String>,
        /// Print per-node compute times and a solve summary.
        #[arg(long)]
        time: bool,
        /// Print stable `binding<TAB>port<TAB>hash` lines instead of
        /// values (scriptable; doc 14's verification currency).
        #[arg(long)]
        hashes: bool,
        /// Cache directory override. Default: the per-project store in the
        /// USER cache dir — never the project folder (DECISIONS.md).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Worker threads; 0 = all cores minus two.
        #[arg(long, default_value_t = 0)]
        threads: usize,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Catalog { check, dir } => cicada_cli::catalog::catalog(check, dir),
        Command::Run {
            pipeline,
            nodes,
            time,
            hashes,
            cache_dir,
            threads,
        } => cicada_cli::run::run(&cicada_cli::run::RunArgs {
            pipeline,
            nodes,
            time,
            hashes,
            cache_dir,
            threads,
        }),
    }
}
