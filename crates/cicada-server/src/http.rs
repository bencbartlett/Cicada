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
use tokio::sync::mpsc::unbounded_channel;

use crate::protocol::{IntentEnvelope, PROTOCOL_VERSION, Role, ServerMessage, encode};
use crate::session::{Outgoing, Session, SessionConfig};

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
    collect_pipelines(
        &state.config.project_dir,
        &state.config.project_dir,
        &mut pipelines,
        0,
    );
    pipelines.sort();
    let open: Vec<String> = state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    axum::Json(serde_json::json!({
        "project": crate::session::display_path(&state.config.project_dir),
        "pipelines": pipelines,
        "default": state.config.pipeline,
        "open": open,
        "engine": format!("cicada {}", env!("CARGO_PKG_VERSION")),
        "protocol": PROTOCOL_VERSION,
    }))
    .into_response()
}

fn collect_pipelines(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            collect_pipelines(root, &path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e == "cic")
            && let Ok(relative) = path.strip_prefix(root)
        {
            out.push(relative.to_string_lossy().replace('\\', "/"));
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
            },
        );
        let _ = sink.send(Message::Text(refusal.into())).await;
        let _ = sink.send(Message::Close(None)).await;
        return;
    }

    let (tx, mut rx) = unbounded_channel::<Outgoing>();
    let (id, role) = session.connect(tx.clone());
    let _ = tx.send(Outgoing::Text(session.hello(id, role)));
    let _ = tx.send(Outgoing::Text(session.snapshot(false, "initial")));
    session.restream_display(id);

    let send_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let outgoing = match message {
                Outgoing::Text(text) => Message::Text(text.into()),
                Outgoing::Binary(bytes) => Message::Binary(bytes),
            };
            if sink.send(outgoing).await.is_err() {
                break;
            }
        }
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
    collect_pipelines(
        &state.config.project_dir,
        &state.config.project_dir,
        &mut pipelines,
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
