//! Server + headless client integration test (doc 14 §Testing standards,
//! Protocol row): start `serve` on an ephemeral port over a scratch
//! project, then drive it exactly as the browser (or an agent) would —
//! token-gated HTTP routes, the WebSocket handshake, `snapshot`, binary
//! frames, an intent through the op pipeline, `/debug/state` as the
//! oracle. No network beyond loopback; deterministic; the store lives in
//! the temp dir (`--cache-dir` discipline).

// Tests are exempt from the expect/unwrap denial (clippy.toml), but the
// exemption recognizes #[test] fns only — not helpers in integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use cicada_server::frames::{Frame, FrameKind, decode};
use cicada_server::protocol::PROTOCOL_VERSION;
use cicada_server::{ServeConfig, serve};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

const PIPELINE: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n";

/// Minimal HTTP/1.1 GET (loopback only): `(status, body)`.
fn http_get(addr: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
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
    // Chunked bodies: hyper uses content-length for these small JSON
    // replies with `Connection: close`; assert rather than assume.
    assert!(
        !head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "unexpected chunked body:\n{head}"
    );
    (status, body.to_owned())
}

/// Minimal HTTP/1.1 POST with a JSON body (loopback only): `(status, body)`.
fn http_post(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_snapshot_frames_intents_and_debug_state() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("p.cic"), PIPELINE).unwrap();
    let mut config = ServeConfig::new(dir.path().to_owned());
    config.pipeline = Some("p.cic".to_owned());
    config.port = 0;
    config.token = Some("t".to_owned());
    config.cache_dir = Some(dir.path().join("cache"));
    config.threads = 2;
    let handle = serve(config).await.expect("serve");
    let addr = handle.addr;
    assert!(handle.url().contains("token=t&pipeline=p.cic"));

    // ---- HTTP: health is open; everything else is token-gated.
    let (status, body) = tokio::task::spawn_blocking(move || http_get(addr, "/health", None))
        .await
        .unwrap();
    assert_eq!((status, body.as_str()), (200, "ok"));
    let (status, _) = tokio::task::spawn_blocking(move || http_get(addr, "/api/project", None))
        .await
        .unwrap();
    assert_eq!(status, 401, "no token → 401");
    let (status, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/api/project", Some("t")))
            .await
            .unwrap();
    assert_eq!(status, 200);
    let project: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(project["pipelines"][0], "p.cic");
    assert_eq!(project["default"], "p.cic");
    let (status, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/api/catalog?token=t", None))
            .await
            .unwrap();
    assert_eq!(status, 200);
    let catalog: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(catalog["format"], 2);
    assert!(catalog["nodes"].as_array().unwrap().len() > 30);
    let (status, _) = tokio::task::spawn_blocking(move || {
        http_get(
            addr,
            "/debug/state?token=t&pipeline=../etc/passwd.cic",
            None,
        )
    })
    .await
    .unwrap();
    assert_eq!(status, 400, "path traversal refused");
    let (status, _) =
        tokio::task::spawn_blocking(move || http_get(addr, "/debug/screenshot?token=t", None))
            .await
            .unwrap();
    assert_eq!(
        status, 503,
        "no client connected → loud 503, never a blank image"
    );

    // ---- WebSocket: hello, snapshot, frames, an intent, the delta.
    let url = format!("ws://{addr}/ws?token=t&pipeline=p.cic");
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    socket
        .send(Message::Text(
            format!(
                r#"{{"v":{PROTOCOL_VERSION},"type":"hello","payload":{{"v":{PROTOCOL_VERSION}}}}}"#
            )
            .into(),
        ))
        .await
        .unwrap();
    let mut types = Vec::new();
    // The wire order, texts by type and frames as `<frame>` — the join's
    // contract (docs/13 §Two lanes, one socket) is a prefix of it.
    let mut wire = Vec::new();
    let mut mesh_frames = 0;
    let mut snapshot: Option<serde_json::Value> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
        let Ok(Some(Ok(message))) = next else { break };
        match message {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["v"], PROTOCOL_VERSION);
                let kind = value["type"].as_str().unwrap().to_owned();
                if kind == "snapshot" {
                    snapshot = Some(value.clone());
                }
                wire.push(kind.clone());
                types.push(kind);
            }
            Message::Binary(bytes) => {
                wire.push("<frame>".to_owned());
                let frame = decode(&bytes).expect("decodable frame");
                if frame.header().kind == FrameKind::Mesh {
                    mesh_frames += 1;
                    if let Frame::Batch { batch, .. } = &frame {
                        assert_eq!(batch.indices.len(), 36, "the box");
                    }
                }
            }
            _ => {}
        }
        if mesh_frames >= 1 && snapshot.is_some() && types.contains(&"status".to_owned()) {
            break;
        }
    }
    assert_eq!(types.first().map(String::as_str), Some("hello"));
    assert!(types.contains(&"snapshot".to_owned()), "{types:?}");
    assert!(types.contains(&"display_reset".to_owned()), "{types:?}");
    assert_eq!(mesh_frames, 1, "one displayed mesh output (block)");
    // The control lane leads (hello, snapshot), and the restream's header
    // precedes every frame: `display_reset` rides the display lane, FIFO
    // with the frames it announces — no frame may reach the client before
    // it, whatever the control lane overtakes.
    assert_eq!(&wire[..2], ["hello", "snapshot"], "{wire:?}");
    let reset_at = wire.iter().position(|w| w == "display_reset").unwrap();
    let first_frame_at = wire.iter().position(|w| w == "<frame>").unwrap();
    assert!(
        reset_at < first_frame_at,
        "display_reset must precede the first frame: {wire:?}"
    );
    let snapshot = snapshot.unwrap();
    assert_eq!(
        snapshot["payload"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(snapshot["payload"]["lease"]["writer"], 1);

    // Wrong protocol version is refused, not guessed around.
    socket
        .send(Message::Text(
            r#"{"v":99,"id":"bad","type":"cancel","payload":{}}"#.into(),
        ))
        .await
        .unwrap();
    // A real gesture: set the slider, expect the delta echoing the id.
    socket
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":"s1","type":"set_param","payload":{{"node":"size","port":"value","value":"3.0"}}}}"#).into(),
        ))
        .await
        .unwrap();
    let mut saw_error = false;
    let mut saw_delta = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && !(saw_error && saw_delta) {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(5), socket.next()).await
        else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        match value["type"].as_str() {
            Some("error") if value["payload"]["intent_id"] == "bad" => {
                assert_eq!(value["payload"]["kind"], "protocol");
                saw_error = true;
            }
            Some("delta") if value["payload"]["source"]["intent_id"] == "s1" => {
                assert!(
                    value["payload"]["text"]
                        .as_str()
                        .unwrap()
                        .contains("slider(value=3.0,")
                );
                saw_delta = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_error && saw_delta,
        "error={saw_error} delta={saw_delta}"
    );
    assert!(
        std::fs::read_to_string(dir.path().join("p.cic"))
            .unwrap()
            .contains("value=3.0"),
        "the op persisted immediately (no save button)"
    );

    // ---- /debug/state after waiting for the solve: the oracle.
    let (status, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/debug/state?token=t&wait=true", None))
            .await
            .unwrap();
    assert_eq!(status, 200);
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["statuses"]["block"]["state"], "done");
    assert_eq!(state["display"]["block.out"]["stats"]["bounds"][1][0], 3.0);
    assert_eq!(state["lease"]["clients"].as_array().unwrap().len(), 1);
    assert!(state["cache_dir"].as_str().unwrap().contains("cache"));

    // ---- v0.1 undo/redo + the agent edit route (docs/13 §Undo/redo).
    // The base an agent reads: the text and its hash, as the file has it.
    let (status, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/api/edit/text?token=t", None))
            .await
            .unwrap();
    assert_eq!(status, 200, "{body}");
    let edit: serde_json::Value = serde_json::from_str(&body).unwrap();
    let file_text = std::fs::read_to_string(dir.path().join("p.cic")).unwrap();
    assert_eq!(edit["path"], "p.cic");
    assert_eq!(edit["text"], file_text);
    let base_hash = blake3::hash(file_text.as_bytes()).to_hex().to_string();
    assert_eq!(
        edit["text_hash"], base_hash,
        "the hash IS the on-disk bytes'"
    );

    // apply_text with the right base: it applies although the socket's
    // client holds the writer lease (the agent acts for the user), and the
    // resulting delta reaches that client.
    let new_text = format!("{file_text}ball = sphere(radius=size)\n");
    let request = serde_json::json!({
        "base_text_hash": base_hash,
        "files": [{"path": "p.cic", "text": new_text}],
        "label": "agent: add a ball",
        "actor": {"kind": "agent", "prompt": "add a ball"},
    })
    .to_string();
    let first = request.clone();
    let (status, body) = tokio::task::spawn_blocking(move || {
        http_post(addr, "/api/edit/apply_text?token=t", &first)
    })
    .await
    .unwrap();
    assert_eq!(status, 200, "{body}");
    let applied: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(applied["ok"], true);
    assert_eq!(applied["history"]["depth"], 2, "set_param + the agent's op");
    assert_eq!(applied["history"]["undo_label"], "agent: add a ball");
    let applied_hash = applied["text_hash"].as_str().unwrap().to_owned();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p.cic")).unwrap(),
        new_text,
        "persisted"
    );
    let mut agent_delta = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && agent_delta.is_none() {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(5), socket.next()).await
        else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        if value["type"] == "delta" && value["payload"]["source"]["label"] == "agent: add a ball" {
            agent_delta = Some(value);
        }
    }
    let agent_delta = agent_delta.expect("the agent's delta reaches the lease holder");
    assert_eq!(
        agent_delta["payload"]["source"]["client"],
        serde_json::Value::Null
    );
    assert_eq!(agent_delta["payload"]["text"], new_text);
    assert_eq!(agent_delta["payload"]["history"]["depth"], 2);
    assert_eq!(agent_delta["payload"]["history"]["can_undo"], true);

    // The same request again is a stale base: 409, with the hash to rebase
    // on, and the file untouched.
    let again = request.clone();
    let (status, body) = tokio::task::spawn_blocking(move || {
        http_post(addr, "/api/edit/apply_text?token=t", &again)
    })
    .await
    .unwrap();
    assert_eq!(status, 409, "{body}");
    let refused: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(refused["kind"], "stale_base");
    assert_eq!(refused["current_text_hash"], applied_hash);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p.cic")).unwrap(),
        new_text
    );
    // A text that does not parse: 422 with diagnostics.
    let broken = serde_json::json!({
        "base_text_hash": applied_hash,
        "files": [{"path": "p.cic", "text": "# cicada 1\nx = (\n"}],
        "label": "broken",
        "actor": {"kind": "human"},
    })
    .to_string();
    let (status, body) = tokio::task::spawn_blocking(move || {
        http_post(addr, "/api/edit/apply_text?token=t", &broken)
    })
    .await
    .unwrap();
    assert_eq!(status, 422, "{body}");
    let refused: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(refused["kind"], "parse_error");
    assert!(!refused["diagnostics"].as_array().unwrap().is_empty());
    // A path outside the allowed set: 422.
    let escape = serde_json::json!({
        "base_text_hash": applied_hash,
        "files": [{"path": "../p.cic", "text": "# cicada 1\n"}],
        "label": "escape",
        "actor": {"kind": "human"},
    })
    .to_string();
    let (status, body) = tokio::task::spawn_blocking(move || {
        http_post(addr, "/api/edit/apply_text?token=t", &escape)
    })
    .await
    .unwrap();
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["kind"],
        "path_not_allowed"
    );
    // Garbage: 400. No token: 401.
    let (status, _) = tokio::task::spawn_blocking(move || {
        http_post(addr, "/api/edit/apply_text?token=t", "{not json")
    })
    .await
    .unwrap();
    assert_eq!(status, 400);
    let (status, _) =
        tokio::task::spawn_blocking(move || http_post(addr, "/api/edit/apply_text", "{}"))
            .await
            .unwrap();
    assert_eq!(status, 401);

    // Undo over the socket: the agent's op is undone like any other, with
    // the documented label; the text is the base again.
    socket
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":"u1","type":"undo","payload":{{}}}}"#).into(),
        ))
        .await
        .unwrap();
    let mut undo_delta = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && undo_delta.is_none() {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(5), socket.next()).await
        else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        if value["type"] == "delta" && value["payload"]["source"]["intent_id"] == "u1" {
            undo_delta = Some(value);
        }
    }
    let undo_delta = undo_delta.expect("the undo's delta");
    assert_eq!(
        undo_delta["payload"]["source"]["label"],
        "undo: agent: add a ball"
    );
    assert_eq!(undo_delta["payload"]["text"], file_text);
    assert_eq!(undo_delta["payload"]["history"]["depth"], 1);
    assert_eq!(undo_delta["payload"]["history"]["can_redo"], true);
    assert_eq!(
        undo_delta["payload"]["history"]["redo_label"],
        "agent: add a ball"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p.cic")).unwrap(),
        file_text
    );
    // /debug/state carries the history and the op list.
    let (_, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/debug/state?token=t&wait=true", None))
            .await
            .unwrap();
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["history"]["depth"], 1);
    assert_eq!(state["history"]["can_redo"], true);
    let ops = state["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 2, "{ops:?}");
    assert_eq!(ops[0]["actor"], serde_json::json!({"kind": "human"}));
    assert_eq!(ops[1]["label"], "agent: add a ball");
    assert_eq!(
        ops[1]["actor"],
        serde_json::json!({"kind": "agent", "prompt": "add a ball"})
    );

    socket.close(None).await.ok();

    // ---- Containment (stage-5 review, critical): a pipeline reference is
    // a plain project-relative path — absolute, rooted, drive-relative,
    // `.`/`..`, and backslash forms are refused with 400, never opened, never
    // written. An outside .cic stays untouched and no session leaks.
    let outside = tempfile::tempdir().unwrap();
    let escape = outside.path().join("escape.cic");
    std::fs::write(&escape, PIPELINE).unwrap();
    let absolute = escape.to_string_lossy().replace('\\', "/");
    let rooted = format!(
        "/{}",
        absolute
            .split_once(":/")
            .map_or(absolute.as_str(), |(_, rest)| rest)
    );
    let drive_relative = format!("C:{}", "escape.cic");
    let backslashed = escape.to_string_lossy().into_owned();
    for bad in [
        absolute.as_str(),
        rooted.as_str(),
        drive_relative.as_str(),
        backslashed.as_str(),
        "../escape.cic",
        "./p.cic",
        "sub/../p.cic",
        "p.txt",
        "",
    ] {
        let path = format!("/debug/state?token=t&pipeline={}", urlencode(bad));
        let (status, body) = tokio::task::spawn_blocking(move || http_get(addr, &path, None))
            .await
            .unwrap();
        assert_eq!(status, 400, "`{bad}` must be refused: {body}");
        assert!(
            body.contains("project-relative"),
            "`{bad}`: the refusal names the rule: {body}"
        );
    }
    let (_, body) = tokio::task::spawn_blocking(move || http_get(addr, "/api/project", Some("t")))
        .await
        .unwrap();
    let project: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        project["open"],
        serde_json::json!(["p.cic"]),
        "no escaped session leaked"
    );
    assert_eq!(
        std::fs::read_to_string(&escape).unwrap(),
        PIPELINE,
        "the outside file is untouched"
    );
    assert!(
        !outside.path().join("escape.cic.layout.json").exists(),
        "no sidecar written outside the project"
    );
    // /api/catalog goes through the same gate (no silent stdlib fallback).
    let path = format!("/api/catalog?token=t&pipeline={}", urlencode(&absolute));
    let (status, _) = tokio::task::spawn_blocking(move || http_get(addr, &path, None))
        .await
        .unwrap();
    assert_eq!(status, 400);
    // The WebSocket route shares it: the upgrade is refused.
    let bad_ws = format!("ws://{addr}/ws?token=t&pipeline={}", urlencode(&absolute));
    assert!(
        tokio_tungstenite::connect_async(bad_ws).await.is_err(),
        "ws upgrade with an absolute pipeline must fail"
    );

    // ---- Handshake: a wrong protocol version is refused at hello — error,
    // close, no lease taken; so is a non-hello first message.
    for first in [
        r#"{"v":99,"type":"hello","payload":{"v":99}}"#.to_owned(),
        format!(r#"{{"v":{PROTOCOL_VERSION},"type":"cancel","payload":{{}}}}"#),
    ] {
        let url = format!("ws://{addr}/ws?token=t&pipeline=p.cic");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
        socket
            .send(Message::Text(first.clone().into()))
            .await
            .unwrap();
        let mut saw_error = false;
        let mut closed = false;
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_secs(5), socket.next()).await
        {
            match message {
                Message::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(value["type"], "error", "{first}: got {text}");
                    assert_eq!(value["payload"]["kind"], "protocol");
                    saw_error = true;
                }
                Message::Close(_) => {
                    closed = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(
            saw_error && closed,
            "{first}: error={saw_error} closed={closed}"
        );
    }
    let (_, body) =
        tokio::task::spawn_blocking(move || http_get(addr, "/debug/state?token=t", None))
            .await
            .unwrap();
    let state: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        state["lease"]["clients"].as_array().unwrap().len(),
        0,
        "refused handshakes never join the session"
    );
    let timings = state["timings"].as_array().unwrap();
    assert!(timings.len() >= 2, "generation timings are exposed");
    // docs/15 measurement currency: every timing carries the queue wait and
    // the start mark (preview latency = queued_ms + elapsed_ms); a
    // never-cancelled generation omits cancel_to_idle_ms.
    let first = &timings[0];
    assert!(
        first["queued_ms"].is_number(),
        "queued_ms is exposed: {first}"
    );
    assert!(
        first["started_ms"].is_number(),
        "started_ms is exposed: {first}"
    );
    assert!(
        timings.iter().all(|t| t.get("cancel_to_idle_ms").is_none()),
        "no Esc happened, so no generation is annotated with cancel_to_idle_ms"
    );

    handle.shutdown().await;
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}
