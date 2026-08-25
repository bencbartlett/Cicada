//! Scrub caching through the session (v0.1 item 5, package S1; docs/12
//! §Speculative warming, DECISIONS.md row 39): the warm queues built off the
//! text, the worker driving them through the idle class, the `set_scrub`
//! gesture and its refusals, the byte cap, the drop on a text change,
//! pre-emption and the parked rule, the live-drag block, and the
//! `scrub_progress` wire shape. Deterministic: the worker is held on the
//! `scrub_gate` seam and the drag gap on the virtual op clock — no sleeps.

use std::sync::{Arc, Mutex, mpsc};

use tokio::sync::mpsc::unbounded_channel;

use super::tests::{drain, preview_generations, project, project_with_clock, texts};
use super::*;
use crate::scrub::SCRUB_BYTE_CAP;

/// 0.5…5.0 by 0.5: ten positions, the committed 2.0 at index 3, a B-rep
/// box in the cone (a real solve per position, milliseconds each).
const PIPELINE: &str = "# cicada 1\n\
     size = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n\
     span = construct_domain(start=0.0, end=size)\n\
     block = box(x=span, y=span, z=span)\n";

/// Open a session whose scrub worker blocks on a gate before EVERY
/// idle-class solve: the test reads which position arrived (`entered`) and
/// releases it when ready (`release`). Returned session-first on purpose:
/// locals drop in reverse order, so the RELEASE SENDER drops before the
/// session — a gate the worker is parked on opens (`recv` → `Err`) before
/// `Drop for Session` joins the worker. (The first cut declared the gate
/// first and a failing assertion deadlocked the teardown instead of
/// reporting.)
fn open_gated(
    mut config: SessionConfig,
) -> (
    Arc<Session>,
    mpsc::Receiver<(String, usize)>,
    mpsc::Sender<()>,
) {
    let (entered_tx, entered_rx) = mpsc::channel::<(String, usize)>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let gate: ScrubGate = Arc::new(move |node: &str, index: usize| {
        let _ = entered_tx.send((node.to_owned(), index));
        let _ = release_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv();
    });
    config.scrub_gate = Some(gate);
    let session = Session::open(config).unwrap();
    (session, entered_rx, release_tx)
}

fn scrub_of(state: &serde_json::Value, node: &str) -> serde_json::Value {
    state["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == node)
        .unwrap()["param"]["scrub"]
        .clone()
}

fn queue_of(state: &serde_json::Value, node: &str) -> serde_json::Value {
    state["scrub"]["queues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["node"] == node)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn hypothetical_solves(session: &Session) -> usize {
    session.debug_state(false)["timings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == "hypothetical" && t["cancelled"] == false)
        .count()
}

fn max_generation(session: &Session) -> u64 {
    session.debug_state(false)["timings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["generation"].as_u64())
        .max()
        .unwrap_or(0)
}

fn gesture(session: &Session, id: u32, intent: &str, message: ClientMessage) {
    session.handle(id, Some(intent.to_owned()), message);
    session.wait_idle();
}

fn set_scrub(session: &Session, id: u32, node: &str, on: bool) {
    gesture(
        session,
        id,
        &format!("scrub-{node}-{on}"),
        ClientMessage::SetScrub {
            node: node.into(),
            on,
        },
    );
}

fn errors(messages: &[serde_json::Value]) -> Vec<(String, String)> {
    messages
        .iter()
        .filter(|m| m["type"] == "error")
        .map(|m| {
            (
                m["payload"]["kind"].as_str().unwrap().to_owned(),
                m["payload"]["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

// The whole contract in one story: an opted-in slider's queue is built off
// the text with the contract's order (nearest the committed value, sides
// alternating, above first), every position ends warm, the committed value
// was skipped as the memo hit it already was (nine solves for ten
// positions), the view and the oracle agree, and a later tick on any
// position is a pure cache read — the payoff compute-on-release's
// hysteresis rule already pays.
#[test]
fn an_opted_in_slider_warms_every_position_nearest_first() {
    let (_dir, config) = project(PIPELINE);
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle", "{}", state["scrub"]);
    assert_eq!(state["scrub"]["max_positions"], scrub::SCRUB_MAX_POSITIONS);
    assert_eq!(state["scrub"]["byte_cap"], SCRUB_BYTE_CAP);
    let q = queue_of(&state, "size");
    assert_eq!(q["port"], "value");
    assert_eq!(q["positions"], 10);
    assert_eq!(
        q["values"],
        serde_json::json!([
            "0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"
        ])
    );
    assert_eq!(
        q["order"],
        serde_json::json!([3, 4, 2, 5, 1, 6, 0, 7, 8, 9]),
        "nearest 2.0 first, then alternating sides, above first"
    );
    assert_eq!(
        q["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    assert_eq!(q["warming"], false);
    assert_eq!(q["capped"], false);
    assert!(
        q["bytes"].as_u64().unwrap() > 0,
        "the solves stored bytes: {q}"
    );
    // The graph view (every snapshot and delta) carries the same state.
    let view = scrub_of(&state, "size");
    assert_eq!(view["on"], true);
    assert_eq!(view["positions"], 10);
    assert_eq!(view["warmed"].as_array().unwrap().len(), 10);
    assert_eq!(view["warming"], false);
    assert_eq!(view["bytes"], q["bytes"]);
    assert!(view.get("ineligible").is_none());
    // Nine solves: the load already solved 2.0 (index 3), and the dry run
    // saw the hit — skip-if-stored, docs/12.
    assert_eq!(hypothetical_solves(&session), 9);
    // Every position predicts as the cache read it now is.
    {
        let inner = session.core.lock_inner();
        for value in q["values"].as_array().unwrap() {
            let value = value.as_str().unwrap();
            let scratch = super::tests::scratch_lowered(&inner, "size", Some("value"), value);
            let cost = session
                .core
                .predict_cone(&inner, &scratch, "size", Some("value"))
                .expect("a cone of hits is a prediction");
            assert_eq!(cost.misses, 0, "{value}: {cost:?}");
        }
    }
    // A tick on a warm position computes nothing.
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    let before = preview_generations(&session);
    session.handle(
        id,
        None,
        ClientMessage::ParamPreview {
            node: "size".into(),
            port: Some("value".into()),
            value: "3.5".into(),
        },
    );
    session.wait_idle();
    assert_eq!(preview_generations(&session), before + 1);
    let last = session.debug_state(false)["timings"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|t| t["kind"] == "preview")
        .cloned()
        .unwrap();
    assert_eq!(last["computed"], 0, "a pure cache read: {last}");
    assert!(last["cached"].as_u64().unwrap() >= 2, "{last}");
}

// `set_scrub` is the toggle: `on` writes `scrub=True` at its spec-order
// position (an op, undoable, the delta carrying the new view, the queue
// built and warmed from it), `off` removes the kwarg; an ineligible slider
// refuses `on` with the contract's reasons; a non-slider refuses both;
// observers are refused at the door.
#[test]
#[allow(clippy::too_many_lines)] // one story: every refusal and the round trip
fn set_scrub_writes_the_kwarg_refuses_the_ineligible_and_undoes() {
    let (_dir, config) = project(
        "# cicada 1\n\
         n = 4.0\n\
         a = slider(value=2.0, min=0.5, max=5.0, step=0.5)\n\
         b = slider(value=0.5, min=0.0, max=1.0, step=0.02)\n\
         c = slider(value=0.5, min=0.0, max=n, step=0.1)\n\
         d = slider(value=0.5)\n\
         w = slider(value=n, min=0.0, max=5.0, step=0.5)\n\
         span = construct_domain(start=0.0, end=a)\n",
    );
    let pipeline = config.pipeline.clone();
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    assert!(
        session.debug_state(false)["scrub"]["queues"]
            .as_array()
            .unwrap()
            .is_empty(),
        "nothing opted in, nothing warms"
    );
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);

    // On: the kwarg lands in spec order, after `step`.
    set_scrub(&session, id, "a", true);
    let msgs = texts(&drain(&mut rx));
    let delta = msgs.iter().find(|m| m["type"] == "delta").expect("a delta");
    assert_eq!(delta["payload"]["source"]["label"], "scrub a on");
    assert_eq!(delta["payload"]["dirty"], serde_json::json!(["a"]));
    let text = std::fs::read_to_string(&pipeline).unwrap();
    assert!(
        text.contains("a = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n"),
        "{text}"
    );
    let view = delta["payload"]["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "a")
        .unwrap()["param"]["scrub"]
        .clone();
    assert_eq!(view["on"], true);
    assert_eq!(view["positions"], 10);
    assert_eq!(delta["payload"]["history"]["undo_label"], "scrub a on");
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(
        queue_of(&state, "a")["warmed"].as_array().unwrap().len(),
        10
    );

    // The refusals, with the contract's reasons.
    let _ = drain(&mut rx);
    set_scrub(&session, id, "b", true);
    set_scrub(&session, id, "c", true);
    set_scrub(&session, id, "d", true);
    set_scrub(&session, id, "w", true);
    set_scrub(&session, id, "n", true);
    set_scrub(&session, id, "n", false);
    set_scrub(&session, id, "nope", true);
    let msgs = texts(&drain(&mut rx));
    assert_eq!(
        errors(&msgs),
        vec![
            (
                "refused".to_owned(),
                "`b`: too many positions (51 > 32)".to_owned()
            ),
            (
                "refused".to_owned(),
                "`c`: max is wired — the positions are a function of literal min, max and step"
                    .to_owned()
            ),
            (
                "refused".to_owned(),
                "`d`: step is 0 — a continuous slider has no positions to warm".to_owned()
            ),
            // A wired `value` IS a slider — one with no widget; not "is not
            // a slider" (review finding, 2026-08-24).
            (
                "refused".to_owned(),
                "`w`: value is wired — a wired slider has no widget to scrub".to_owned()
            ),
            (
                "refused".to_owned(),
                "`n` is not a slider — only sliders scrub-cache".to_owned()
            ),
            (
                "refused".to_owned(),
                "`n` is not a slider — only sliders scrub-cache".to_owned()
            ),
            ("unknown".to_owned(), "no node named `nope`".to_owned()),
        ]
    );
    assert!(
        !msgs.iter().any(|m| m["type"] == "delta"),
        "a refusal edits nothing: {msgs:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&pipeline).unwrap(),
        text,
        "the file is untouched by refusals"
    );
    // Off on a slider that never carried the kwarg: nothing to remove, no op.
    let depth_before = session.history().depth;
    set_scrub(&session, id, "b", false);
    assert_eq!(
        session.history().depth,
        depth_before,
        "a no-op pushes no op"
    );
    assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), text);

    // Undo takes the kwarg out and the queue with it; redo brings both back.
    let _ = drain(&mut rx);
    gesture(&session, id, "undo", ClientMessage::Undo {});
    let text_undone = std::fs::read_to_string(&pipeline).unwrap();
    assert!(
        text_undone.contains("a = slider(value=2.0, min=0.5, max=5.0, step=0.5)\n"),
        "{text_undone}"
    );
    session.wait_scrub();
    let state = session.debug_state(false);
    assert!(queue_of(&state, "a").is_null(), "{}", state["scrub"]);
    assert_eq!(scrub_of(&state, "a")["on"], false);
    assert_eq!(scrub_of(&state, "a")["warmed"], serde_json::json!([]));
    gesture(&session, id, "redo", ClientMessage::Redo {});
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(
        queue_of(&state, "a")["warmed"].as_array().unwrap().len(),
        10
    );
    // Off removes the kwarg: the line is as it was written.
    set_scrub(&session, id, "a", false);
    let msgs = texts(&drain(&mut rx));
    let delta = msgs.iter().rfind(|m| m["type"] == "delta").unwrap();
    assert_eq!(delta["payload"]["source"]["label"], "scrub a off");
    assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), text_undone);
    session.wait_scrub();
    assert!(queue_of(&session.debug_state(false), "a").is_null());

    // An observer's toggle is refused at the door (a write).
    let (tx_o, mut rx_o) = unbounded_channel();
    let (observer, role) = session.connect(ClientLanes::merged(tx_o));
    assert_eq!(role, Role::Observer);
    let _ = drain(&mut rx_o);
    set_scrub(&session, observer, "a", true);
    assert_eq!(
        errors(&texts(&drain(&mut rx_o)))
            .into_iter()
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>(),
        ["lease"]
    );
}

// A hand-written `scrub=True` on a slider that cannot scrub warms nothing
// — the view says so (`on: true`, `ineligible`), no queue exists.
#[test]
fn a_hand_written_opt_in_on_an_ineligible_slider_warms_nothing() {
    let (_dir, config) = project(
        "# cicada 1\n\
         s = slider(value=0.5, min=0.0, max=1.0, step=0.02, scrub=True)\n\
         span = construct_domain(start=0.0, end=s)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    assert!(state["scrub"]["queues"].as_array().unwrap().is_empty());
    let view = scrub_of(&state, "s");
    assert_eq!(view["on"], true);
    assert_eq!(view["positions"], 0);
    assert_eq!(view["ineligible"], "too many positions (51 > 32)");
    assert_eq!(hypothetical_solves(&session), 0);
}

// A text change drops the queue (the contract: min/max/step/value or
// `scrub` toggled off — and every other text change too, since the warm
// set was verified against the old graph's keys); the new queue is
// re-verified from the memo, so a change outside the cone costs no solve;
// a sidecar-only change keeps the queue and its progress.
#[test]
fn a_text_change_drops_the_queue_and_a_sidecar_change_keeps_it() {
    let (_dir, config) = project(
        "# cicada 1\n\
         k = 1.0\n\
         size = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n\
         span = construct_domain(start=0.0, end=size)\n\
         block = box(x=span, y=span, z=span)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let first = queue_of(&session.debug_state(false), "size");
    assert_eq!(first["warmed"].as_array().unwrap().len(), 10);
    let first_id = first["id"].as_u64().unwrap();
    let solves = hypothetical_solves(&session);
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);

    // Sidecar only: the same queue, untouched.
    gesture(
        &session,
        id,
        "move",
        ClientMessage::MoveNode {
            node: "size".into(),
            cell: Some([4, 4]),
        },
    );
    session.wait_scrub();
    let kept = queue_of(&session.debug_state(false), "size");
    assert_eq!(kept["id"], first_id);
    assert_eq!(kept["warmed"].as_array().unwrap().len(), 10);
    assert_eq!(hypothetical_solves(&session), solves);

    // A text change outside the cone: a NEW queue, every position
    // re-verified as the memo hit it still is — zero solves.
    gesture(
        &session,
        id,
        "k",
        ClientMessage::SetParam {
            node: "k".into(),
            port: None,
            value: "2.0".into(),
        },
    );
    session.wait_scrub();
    let rebuilt = queue_of(&session.debug_state(false), "size");
    assert_ne!(rebuilt["id"], first_id, "dropped and recreated");
    assert_eq!(rebuilt["warmed"].as_array().unwrap().len(), 10);
    assert_eq!(
        hypothetical_solves(&session),
        solves,
        "hits confirm without a solve"
    );

    // The slider's own literals: a new range, a new queue.
    gesture(
        &session,
        id,
        "max",
        ClientMessage::SetParam {
            node: "size".into(),
            port: Some("max".into()),
            value: "3.0".into(),
        },
    );
    let state = session.debug_state(false);
    let narrowed = queue_of(&state, "size");
    assert_eq!(narrowed["positions"], 6);
    assert_eq!(
        narrowed["values"],
        serde_json::json!(["0.5", "1.0", "1.5", "2.0", "2.5", "3.0"])
    );
    session.wait_scrub();
    let narrowed = queue_of(&session.debug_state(false), "size");
    assert_eq!(narrowed["warmed"], serde_json::json!([0, 1, 2, 3, 4, 5]));
    // `max` is an INPUT of the slider node, so every position's slider key
    // is new and the dry run rightly calls it a miss: six solves, each
    // recomputing the slider alone (microseconds) and finding its whole
    // cone — span, block — in the memo.
    let state = session.debug_state(false);
    let since: Vec<&serde_json::Value> = state["timings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == "hypothetical" && t["cancelled"] == false)
        .skip(solves)
        .collect();
    assert_eq!(since.len(), 6, "{since:?}");
    for t in since {
        assert_eq!(t["computed"], 1, "the slider node itself: {t}");
        assert_eq!(t["cached"], 2, "span and block were warm: {t}");
    }
}

// The per-slider byte cap: with a cap of one byte, the first position the
// warming SOLVES crosses it (a dry-run hit costs nothing and never caps);
// the queue stops capped, the positions warmed so far stay warm, the view
// and the progress message say `capped`.
#[test]
fn the_byte_cap_stops_the_queue() {
    let (_dir, mut config) = project(PIPELINE);
    config.scrub_byte_cap = 1;
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    let q = queue_of(&state, "size");
    assert_eq!(q["capped"], true, "{q}");
    assert_eq!(q["warming"], false);
    assert_eq!(
        q["warmed"],
        serde_json::json!([3, 4]),
        "the load's 2.0 was a hit (0 bytes); 2.5 solved and crossed the cap"
    );
    assert!(q["bytes"].as_u64().unwrap() >= 1);
    assert_eq!(q["next"], serde_json::Value::Null);
    assert_eq!(hypothetical_solves(&session), 1);
    let view = scrub_of(&state, "size");
    assert_eq!(view["capped"], true);
    assert_eq!(view["warming"], false);
    assert_eq!(view["warmed"], serde_json::json!([3, 4]));
    // The progress broadcast carries the cap.
    let (tx, mut rx) = unbounded_channel();
    let _ = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    session.core.lock_scrub().dirty.insert("size".to_owned());
    session.flush_scrub_progress();
    let msgs = texts(&drain(&mut rx));
    let progress: Vec<&serde_json::Value> = msgs
        .iter()
        .filter(|m| m["type"] == "scrub_progress")
        .collect();
    assert_eq!(progress.len(), 1);
    assert_eq!(
        progress[0]["payload"],
        serde_json::json!({
            "node": "size", "port": "value", "warmed": [3, 4], "warming": false,
            "bytes": q["bytes"], "capped": true,
        })
    );
}

// Pre-emption: a real preview arriving while the worker is about to solve
// a position lands FIRST (the idle-class solve waits for the loop to go
// idle; generation numbers say which ran when), and the queue resumes to
// the end afterwards. Progress is broadcast as it goes, coalesced, and the
// last message is the finished state.
#[test]
fn a_real_preview_lands_first_and_the_queue_resumes() {
    let (_dir, config) = project(PIPELINE);
    let (session, entered, release) = open_gated(config);
    session.wait_idle();
    // The worker is held before its first solve (the first MISS — 2.0 was
    // a hit and needed no solve).
    let (node, index) = entered.recv().unwrap();
    assert_eq!((node.as_str(), index), ("size", 4), "2.5 is the first miss");
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    // Real work arrives while the worker is held: a tick on a cold value.
    session.handle(
        id,
        None,
        ClientMessage::ParamPreview {
            node: "size".into(),
            port: Some("value".into()),
            value: "1.0".into(),
        },
    );
    session.wait_idle();
    let preview_generation = max_generation(&session);
    let before = hypothetical_solves(&session);
    // The pointer comes up on the committed value: the drag ends, so the
    // warming is not blocked by it.
    session.handle(
        id,
        None,
        ClientMessage::EndDrag {
            node: "size".into(),
            port: Some("value".into()),
        },
    );
    // Release the held position and every later one.
    release.send(()).unwrap();
    for _ in 0..20 {
        let _ = release.send(());
    }
    session.wait_scrub();
    let state = session.debug_state(false);
    let q = queue_of(&state, "size");
    assert_eq!(
        q["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    assert_eq!(state["scrub"]["state"], "idle");
    // The held position's solve ran AFTER the preview: its generation is
    // newer, and it was not cancelled (nothing pre-empted it once it ran).
    let hypotheticals: Vec<&serde_json::Value> = state["timings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == "hypothetical")
        .collect();
    let resumed = hypotheticals
        .iter()
        .find(|t| t["generation"].as_u64().unwrap() > preview_generation)
        .expect("the held position solved after the preview");
    assert_eq!(resumed["cancelled"], false);
    // The preview's value 1.0 (index 1) became a hit for the warming: eight
    // solves in total after it (ten positions, 2.0 and 1.0 already warm).
    assert_eq!(
        hypothetical_solves(&session) - before,
        8,
        "{hypotheticals:?}"
    );
    // Progress went out coalesced; the last message is the finished state.
    session.flush_scrub_progress();
    let msgs = texts(&drain(&mut rx));
    let progress: Vec<&serde_json::Value> = msgs
        .iter()
        .filter(|m| m["type"] == "scrub_progress")
        .collect();
    assert!(!progress.is_empty());
    assert!(progress.len() <= 10, "coalesced: {}", progress.len());
    for p in &progress {
        assert_eq!(p["payload"]["node"], "size");
        assert_eq!(p["payload"]["port"], "value");
    }
    let last = progress.last().unwrap();
    assert_eq!(last["payload"]["warmed"].as_array().unwrap().len(), 10);
    assert_eq!(last["payload"]["warming"], false);
    assert!(last["payload"].get("capped").is_none(), "{last}");
}

// The parked rule's mechanics (the solve loop's own tests cover the
// cancellation of an idle token by real work and by Esc): a position that
// came back pre-empted parks the worker until a real generation NEWER than
// the pre-empted solve completes — an older completion does not unpark it,
// and a pre-emption whose pre-empting generation already finished does not
// park at all.
#[test]
fn a_preempted_position_parks_the_worker_until_a_newer_generation_completes() {
    let (_dir, config) = project(
        "# cicada 1\n\
         size = slider(value=2.0, min=0.5, max=5.0, step=0.5)\n\
         span = construct_domain(start=0.0, end=size)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    assert_eq!(session.debug_state(false)["scrub"]["state"], "idle");
    let core = &session.core;
    let latest = core.lock_scrub().last_real_generation;
    assert!(latest >= 1, "the load completed");
    // A pre-emption by a generation numbered above everything so far.
    core.scrub_settle(
        0,
        0,
        Settled::Preempted {
            generation: latest + 5,
        },
    );
    core.wake_scrub();
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "parked");
    assert_eq!(state["scrub"]["parked_until"], latest + 5);
    // An older completion changes nothing.
    core.scrub_generation_completed(latest + 5);
    session.wait_scrub();
    assert_eq!(session.debug_state(false)["scrub"]["state"], "parked");
    // A newer one unparks.
    core.scrub_generation_completed(latest + 6);
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle");
    assert_eq!(state["scrub"]["parked_until"], serde_json::Value::Null);
    // A pre-emption the pre-empting generation already outran: not parked.
    core.scrub_settle(
        0,
        0,
        Settled::Preempted {
            generation: latest + 2,
        },
    );
    core.wake_scrub();
    session.wait_scrub();
    assert_eq!(session.debug_state(false)["scrub"]["state"], "idle");
}

// A live drag is not idle time: the worker blocks while a drag stands
// (within the drag gap — frozen here on the virtual op clock) and resumes
// on `end_drag`.
#[test]
fn a_live_drag_blocks_the_warming_and_its_end_resumes_it() {
    let (_dir, config, _clock) = project_with_clock(PIPELINE);
    let (session, entered, release) = open_gated(config);
    session.wait_idle();
    let _held = entered.recv().unwrap();
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    // A drag on the slider starts while the worker is held; the virtual
    // clock never lets the gap elapse.
    session.handle(
        id,
        None,
        ClientMessage::ParamPreview {
            node: "size".into(),
            port: Some("value".into()),
            value: "1.2".into(),
        },
    );
    session.wait_idle();
    // Release the held position: it solves (after the preview), then the
    // worker finds the drag live and blocks.
    release.send(()).unwrap();
    for _ in 0..20 {
        let _ = release.send(());
    }
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "blocked", "{}", state["scrub"]);
    let q = queue_of(&state, "size");
    assert!(q["warmed"].as_array().unwrap().len() < 10, "{q}");
    assert_eq!(q["warming"], true);
    assert_eq!(scrub_of(&state, "size")["warming"], true);
    // The pointer comes up: the drag ends, the warming resumes to the end.
    session.handle(
        id,
        None,
        ClientMessage::EndDrag {
            node: "size".into(),
            port: Some("value".into()),
        },
    );
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle", "{}", state["scrub"]);
    assert_eq!(
        queue_of(&state, "size")["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
}

// Two opted-in sliders share the worker round robin: both finish, and the
// `/debug/state.scrub` listing carries every queue.
#[test]
fn several_sliders_warm_round_robin_and_all_finish() {
    let (_dir, config) = project(
        "# cicada 1\n\
         a = slider(value=2.0, min=1.0, max=3.0, step=1.0, scrub=True)\n\
         b = slider(value=1.0, min=1.0, max=4.0, step=1.0, scrub=True)\n\
         da = construct_domain(start=0.0, end=a)\n\
         db = construct_domain(start=0.0, end=b)\n\
         box_a = box(x=da, y=da, z=da)\n\
         box_b = box(x=db, y=db, z=db)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    let queues = state["scrub"]["queues"].as_array().unwrap();
    assert_eq!(queues.len(), 2);
    assert_eq!(
        queue_of(&state, "a")["warmed"],
        serde_json::json!([0, 1, 2])
    );
    assert_eq!(
        queue_of(&state, "b")["warmed"],
        serde_json::json!([0, 1, 2, 3])
    );
    // 3 + 4 positions, two of them (the committed values) hits from the load.
    assert_eq!(hypothetical_solves(&session), 5);
}

// Esc parks the warming OUTRIGHT — "stop solving" includes it — whether or
// not a position was mid-solve. Here the worker is held on the gate with a
// position decided but not yet submitted when Esc lands: released, it
// submits nothing (the position is withheld and next again), the worker
// parks on the newest generation issued, a second Esc changes nothing, and
// the user's next action — a tick, a real generation above that number —
// unparks it; the queue then finishes. (The first cut parked only a solve
// cut short, so an Esc between positions let the worker take the next one
// at once — review finding, 2026-08-24.)
#[test]
fn esc_parks_the_warming_until_the_next_real_generation() {
    let (_dir, config) = project(PIPELINE);
    let (session, entered, release) = open_gated(config);
    session.wait_idle();
    let (node, index) = entered.recv().unwrap();
    assert_eq!((node.as_str(), index), ("size", 4), "2.5 is the first miss");
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    let newest = session.solve.last_generation();
    assert!(newest >= 1, "the load's generation was issued");
    // Esc lands while the worker stands between its decision and its solve.
    session.handle(id, None, ClientMessage::Cancel {});
    // Released — with releases to spare, so a worker that wrongly walks on
    // runs to the end and FAILS the assertions below instead of hanging on
    // the next gate — the held position is withheld (no solve) and the
    // worker parks instead of taking the next position.
    for _ in 0..20 {
        let _ = release.send(());
    }
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "parked", "{}", state["scrub"]);
    assert_eq!(state["scrub"]["parked_until"], newest);
    let q = queue_of(&state, "size");
    assert_eq!(q["warmed"], serde_json::json!([3]), "only the load's hit");
    assert_eq!(q["in_flight"], serde_json::Value::Null);
    assert_eq!(q["next"], 4, "the withheld position is next again");
    assert_eq!(q["warming"], true, "work remains — the bar keeps pulsing");
    assert_eq!(
        hypothetical_solves(&session),
        0,
        "nothing was submitted after Esc"
    );
    // A second Esc: still parked, on the same number.
    session.handle(id, None, ClientMessage::Cancel {});
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "parked");
    assert_eq!(state["scrub"]["parked_until"], newest);
    // The user's next action — a tick on a cold value — is a real
    // generation numbered above the Esc's: it unparks the worker; the
    // pointer coming up ends the drag that would otherwise block it.
    session.handle(
        id,
        None,
        ClientMessage::ParamPreview {
            node: "size".into(),
            port: Some("value".into()),
            value: "1.0".into(),
        },
    );
    session.wait_idle();
    assert!(max_generation(&session) > newest);
    session.handle(
        id,
        None,
        ClientMessage::EndDrag {
            node: "size".into(),
            port: Some("value".into()),
        },
    );
    for _ in 0..20 {
        let _ = release.send(());
    }
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle", "{}", state["scrub"]);
    assert_eq!(state["scrub"]["parked_until"], serde_json::Value::Null);
    assert_eq!(
        queue_of(&state, "size")["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    // Eight solves: ten positions, 2.0 (the load) and 1.0 (the tick) hits.
    assert_eq!(hypothetical_solves(&session), 8);
}

// `set_scrub` rides a `batch` like any gesture: one op, one delta, the
// queue built from the result; all or nothing — a batch whose second
// element is an ineligible slider's `on` changes nothing and the refusal
// names the element; undo takes the whole batch back, the queue with it.
#[test]
#[allow(clippy::too_many_lines)] // one story: the batch, its refusal, its undo
fn set_scrub_rides_a_batch_as_one_op() {
    let (_dir, config) = project(
        "# cicada 1\n\
         k = 1.0\n\
         a = slider(value=2.0, min=0.5, max=5.0, step=0.5)\n\
         b = slider(value=0.5, min=0.0, max=1.0, step=0.02)\n\
         span = construct_domain(start=0.0, end=a)\n",
    );
    let pipeline = config.pipeline.clone();
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let (tx, mut rx) = unbounded_channel();
    let (id, _) = session.connect(ClientLanes::merged(tx));
    let _ = drain(&mut rx);
    session.handle(
        id,
        Some("b1".into()),
        ClientMessage::Batch {
            label: "set k, scrub a".into(),
            ops: vec![
                ClientMessage::SetParam {
                    node: "k".into(),
                    port: None,
                    value: "2.0".into(),
                },
                ClientMessage::SetScrub {
                    node: "a".into(),
                    on: true,
                },
            ],
        },
    );
    session.wait_idle();
    let msgs = texts(&drain(&mut rx));
    let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
    assert_eq!(deltas.len(), 1, "one delta for the batch: {msgs:?}");
    assert_eq!(deltas[0]["payload"]["source"]["intent_id"], "b1");
    assert_eq!(deltas[0]["payload"]["source"]["label"], "set k, scrub a");
    assert_eq!(deltas[0]["payload"]["history"]["depth"], 1);
    let text = std::fs::read_to_string(&pipeline).unwrap();
    assert!(text.contains("k = 2.0\n"), "{text}");
    assert!(
        text.contains("a = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n"),
        "{text}"
    );
    let view = deltas[0]["payload"]["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "a")
        .unwrap()["param"]["scrub"]
        .clone();
    assert_eq!(view["on"], true);
    assert_eq!(view["positions"], 10);
    session.wait_scrub();
    assert_eq!(
        queue_of(&session.debug_state(false), "a")["warmed"]
            .as_array()
            .unwrap()
            .len(),
        10
    );

    // All or nothing: the second element's refusal takes the first (a valid
    // `off`) with it — text, disk and the queue are exactly as before.
    let _ = drain(&mut rx);
    session.handle(
        id,
        Some("b2".into()),
        ClientMessage::Batch {
            label: "doomed".into(),
            ops: vec![
                ClientMessage::SetScrub {
                    node: "a".into(),
                    on: false,
                },
                ClientMessage::SetScrub {
                    node: "b".into(),
                    on: true,
                },
            ],
        },
    );
    session.wait_idle();
    let msgs = texts(&drain(&mut rx));
    assert!(
        !msgs.iter().any(|m| m["type"] == "delta"),
        "a failed batch broadcasts no delta: {msgs:?}"
    );
    let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
    assert_eq!(error["payload"]["intent_id"], "b2");
    assert_eq!(error["payload"]["kind"], "refused");
    assert_eq!(error["payload"]["index"], 1, "names the failing element");
    assert_eq!(
        error["payload"]["message"],
        "batch `doomed` failed at op 1 (set_scrub): `b`: too many positions (51 > 32)"
    );
    assert_eq!(
        std::fs::read_to_string(&pipeline).unwrap(),
        text,
        "the rolled-back batch changed nothing"
    );
    assert_eq!(session.history().depth, 1, "no op recorded");
    session.wait_scrub();
    assert_eq!(
        queue_of(&session.debug_state(false), "a")["warmed"]
            .as_array()
            .unwrap()
            .len(),
        10,
        "the queue survived the rolled-back batch"
    );

    // Undo takes both gestures back at once; the queue goes with the kwarg.
    gesture(&session, id, "undo", ClientMessage::Undo {});
    let undone = std::fs::read_to_string(&pipeline).unwrap();
    assert!(undone.contains("k = 1.0\n"), "{undone}");
    assert!(
        undone.contains("a = slider(value=2.0, min=0.5, max=5.0, step=0.5)\n"),
        "{undone}"
    );
    session.wait_scrub();
    let state = session.debug_state(false);
    assert!(queue_of(&state, "a").is_null(), "{}", state["scrub"]);
    assert_eq!(scrub_of(&state, "a")["on"], false);
}

// `warmed` means the position's MEMOIZABLE cone is stored (docs/12). A
// volatile node (`clock`) recomputes in every generation by design, so a
// tick on a warm position computes it alone — everything downstream of
// its unchanged value is the hit the warming stored. Two shapes: the clock
// BESIDE the slider's cone (feeding the same `add` — the review's
// pipeline) leaves the dry run exact, the committed value a hit as ever;
// the clock INSIDE the cone (`clock(speed=size)`) is a miss in every dry
// run, so no position is ever confirmed without a solve — each solves
// once, the committed value too. Either way `slider_loop.mjs --expect
// warm`, which counts every computed node, fails by construction on such
// a pipeline (the review's note, recorded 2026-08-24).
#[test]
fn a_volatile_node_in_the_cone_keeps_the_rest_warm() {
    let tick_computes_the_clock_alone = |session: &Session| {
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(ClientLanes::merged(tx));
        let _ = drain(&mut rx);
        session.handle(
            id,
            None,
            ClientMessage::ParamPreview {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.5".into(),
            },
        );
        session.wait_idle();
        let last = session.debug_state(false)["timings"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|t| t["kind"] == "preview")
            .cloned()
            .expect("the tick was a live preview generation");
        assert_eq!(last["computed"], 1, "the clock alone: {last}");
        assert_eq!(last["cached"], 3, "slider, add, domain: {last}");
    };

    // Beside the cone: `t` is upstream of `add`, not downstream of `size`,
    // so the dry run takes its hash from the last complete generation and
    // the load's 2.0 is a hit — nine solves for ten positions, as without
    // the clock.
    let (_dir, config) = project(
        "# cicada 1\n\
         size = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n\
         t = clock(speed=1.0)\n\
         r = add(a=size, b=t)\n\
         span = construct_domain(start=0.0, end=r)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle", "{}", state["scrub"]);
    assert_eq!(
        queue_of(&state, "size")["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    assert_eq!(scrub_of(&state, "size")["warming"], false);
    assert_eq!(hypothetical_solves(&session), 9);
    tick_computes_the_clock_alone(&session);
    drop(session);

    // Inside the cone: every dry run meets the volatile miss, so every
    // position solves once — the load's 2.0 too, computing the clock alone
    // (its add and domain are the load's hits).
    let (_dir, config) = project(
        "# cicada 1\n\
         size = slider(value=2.0, min=0.5, max=5.0, step=0.5, scrub=True)\n\
         t = clock(speed=size)\n\
         r = add(a=size, b=t)\n\
         span = construct_domain(start=0.0, end=r)\n",
    );
    let session = Session::open(config).unwrap();
    session.wait_idle();
    session.wait_scrub();
    let state = session.debug_state(false);
    assert_eq!(state["scrub"]["state"], "idle", "{}", state["scrub"]);
    assert_eq!(
        queue_of(&state, "size")["warmed"],
        serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    assert_eq!(hypothetical_solves(&session), 10, "no dry-run hit possible");
    let committed = state["timings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "hypothetical")
        .cloned()
        .unwrap();
    assert_eq!(
        committed["computed"], 1,
        "the load's own value: the clock alone recomputed: {committed}"
    );
    tick_computes_the_clock_alone(&session);
}
