//! Bounded off-frame native planning requests and immutable proposal delivery.
use crate::{AiConfig, AiContext, AiError, AiProposal, provider};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};
use uuid::Uuid;

#[derive(Debug)]
pub enum AiEvent {
    Started { id: Uuid },
    Proposal { id: Uuid, proposal: Box<AiProposal> },
    Failed { id: Uuid, message: String },
    Cancelled { id: Uuid },
}

struct Job {
    id: Uuid,
    config: AiConfig,
    context: AiContext,
    prompt: String,
    cancel: Arc<AtomicBool>,
}

struct State {
    pending: Option<Job>,
    active: Option<Arc<AtomicBool>>,
    events: VecDeque<AiEvent>,
    stopping: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// One worker, one active request and at most eight unread events.
/// Submission, polling, cancellation and Drop do no provider or filesystem I/O.
pub struct AiRuntime {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl AiRuntime {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, AiError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: None,
                active: None,
                events: VecDeque::new(),
                stopping: false,
            }),
            changed: Condvar::new(),
        });
        let owned = shared.clone();
        let worker = thread::Builder::new()
            .name("neptune-video-ai".into())
            .spawn(move || run(owned, wake))
            .map_err(|_| AiError::WorkerStopped)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub fn request(
        &self,
        config: AiConfig,
        context: AiContext,
        prompt: String,
    ) -> Result<Uuid, AiError> {
        config.validate()?;
        if prompt.trim().is_empty() || prompt.len() > 32 * 1024 || prompt.contains('\0') {
            return Err(AiError::Invalid("Enter a request of 1–32768 bytes".into()));
        }
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopping {
            return Err(AiError::WorkerStopped);
        }
        if state.active.is_some() || state.events.len() > 6 {
            return Err(AiError::Busy);
        }
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        state.active = Some(cancel.clone());
        state.pending = Some(Job {
            id,
            config,
            context,
            prompt,
            cancel,
        });
        self.shared.changed.notify_one();
        Ok(id)
    }

    pub fn poll(&self) -> Option<AiEvent> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .pop_front()
    }

    pub fn cancel(&self) {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cancel) = &state.active {
            cancel.store(true, Ordering::Release);
        }
        self.shared.changed.notify_one();
    }
}

fn run(shared: Arc<Shared>, wake: impl Fn()) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.pending.is_none() && !state.stopping {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if state.stopping {
                return;
            }
            let job = state.pending.take();
            if let Some(job) = &job {
                state.events.push_back(AiEvent::Started { id: job.id });
            }
            job
        };
        let Some(job) = job else {
            continue;
        };
        wake();
        let result = if job.cancel.load(Ordering::Acquire) {
            Err(AiError::Cancelled)
        } else {
            provider::plan(&job.config, &job.context, &job.prompt, job.cancel.clone()).and_then(
                |output| {
                    AiProposal::validate(
                        job.id,
                        job.context,
                        job.config.provider,
                        output.model,
                        &output.bytes,
                    )
                },
            )
        };
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        // Cancellation wins even if parsing/validation finished concurrently.
        let event = if job.cancel.load(Ordering::Acquire) {
            AiEvent::Cancelled { id: job.id }
        } else {
            match result {
                Ok(proposal) => AiEvent::Proposal {
                    id: job.id,
                    proposal: Box::new(proposal),
                },
                Err(AiError::Cancelled) => AiEvent::Cancelled { id: job.id },
                Err(error) => AiEvent::Failed {
                    id: job.id,
                    message: error.to_string(),
                },
            }
        };
        state.active = None;
        if !state.stopping {
            state.events.push_back(event);
        }
        drop(state);
        wake();
    }
}

impl Drop for AiRuntime {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.stopping = true;
            if let Some(cancel) = &state.active {
                cancel.store(true, Ordering::Release);
            }
            self.shared.changed.notify_one();
        }
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::AiProvider;
    use editor_core::{Command, Editor, Project, Time};
    use std::{
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn provider(script: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    fn terminal(runtime: &AiRuntime) -> AiEvent {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(event) = runtime.poll()
                && !matches!(event, AiEvent::Started { .. })
            {
                return event;
            }
            assert!(Instant::now() < deadline, "no terminal event");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn worker_delivers_validated_proposal_without_applying_it() {
        let (_directory, executable) = provider(
            "cat >/dev/null; printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"structured_output\":{\"summary\":\"Rename\",\"commands\":[{\"type\":\"rename_project\",\"name\":\"Reviewed\"}]}}'",
        );
        let mut editor = Editor::new(Project::new("Original")).unwrap();
        let runtime = AiRuntime::new(|| {}).unwrap();
        let id = runtime
            .request(
                AiConfig {
                    executable: Some(executable),
                    ..AiConfig::default()
                },
                AiContext::capture(&editor, &[], Time::ZERO),
                "Rename".into(),
            )
            .unwrap();
        let AiEvent::Proposal {
            id: delivered,
            proposal,
        } = terminal(&runtime)
        else {
            panic!("expected proposal");
        };
        assert_eq!(id, delivered);
        assert_eq!(editor.project().name, "Original");
        proposal.apply(&mut editor).unwrap();
        assert_eq!(editor.project().name, "Reviewed");
        assert!(editor.undo());
        assert_eq!(editor.project().name, "Original");
    }

    #[test]
    fn claude_launcher_disables_customizations_before_requesting_a_proposal() {
        let (_directory, executable) = provider(
            "safe=; for argument do if [ \"$argument\" = --safe-mode ]; then safe=yes; fi; done; \
             [ \"$safe\" = yes ] || exit 7; cat >/dev/null; \
             printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"structured_output\":{\"summary\":\"Isolated planning\",\"commands\":[]}}'",
        );
        let editor = Editor::default();
        let runtime = AiRuntime::new(|| {}).unwrap();
        let id = runtime
            .request(
                AiConfig {
                    executable: Some(executable),
                    ..AiConfig::default()
                },
                AiContext::capture(&editor, &[], Time::ZERO),
                "Inspect this owned project".into(),
            )
            .unwrap();
        let AiEvent::Proposal {
            id: delivered,
            proposal,
        } = terminal(&runtime)
        else {
            panic!("launcher omitted customization isolation");
        };
        assert_eq!(delivered, id);
        assert_eq!(proposal.summary, "Isolated planning");
        assert!(proposal.commands.is_empty());
        assert_eq!(editor.revision(), 0);
    }

    #[test]
    fn codex_launcher_disables_instruction_discovery_and_external_notifiers() {
        let frames = [
            serde_json::json!({"id":"init","result":{}}),
            serde_json::json!({"id":"account","result":{"requiresOpenaiAuth":false,"account":null}}),
            serde_json::json!({"id":"config","result":{"config":{}}}),
            serde_json::json!({"id":"thread","result":{
                "thread":{"id":"owned-thread"}, "model":"owned-fake-model",
                "approvalPolicy":"never", "approvalsReviewer":"user",
                "sandbox":{"type":"readOnly","networkAccess":false}, "instructionSources":[]
            }}),
            serde_json::json!({"id":"turn","result":{"turn":{
                "id":"owned-turn","items":[],"status":"inProgress","error":null
            }}}),
            serde_json::json!({"method":"turn/completed","params":{
                "threadId":"owned-thread", "turn":{"id":"owned-turn", "status":"completed", "error":null,
                    "items":[{"type":"agentMessage","phase":"final_answer",
                        "text":"{\"summary\":\"Isolated Codex planning\",\"commands\":[]}"}]
                }
            }}),
        ];
        let mut script = String::from(
            "docs=; fallback=; notify=; model_file=; for argument do case \"$argument\" in \
             project_doc_max_bytes=0) docs=yes;; \
             'project_doc_fallback_filenames=[]') fallback=yes;; \
             'notify=[]') notify=yes;; \
             model_instructions_file=\\\"*neptune-video-model-instructions.txt\\\") model_file=yes;; esac; done; \
             [ \"$docs$fallback$notify$model_file\" = yesyesyesyes ] || exit 7; \
             [ -f ./neptune-video-model-instructions.txt ] && [ -s ./neptune-video-model-instructions.txt ] || exit 8; ",
        );
        for frame in frames {
            // Fixture frames contain only our ASCII literals, never apostrophes
            // or user/provider input; shell quotes cannot introduce code here.
            script.push_str(&format!("printf '%s\\n' '{frame}'; "));
        }
        script.push_str("cat >/dev/null");
        let (_directory, executable) = provider(&script);
        let editor = Editor::default();
        let runtime = AiRuntime::new(|| {}).unwrap();
        let id = runtime
            .request(
                AiConfig {
                    provider: AiProvider::Codex,
                    executable: Some(executable),
                    ..AiConfig::default()
                },
                AiContext::capture(&editor, &[], Time::ZERO),
                "Inspect this owned project".into(),
            )
            .unwrap();
        let AiEvent::Proposal {
            id: delivered,
            proposal,
        } = terminal(&runtime)
        else {
            panic!("launcher omitted planning isolation");
        };
        assert_eq!(delivered, id);
        assert_eq!(proposal.summary, "Isolated Codex planning");
        assert!(proposal.commands.is_empty());
        assert_eq!(editor.revision(), 0);
    }

    #[test]
    fn busy_cancel_and_drop_do_not_wait_for_provider() {
        let (_directory, executable) = provider("sleep 60");
        let editor = Editor::default();
        let runtime = AiRuntime::new(|| {}).unwrap();
        let config = AiConfig {
            executable: Some(executable),
            ..AiConfig::default()
        };
        runtime
            .request(
                config.clone(),
                AiContext::capture(&editor, &[], Time::ZERO),
                "Wait".into(),
            )
            .unwrap();
        assert!(matches!(
            runtime.request(
                config,
                AiContext::capture(&editor, &[], Time::ZERO),
                "Second".into()
            ),
            Err(AiError::Busy)
        ));
        let started = Instant::now();
        runtime.cancel();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(matches!(terminal(&runtime), AiEvent::Cancelled { .. }));
        let started = Instant::now();
        drop(runtime);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn late_proposal_stays_bound_to_the_original_revision() {
        let (_directory, executable) = provider(
            "cat >/dev/null; sleep 0.08; printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"structured_output\":{\"summary\":\"Rename\",\"commands\":[{\"type\":\"rename_project\",\"name\":\"Stale\"}]}}'",
        );
        let mut editor = Editor::default();
        let runtime = AiRuntime::new(|| {}).unwrap();
        runtime
            .request(
                AiConfig {
                    provider: AiProvider::Claude,
                    executable: Some(executable),
                    ..AiConfig::default()
                },
                AiContext::capture(&editor, &[], Time::ZERO),
                "Rename".into(),
            )
            .unwrap();
        editor
            .dispatch(Command::RenameProject {
                name: "Human".into(),
            })
            .unwrap();
        let AiEvent::Proposal { proposal, .. } = terminal(&runtime) else {
            panic!("expected proposal");
        };
        assert!(matches!(proposal.apply(&mut editor), Err(AiError::Stale)));
        assert_eq!(editor.project().name, "Human");
    }

    #[test]
    fn missing_provider_reports_failure_without_touching_document() {
        let editor = Editor::default();
        let runtime = AiRuntime::new(|| {}).unwrap();
        runtime
            .request(
                AiConfig {
                    executable: Some(PathBuf::from("/neptune-video-no-such-native-provider")),
                    ..AiConfig::default()
                },
                AiContext::capture(&editor, &[], Time::ZERO),
                "Rename".into(),
            )
            .unwrap();
        assert!(matches!(terminal(&runtime), AiEvent::Failed { .. }));
        assert_eq!(editor.revision(), 0);
    }
}
