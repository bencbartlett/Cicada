//! `cicada serve` (stage 5, doc 15): the app. Binds localhost with a
//! Jupyter-style token, prints the URL, serves until Ctrl-C. The logic is
//! `cicada_server::serve`; this file resolves arguments (a project
//! directory or a `.cic` file — the file's directory becomes the project
//! and the file the default pipeline) and runs the tokio runtime.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cicada_server::{ServeConfig, serve};

/// Arguments of `cicada serve`.
pub struct ServeArgs {
    /// A project directory or a `.cic` pipeline file (default: `.`).
    pub path: PathBuf,
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

/// Run the server until Ctrl-C.
///
/// # Errors
///
/// Bad paths, bind failures, a default pipeline that fails to open.
pub fn serve_command(args: &ServeArgs) -> anyhow::Result<()> {
    serve_with(args, "serve", |_url| {})
}

/// [`serve_command`] with a hook: once the server is bound and its URL
/// printed, `on_ready(url)` runs — `cicada app` opens the browser there.
/// `command` names the subcommand in the console line. ONE resolution of
/// the path argument serves both subcommands (`app` inherits whatever
/// `serve` does with it).
///
/// # Errors
///
/// Bad paths, bind failures, a default pipeline that fails to open.
pub fn serve_with(
    args: &ServeArgs,
    command: &str,
    on_ready: impl FnOnce(&str),
) -> anyhow::Result<()> {
    let path = plain(
        &std::fs::canonicalize(&args.path)
            .with_context(|| format!("resolving {}", args.path.display()))?,
    );
    let (project_dir, pipeline) = split_target(&path)?;
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
    // against the PROJECT directory — the same rule `cicada run` applies to
    // the pipeline's directory — never against wherever the server
    // happened to be launched.
    std::env::set_current_dir(&project_dir)
        .with_context(|| format!("entering {}", project_dir.display()))?;
    let mut config = ServeConfig::new(project_dir);
    config.pipeline = pipeline;
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
        let url = handle.url();
        println!("cicada {command} — {url}");
        println!("  Ctrl-C stops the server; {cache_note}.");
        on_ready(&url);
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

/// A directory → `(dir, None)`; a `.cic` file → `(its dir, Some(relative))`.
/// Shared with `cicada mcp --project`, which takes the same argument.
pub(crate) fn split_target(path: &Path) -> anyhow::Result<(PathBuf, Option<String>)> {
    if path.is_dir() {
        return Ok((path.to_owned(), None));
    }
    if path.extension().is_some_and(|e| e == "cic") {
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
