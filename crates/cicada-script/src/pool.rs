//! The Python worker pool: persistent subprocesses, length-framed
//! `MessagePack` requests, kill-the-worker cancellation (docs/12 §The
//! escape hatch — a killed worker costs one respawn, and Esc always
//! works).
//!
//! Concurrency shape: each worker owns a DETACHED reader thread feeding a
//! channel; calls write a request and wait on the channel with a kill
//! poll. A kill terminates the child and returns immediately — the reader
//! thread unblocks on pipe EOF on its own (and if a script's grandchild
//! process holds the pipe open, the reader parks harmlessly until it
//! exits — nothing ever JOINS it on the cancel path, so Esc never hangs).
//!
//! Script SOURCE travels with every request: the bytes the host hashed
//! for the cache key are the bytes the worker executes, by construction —
//! no time-of-check/time-of-use window against concurrent file edits.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada_core::value::HashedValue;
use rmpv::Value as Wire;

use crate::{ScriptError, value};

/// Hard-cancellation flag for in-flight script calls. Cloneable; all
/// clones share the flag. The integration layer wires the scheduler's
/// `CancelToken` to this (the dependency law keeps `cicada-script` off
/// `cicada-sched`).
#[derive(Debug, Clone, Default)]
pub struct KillSwitch(Arc<AtomicBool>);

impl KillSwitch {
    /// A fresh, unkilled switch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Kill: every in-flight call on this switch has its worker killed.
    pub fn kill(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Has anyone killed?
    #[must_use]
    pub fn is_killed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One input port of a described script node.
#[derive(Debug, Clone)]
pub struct PortDesc {
    /// Port name (the Python parameter name).
    pub name: String,
    /// Catalog type notation from the annotation (`"[Point]"`, `"Number"`).
    pub ty: String,
    /// The Python-side default, marshalled (None = required port).
    pub default: Option<Arc<HashedValue>>,
}

/// One `@cicada.node` function found in a script file.
#[derive(Debug, Clone)]
pub struct ScriptNodeDesc {
    /// Dialect name = the Python function name.
    pub name: String,
    /// Node title.
    pub title: String,
    /// One-line description.
    pub description: String,
    /// Input ports in signature order.
    pub inputs: Vec<PortDesc>,
    /// Catalog type notation of the single `out` port.
    pub output: String,
}

/// A described script file: its nodes plus the interpreter identity
/// (`sys.version`) — cache-key material (docs/12: script `node_version` =
/// source + toolchain version).
#[derive(Debug, Clone)]
pub struct Described {
    /// The `@cicada.node` functions, in the worker's reported order.
    pub nodes: Vec<ScriptNodeDesc>,
    /// The interpreter's `sys.version` string.
    pub python_version: String,
}

/// The pool. Thread-safe; workers are checked out per call and returned
/// on success (a failed or killed call's worker is discarded — respawn is
/// the recovery; a request-write failure retries once on a fresh worker,
/// so a stale dead idle worker never fails a healthy call).
pub struct WorkerPool {
    python: PathBuf,
    worker_file: tempfile::NamedTempFile,
    idle: Mutex<Vec<Worker>>,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    /// Frames from the detached reader thread; an `Err` is the reader's
    /// terminal report (EOF/IO), after which it exits.
    frames: Receiver<Result<Wire, String>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing stdin makes the loop exit; kill is the backstop. The
        // reader thread ends on pipe EOF — never joined, never waited on.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Resolve the interpreter: `CICADA_PYTHON`, else the first of
/// `python`/`python3` that answers `--version`.
fn find_python() -> Result<PathBuf, ScriptError> {
    if let Ok(explicit) = std::env::var("CICADA_PYTHON") {
        return Ok(PathBuf::from(explicit));
    }
    for candidate in ["python", "python3"] {
        let probe = Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if probe.is_ok_and(|status| status.success()) {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err(ScriptError::NoPython {
        tried: "CICADA_PYTHON, python, python3".to_owned(),
    })
}

/// Read one length-framed `MessagePack` value (the reader thread's loop
/// body).
fn read_frame(stdout: &mut ChildStdout) -> Result<Wire, String> {
    let mut header = [0_u8; 4];
    stdout
        .read_exact(&mut header)
        .map_err(|error| format!("worker closed its pipe: {error}"))?;
    let length = u32::from_le_bytes(header) as usize;
    let mut body = vec![0_u8; length];
    stdout
        .read_exact(&mut body)
        .map_err(|error| format!("worker died mid-frame: {error}"))?;
    rmpv::decode::read_value(&mut body.as_slice()).map_err(|error| error.to_string())
}

impl WorkerPool {
    /// Create the pool (resolves the interpreter, materializes the worker
    /// program; spawns nothing yet).
    ///
    /// # Errors
    ///
    /// [`ScriptError::NoPython`] / [`ScriptError::Io`].
    pub fn new() -> Result<Self, ScriptError> {
        let python = find_python()?;
        let mut worker_file = tempfile::Builder::new()
            .prefix("cicada-worker-")
            .suffix(".py")
            .tempfile()?;
        worker_file.write_all(crate::WORKER_SOURCE.as_bytes())?;
        worker_file.flush()?;
        Ok(Self {
            python,
            worker_file,
            idle: Mutex::new(Vec::new()),
        })
    }

    fn spawn_worker(&self) -> Result<Worker, ScriptError> {
        let mut child = Command::new(&self.python)
            .arg(self.worker_file.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ScriptError::Protocol("worker spawned without stdin".to_owned()))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| ScriptError::Protocol("worker spawned without stdout".to_owned()))?;
        let (sender, frames) = std::sync::mpsc::channel();
        // Detached on purpose: the cancel path must never join a thread
        // that may be blocked on a pipe a grandchild still holds open.
        std::thread::Builder::new()
            .name("cicada-worker-reader".to_owned())
            .spawn(move || {
                loop {
                    let frame = read_frame(&mut stdout);
                    let terminal = frame.is_err();
                    if sender.send(frame).is_err() || terminal {
                        return; // pool side gone, or pipe closed
                    }
                }
            })?;
        Ok(Worker {
            child,
            stdin,
            frames,
        })
    }

    fn checkout(&self) -> Result<Worker, ScriptError> {
        if let Some(worker) = self
            .idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
        {
            return Ok(worker);
        }
        self.spawn_worker()
    }

    fn checkin(&self, worker: Worker) {
        self.idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
    }

    /// One request/response round-trip, kill-cancellable. A request-write
    /// failure (stale dead idle worker) retries ONCE on a fresh worker.
    fn call(&self, request: &Wire, kill: &KillSwitch) -> Result<Wire, ScriptError> {
        let mut frame = Vec::new();
        rmpv::encode::write_value(&mut frame, request)
            .map_err(|error| ScriptError::Protocol(error.to_string()))?;
        let length = u32::try_from(frame.len())
            .map_err(|_| ScriptError::Protocol("request over 4 GiB".to_owned()))?;

        let mut worker = self.checkout()?;
        if write_request(&mut worker, length, &frame).is_err() {
            // The idle worker died since its last call; one fresh retry.
            drop(worker);
            worker = self.spawn_worker()?;
            write_request(&mut worker, length, &frame)?;
        }

        loop {
            match worker.frames.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(reply)) => {
                    let outcome = decode_reply(&reply);
                    if outcome.is_ok() {
                        self.checkin(worker);
                    }
                    return outcome;
                }
                Ok(Err(reader_error)) => {
                    return Err(ScriptError::Protocol(format!(
                        "worker died without answering: {reader_error} (its stderr \
                         above carries the Python-side reason, if any)"
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if kill.is_killed() {
                        let _ = worker.child.kill();
                        return Err(ScriptError::Cancelled);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(ScriptError::Protocol(
                        "worker reader vanished without a report".to_owned(),
                    ));
                }
            }
        }
    }

    /// Describe the `@cicada.node` functions of one script SOURCE (the
    /// path labels tracebacks and diagnostics only).
    ///
    /// # Errors
    ///
    /// [`ScriptError`]: interpreter/protocol trouble, or a script-side
    /// import/signature failure (with traceback).
    pub fn describe(&self, path: &Path, source: &str) -> Result<Described, ScriptError> {
        let request = Wire::Map(vec![
            (Wire::from("op"), Wire::from("describe")),
            (
                Wire::from("path"),
                Wire::from(path.to_string_lossy().as_ref()),
            ),
            (Wire::from("source"), Wire::from(source)),
        ]);
        let reply = self.call(&request, &KillSwitch::new())?;
        let python_version = map_get(&reply, "python")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ScriptError::Protocol("describe reply has no python version".to_owned())
            })?
            .to_owned();
        let nodes = map_get(&reply, "nodes")
            .and_then(|nodes| nodes.as_array())
            .ok_or_else(|| ScriptError::Protocol("describe reply has no nodes".to_owned()))?
            .iter()
            .map(decode_node_desc)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Described {
            nodes,
            python_version,
        })
    }

    /// Invoke one script node over its SOURCE text. `inputs` maps port
    /// names to values; absent optional ports are omitted (the Python
    /// default applies — one source of default truth).
    ///
    /// # Errors
    ///
    /// [`ScriptError`]: including [`ScriptError::Cancelled`] when the
    /// switch killed the worker mid-call, and [`ScriptError::Script`]
    /// (with traceback) when the script raised.
    pub fn invoke(
        &self,
        path: &Path,
        source: &str,
        fn_name: &str,
        inputs: &BTreeMap<String, Arc<HashedValue>>,
        kill: &KillSwitch,
    ) -> Result<Arc<HashedValue>, ScriptError> {
        let mut wire_inputs = Vec::with_capacity(inputs.len());
        for (name, input) in inputs {
            wire_inputs.push((Wire::from(name.as_str()), value::to_wire(input)?));
        }
        let request = Wire::Map(vec![
            (Wire::from("op"), Wire::from("invoke")),
            (
                Wire::from("path"),
                Wire::from(path.to_string_lossy().as_ref()),
            ),
            (Wire::from("source"), Wire::from(source)),
            (Wire::from("fn"), Wire::from(fn_name)),
            (Wire::from("inputs"), Wire::Map(wire_inputs)),
        ]);
        let reply = self.call(&request, kill)?;
        let output = map_get(&reply, "output")
            .ok_or_else(|| ScriptError::Protocol("invoke reply has no output".to_owned()))?;
        value::from_wire(output)
    }
}

fn write_request(worker: &mut Worker, length: u32, frame: &[u8]) -> Result<(), ScriptError> {
    worker.stdin.write_all(&length.to_le_bytes())?;
    worker.stdin.write_all(frame)?;
    worker.stdin.flush()?;
    Ok(())
}

fn map_get<'w>(wire: &'w Wire, key: &str) -> Option<&'w Wire> {
    let Wire::Map(map) = wire else { return None };
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}

/// `{"ok": bool, ...}` → the reply, or the script's error.
fn decode_reply(reply: &Wire) -> Result<Wire, ScriptError> {
    match map_get(reply, "ok").and_then(rmpv::Value::as_bool) {
        Some(true) => Ok(reply.clone()),
        Some(false) => Err(ScriptError::Script(
            map_get(reply, "error")
                .and_then(|e| e.as_str())
                .unwrap_or("script failed with no message")
                .trim_end()
                .to_owned(),
        )),
        None => Err(ScriptError::Protocol(format!(
            "reply has no `ok` field: {reply}"
        ))),
    }
}

fn decode_node_desc(wire: &Wire) -> Result<ScriptNodeDesc, ScriptError> {
    let text = |key: &str| {
        map_get(wire, key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| ScriptError::Protocol(format!("node desc missing `{key}`")))
    };
    let inputs = map_get(wire, "inputs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ScriptError::Protocol("node desc missing inputs".to_owned()))?
        .iter()
        .map(|port| {
            let name = map_get(port, "name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ScriptError::Protocol("port missing name".to_owned()))?
                .to_owned();
            let ty = map_get(port, "type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ScriptError::Protocol(format!("port `{name}` missing type")))?
                .to_owned();
            let default = match map_get(port, "default") {
                None | Some(Wire::Nil) => None,
                Some(present) => Some(value::from_wire(present)?),
            };
            Ok(PortDesc { name, ty, default })
        })
        .collect::<Result<Vec<_>, ScriptError>>()?;
    Ok(ScriptNodeDesc {
        name: text("name")?,
        title: text("title")?,
        description: text("description")?,
        inputs,
        output: text("output")?,
    })
}
