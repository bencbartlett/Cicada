//! The per-solve cancel handle (docs/12 §Cancellation: "one token per
//! generation, checked between nodes, between element chunks, and at safe
//! points inside long stdlib loops").
//!
//! Every `Scheduler::solve` call owns exactly one [`CancelToken`]; the
//! executor hands it to every node invocation through [`NodeCtx`], so a
//! node (or a host bridge — the Python worker pool, later the WASM epoch)
//! kills its own long-running work when ITS generation is cancelled and
//! nobody else's. Three generations can be in flight on one scheduler at
//! once — an explicit effectful run, the interactive latest-wins loop, and
//! an idle-class hypothetical solve — and cancelling one never touches
//! another, because there is no session-global switch to share: the token
//! IS the generation's identity (docs/13: "a slider drag never cancels an
//! export").
//!
//! **Hooks**: [`CancelToken::on_cancel`] runs a closure when the token is
//! cancelled (immediately, when it already is). This is how a host bridge
//! that cannot poll the token — a worker subprocess blocked in a call —
//! gets killed by construction: whoever cancels the token (Esc,
//! supersession, idle pre-emption, a fatal store error) kills the in-flight
//! script calls of that generation without knowing they exist. The hook's
//! guard removes it on drop, so a completed call leaves nothing behind.
//! `is_cancelled` stays a single atomic load — the hot path between
//! elements pays nothing for the hook machinery.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Process-wide token ids — diagnostics and bridges key by them; never
/// reused within a process.
static NEXT_TOKEN_ID: AtomicU64 = AtomicU64::new(1);

type Hook = Box<dyn FnOnce() + Send>;

struct TokenInner {
    id: u64,
    cancelled: AtomicBool,
    /// Registered hooks, by hook id. Drained (and run) exactly once, by the
    /// cancel that flips the flag; a later `on_cancel` runs immediately.
    hooks: Mutex<Vec<(u64, Hook)>>,
    next_hook: AtomicU64,
}

/// One generation's cancellation token. Cloneable; all clones share the
/// flag and the hooks.
#[derive(Clone)]
pub struct CancelToken(Arc<TokenInner>);

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("id", &self.0.id)
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancelToken {
    /// A fresh, uncancelled token with a process-unique id.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(TokenInner {
            id: NEXT_TOKEN_ID.fetch_add(1, Ordering::Relaxed),
            cancelled: AtomicBool::new(false),
            hooks: Mutex::new(Vec::new()),
            next_hook: AtomicU64::new(1),
        }))
    }

    /// The token's process-unique id (diagnostics; bridges that key
    /// in-flight work by generation).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// True when both handles are the same token (not merely equal state).
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Cancel. Idempotent, callable from any thread. Runs every registered
    /// hook exactly once, synchronously, on the calling thread — hooks must
    /// be cheap (flip a switch, bump an epoch) and must not call back into
    /// the scheduler.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let hooks = std::mem::take(
            &mut *self
                .0
                .hooks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, hook) in hooks {
            hook();
        }
    }

    /// Has anyone cancelled? One atomic load — the between-elements check.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Run `hook` when this token is cancelled — immediately, on this
    /// thread, when it already is. Dropping the returned guard before the
    /// cancel removes the hook (it will never run). Hooks run at most once.
    #[must_use = "dropping the guard unregisters the hook"]
    pub fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) -> CancelHook {
        let id = self.0.next_hook.fetch_add(1, Ordering::Relaxed);
        let hook: Hook = Box::new(hook);
        // The flag is read UNDER the hooks lock, and `cancel` flips it before
        // taking the lock: a hook registered after the flip either sees it
        // (runs now) or is in the vector the cancel drains. No window.
        let run_now = {
            let mut hooks = self
                .0
                .hooks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_cancelled() {
                Some(hook)
            } else {
                hooks.push((id, hook));
                None
            }
        };
        if let Some(hook) = run_now {
            hook();
        }
        CancelHook {
            token: self.clone(),
            id,
        }
    }

    fn remove_hook(&self, id: u64) {
        self.0
            .hooks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(hook_id, _)| *hook_id != id);
    }
}

/// A registered cancel hook; dropping it unregisters the hook (a no-op once
/// the hook has run).
#[must_use = "dropping the guard unregisters the hook"]
pub struct CancelHook {
    token: CancelToken,
    id: u64,
}

impl std::fmt::Debug for CancelHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelHook")
            .field("token", &self.token.id())
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for CancelHook {
    fn drop(&mut self) {
        self.token.remove_hook(self.id);
    }
}

/// What a node invocation sees of the generation running it. Passed to
/// every [`crate::NodeFn`] call by the executor; long-running nodes poll
/// `cancel` at safe points, host bridges hook it.
#[derive(Debug, Clone, Copy)]
pub struct NodeCtx<'a> {
    /// The generation's cancel handle — THIS solve's, never a neighbour's.
    pub cancel: &'a CancelToken,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn tokens_are_independent_and_identified() {
        let a = CancelToken::new();
        let b = CancelToken::new();
        assert_ne!(a.id(), b.id());
        assert!(a.same(&a.clone()));
        assert!(!a.same(&b));
        a.cancel();
        assert!(a.is_cancelled());
        assert!(
            !b.is_cancelled(),
            "cancelling one token never touches another"
        );
    }

    #[test]
    fn hooks_run_once_on_cancel_and_immediately_when_already_cancelled() {
        let token = CancelToken::new();
        let fired = Arc::new(AtomicUsize::new(0));
        let guard = {
            let fired = Arc::clone(&fired);
            token.on_cancel(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            })
        };
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        token.cancel();
        token.cancel();
        assert_eq!(fired.load(Ordering::SeqCst), 1, "a hook runs exactly once");
        drop(guard);
        // Late registration on a cancelled token runs now, on this thread.
        let late = Arc::new(AtomicUsize::new(0));
        let _late_guard = {
            let late = Arc::clone(&late);
            token.on_cancel(move || {
                late.fetch_add(1, Ordering::SeqCst);
            })
        };
        assert_eq!(late.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_the_guard_unregisters_the_hook() {
        let token = CancelToken::new();
        let fired = Arc::new(AtomicUsize::new(0));
        let guard = {
            let fired = Arc::clone(&fired);
            token.on_cancel(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            })
        };
        drop(guard);
        token.cancel();
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "a completed call's hook never fires after it is gone"
        );
    }

    #[test]
    fn clones_share_state_and_hooks() {
        let token = CancelToken::new();
        let clone = token.clone();
        let fired = Arc::new(AtomicUsize::new(0));
        let _guard = {
            let fired = Arc::clone(&fired);
            clone.on_cancel(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            })
        };
        token.cancel();
        assert!(clone.is_cancelled());
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }
}
