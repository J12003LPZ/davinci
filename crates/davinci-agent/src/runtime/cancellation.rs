//! Hierarchical cancellation tokens for agents, workers, and background jobs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

type CancellationCallback = Box<dyn Fn() + Send + Sync + 'static>;
type ChildList = Arc<Mutex<Vec<Weak<CancellationInner>>>>;
type CallbackList = Arc<Mutex<Vec<CancellationCallback>>>;

/// Inner state tracked for a cancellation token node.
pub struct CancellationInner {
    pub inner: Arc<AtomicBool>,
    pub children: ChildList,
    pub callbacks: CallbackList,
}

/// Hierarchical cancellation token.
/// Cancelling a parent propagates downward to all active children and attached callbacks.
/// Cancelling a child does not cancel the parent.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<AtomicBool>,
    children: ChildList,
    callbacks: CallbackList,
    node: Arc<CancellationInner>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    /// Create a new, independent root cancellation token.
    pub fn new() -> Self {
        let inner = Arc::new(AtomicBool::new(false));
        let children = Arc::new(Mutex::new(Vec::new()));
        let callbacks = Arc::new(Mutex::new(Vec::new()));
        let node = Arc::new(CancellationInner {
            inner: inner.clone(),
            children: children.clone(),
            callbacks: callbacks.clone(),
        });
        Self {
            inner,
            children,
            callbacks,
            node,
        }
    }

    /// Check if this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    /// Exposes an `Arc<AtomicBool>` view suitable for APIs expecting a raw flag (e.g. `davinci-ai`).
    pub fn as_atomic_bool(&self) -> Arc<AtomicBool> {
        self.inner.clone()
    }

    /// Cancel this token and propagate downward to all attached child tokens and callbacks.
    pub fn cancel(&self) {
        if self.inner.swap(true, Ordering::SeqCst) {
            // Already cancelled
            return;
        }

        // Fire callbacks
        let callbacks = {
            let mut guard = self.callbacks.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        for cb in callbacks {
            cb();
        }

        // Propagate to child tokens
        let children: Vec<Arc<CancellationInner>> = {
            let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
            let list = guard.iter().filter_map(|w| w.upgrade()).collect();
            guard.retain(|w| w.strong_count() > 0);
            list
        };
        for child_node in children {
            let child_token = CancellationToken {
                inner: child_node.inner.clone(),
                children: child_node.children.clone(),
                callbacks: child_node.callbacks.clone(),
                node: child_node,
            };
            child_token.cancel();
        }
    }

    /// Creates a child token attached to this parent.
    /// When this parent is cancelled, the child will also be cancelled.
    /// Cancelling the child does not cancel this parent.
    pub fn child_token(&self) -> Self {
        let child = Self::new();
        if self.is_cancelled() {
            child.cancel();
        } else {
            let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(Arc::downgrade(&child.node));
        }
        child
    }

    /// Creates a detached child token.
    /// Detached means it does NOT inherit cancellation from `self` (e.g. a parent turn),
    /// but if `ancestor` (e.g. a session or run root) is supplied, it inherits from `ancestor`.
    pub fn detached_child(&self, ancestor: Option<&CancellationToken>) -> Self {
        if let Some(anc) = ancestor {
            anc.child_token()
        } else {
            Self::new()
        }
    }

    /// Register a callback to execute immediately when cancelled, or synchronously now if already cancelled.
    pub fn on_cancel<F>(&self, f: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut guard = self.callbacks.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() {
            drop(guard);
            f();
        } else {
            guard.push(Box::new(f));
        }
    }

    /// Attach a `JobBook` so that cancelling this token immediately executes `JobBook::kill_all()`.
    pub fn attach_job_book(&self, jobs: Arc<Mutex<crate::jobs::JobBook>>) {
        self.on_cancel(move || {
            if let Ok(mut book) = jobs.lock() {
                book.kill_all();
            }
        });
    }

    /// Bind the host's shared job book to its current runtime only.
    pub(crate) fn bind_job_book(&self, jobs: &Arc<Mutex<crate::jobs::JobBook>>) {
        let owner = Arc::downgrade(&self.inner);
        {
            let mut book = jobs.lock().unwrap_or_else(|e| e.into_inner());
            if book
                .cancellation_owner
                .as_ref()
                .is_some_and(|current| current.ptr_eq(&owner))
            {
                return;
            }
            book.cancellation_owner = Some(owner.clone());
        }
        let jobs = Arc::downgrade(jobs);
        self.on_cancel(move || {
            if let Some(jobs) = jobs.upgrade() {
                let mut book = jobs.lock().unwrap_or_else(|e| e.into_inner());
                if book
                    .cancellation_owner
                    .as_ref()
                    .is_some_and(|current| current.ptr_eq(&owner))
                {
                    book.kill_all();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f03_job_binding_is_bounded_and_does_not_retain_book() {
        let token = CancellationToken::new();
        let jobs = Arc::new(Mutex::new(crate::jobs::JobBook::default()));
        let weak = Arc::downgrade(&jobs);
        token.bind_job_book(&jobs);
        token.bind_job_book(&jobs);
        assert_eq!(token.callbacks.lock().unwrap().len(), 1);
        drop(jobs);
        assert!(weak.upgrade().is_none());
        token.cancel();
    }

    #[test]
    fn cancellation_registration_runs_once_across_cancel_race() {
        for _ in 0..64 {
            let token = CancellationToken::new();
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let start = Arc::new(std::sync::Barrier::new(2));
            std::thread::scope(|scope| {
                let token = &token;
                let start = &start;
                let count = count.clone();
                scope.spawn(move || {
                    start.wait();
                    token.on_cancel(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                    });
                });
                start.wait();
                token.cancel();
            });
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn test_parent_cancellation_reaches_child() {
        let parent = CancellationToken::new();
        let child1 = parent.child_token();
        let child2 = child1.child_token();

        assert!(!parent.is_cancelled());
        assert!(!child1.is_cancelled());
        assert!(!child2.is_cancelled());

        parent.cancel();

        assert!(parent.is_cancelled());
        assert!(child1.is_cancelled());
        assert!(child2.is_cancelled());
        assert!(child2.as_atomic_bool().load(Ordering::SeqCst));
    }

    #[test]
    fn test_child_cancel_does_not_cancel_parent() {
        let parent = CancellationToken::new();
        let child = parent.child_token();

        child.cancel();

        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_detached_child_cancellation() {
        let root = CancellationToken::new();
        let turn = root.child_token();
        let detached = turn.detached_child(Some(&root));

        assert!(!root.is_cancelled());
        assert!(!turn.is_cancelled());
        assert!(!detached.is_cancelled());

        // Cancelling the turn does NOT cancel the detached child
        turn.cancel();
        assert!(turn.is_cancelled());
        assert!(!detached.is_cancelled());
        assert!(!root.is_cancelled());

        // Cancelling the root DOES cancel the detached child
        root.cancel();
        assert!(root.is_cancelled());
        assert!(detached.is_cancelled());
    }

    #[test]
    fn test_cancellation_reaches_jobs() {
        let token = CancellationToken::new();
        let jobs = Arc::new(Mutex::new(crate::jobs::JobBook::default()));
        token.attach_job_book(jobs.clone());

        assert_eq!(jobs.lock().unwrap().running(), 0);
        token.cancel();
        // jobs was notified and kill_all executed without panics
        assert!(token.is_cancelled());
    }
}
