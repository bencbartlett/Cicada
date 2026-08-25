//! `cicada serve [path]` (stage 5, doc 15; the root since v0.1 wave 4):
//! the app. Binds localhost with a Jupyter-style token, prints the URL,
//! serves until Ctrl-C. The logic is `cicada_server::serve`; this file
//! resolves the argument ([`resolve_root`] — shared with `cicada app`) and
//! runs the tokio runtime.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cicada_server::{ServeConfig, serve};

/// Arguments of `cicada serve`.
pub struct ServeArgs {
    /// A directory (the root), a `.cic` pipeline file (its directory is
    /// the root, the file opens), or nothing: the user's home directory
    /// as the root and nothing opened.
    pub path: Option<PathBuf>,
    /// Bind host.
    pub host: IpAddr,
    /// Port (0 = ephemeral).
    pub port: u16,
    /// Fixed token (tests); default random.
    pub token: Option<String>,
    /// Store override; default = the per-project user cache dir.
    pub cache_dir: Option<PathBuf>,
    /// Worker threads; 0 = cores − 2.
    pub threads: usize,
    /// Serve a built SPA from this directory.
    pub web_dir: Option<PathBuf>,
}

/// What `cicada serve [path]` — and `cicada app [path]` — serve: the root
/// directory, and the pipeline to open when the argument named one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// The root: an existing directory, canonical, without Windows'
    /// verbatim prefix.
    pub dir: PathBuf,
    /// The pipeline to open by default — its file name, root-relative.
    pub pipeline: Option<String>,
}

/// Resolve the optional path argument (docs/13 §Projects, pipelines,
/// sessions; docs/17 wave 4 O1): no path → the user's home directory is
/// the root and nothing opens; a directory → it is the root; a `.cic` file
/// → its directory is the root and the file opens.
///
/// # Errors
///
/// No home directory for this user (no path was given); a path that does
/// not exist, or is neither a directory nor a `.cic` file.
pub fn resolve_root(path: Option<&Path>) -> anyhow::Result<Root> {
    let given = match path {
        Some(path) => path.to_owned(),
        None => std::env::home_dir().context(
            "no home directory for this user — pass a directory or a .cic file to serve",
        )?,
    };
    let canonical = plain(
        &std::fs::canonicalize(&given).with_context(|| format!("resolving {}", given.display()))?,
    );
    let (dir, pipeline) = split_target(&canonical)?;
    Ok(Root { dir, pipeline })
}

/// Run the server until Ctrl-C.
///
/// # Errors
///
/// Bad paths, bind failures, a default pipeline that fails to open.
pub fn serve_command(args: &ServeArgs) -> anyhow::Result<()> {
    let root = resolve_root(args.path.as_deref())?;
    // Every path argument becomes absolute BEFORE the chdir below.
    let cache_dir = args
        .cache_dir
        .as_ref()
        .map(|dir| std::path::absolute(dir).with_context(|| format!("resolving {}", dir.display())))
        .transpose()?;
    let web_dir = args
        .web_dir
        .as_ref()
        .map(|dir| {
            std::fs::canonicalize(dir)
                .map(|d| plain(&d))
                .with_context(|| format!("resolving {}", dir.display()))
        })
        .transpose()?;
    // Relative paths inside pipelines (exporter `path=` literals) resolve
    // against the ROOT — the project directory when one was served, as
    // since stage 5 — never against wherever the server happened to be
    // launched. (`cicada run` resolves against the pipeline's own
    // directory; the two agree when the pipeline sits at the root.)
    std::env::set_current_dir(&root.dir)
        .with_context(|| format!("entering {}", root.dir.display()))?;
    let mut config = ServeConfig::new(root.dir.clone());
    config.pipeline.clone_from(&root.pipeline);
    config.host = args.host;
    config.port = args.port;
    config.token.clone_from(&args.token);
    config.cache_dir = cache_dir;
    config.threads = args.threads;
    config.web_dir = web_dir;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the tokio runtime")?;
    runtime.block_on(async move {
        let cache_note = match &args.cache_dir {
            Some(dir) => format!("the store lives in {} (--cache-dir)", dir.display()),
            None => "the store lives in the user cache dir (never the project)".to_owned(),
        };
        let handle = serve(config).await?;
        println!("cicada serve — {}", handle.url());
        match &root.pipeline {
            Some(pipeline) => println!("  root {} — {pipeline} open", root.dir.display()),
            None => println!(
                "  root {} — no pipeline open; pick one in the app, or pass a .cic",
                root.dir.display()
            ),
        }
        println!("  Ctrl-C stops the server; {cache_note}.");
        let waiter = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
        });
        tokio::select! {
            _ = waiter => {
                println!("shutting down");
                handle.shutdown().await;
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

/// Strip Windows' `\\?\` verbatim prefix from a canonical path — it is
/// correct for the OS and unreadable for humans in every message.
pub(crate) fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path.to_owned(),
    }
}

/// A directory → `(dir, None)`; a `.cic` file → `(its dir, Some(relative))`
/// — the extension case-insensitive, as `GET /api/files` lists pipelines
/// and `?pipeline=` opens them (one rule for what a pipeline file is).
/// Shared with `cicada mcp --project`, which takes the same argument.
pub(crate) fn split_target(path: &Path) -> anyhow::Result<(PathBuf, Option<String>)> {
    if path.is_dir() {
        return Ok((path.to_owned(), None));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cic"))
    {
        let dir = path
            .parent()
            .map(Path::to_owned)
            .context("pipeline has no parent directory")?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .context("pipeline has no file name")?;
        return Ok((dir, Some(name)));
    }
    bail!("{} is neither a directory nor a .cic file", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_plain(path: &Path) -> PathBuf {
        plain(&std::fs::canonicalize(path).unwrap())
    }

    #[test]
    fn a_directory_is_the_root_and_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("p.cic"), "# cicada 1\n").unwrap();
        let root = resolve_root(Some(dir.path())).unwrap();
        assert_eq!(
            root,
            Root {
                dir: canonical_plain(dir.path()),
                pipeline: None,
            }
        );
    }

    #[test]
    fn a_pipeline_makes_its_directory_the_root_and_opens_itself() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("p.cic"), "# cicada 1\n").unwrap();
        let root = resolve_root(Some(&sub.join("p.cic"))).unwrap();
        assert_eq!(
            root,
            Root {
                dir: canonical_plain(&sub),
                pipeline: Some("p.cic".to_owned()),
            }
        );
    }

    /// `Upper.CIC` is a pipeline to the file list and to `?pipeline=`; the
    /// command line must not be the one place that refuses it.
    #[test]
    fn an_upper_case_extension_is_a_pipeline_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Upper.CIC"), "# cicada 1\n").unwrap();
        let root = resolve_root(Some(&dir.path().join("Upper.CIC"))).unwrap();
        assert_eq!(
            root,
            Root {
                dir: canonical_plain(dir.path()),
                pipeline: Some("Upper.CIC".to_owned()),
            }
        );
    }

    #[test]
    fn no_path_is_the_home_directory_with_nothing_open() {
        let home = std::env::home_dir().expect("this user has a home directory");
        let root = resolve_root(None).unwrap();
        assert_eq!(
            root,
            Root {
                dir: canonical_plain(&home),
                pipeline: None,
            }
        );
        assert!(root.dir.is_dir());
    }

    #[test]
    fn anything_else_is_refused_with_the_path_named() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "not a pipeline\n").unwrap();
        let error = resolve_root(Some(&other)).unwrap_err().to_string();
        assert!(
            error.contains("neither a directory nor a .cic file") && error.contains("notes.txt"),
            "{error}"
        );
        let missing = dir.path().join("missing.cic");
        let error = resolve_root(Some(&missing)).unwrap_err().to_string();
        assert!(
            error.contains("resolving") && error.contains("missing.cic"),
            "{error}"
        );
    }
}
