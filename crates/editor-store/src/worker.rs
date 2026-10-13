use crate::{StoreError, recovery, save_project};
use editor_core::{Editor, SessionState};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct SaveResult {
    pub context_id: editor_core::AssetId,
    pub project_id: editor_core::AssetId,
    pub path: PathBuf,
    pub revision: u64,
    pub recovery: bool,
    pub result: Result<(), StoreError>,
}
struct Request {
    _lease: Option<Arc<crate::RecoveryLease>>,
    document: Option<Arc<crate::DocumentSession>>,
    recovery_binding: Option<crate::document::RecoveryDocumentBinding>,
    context_id: editor_core::AssetId,
    path: PathBuf,
    state: SessionState,
    recovery: bool,
}
struct State {
    pending: BTreeMap<(PathBuf, bool), Request>,
    results: VecDeque<SaveResult>,
    stopping: bool,
    active: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
pub struct SaveWorker {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl SaveWorker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: BTreeMap::new(),
                results: VecDeque::new(),
                stopping: false,
                active: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("aster-save".into())
            .spawn(move || run(worker_shared, wake))
            .ok();
        Self { shared, worker }
    }
    /// Queues recovery or a compatibility one-shot unconditional project write.
    /// Ordinary editing must use [`Self::request_document`] to retain its disk
    /// baseline. Call Editor::register_save before an explicit project save.
    pub fn request(
        &self,
        path: PathBuf,
        editor: &Editor,
        recovery: bool,
    ) -> Result<(), StoreError> {
        self.request_inner(path, editor, recovery, None, None)
    }
    /// Keep recovery ownership alive until the accepted snapshot has been
    /// durably written, even if its editor window switches projects or closes.
    pub fn request_with_lease(
        &self,
        path: PathBuf,
        editor: &Editor,
        recovery: bool,
        lease: Arc<crate::RecoveryLease>,
    ) -> Result<(), StoreError> {
        self.request_inner(path, editor, recovery, Some(lease), None)
    }

    /// Queues a protected ordinary-project save without filesystem I/O here.
    /// Call Editor::register_save first. Hold the same document session for all
    /// saves in one editing context, including while earlier writes are active.
    pub fn request_document(
        &self,
        document: Arc<crate::DocumentSession>,
        editor: &Editor,
    ) -> Result<(), StoreError> {
        self.request_inner(
            document.path().to_path_buf(),
            editor,
            false,
            None,
            Some(document),
        )
    }
    /// Queues a recovery snapshot and its immutable ordinary disk baseline.
    /// Capture happens at submission, in memory: later successful manual saves
    /// cannot rebaseline an older queued archive. Pass `None` for private or
    /// unsaved recovery. A binding is accepted only at its canonical destination;
    /// copying recovery to a per-window path must not bind the ordinary file.
    pub fn request_recovery(
        &self,
        path: PathBuf,
        editor: &Editor,
        lease: Arc<crate::RecoveryLease>,
        document: Option<Arc<crate::DocumentSession>>,
    ) -> Result<(), StoreError> {
        if document
            .as_ref()
            .is_some_and(|document| document.path() != path)
        {
            return Err(StoreError::Invalid(
                "Private recovery cannot bind an ordinary document".into(),
            ));
        }
        self.request_inner(path, editor, true, Some(lease), document)
    }
    fn request_inner(
        &self,
        path: PathBuf,
        editor: &Editor,
        recovery: bool,
        lease: Option<Arc<crate::RecoveryLease>>,
        document: Option<Arc<crate::DocumentSession>>,
    ) -> Result<(), StoreError> {
        if self.worker.is_none() {
            return Err(StoreError::WorkerStopped);
        }
        let recovery_binding = recovery
            .then(|| {
                document
                    .as_ref()
                    .map(|document| document.recovery_binding())
            })
            .flatten();
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopping {
            return Err(StoreError::WorkerStopped);
        }
        let key = (path.clone(), recovery);
        if !state.pending.contains_key(&key) && state.pending.len() >= 16 {
            return Err(StoreError::QueueFull);
        }
        state.pending.insert(
            key,
            Request {
                _lease: lease,
                document,
                recovery_binding,
                context_id: editor.instance_id(),
                path,
                state: editor.session_state(),
                recovery,
            },
        );
        self.shared.changed.notify_one();
        Ok(())
    }
    pub fn poll(&self) -> Option<SaveResult> {
        let result = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .results
            .pop_front();
        if result.is_some() {
            self.shared.changed.notify_one();
        }
        result
    }
    pub fn flush(&self, timeout: Duration) -> Result<(), StoreError> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.active || !state.pending.is_empty() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(StoreError::FlushTimeout);
            };
            let (next, wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            if wait.timed_out() && (state.active || !state.pending.is_empty()) {
                return Err(StoreError::FlushTimeout);
            }
        }
        Ok(())
    }
}
fn run(shared: Arc<Shared>, wake: impl Fn()) {
    loop {
        let request = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.pending.is_empty() && !state.stopping {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if state.pending.is_empty() && state.stopping {
                return;
            }
            // Backpressure includes completions; acknowledged saves must never be
            // lost, or a stale saved indicator could mask an unwritten document.
            while state.results.len() >= 32 && !state.stopping {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            let request = state.pending.pop_first().map(|(_, request)| request);
            state.active = request.is_some();
            request
        };
        let Some(request) = request else {
            continue;
        };
        let result = if request.recovery {
            recovery::save_with_binding(
                &crate::recovery_path(&request.path),
                &request.state,
                request.recovery_binding,
            )
        } else if let Some(document) = &request.document {
            crate::save_project_with_session(document, &request.state.current)
        } else {
            save_project(&request.path, &request.state.current)
        };
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.results.push_back(SaveResult {
            context_id: request.context_id,
            project_id: request.state.current.id,
            path: request.path,
            revision: request.state.revision,
            recovery: request.recovery,
            result,
        });
        state.active = false;
        shared.changed.notify_all();
        drop(state);
        wake();
    }
}
impl Drop for SaveWorker {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.stopping = true;
            self.shared.changed.notify_all();
        }
        let _ = self.flush(Duration::from_secs(2));
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
}
