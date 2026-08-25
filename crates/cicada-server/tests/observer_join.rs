//! The join hint on the wire (docs/13 §Projects, pipelines, sessions; wave 4
//! O3 — the pop-out viewport): a socket whose `hello` carries `role:
//! "observer"` joins the pipeline's session as a DECLARED observer — its
//! `hello` answer says `observer` although the lease is free, the next
//! default join takes the lease, its `take_lease` is refused (kind `lease`)
//! with the writer unchanged in `/debug/state`, and it still receives the
//! writer's deltas (same session, same display set). The session-level
//! rules behind it (promotion skips a declared observer, the reconnect
//! takes the lease back) are `session.rs`'s `a_declared_observer_never_
//! takes_the_lease`; this file proves the hint travels from the socket's
//! handshake to the join. Loopback only; the store lives in the temp dir.

// Tests are exempt from the expect/unwrap denial (clippy.toml), but the
// exemption recognizes #[test] fns only — not helpers in integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use cicada_server::protocol::PROTOCOL_VERSION;
use cicada_server::{ServeConfig, serve};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const PIPELINE: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n";

/// Minimal HTTP/1.1 GET (loopback only): the body.
fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nX-Cicada-Token: t\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (_, body) = text.split_once("\r\n\r\n").expect("http response");
    body.to_owned()
}

fn debug_state(addr: SocketAddr) -> serde_json::Value {
    serde_json::from_str(&http_get(addr, "/debug/state?token=t&wait=true")).unwrap()
}

async fn connect(addr: SocketAddr, hello_payload: &str) -> Socket {
    let url = format!("ws://{addr}/ws?token=t&pipeline=p.cic");
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    socket
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"type":"hello","payload":{hello_payload}}}"#)
                .into(),
        ))
        .await
        .unwrap();
    socket
}

/// The next control-plane text of type `kind` (frames and other texts are
/// skipped), within 10 s.
async fn next_of_type(socket: &mut Socket, kind: &str) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_secs(10), socket.next()).await;
        let Ok(Some(Ok(message))) = next else {
            panic!("socket ended before a `{kind}` arrived")
        };
        if let Message::Text(text) = message {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if value["type"] == kind {
                return value;
            }
        }
    }
    panic!("no `{kind}` within 10 s")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hello_with_role_observer_joins_as_a_declared_observer() {
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

    // The pop-out joins FIRST, declaring itself: an observer on a free lease.
    let mut popout = connect(
        addr,
        &format!(r#"{{"v":{PROTOCOL_VERSION},"role":"observer"}}"#),
    )
    .await;
    let hello = next_of_type(&mut popout, "hello").await;
    assert_eq!(hello["payload"]["role"], "observer", "{hello}");
    let popout_id = hello["payload"]["client_id"].as_u64().unwrap();
    let snapshot = next_of_type(&mut popout, "snapshot").await;
    assert_eq!(
        snapshot["payload"]["lease"]["writer"],
        serde_json::Value::Null,
        "a declared observer leaves the lease free"
    );

    // The main window joins by the default rule (an older client's hello,
    // no `role`) and takes the lease.
    let mut main = connect(addr, &format!(r#"{{"v":{PROTOCOL_VERSION}}}"#)).await;
    let hello = next_of_type(&mut main, "hello").await;
    assert_eq!(hello["payload"]["role"], "writer", "{hello}");
    let main_id = hello["payload"]["client_id"].as_u64().unwrap();
    assert_ne!(main_id, popout_id);
    let roster = next_of_type(&mut popout, "lease").await;
    assert_eq!(roster["payload"]["role"], "observer");
    assert_eq!(roster["payload"]["lease"]["writer"], main_id);

    // `take_lease` from the declared observer is refused, kind `lease`, and
    // the writer is unchanged.
    popout
        .send(Message::Text(
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":"take","type":"take_lease","payload":{{}}}}"#)
                .into(),
        ))
        .await
        .unwrap();
    let error = next_of_type(&mut popout, "error").await;
    assert_eq!(error["payload"]["kind"], "lease", "{error}");
    assert_eq!(error["payload"]["intent_id"], "take");
    assert!(
        error["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("declared observer"),
        "{error}"
    );
    let state = tokio::task::spawn_blocking(move || debug_state(addr))
        .await
        .unwrap();
    assert_eq!(state["lease"]["writer"], main_id);
    assert_eq!(state["lease"]["clients"].as_array().unwrap().len(), 2);

    // The writer's edit reaches the declared observer: one session, one
    // display set.
    main.send(Message::Text(
        format!(
            r#"{{"v":{PROTOCOL_VERSION},"id":"w","type":"set_param","payload":{{"node":"size","port":"value","value":"3.0"}}}}"#
        )
        .into(),
    ))
    .await
    .unwrap();
    let delta = next_of_type(&mut popout, "delta").await;
    assert!(
        delta["payload"]["text"]
            .as_str()
            .unwrap()
            .contains("size = slider(value=3.0"),
        "{delta}"
    );
    assert_eq!(delta["payload"]["source"]["client"], main_id);

    handle.shutdown().await;
}
