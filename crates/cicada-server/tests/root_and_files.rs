//! The root and the file list (docs/17 wave 4 O1; docs/13 §Projects,
//! pipelines, sessions, §HTTP surface `GET /api/files`) over a REAL served
//! root in a temp dir: a root with no default pipeline opens nothing and
//! serves its listing one directory at a time in the documented shape;
//! every escape is refused with the typed body and nothing above the root
//! is ever named; an unreadable directory is `io_error`; a pipeline in a
//! SUBDIRECTORY of the root opens by its root-relative name. No network
//! beyond loopback; the store lives in the temp dir.
//!
//! Two fixtures need the OS's co-operation and SKIP LOUDLY (a printed
//! reason, those assertions left out — never `#[ignore]`, never a silent
//! pass) when it is refused: a directory link (Windows without Developer
//! Mode refuses symlinks; a junction is tried then), and a directory the
//! process may not read (root on Unix reads everything).

// Tests are exempt from the expect/unwrap denial (clippy.toml), but the
// exemption recognizes #[test] fns only — not helpers in integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cicada_server::{ServeConfig, ServerHandle, serve};

const PIPELINE: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n";

/// Minimal HTTP/1.1 GET (loopback only): `(status, body)`.
fn http_get(addr: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let auth = token.map_or(String::new(), |t| format!("X-Cicada-Token: {t}\r\n"));
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    assert!(
        !head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "unexpected chunked body:\n{head}"
    );
    (status, body.to_owned())
}

/// `GET` with the token; the body parsed when it is JSON, the text otherwise.
async fn get(addr: SocketAddr, path: &str) -> (u16, serde_json::Value) {
    let path = path.to_owned();
    let (status, body) = tokio::task::spawn_blocking(move || http_get(addr, &path, Some("t")))
        .await
        .unwrap();
    let value = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, value)
}

/// Percent-encode a query value (everything but the unreserved set).
fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").unwrap();
        }
    }
    out
}

/// `<tmp>/root` — every kind of entry the listing rules speak about — plus
/// `<tmp>/elsewhere/secret.cic` BEYOND the root and the store beside it,
/// so neither can appear in a listing by accident.
struct Fixture {
    /// Owns the whole temp tree (the root, the outside, the store); its
    /// name is what no listing may ever contain.
    dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    cache: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    for sub in [
        "sub/inner",
        "Zeta",
        "alpha",
        ".hidden",
        "node_modules",
        "target",
    ] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    for (file, text) in [
        ("top.cic", PIPELINE),
        ("Beta.cic", PIPELINE),
        ("alpha.cic", PIPELINE),
        ("README.md", "not a pipeline\n"),
        ("sub/p.cic", PIPELINE),
        ("sub/inner/q.cic", PIPELINE),
    ] {
        std::fs::write(root.join(file), text).unwrap();
    }
    // Named so that no server prose can contain it by accident (the sweep
    // below greps responses for it).
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.cic"), PIPELINE).unwrap();
    let cache = dir.path().join("cache");
    Fixture {
        dir,
        root,
        outside,
        cache,
    }
}

/// Serve the fixture's root with NO default pipeline on an ephemeral port.
async fn start(fx: &Fixture) -> ServerHandle {
    let mut config = ServeConfig::new(fx.root.clone());
    config.port = 0;
    config.token = Some("t".to_owned());
    config.cache_dir = Some(fx.cache.clone());
    config.threads = 2;
    serve(config).await.expect("serve")
}

fn entries(listing: &serde_json::Value) -> Vec<(String, String)> {
    listing["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("no entries in {listing}"))
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, k)| ((*n).to_owned(), (*k).to_owned()))
        .collect()
}

/// A directory link: a symlink, or on Windows — where symlinks need
/// Developer Mode — a junction (`mklink /J`, no privilege). `Err` with the
/// reason when the OS refuses both.
fn link_dir(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map_err(|e| format!("symlink: {e}"))
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => Ok(()),
            Err(first) => {
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                match status {
                    Ok(status) if status.success() => Ok(()),
                    other => Err(format!("symlink_dir: {first}; mklink /J: {other:?}")),
                }
            }
        }
    }
}

/// A directory the process may not list, restored on drop. `None` when
/// the denial cannot be arranged or does not take (root on Unix).
struct Denied(PathBuf);

fn deny_listing(dir: &Path) -> Option<Denied> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).ok()?;
    }
    #[cfg(windows)]
    {
        // `*S-1-1-0` is Everyone by SID (locale-independent); `RD` is
        // "read data / list directory".
        let status = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
    }
    let denied = Denied(dir.to_owned());
    if std::fs::read_dir(dir).is_ok() {
        // The denial did not take (a privileged user): the guard restores.
        drop(denied);
        return None;
    }
    Some(denied)
}

impl Drop for Denied {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_root_without_a_pipeline_opens_nothing_and_lists_one_directory_at_a_time() {
    let fx = fixture();
    let handle = start(&fx).await;
    let addr = handle.addr;
    assert_eq!(
        handle.url(),
        format!("http://{addr}/?token=t"),
        "no pipeline → no `&pipeline=` in the URL: the page with only a token is the picker"
    );
    let (status, project) = get(addr, "/api/project").await;
    assert_eq!(status, 200, "{project}");
    assert_eq!(project["default"], serde_json::Value::Null);
    assert_eq!(project["open"], serde_json::json!([]), "nothing opened");
    let (status, body) = get(addr, "/debug/state").await;
    assert_eq!(status, 400, "a pipeline-less read says so: {body}");

    // Token-gated like every /api route.
    let (status, _) = tokio::task::spawn_blocking(move || http_get(addr, "/api/files", None))
        .await
        .unwrap();
    assert_eq!(status, 401);

    // ---- The root: the documented shape, exactly.
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200, "{listing}");
    assert_eq!(listing["root"], "root", "the root's own name, not its path");
    assert_eq!(listing["dir"], "");
    assert_eq!(listing["parent"], serde_json::Value::Null);
    assert_eq!(
        listing.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["dir", "entries", "parent", "root"],
        "exactly root, dir, parent, entries"
    );
    assert_eq!(
        entries(&listing),
        pairs(&[
            ("alpha", "dir"),
            ("sub", "dir"),
            ("Zeta", "dir"),
            ("alpha.cic", "pipeline"),
            ("Beta.cic", "pipeline"),
            ("top.cic", "pipeline"),
        ]),
        "directories first, case-insensitive order; .hidden, node_modules, target, README.md absent"
    );
    for entry in listing["entries"].as_array().unwrap() {
        assert_eq!(
            entry.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["kind", "modified_ms", "name"],
            "exactly name, kind, modified_ms: {entry}"
        );
        assert!(
            entry["modified_ms"].as_i64().unwrap() > 1_600_000_000_000,
            "a real file time: {entry}"
        );
    }
    // The same request spelled with an explicit empty dir.
    let (status, again) = get(addr, "/api/files?dir=").await;
    assert_eq!(status, 200);
    assert_eq!(again, listing);

    // ---- One directory down, and two, under every spelling the
    // normaliser accepts.
    let (status, sub) = get(addr, "/api/files?dir=sub").await;
    assert_eq!(status, 200, "{sub}");
    assert_eq!(
        (sub["dir"].as_str(), sub["parent"].as_str()),
        (Some("sub"), Some(""))
    );
    assert_eq!(
        entries(&sub),
        pairs(&[("inner", "dir"), ("p.cic", "pipeline")])
    );
    for spelling in ["sub/inner", "sub//inner/", "./sub/./inner"] {
        let (status, inner) = get(addr, &format!("/api/files?dir={}", urlencode(spelling))).await;
        assert_eq!(status, 200, "{spelling}: {inner}");
        assert_eq!(
            (inner["dir"].as_str(), inner["parent"].as_str()),
            (Some("sub/inner"), Some("sub")),
            "{spelling}"
        );
        assert_eq!(entries(&inner), pairs(&[("q.cic", "pipeline")]));
    }

    // ---- Listing opened nothing; the subdirectory pipeline opens by its
    // root-relative name and is keyed by it.
    assert_eq!(
        get(addr, "/api/project").await.1["open"],
        serde_json::json!([])
    );
    let (status, state) = get(addr, "/debug/state?pipeline=sub/p.cic&wait=true").await;
    assert_eq!(status, 200, "{state}");
    assert_eq!(
        get(addr, "/api/project").await.1["open"],
        serde_json::json!(["sub/p.cic"])
    );
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_file_list_refuses_every_escape_and_names_nothing_above_the_root() {
    let fx = fixture();
    // A link that stays under the root, one that leaves it, and one that
    // dangles — when the OS lets a test make them.
    let links = match (
        link_dir(&fx.root.join("sub"), &fx.root.join("alias")),
        link_dir(&fx.outside, &fx.root.join("escape")),
        link_dir(&fx.root.join("never-made"), &fx.root.join("dangling")),
    ) {
        (Ok(()), Ok(()), Ok(())) => true,
        (inside, outside, dangling) => {
            eprintln!(
                "SKIPPING the directory-link assertions: this OS/user cannot create directory \
                 links here (inside: {inside:?}; outside: {outside:?}; dangling: {dangling:?})"
            );
            false
        }
    };
    let handle = start(&fx).await;
    let addr = handle.addr;
    let temp_name = fx
        .dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut bodies = Vec::new();

    // ---- Lexical escapes: refused before the file system is consulted.
    for dir in [
        "..",
        "../",
        "a/..",
        "a/../..",
        "../elsewhere",
        "../elsewhere/secret.cic",
        "/etc",
        "/",
        "//server/share",
        "C:/Windows",
        "C:\\Windows",
        "a\\b",
        "a:b",
        "a\0b",
    ] {
        let (status, body) = get(addr, &format!("/api/files?dir={}", urlencode(dir))).await;
        assert_eq!(status, 400, "{dir:?}: {body}");
        assert_eq!(body["kind"], "path_not_allowed", "{dir:?}: {body}");
        assert_eq!(body["path"], dir, "{dir:?}: {body}");
        assert!(
            body["message"].as_str().is_some_and(|m| !m.is_empty()),
            "{dir:?}: {body}"
        );
        // A refusal echoes the REQUEST (`path`, and the message quotes
        // it): the client's own words are not a disclosure, so the sweep
        // below skips the requests that named the outside themselves.
        if !dir.contains("elsewhere") {
            bodies.push(body.to_string());
        }
    }

    // ---- Not found: a missing directory, and a file.
    for (dir, words) in [
        ("nowhere", "no directory"),
        ("sub/p.cic", "not a directory"),
    ] {
        let (status, body) = get(addr, &format!("/api/files?dir={}", urlencode(dir))).await;
        assert_eq!(status, 404, "{dir:?}: {body}");
        assert_eq!(body["kind"], "not_found", "{dir:?}: {body}");
        assert_eq!(body["path"], dir);
        assert!(
            body["message"].as_str().unwrap().contains(words),
            "{dir:?}: {body}"
        );
        bodies.push(body.to_string());
    }

    // ---- Links.
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200);
    bodies.push(listing.to_string());
    if links {
        let names: Vec<String> = entries(&listing).into_iter().map(|(n, _)| n).collect();
        assert!(
            names.contains(&"alias".to_owned()),
            "a link staying under the root is a directory of it: {names:?}"
        );
        assert!(
            !names.contains(&"escape".to_owned()),
            "a link leaving the root is not listed: {names:?}"
        );
        assert!(
            !names.contains(&"dangling".to_owned()),
            "a dangling link is not listed: {names:?}"
        );
        // Entering the dangling link: nothing is there.
        let (status, body) = get(addr, "/api/files?dir=dangling").await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["kind"], "not_found", "{body}");
        bodies.push(body.to_string());
        let (status, alias) = get(addr, "/api/files?dir=alias").await;
        assert_eq!(status, 200, "{alias}");
        assert_eq!(
            (alias["dir"].as_str(), alias["parent"].as_str()),
            (Some("alias"), Some("")),
            "the listed dir is the request, not the link's target"
        );
        assert_eq!(
            entries(&alias),
            pairs(&[("inner", "dir"), ("p.cic", "pipeline")])
        );
        let (status, body) = get(addr, "/api/files?dir=escape").await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["kind"], "path_not_allowed");
        assert_eq!(body["path"], "escape");
        bodies.push(body.to_string());
        let (status, body) = get(addr, "/api/files?dir=escape/").await;
        assert_eq!(status, 400, "{body}");
        bodies.push(body.to_string());
        // A pipeline reference through the link is refused too (the
        // pipeline routes' own, pre-existing text — not the listing's).
        let (status, body) = get(addr, "/debug/state?pipeline=escape/secret.cic").await;
        assert_eq!(status, 400, "{body}");
    }

    // ---- No LISTING response ever named anything above the root: not
    // the temp dir, not the directory beyond the root, not the file in it
    // — not even the refusal of the link that points there.
    assert!(
        bodies.len() >= 13,
        "the sweep covers every listing response"
    );
    for body in &bodies {
        assert!(
            !body.contains(&temp_name) && !body.contains("elsewhere") && !body.contains("secret"),
            "a response named something above the root: {body}"
        );
    }
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreadable_directory_is_io_error_and_still_a_directory_of_its_parent() {
    let fx = fixture();
    let locked = fx.root.join("locked");
    std::fs::create_dir(&locked).unwrap();
    let Some(denied) = deny_listing(&locked) else {
        eprintln!(
            "SKIPPING: this OS/user cannot make a directory unreadable to itself here \
             (root on Unix reads everything)"
        );
        return;
    };
    let handle = start(&fx).await;
    let addr = handle.addr;
    let (status, body) = get(addr, "/api/files?dir=locked").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["kind"], "io_error", "{body}");
    assert_eq!(body["path"], "locked");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("reading `locked`:"),
        "{body}"
    );
    // Its parent still lists it: a directory you may not read is still a
    // directory of the root (the refusal comes when it is entered).
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200, "{listing}");
    assert!(
        entries(&listing).contains(&("locked".to_owned(), "dir".to_owned())),
        "{listing}"
    );
    handle.shutdown().await;
    drop(denied);
}
