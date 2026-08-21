//! The axum app (docs/13 §HTTP surface, §Security and serving): `cicada
//! serve` binds 127.0.0.1 with a Jupyter-style token, serves the SPA
//! (embedded behind the `embed` feature, or from `--web-dir`, or — dev — the
//! Vite proxy points here), the API routes, one WebSocket per client
//! session, and the `/debug/*` endpoints agents verify UI changes with.
//!
//! Auth: every `/api`, `/ws`, and `/debug` request carries the token
//! (`?token=`, `Authorization: Bearer`, or `X-Cicada-Token`); the SPA and
//! its assets do not (the page loads, then reads the token from its URL —
//! exactly Jupyter's shape). Nothing here assumes locality except the
//! default bind; a reverse proxy with real auth is the remote story.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
#[cfg(not(feature = "embed"))]
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use cicada_core::config::ProjectConfig;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::git::{Git, GitRefusal, Scope};
use crate::protocol::{
    ApplyTextRequest, CommitRequest, DeltaSource, GitErrorKind, IntentEnvelope, PROTOCOL_VERSION,
    RevertRequest, Role, ServerMessage, encode,
};
use crate::session::{ClientLanes, IntentError, Outgoing, Session, SessionConfig};

/// The lease hand-off grace after a writer disconnects (docs/13: 5 s).
pub const LEASE_GRACE: Duration = Duration::from_secs(5);
/// Default port.
pub const DEFAULT_PORT: u16 = 8420;

/// `cicada serve` options.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// The project directory.
    pub project_dir: PathBuf,
    /// The pipeline to open by default (relative to the project), if any.
    pub pipeline: Option<String>,
    /// Bind address (default 127.0.0.1).
    pub host: IpAddr,
    /// Port (0 = ephemeral, for tests).
    pub port: u16,
    /// Session token; `None` = random.
    pub token: Option<String>,
    /// Store override (tests/CI); default = per-project user cache dir.
    pub cache_dir: Option<PathBuf>,
    /// Worker threads (0 = cores − 2).
    pub threads: usize,
    /// Serve a built SPA from this directory (release-like without the
    /// `embed` feature).
    pub web_dir: Option<PathBuf>,
    /// Project configuration (units, tolerance).
    pub project: ProjectConfig,
}

impl ServeConfig {
    /// Sensible defaults for a project directory.
    #[must_use]
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            project_dir,
            pipeline: None,
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            token: None,
            cache_dir: None,
            threads: 0,
            web_dir: None,
            project: ProjectConfig::default(),
        }
    }
}

/// Server start failures.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The project directory does not exist.
    #[error("project directory {0} does not exist")]
    NoProject(PathBuf),
    /// Binding failed.
    #[error("binding {addr}: {source}")]
    Bind {
        /// The address.
        addr: SocketAddr,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The default pipeline could not open.
    #[error(transparent)]
    Session(#[from] crate::session::SessionError),
    /// The file watcher could not start.
    #[error("file watcher: {0}")]
    Watch(String),
    /// A pipeline reference is not a plain project-relative `.cic` path.
    #[error(
        "`{0}` is not a project-relative .cic path (no absolute paths, drive or root prefixes, \
         `.`/`..` segments)"
    )]
    BadPipelineRef(String),
    /// A pipeline resolved outside the served project (symlink or other
    /// indirection) — refused, never opened.
    #[error("{path} lies outside the served project {project}")]
    OutsideProject {
        /// The offending resolved path.
        path: PathBuf,
        /// The project directory.
        project: PathBuf,
    },
    /// No OS randomness for the session token.
    #[error("OS randomness unavailable — refusing to serve with a guessable token; pass --token")]
    NoRandomness,
    /// The project has no such pipeline file.
    #[error("no pipeline `{0}` in the project (see /api/project for the list)")]
    NoSuchPipeline(String),
    /// An HTTP action needs the write lease (`X-Cicada-Client` must be the
    /// writer's id).
    #[error("the write lease is required: send X-Cicada-Client: <your client id> as the writer")]
    NotWriter,
}

struct AppState {
    config: ServeConfig,
    token: String,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// The project's git handle (doc 17 item 2): every git route runs
    /// through it, with the project dir as cwd.
    git: Git,
}

/// A running server.
pub struct ServerHandle {
    /// The bound address.
    pub addr: SocketAddr,
    /// The session token.
    pub token: String,
    state: Arc<AppState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    /// The URL to open (token embedded; `pipeline` when a default exists).
    #[must_use]
    pub fn url(&self) -> String {
        let mut url = format!("http://{}/?token={}", self.addr, self.token);
        if let Some(pipeline) = &self.state.config.pipeline {
            url.push_str("&pipeline=");
            url.push_str(pipeline);
        }
        url
    }

    /// Stop serving (drops sessions).
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = (&mut self.task).await;
        self.state
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Wait until the server task ends (Ctrl-C handling is the caller's).
    pub async fn wait(mut self) {
        let _ = (&mut self.task).await;
    }
}

/// Start serving. Opens the default pipeline eagerly (so a broken project
/// fails at startup, loudly), starts the project watcher, binds, and
/// returns a handle.
///
/// # Errors
///
/// [`ServeError`].
pub async fn serve(mut config: ServeConfig) -> Result<ServerHandle, ServeError> {
    if !config.project_dir.is_dir() {
        return Err(ServeError::NoProject(config.project_dir.clone()));
    }
    // One canonical project root: every containment check and every
    // session key compares against it.
    config.project_dir = std::fs::canonicalize(&config.project_dir)
        .map_err(|_| ServeError::NoProject(config.project_dir.clone()))?;
    let token = match config.token.clone() {
        Some(token) => token,
        None => random_token()?,
    };
    let state = Arc::new(AppState {
        token: token.clone(),
        sessions: Mutex::new(HashMap::new()),
        watcher: Mutex::new(None),
        git: Git::new(&config.project_dir),
        config,
    });
    if let Some(pipeline) = state.config.pipeline.clone() {
        let opening = Arc::clone(&state);
        tokio::task::spawn_blocking(move || open_session(&opening, &pipeline))
            .await
            .map_err(|e| ServeError::Watch(e.to_string()))??;
    }
    start_watcher(&state)?;

    let app = router(Arc::clone(&state));
    let addr = SocketAddr::new(state.config.host, state.config.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ServeError::Bind { addr, source })?;
    let bound = listener
        .local_addr()
        .map_err(|source| ServeError::Bind { addr, source })?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            eprintln!("cicada serve: {error}");
        }
    });
    Ok(ServerHandle {
        addr: bound,
        token,
        state,
        shutdown: Some(shutdown_tx),
        task,
    })
}

fn random_token() -> Result<String, ServeError> {
    let mut bytes = [0u8; 24];
    // No fallback: a token derived from process state would be guessable,
    // and a guessable token is worse than not serving.
    getrandom::fill(&mut bytes).map_err(|_| ServeError::NoRandomness)?;
    let mut out = String::with_capacity(48);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

/// A pipeline reference from a client is a plain, project-relative `.cic`
/// path: only normal components (no root, prefix, `.`, or `..`) and the
/// `.cic` extension. Anything else is refused as a bad reference — the
/// only way to name a pipeline is by its place inside the project.
///
/// # Errors
///
/// [`ServeError::BadPipelineRef`].
pub fn validate_pipeline_ref(relative: &str) -> Result<PathBuf, ServeError> {
    use std::path::Component;
    let path = Path::new(relative);
    let bad = || ServeError::BadPipelineRef(relative.to_owned());
    if relative.is_empty()
        || path.is_absolute()
        || path.has_root()
        || relative.contains('\\')
        || relative.contains(':')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(bad());
    }
    let is_cic = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cic"));
    if !is_cic {
        return Err(bad());
    }
    Ok(path.to_owned())
}

/// Resolve a validated reference INSIDE the project: the joined path is
/// canonicalized and must start with the canonical project dir (a
/// symlink pointing outside is refused). Returns the canonical file path
/// and the canonical project-relative key (one session per real file —
/// `p.cic` and `./p.cic` cannot open two writers on one file).
fn resolve_in_project(project_dir: &Path, relative: &str) -> Result<(PathBuf, String), ServeError> {
    let checked = validate_pipeline_ref(relative)?;
    let candidate = project_dir.join(&checked);
    let canonical_project =
        std::fs::canonicalize(project_dir).map_err(|source| ServeError::Bind {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            source,
        })?;
    if !candidate.is_file() {
        return Err(ServeError::NoSuchPipeline(relative.to_owned()));
    }
    let canonical = std::fs::canonicalize(&candidate).map_err(|_| ServeError::OutsideProject {
        path: candidate.clone(),
        project: project_dir.to_owned(),
    })?;
    let key = canonical
        .strip_prefix(&canonical_project)
        .map_err(|_| ServeError::OutsideProject {
            path: canonical.clone(),
            project: canonical_project.clone(),
        })?
        .to_string_lossy()
        .replace('\\', "/");
    Ok((canonical, key))
}

fn open_session(state: &AppState, relative: &str) -> Result<Arc<Session>, ServeError> {
    let (pipeline, key) = resolve_in_project(&state.config.project_dir, relative)?;
    if let Some(existing) = state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return Ok(Arc::clone(existing));
    }
    let session = Session::open(SessionConfig {
        project_dir: state.config.project_dir.clone(),
        pipeline,
        cache_dir: state.config.cache_dir.clone(),
        threads: state.config.threads,
        project: state.config.project,
        op_clock: None,
    })?;
    // Two clients racing to open the same pipeline: the second finds the
    // first's session already inserted and drops its own.
    let mut sessions = state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = Arc::clone(sessions.entry(key).or_insert(session));
    Ok(session)
}

/// The project watcher (docs/13 §External changes): debounced; `.cic`,
/// sidecar, and `scripts/*.py` changes reload the affected sessions with a
/// barrier snapshot. The sessions ignore their own writes by text hash.
fn start_watcher(state: &Arc<AppState>) -> Result<(), ServeError> {
    use notify::Watcher as _;
    let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            // Drop ACCESS events (open / read / close): a read is not a
            // change. This matters because `reload_from_disk` READS the
            // `.cic` and its sidecar — on Linux inotify those reads fire
            // CLOSE_NOWRITE events, so forwarding them turned every reload
            // into read → event → reload → read, a self-sustaining storm
            // (Windows/macOS never report reads as change events, so it
            // only showed on Linux). Mutations (Create/Modify/Remove) and
            // the imprecise catch-alls (Any/Other) still flow.
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
    })
    .map_err(|e| ServeError::Watch(e.to_string()))?;
    watcher
        .watch(&state.config.project_dir, notify::RecursiveMode::Recursive)
        .map_err(|e| ServeError::Watch(e.to_string()))?;
    *state
        .watcher
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(watcher);
    let weak = Arc::downgrade(state);
    std::thread::Builder::new()
        .name("cicada-watch".to_owned())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut changed = vec![first];
                // Coalesce a burst (git checkout touches many files) without
                // eating the canvas-round-trip budget (docs/15: < 500 ms).
                std::thread::sleep(Duration::from_millis(80));
                while let Ok(more) = rx.try_recv() {
                    changed.push(more);
                }
                let Some(state) = weak.upgrade() else { return };
                let sessions: Vec<Arc<Session>> = state
                    .sessions
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .values()
                    .cloned()
                    .collect();
                for session in sessions {
                    let pipeline = session.pipeline().to_owned();
                    let sidecar = crate::sidecar::Sidecar::path_for(&pipeline);
                    let scripts_dir = pipeline.parent().map(|d| d.join("scripts"));
                    let mut reload = false;
                    let mut rescan = false;
                    for path in &changed {
                        if same_file(path, &pipeline) || same_file(path, &sidecar) {
                            reload = true;
                        }
                        if path.extension().is_some_and(|e| e == "py")
                            && scripts_dir
                                .as_ref()
                                .is_some_and(|dir| path.parent().is_some_and(|p| same_file(p, dir)))
                        {
                            reload = true;
                            rescan = true;
                        }
                    }
                    if reload {
                        match session.reload_from_disk("external file change", rescan) {
                            Ok(true) => {
                                eprintln!("reloaded {} (external change)", session.relative());
                            }
                            Ok(false) => {}
                            Err(error) => eprintln!(
                                "reload of {} failed: {error} — previous state stays live",
                                session.relative()
                            ),
                        }
                    }
                }
            }
        })
        .map_err(|e| ServeError::Watch(e.to_string()))?;
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/catalog", get(api_catalog))
        .route("/api/project", get(api_project))
        .route("/api/blob/{hash}", get(api_blob))
        .route("/api/run/{node}", post(api_run))
        .route("/api/edit/text", get(api_edit_text))
        .route("/api/edit/apply_text", post(api_edit_apply_text))
        .route("/api/git/status", get(api_git_status))
        .route("/api/git/commit", post(api_git_commit))
        .route("/api/git/revert", post(api_git_revert))
        .route("/ws", get(ws_upgrade))
        .route("/debug/state", get(debug_state))
        .route("/debug/screenshot", get(debug_screenshot))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            require_token,
        ));
    let mut app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(api);
    if let Some(web_dir) = &state.config.web_dir {
        let index = web_dir.join("index.html");
        app = app.fallback_service(
            tower_http::services::ServeDir::new(web_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    } else {
        app = app.fallback(spa_fallback);
    }
    app.with_state(state)
}

#[derive(serde::Deserialize, Default)]
struct TokenQuery {
    token: Option<String>,
}

async fn require_token(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let presented = query
        .token
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::to_owned)
        })
        .or_else(|| {
            headers
                .get("x-cicada-token")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        });
    if presented.as_deref() == Some(state.token.as_str()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "missing or wrong token — use the URL `cicada serve` printed (?token=…)",
        )
            .into_response()
    }
}

#[derive(serde::Deserialize, Default)]
struct PipelineQuery {
    pipeline: Option<String>,
    values: Option<bool>,
    wait: Option<bool>,
    target: Option<String>,
}

fn session_for(
    state: &Arc<AppState>,
    query: &PipelineQuery,
) -> Result<Arc<Session>, Box<Response>> {
    let relative = query
        .pipeline
        .clone()
        .or_else(|| state.config.pipeline.clone())
        .ok_or_else(|| {
            Box::new(
                (
                    StatusCode::BAD_REQUEST,
                    "no pipeline: pass ?pipeline=<relative .cic path> (see /api/project)",
                )
                    .into_response(),
            )
        })?;
    open_session(state, &relative).map_err(|error| {
        let status = match error {
            ServeError::BadPipelineRef(_) | ServeError::OutsideProject { .. } => {
                StatusCode::BAD_REQUEST
            }
            ServeError::NoSuchPipeline(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Box::new((status, format!("opening {relative}: {error}")).into_response())
    })
}

async fn api_catalog(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    if query.pipeline.is_none() && state.config.pipeline.is_none() {
        // No pipeline in play at all: the stdlib catalog, explicitly.
        return axum::Json(crate::catalog::catalog_value(cicada_stdlib::registry()))
            .into_response();
    }
    // Project-aware: stdlib + this pipeline's script nodes — through the
    // same validated session path as everything else; a failure is a
    // failure (never a silent stdlib fallback).
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    axum::Json(session.catalog_value()).into_response()
}

async fn api_project(State(state): State<Arc<AppState>>) -> Response {
    let mut pipelines = Vec::new();
    let mut scripts = Vec::new();
    collect_pipelines(
        &state.config.project_dir,
        &state.config.project_dir,
        &mut pipelines,
        &mut scripts,
        0,
    );
    pipelines.sort();
    scripts.sort();
    let open: Vec<String> = state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    // The git summary shells out (two short reads): off the async runtime.
    let summary = {
        let state = Arc::clone(&state);
        match tokio::task::spawn_blocking(move || state.git.summary()).await {
            Ok(summary) => serde_json::to_value(summary).unwrap_or_default(),
            Err(error) => serde_json::json!({
                "kind": "error",
                "branch": null,
                "dirty_count": 0,
                "error": error.to_string(),
            }),
        }
    };
    axum::Json(serde_json::json!({
        "project": crate::session::display_path(&state.config.project_dir),
        "pipelines": pipelines,
        "scripts": scripts,
        "default": state.config.pipeline,
        "open": open,
        "git": summary,
        "engine": format!("cicada {}", env!("CARGO_PKG_VERSION")),
        "protocol": PROTOCOL_VERSION,
    }))
    .into_response()
}

/// Walk the project for `*.cic` pipelines and the `scripts/*.py` beside
/// them (project-relative, `/`-separated); shallow, skipping the usual
/// non-project directories.
fn collect_pipelines(
    root: &Path,
    dir: &Path,
    pipelines: &mut Vec<String>,
    scripts: &mut Vec<String>,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let in_scripts_dir = dir.file_name().is_some_and(|n| n == "scripts");
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            collect_pipelines(root, &path, pipelines, scripts, depth + 1);
        } else if let Ok(relative) = path.strip_prefix(root) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if path.extension().is_some_and(|e| e == "cic") {
                pipelines.push(relative);
            } else if in_scripts_dir && path.extension().is_some_and(|e| e == "py") {
                scripts.push(relative);
            }
        }
    }
}

async fn api_blob(
    State(state): State<Arc<AppState>>,
    AxumPath(hash): AxumPath<String>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    match tokio::task::spawn_blocking(move || session.blob_summary(&hash)).await {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(message)) => (StatusCode::NOT_FOUND, message).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// `GET /api/edit/text` → `{path, text, text_hash}`: the base an agent
/// reads before an `apply_text` (docs/13 §Undo/redo).
async fn api_edit_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    axum::Json(session.edit_text()).into_response()
}

/// `POST /api/edit/apply_text` (JSON body = the `apply_text` payload): the
/// atomic whole-file edit for agents / MCP. It applies even while a human
/// holds the writer lease — the agent acts FOR the user (docs/13), and the
/// resulting delta reaches every connected client. Errors come back as
/// `{kind, message, …details}` with the WS error kinds: 409 `stale_base`
/// (with `current_text_hash`), 422 `parse_error` (with `diagnostics`) /
/// `path_not_allowed`, 500 `io_error`, 400 for a malformed request.
async fn api_edit_apply_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
    body: axum::body::Bytes,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let request: ApplyTextRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "kind": "protocol",
                    "message": format!(
                        "unreadable apply_text body (expected {{base_text_hash, files: [{{path, text}}], label, actor}}): {error}"
                    ),
                })),
            )
                .into_response();
        }
    };
    let source = DeltaSource {
        client: None,
        intent_id: None,
        label: request.label.clone(),
    };
    match tokio::task::spawn_blocking(move || session.apply_text(&request, source)).await {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(error)) => (
            intent_status(&error),
            axum::Json(crate::session::error_body(&error)),
        )
            .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// The HTTP status for a refused edit — the same kinds the WS `error`
/// message carries.
fn intent_status(error: &IntentError) -> StatusCode {
    match error {
        IntentError::StaleBase { .. } => StatusCode::CONFLICT,
        IntentError::ParseError { .. }
        | IntentError::PathNotAllowed(_)
        | IntentError::Refused(_)
        | IntentError::Writer(_)
        | IntentError::Unknown(_)
        | IntentError::NothingToUndo(_)
        | IntentError::NothingToRedo(_) => StatusCode::UNPROCESSABLE_ENTITY,
        IntentError::Io(_) | IntentError::Persist(_) => StatusCode::INTERNAL_SERVER_ERROR,
        IntentError::Protocol(_) => StatusCode::BAD_REQUEST,
        IntentError::Lease => StatusCode::FORBIDDEN,
        IntentError::Batch { source, .. } => intent_status(source),
    }
}

async fn api_run(
    State(state): State<Arc<AppState>>,
    AxumPath(node): AxumPath<String>,
    Query(query): Query<PipelineQuery>,
    headers: HeaderMap,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    // Effectful runs write files: the write lease is required (docs/13 —
    // observers are read-only). HTTP has no socket identity, so the
    // caller names its client id; the session checks it holds the lease.
    let client = headers
        .get("x-cicada-client")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok());
    if !client.is_some_and(|id| session.is_writer(id)) {
        return (StatusCode::FORBIDDEN, ServeError::NotWriter.to_string()).into_response();
    }
    match tokio::task::spawn_blocking(move || session.run_effectful(&node)).await {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(message)) => (StatusCode::CONFLICT, message).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// The writer gate for HTTP actions that are not document edits (`POST
/// /api/run/{node}` set the pattern): the caller names its WS client id —
/// `X-Cicada-Client`, or `client` in the body — and the session checks it
/// holds the lease. `apply_text` deliberately bypasses this (an agent acts
/// FOR the user on the document); committing and reverting are git actions
/// on the project, so they need the writer.
fn writer_client(headers: &HeaderMap, body_client: Option<u32>) -> Option<u32> {
    body_client.or_else(|| {
        headers
            .get("x-cicada-client")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok())
    })
}

/// The HTTP status of a refused git route — one per [`GitErrorKind`].
fn git_status(refusal: &GitRefusal) -> StatusCode {
    match refusal.kind() {
        GitErrorKind::Protocol => StatusCode::BAD_REQUEST,
        GitErrorKind::NoSuchPipeline => StatusCode::NOT_FOUND,
        GitErrorKind::Lease => StatusCode::FORBIDDEN,
        GitErrorKind::NotARepo
        | GitErrorKind::GitNotFound
        | GitErrorKind::NothingToCommit
        | GitErrorKind::NothingToRevert
        | GitErrorKind::Untracked
        | GitErrorKind::Ignored
        | GitErrorKind::OperationInProgress => StatusCode::CONFLICT,
        GitErrorKind::EmptyMessage | GitErrorKind::PathNotAllowed => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        GitErrorKind::Locked => StatusCode::LOCKED,
        GitErrorKind::GitFailed
        | GitErrorKind::GitTimeout
        | GitErrorKind::IoError
        | GitErrorKind::ReloadFailed
        | GitErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn git_refused(refusal: &GitRefusal) -> Response {
    (git_status(refusal), axum::Json(refusal.body())).into_response()
}

/// The pipeline a git route is about, as the project-relative key every
/// session uses — validated and resolved exactly like `open_session`
/// (traversal, `.cic`, inside the project, exists) but WITHOUT opening a
/// session: status is a read about a file, and polling it for a pipeline
/// nobody has open must not start hydrating (and solving) one. Failures
/// are git-route JSON (`protocol` 400 / `no_such_pipeline` 404), not the
/// text of the other routes.
fn git_pipeline_key(state: &AppState, query: &PipelineQuery) -> Result<String, GitRefusal> {
    let relative = query
        .pipeline
        .clone()
        .or_else(|| state.config.pipeline.clone())
        .ok_or_else(|| {
            GitRefusal::Protocol(
                "no pipeline: pass ?pipeline=<relative .cic path> (see /api/project)".to_owned(),
            )
        })?;
    match resolve_in_project(&state.config.project_dir, &relative) {
        Ok((_, key)) => Ok(key),
        Err(ServeError::NoSuchPipeline(path)) => Err(GitRefusal::NoSuchPipeline(path)),
        Err(error @ (ServeError::BadPipelineRef(_) | ServeError::OutsideProject { .. })) => {
            Err(GitRefusal::Protocol(format!("{relative}: {error}")))
        }
        Err(error) => Err(GitRefusal::Internal(format!(
            "resolving `{relative}` in the project: {error}"
        ))),
    }
}

/// The OPEN session of the pipeline a writer-gated git route is about.
/// Commit and revert need the lease holder, and a lease exists only on an
/// open session — so a pipeline nobody has open is refused `lease` (with
/// the reason) rather than opened on the caller's behalf.
fn git_session(state: &AppState, query: &PipelineQuery) -> Result<Arc<Session>, GitRefusal> {
    let key = git_pipeline_key(state, query)?;
    state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
        .ok_or(GitRefusal::NoWriter(key))
}

/// A blocking git task's outcome as the route's response: the join error
/// (a panic inside) is JSON too — every git-route failure is `{kind,
/// message, …}` with the route's status code (the token middleware's 401
/// is the one exception, shared with every route).
fn git_response<T: serde::Serialize>(
    outcome: Result<Result<T, GitRefusal>, tokio::task::JoinError>,
) -> Response {
    match outcome {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(refusal)) => git_refused(&refusal),
        Err(error) => git_refused(&GitRefusal::Internal(format!(
            "the git task did not complete: {error}"
        ))),
    }
}

/// `GET /api/git/status?pipeline=` → the git state, this pipeline's node
/// markers (working tree vs HEAD), and the dirty files of its commit scope.
/// Reads only — `--no-optional-locks` on every call, so a refresh never
/// touches the project and never wakes the watcher; and no session is
/// opened for it.
async fn api_git_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    let key = match git_pipeline_key(&state, &query) {
        Ok(key) => key,
        Err(refusal) => return git_refused(&refusal),
    };
    let scope = Scope::for_pipeline(&key);
    let git_state = Arc::clone(&state);
    git_response(tokio::task::spawn_blocking(move || git_state.git.status(&scope)).await)
}

/// `POST /api/git/commit` `{message, client?}` (writer-gated): stage the
/// dirty scope files and commit exactly them, the message verbatim →
/// `{hash, short, summary, files}`. 422 `empty_message`, 409
/// `nothing_to_commit` / `not_a_repo` / `git_not_found`, 423 `locked`, 403
/// `lease`, 500 `git_failed` (with `command`, `code`, `stderr`).
async fn api_git_commit(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let session = match git_session(&state, &query) {
        Ok(session) => session,
        Err(refusal) => return git_refused(&refusal),
    };
    let request: CommitRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return git_refused(&GitRefusal::Protocol(format!(
                "unreadable commit body (expected {{message, client?}}): {error}"
            )));
        }
    };
    let client = writer_client(&headers, request.client);
    if !client.is_some_and(|id| session.is_writer(id)) {
        return git_refused(&GitRefusal::Lease);
    }
    let scope = Scope::for_pipeline(session.relative());
    let git_state = Arc::clone(&state);
    git_response(
        tokio::task::spawn_blocking(move || git_state.git.commit(&scope, &request.message)).await,
    )
}

/// `POST /api/git/revert` `{paths?, client?}` (writer-gated): `git checkout
/// HEAD --` the dirty scope files (or the given subset, validated against
/// the scope), then reload the session through the external-change path —
/// `reload_from_disk` → ONE barrier snapshot, `reason: "git revert"`. Both
/// happen under the session's write hold (`Session::hold_writes`): no
/// intent can persist between the checkout and the reload (it would
/// overwrite the restored file and turn the reload into a no-op — a
/// silently lost revert), and the watcher's later wake finds disk ==
/// memory and does nothing. → `{reverted, untracked, reloaded}`. 409
/// `untracked` for a pipeline with no HEAD version, 409
/// `nothing_to_revert`, 422 `path_not_allowed`, 500 `reload_failed` when
/// the files are back but the session could not load them.
async fn api_git_revert(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let session = match git_session(&state, &query) {
        Ok(session) => session,
        Err(refusal) => return git_refused(&refusal),
    };
    let request: RevertRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return git_refused(&GitRefusal::Protocol(format!(
                "unreadable revert body (expected {{paths?, client?}}): {error}"
            )));
        }
    };
    let client = writer_client(&headers, request.client);
    if !client.is_some_and(|id| session.is_writer(id)) {
        return git_refused(&GitRefusal::Lease);
    }
    let scope = Scope::for_pipeline(session.relative());
    let git_state = Arc::clone(&state);
    git_response(
        tokio::task::spawn_blocking(move || {
            // The hold spans the checkout and the reload; a refused revert
            // drops it on the way out.
            let hold = session.hold_writes();
            let reverted = git_state.git.revert(&scope, request.paths.as_deref())?;
            let reloaded = session
                .reload_from_disk_held(hold, "git revert", reverted.touched_scripts)
                .map_err(|error| GitRefusal::Reload(error.to_string()))?;
            Ok::<_, GitRefusal>(reverted.into_response(reloaded))
        })
        .await,
    )
}

async fn debug_state(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let with_values = query.values.unwrap_or(false);
    let wait = query.wait.unwrap_or(false);
    match tokio::task::spawn_blocking(move || {
        if wait {
            session.wait_idle();
        }
        session.debug_state(with_values)
    })
    .await
    {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn debug_screenshot(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let target = query.target.unwrap_or_else(|| "viewport".to_owned());
    if target != "viewport" {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "only target=viewport can be rendered by the connected client; for the whole page use \
             Playwright's page.screenshot()",
        )
            .into_response();
    }
    let Some(rx) = session.request_screenshot(&target) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no client connected to render the viewport — open the app (or the Playwright page) first",
        )
            .into_response();
    };
    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(Ok(png))) => ([(header::CONTENT_TYPE, "image/png")], png).into_response(),
        Ok(Ok(Err(message))) => (
            StatusCode::BAD_GATEWAY,
            format!("client could not render: {message}"),
        )
            .into_response(),
        Ok(Err(_)) => {
            (StatusCode::BAD_GATEWAY, "client went away before answering").into_response()
        }
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            "client did not answer within 15 s",
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------- ws --

/// Where the write task puts a message: the WebSocket's sink in `serve`,
/// a recording sink in tests. One message at a time — `send` resolves when
/// the transport accepted it, which is the backpressure the lanes' priority
/// rides on (at most one display frame is ever "in the sink" ahead of a
/// control text).
pub(crate) trait WireSink {
    /// Hand one message to the transport; `Err` means the socket is gone.
    fn put(&mut self, message: Message) -> impl Future<Output = Result<(), ()>> + Send;
}

impl<S> WireSink for S
where
    S: futures_util::Sink<Message> + Unpin + Send,
{
    async fn put(&mut self, message: Message) -> Result<(), ()> {
        self.send(message).await.map_err(|_| ())
    }
}

/// The per-client write task (docs/13 §Two lanes, one socket): drain the
/// control lane first — `biased`, so whenever both lanes hold a message
/// the control text goes — and the display lane otherwise, in its FIFO
/// order. Control texts are small and ≤ 10 Hz (statuses are coalesced), so
/// the display lane is never starved in practice; a display frame in
/// flight is never pre-empted (the sink takes whole messages), which bounds
/// a control text's wait to one frame. Returns when both lanes are closed
/// or the sink refuses a message.
pub(crate) async fn pump_lanes<S: WireSink>(
    mut control: UnboundedReceiver<Outgoing>,
    mut display: UnboundedReceiver<Outgoing>,
    sink: &mut S,
) {
    let mut control_open = true;
    let mut display_open = true;
    while control_open || display_open {
        let next = tokio::select! {
            biased;
            message = control.recv(), if control_open => {
                let Some(message) = message else {
                    control_open = false;
                    continue;
                };
                message
            },
            message = display.recv(), if display_open => {
                let Some(message) = message else {
                    display_open = false;
                    continue;
                };
                message
            },
        };
        let outgoing = match next {
            Outgoing::Text(text) => Message::Text(text.into()),
            Outgoing::Binary(bytes) => Message::Binary(bytes),
        };
        if sink.put(outgoing).await.is_err() {
            break;
        }
    }
}

async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PipelineQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let session = match session_for(&state, &query) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    ws.on_upgrade(move |socket| client_loop(socket, session))
}

#[allow(clippy::too_many_lines)] // handshake, then the one receive loop
async fn client_loop(socket: WebSocket, session: Arc<Session>) {
    let (mut sink, mut stream) = socket.split();

    // Handshake FIRST (docs/13): the client's `hello` carries its protocol
    // version; a mismatch is refused — error + close, no lease taken, no
    // hydration — never guessed around. Anything but a hello first is a
    // protocol error too.
    let first = tokio::time::timeout(Duration::from_secs(10), stream.next()).await;
    let handshake = match first {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<IntentEnvelope>(&text) {
            Ok(envelope) => match envelope.message {
                crate::protocol::ClientMessage::Hello { v }
                    if v == PROTOCOL_VERSION && envelope.v == PROTOCOL_VERSION =>
                {
                    Ok(())
                }
                crate::protocol::ClientMessage::Hello { v } => Err(format!(
                    "protocol version {} — this server speaks {PROTOCOL_VERSION}; reload the app",
                    if envelope.v == PROTOCOL_VERSION {
                        v
                    } else {
                        envelope.v
                    }
                )),
                other => Err(format!(
                    "the first message must be `hello`, got `{}`",
                    serde_json::to_value(&other)
                        .ok()
                        .and_then(|v| v["type"].as_str().map(str::to_owned))
                        .unwrap_or_default()
                )),
            },
            Err(error) => Err(format!("unreadable hello: {error}")),
        },
        Ok(Some(Ok(_))) => Err("the first message must be a JSON `hello`".to_owned()),
        Ok(Some(Err(error))) => Err(format!("socket error before hello: {error}")),
        Ok(None) => Err("socket closed before hello".to_owned()),
        Err(_) => Err("no hello within 10 s".to_owned()),
    };
    if let Err(message) = handshake {
        let refusal = encode(
            0,
            &ServerMessage::Error {
                intent_id: None,
                kind: "protocol".to_owned(),
                message,
                details: serde_json::Map::new(),
            },
        );
        let _ = sink.send(Message::Text(refusal.into())).await;
        let _ = sink.send(Message::Close(None)).await;
        return;
    }

    // Two lanes to one socket (docs/13 §Two lanes, one socket): the
    // control plane and the display plane each get a channel, and the write
    // task drains control first — `hello`, `snapshot`, every delta, status
    // and `preview_policy` go out ahead of whatever display restream is
    // still queued (the wall's is ~350 MB). `tx` here is this socket's own
    // control-lane handle for the protocol errors below.
    let (tx, control_rx) = unbounded_channel::<Outgoing>();
    let (display_tx, display_rx) = unbounded_channel::<Outgoing>();
    let (id, role) = session.connect(ClientLanes {
        control: tx.clone(),
        display: display_tx,
    });
    let _ = tx.send(Outgoing::Text(session.hello(id, role)));
    let _ = tx.send(Outgoing::Text(session.snapshot(false, "initial")));
    session.restream_display(id);

    let send_task = tokio::spawn(async move {
        pump_lanes(control_rx, display_rx, &mut sink).await;
    });

    // Intents are handled IN ORDER on one blocking thread per client.
    let (intent_tx, intent_rx) =
        std::sync::mpsc::channel::<(Option<String>, crate::protocol::ClientMessage)>();
    let handler_session = Arc::clone(&session);
    let handler = tokio::task::spawn_blocking(move || {
        while let Ok((intent_id, message)) = intent_rx.recv() {
            handler_session.handle(id, intent_id, message);
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Text(text) => match serde_json::from_str::<IntentEnvelope>(&text) {
                Ok(envelope) if envelope.v != PROTOCOL_VERSION => {
                    let _ = tx.send(Outgoing::Text(encode(
                        0,
                        &ServerMessage::Error {
                            intent_id: envelope.id,
                            kind: "protocol".to_owned(),
                            message: format!(
                                "protocol version {} — this server speaks {PROTOCOL_VERSION}; reload the app",
                                envelope.v
                            ),
                            details: serde_json::Map::new(),
                        },
                    )));
                }
                // Esc must never queue behind a backlog of intents (docs/12
                // "Esc always works"): cancel goes straight to the loop.
                Ok(IntentEnvelope {
                    message: crate::protocol::ClientMessage::Cancel {},
                    ..
                }) => {
                    if session.is_writer(id) {
                        session.cancel();
                    } else {
                        let _ = tx.send(Outgoing::Text(encode(
                            0,
                            &ServerMessage::Error {
                                intent_id: None,
                                kind: "lease".to_owned(),
                                message: "read-only observer — take the lease to cancel".to_owned(),
                                details: serde_json::Map::new(),
                            },
                        )));
                    }
                }
                Ok(envelope) => {
                    if intent_tx.send((envelope.id, envelope.message)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Outgoing::Text(encode(
                        0,
                        &ServerMessage::Error {
                            intent_id: None,
                            kind: "protocol".to_owned(),
                            message: format!("unreadable intent: {error}"),
                            details: serde_json::Map::new(),
                        },
                    )));
                }
            },
            Message::Binary(_) => {
                let _ = tx.send(Outgoing::Text(encode(
                    0,
                    &ServerMessage::Error {
                        intent_id: None,
                        kind: "protocol".to_owned(),
                        message:
                            "clients send JSON intents only; binary frames flow server → client"
                                .to_owned(),
                        details: serde_json::Map::new(),
                    },
                )));
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
    drop(intent_tx);
    let _ = handler.await;
    session.disconnect(id);
    send_task.abort();
    if role == Role::Writer {
        let grace = Arc::clone(&session);
        tokio::spawn(async move {
            tokio::time::sleep(LEASE_GRACE).await;
            grace.transfer_lease_if_free();
        });
    }
}

// --------------------------------------------------------------- SPA --

#[cfg(feature = "embed")]
#[derive(rust_embed::Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

#[cfg(feature = "embed")]
async fn spa_fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };
    let file = Assets::get(candidate).or_else(|| Assets::get("index.html"));
    match file {
        Some(content) => {
            let mime = mime_guess::from_path(if Assets::get(candidate).is_some() {
                candidate
            } else {
                "index.html"
            })
            .first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_owned())],
                axum::body::Bytes::copy_from_slice(content.data.as_ref()),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "the embedded SPA has no index.html — was `npm run build` run before `--features embed`?").into_response(),
    }
}

#[cfg(not(feature = "embed"))]
async fn spa_fallback(State(state): State<Arc<AppState>>, uri: Uri) -> Response {
    let _ = uri;
    let mut pipelines = Vec::new();
    let mut scripts = Vec::new();
    collect_pipelines(
        &state.config.project_dir,
        &state.config.project_dir,
        &mut pipelines,
        &mut scripts,
        0,
    );
    pipelines.sort();
    // No token here: this page is served without one, so it must never
    // reveal it (the token lives only in the URL `cicada serve` printed).
    let mut links = String::new();
    for p in &pipelines {
        use std::fmt::Write as _;
        let _ = write!(
            links,
            "<li><code>{p}</code> — <code>/debug/state?pipeline={p}&amp;token=…</code></li>"
        );
    }
    Html(format!(
        "<!doctype html><meta charset=utf-8><title>cicada serve</title>\
         <body style=\"font:14px system-ui;max-width:52em;margin:3em auto;line-height:1.5\">\
         <h1>cicada serve — API only</h1>\
         <p>This build has <b>no embedded SPA</b> and no <code>--web-dir</code>. Three honest ways to get the app:</p>\
         <ol>\
         <li><b>Dev</b>: <code>cd web &amp;&amp; npm run dev</code>, then open the Vite URL it prints with this page's query string \
         (<code>?token=…&amp;pipeline=…</code>) — Vite proxies <code>/api</code>, <code>/ws</code>, <code>/debug</code> here.</li>\
         <li><b>Built SPA on disk</b>: <code>cd web &amp;&amp; npm run build</code>, then <code>cicada serve … --web-dir web/dist</code>.</li>\
         <li><b>Embedded (release shape)</b>: build the SPA, then <code>cargo build -p cicada-cli --features embed</code>.</li>\
         </ol>\
         <p>Project: <code>{project}</code>. Pipelines:</p><ul>{links}</ul>\
         <p>Engine endpoints work now: <code>/api/catalog</code>, <code>/api/project</code>, <code>/ws</code>, <code>/debug/state</code> (all need <code>?token=</code>).</p>\
         </body>",
        project = state.config.project_dir.to_string_lossy(),
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ClientMessage;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::Semaphore;
    use tokio::sync::mpsc::UnboundedSender;

    /// What the mock client saw, in wire order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Seen {
        /// A JSON text: its `type`.
        Text(String),
        /// A binary frame: its length.
        Binary(usize),
    }

    /// A recording sink that models the wire: it accepts one message per
    /// permit, so the test decides exactly when the link has room — no
    /// sleeps, no wall clock — and records what went out, in which order.
    /// A message awaiting a permit is "in flight": handed to the transport,
    /// not yet on the wire — exactly a WebSocket sink's `send` blocked on a
    /// full socket buffer.
    struct GatedRecorder {
        room: Arc<Semaphore>,
        seen: Arc<StdMutex<Vec<Seen>>>,
        sent: UnboundedSender<Seen>,
    }

    impl WireSink for GatedRecorder {
        async fn put(&mut self, message: Message) -> Result<(), ()> {
            let permit = self.room.acquire().await.map_err(|_| ())?;
            permit.forget();
            let entry = match message {
                Message::Text(text) => Seen::Text(
                    serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| v["type"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "<not json>".to_owned()),
                ),
                Message::Binary(bytes) => Seen::Binary(bytes.len()),
                Message::Ping(_) | Message::Pong(_) | Message::Close(_) => return Err(()),
            };
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(entry.clone());
            let _ = self.sent.send(entry);
            Ok(())
        }
    }

    /// The mock client: the recorder plus the test's side of it.
    struct Wire {
        room: Arc<Semaphore>,
        seen: Arc<StdMutex<Vec<Seen>>>,
        sent: tokio::sync::mpsc::UnboundedReceiver<Seen>,
    }

    impl Wire {
        fn new() -> (Self, GatedRecorder) {
            let room = Arc::new(Semaphore::new(0));
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                Self {
                    room: Arc::clone(&room),
                    seen: Arc::clone(&seen),
                    sent: rx,
                },
                GatedRecorder {
                    room,
                    seen,
                    sent: tx,
                },
            )
        }

        /// Let ONE message onto the wire and return it.
        async fn release_one(&mut self) -> Seen {
            self.room.add_permits(1);
            self.sent.recv().await.expect("the pump is alive")
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    fn text(type_tag: &str) -> Outgoing {
        Outgoing::Text(format!(
            r#"{{"v":1,"seq":0,"type":"{type_tag}","payload":{{}}}}"#
        ))
    }

    fn binary(len: usize) -> Outgoing {
        Outgoing::Binary(bytes::Bytes::from(vec![0_u8; len]))
    }

    #[tokio::test]
    async fn the_pump_prefers_the_control_lane_and_keeps_the_display_lane_fifo() {
        let (control_tx, control_rx) = unbounded_channel::<Outgoing>();
        let (display_tx, display_rx) = unbounded_channel::<Outgoing>();
        let (mut wire, mut recorder) = Wire::new();
        let pump = tokio::spawn(async move {
            pump_lanes(control_rx, display_rx, &mut recorder).await;
        });

        // Both lanes pre-filled: control goes first, display keeps its order.
        for len in [1, 2, 3] {
            display_tx.send(binary(len)).unwrap();
        }
        control_tx.send(text("status")).unwrap();
        assert_eq!(wire.release_one().await, Seen::Text("status".into()));
        assert_eq!(wire.release_one().await, Seen::Binary(1));
        // The pump now holds frame 2 in flight; a control text enqueued here
        // waits for exactly that one frame, then overtakes frame 3.
        control_tx.send(text("delta")).unwrap();
        assert_eq!(wire.release_one().await, Seen::Binary(2));
        assert_eq!(wire.release_one().await, Seen::Text("delta".into()));
        assert_eq!(wire.release_one().await, Seen::Binary(3));

        // Closing one lane does not end the pump while the other still
        // speaks; closing both does.
        drop(control_tx);
        display_tx.send(binary(4)).unwrap();
        assert_eq!(wire.release_one().await, Seen::Binary(4));
        drop(display_tx);
        pump.await.unwrap();
        assert_eq!(
            wire.seen(),
            vec![
                Seen::Text("status".into()),
                Seen::Binary(1),
                Seen::Binary(2),
                Seen::Text("delta".into()),
                Seen::Binary(3),
                Seen::Binary(4),
            ]
        );
    }

    /// The follow-up that made the lanes (docs/17, measured 2026-08-20): a
    /// fresh page receives the whole display set — the wall: ~350 MB — and
    /// every text queued ~26 s behind it. The modeled link below is that
    /// measurement's effective rate; the status cadence is docs/13's
    /// ≤ 10 Hz. The synthetic restream's contents are zeros: the wire, the
    /// pump and the sink never look inside a frame.
    const MODELED_LINK_BYTES_PER_S: f64 = 350.0e6 / 26.0;
    const STATUS_CADENCE_MS: f64 = 100.0;
    const FRAME_BYTES: usize = 1 << 20;
    const RESTREAM_FRAMES: usize = 320;

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one story: join, a 320 MiB restream, an edit's texts overtake it
    async fn a_control_text_reaches_the_client_within_the_status_cadence_behind_a_restream() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = dir.path().join("p.cic");
        std::fs::write(
            &pipeline,
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n",
        )
        .unwrap();
        let session = Session::open(SessionConfig {
            project_dir: dir.path().to_owned(),
            pipeline,
            cache_dir: Some(dir.path().join("cache")),
            threads: 2,
            project: ProjectConfig::default(),
            op_clock: None,
        })
        .unwrap();
        session.wait_idle();

        // The join, exactly as `client_loop` does it.
        let (control_tx, control_rx) = unbounded_channel::<Outgoing>();
        let (display_tx, display_rx) = unbounded_channel::<Outgoing>();
        let (id, role) = session.connect(ClientLanes {
            control: control_tx.clone(),
            display: display_tx.clone(),
        });
        assert_eq!(role, Role::Writer);
        control_tx
            .send(Outgoing::Text(session.hello(id, role)))
            .unwrap();
        control_tx
            .send(Outgoing::Text(session.snapshot(false, "initial")))
            .unwrap();
        session.restream_display(id);
        // … plus the wall-sized restream the box cannot produce: 320 MiB of
        // frames on the display lane, where `restream_display` puts its own.
        for _ in 0..RESTREAM_FRAMES {
            display_tx.send(binary(FRAME_BYTES)).unwrap();
        }
        let (mut wire, mut recorder) = Wire::new();
        let pump = tokio::spawn(async move {
            pump_lanes(control_rx, display_rx, &mut recorder).await;
        });

        // The join's texts lead; the restream header and the box's frames
        // follow; then the big restream starts.
        assert_eq!(wire.release_one().await, Seen::Text("hello".into()));
        assert_eq!(wire.release_one().await, Seen::Text("snapshot".into()));
        assert_eq!(wire.release_one().await, Seen::Text("display_reset".into()));
        let mut synthetic_sent = 0;
        while synthetic_sent < 3 {
            match wire.release_one().await {
                Seen::Binary(len) if len == FRAME_BYTES => synthetic_sent += 1,
                Seen::Binary(_) => {} // the box's own mesh frame
                Seen::Text(kind) => {
                    panic!("a text amid the restream before any was queued: {kind}")
                }
            }
        }
        let restream_remaining = (RESTREAM_FRAMES - synthetic_sent) * FRAME_BYTES;

        // The edit: a slider drag tick. Its statuses are control texts; the
        // repaint it earns is a frame, behind the restream.
        session.handle(
            id,
            Some("tick".into()),
            ClientMessage::ParamPreview {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();

        // At most the frame in flight precedes the first text; every text
        // the tick produced precedes the rest of the restream; the tick's
        // frame comes after it.
        let mut bytes_ahead = 0_usize;
        let mut first_text = None;
        for position in 0.. {
            match wire.release_one().await {
                Seen::Binary(len) => {
                    bytes_ahead += len;
                    synthetic_sent += 1;
                }
                Seen::Text(kind) => {
                    first_text = Some((position, kind));
                    break;
                }
            }
            assert!(
                position < 2,
                "the text is still queued behind {bytes_ahead} bytes of restream"
            );
        }
        let (position, kind) = first_text.unwrap();
        assert_eq!(kind, "status", "the tick's first text is its status");
        assert!(position <= 1, "at most one frame in flight ahead of it");
        assert!(bytes_ahead <= FRAME_BYTES);
        let modeled_ms = |bytes: usize| {
            f64::from(u32::try_from(bytes).expect("the restream fits u32 bytes"))
                / MODELED_LINK_BYTES_PER_S
                * 1000.0
        };
        let text_delay_ms = modeled_ms(bytes_ahead);
        let backlog_delay_ms = modeled_ms(restream_remaining);
        assert!(
            text_delay_ms <= STATUS_CADENCE_MS,
            "text behind {bytes_ahead} bytes = {text_delay_ms:.1} ms on the modeled link"
        );
        // Not vacuous: without the lanes the same text would have waited
        // for the whole remaining restream — far past the cadence.
        assert!(
            backlog_delay_ms > 100.0 * STATUS_CADENCE_MS,
            "the synthetic restream is too small to prove anything: {backlog_delay_ms:.0} ms"
        );

        // Every further control text drains before the next restream frame.
        let mut texts_after = Vec::new();
        loop {
            match wire.release_one().await {
                Seen::Text(kind) => texts_after.push(kind),
                Seen::Binary(len) => {
                    assert_eq!(len, FRAME_BYTES, "the restream resumes where it stopped");
                    synthetic_sent += 1;
                    break;
                }
            }
        }
        assert!(
            texts_after.iter().all(|k| k == "status"),
            "only statuses were pending: {texts_after:?}"
        );
        // The remaining restream, FIFO (a late coalesced status may still
        // overtake it), then the tick's own repaint frame.
        while synthetic_sent < RESTREAM_FRAMES {
            match wire.release_one().await {
                Seen::Binary(len) => {
                    assert_eq!(len, FRAME_BYTES);
                    synthetic_sent += 1;
                }
                Seen::Text(kind) => assert_eq!(kind, "status"),
            }
        }
        loop {
            match wire.release_one().await {
                Seen::Binary(len) => {
                    assert!(len < FRAME_BYTES, "the box's preview frame");
                    break;
                }
                Seen::Text(kind) => assert_eq!(kind, "status"),
            }
        }

        drop(control_tx);
        drop(display_tx);
        drop(session);
        pump.await.unwrap();
        let seen = wire.seen();
        assert_eq!(
            seen.iter()
                .filter(|s| **s == Seen::Binary(FRAME_BYTES))
                .count(),
            RESTREAM_FRAMES,
            "every restream frame went out exactly once"
        );
    }
}
