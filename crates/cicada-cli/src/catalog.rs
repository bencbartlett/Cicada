//! `cicada catalog`: regenerate (or verify) `docs/generated/CATALOG.md` +
//! `catalog.json` from the node registry (doc 14 §Documentation pipeline).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, bail};

/// Repo-root generated-docs dir, resolved from this crate's manifest
/// directory so `cargo run -p cicada-cli` works from any working directory.
fn default_generated_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/generated")
}

/// Write (or `--check`) the generated catalog files.
///
/// # Errors
///
/// I/O problems, or staleness in check mode.
pub fn catalog(check: bool, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = dir.unwrap_or_else(default_generated_dir);
    let specs = cicada_stdlib::registry();
    let markdown = cicada_core::catalog::render_markdown(specs);
    // Same renderer as `GET /api/catalog` — one JSON shape, two outlets.
    let json = cicada_server::catalog::render_json(specs).context("serializing catalog.json")?;

    let outputs: [(&str, &str); 2] = [("CATALOG.md", &markdown), ("catalog.json", &json)];
    if check {
        for (file, rendered) in outputs {
            let path = dir.join(file);
            // Byte-exact on purpose: .gitattributes pins LF everywhere, so
            // any difference — including line endings — is real staleness.
            let committed =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            if committed != rendered {
                bail!(
                    "{} is stale — regenerate with `cargo run -p cicada-cli -- catalog` \
                     and commit the diff",
                    path.display()
                );
            }
            println!("catalog fresh: {}", path.display());
        }
    } else {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for (file, rendered) in outputs {
            let path = dir.join(file);
            fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
            println!("catalog written: {}", path.display());
        }
    }
    Ok(())
}
