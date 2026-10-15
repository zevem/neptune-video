//! A conversation with the person's signed-in Claude or Codex whose tools are
//! one Aster MCP server's. The provider edits through that server, so every
//! edit is the same guarded request an outside agent sends, validated and
//! undoable where it lands. This runtime only owns the provider's process
//! and reports what it said and did; it never edits.
//!
//! One worker, one conversation and one request at a time. Asking, polling,
//! stopping and dropping do no provider or filesystem I/O.
use crate::{AiConfig, AiError, AiProvider, provider};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// What the provider is told, before the person's first request.
pub const INSTRUCTIONS: &str = "\
You are the editing agent inside Aster, a video editor. The person is \
working in Aster's window and asked you for something in its Agent panel. \
You work on the project open in that window through the aster tools, \
which are the only tools you have. Every edit you make appears in the \
window at once and is one step of its undo history, which the person \
shares with you.

Look before you change: project_inspect, media_inspect, frame_render and \
contact_sheet show you the project and its pictures. Pass \
expected_revision on edits so a change the person made meanwhile is never \
overwritten. If a tool says person_editing or desktop_busy, the person is \
in the middle of something: wait a moment and try again. Check your work \
by looking at frames, and fix what you see. Long work (transcription, \
scene and loudness analysis, exports) runs as jobs: start it, then wait \
with the job tool.

You start on the project open in that window. Like any agent with these \
tools you may also create, open, save and close projects: one that no \
window shows is edited by the server itself and saved after every edit; \
desktop {op: open} shows it in a new window, and desktop {op: attach} \
naming no window brings you back to the person's window. Say so when you \
leave the person's project, and come back to it unless asked otherwise. \
project_save with a destination saves a copy as a new file; a window \
writes it on its own time and project_status shows last_save when it is \
done. Only folders the person allowed can be used; a path outside them is \
refused, and the person can add one in Settings \u{203a} Agent. Undo only your own edits unless asked. \
Ask the person instead of guessing when the request is unclear, and say \
plainly what you could not do. Answer briefly, in the person's language, \
and say what you changed.";

/// The longest a conversation's process may live.
const LIFETIME_SECONDS: u64 = 12 * 60 * 60;
/// How long a request may go without a word from the provider.
const SILENCE: Duration = Duration::from_secs(600);
/// Unread events kept; the oldest steps go first, never a request's end.
const MOST_EVENTS: usize = 4096;
const MOST_PROMPT_BYTES: usize = 64 * 1024;

/// The MCP server a conversation edits through: a program and its
/// arguments, passed to the provider as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolServer {
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// What happened in a request, in order. `Started` comes first and exactly
/// one of `Finished`, `Failed` or `Cancelled` comes last.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnEvent {
    /// The request reached the provider; `fresh` when a new conversation
    /// was started for it.
    Started {
        turn: Uuid,
        fresh: bool,
    },
    Said {
        turn: Uuid,
        text: String,
    },
    /// A tool call began.
    Called {
        turn: Uuid,
        call: String,
        tool: String,
        op: Option<String>,
    },
    /// A tool call ended: its edit's label, or why it was refused.
    Returned {
        turn: Uuid,
        call: String,
        ok: bool,
        detail: Option<String>,
    },
    /// Answered; the conversation waits for the next request.
    Finished {
        turn: Uuid,
        model: Option<String>,
    },
    /// The request failed and its conversation ended.
    Failed {
        turn: Uuid,
        message: String,
    },
    /// Stopped by the person; its conversation ended.
    Cancelled {
        turn: Uuid,
    },
}

impl TurnEvent {
    pub fn turn(&self) -> Uuid {
        match self {
            TurnEvent::Started { turn, .. }
            | TurnEvent::Said { turn, .. }
            | TurnEvent::Called { turn, .. }
            | TurnEvent::Returned { turn, .. }
            | TurnEvent::Finished { turn, .. }
            | TurnEvent::Failed { turn, .. }
            | TurnEvent::Cancelled { turn } => *turn,
        }
    }

    fn ends(&self) -> bool {
        matches!(
            self,
            TurnEvent::Finished { .. } | TurnEvent::Failed { .. } | TurnEvent::Cancelled { .. }
        )
    }
}

/// Who a conversation is with: a change of any of these starts a new one.
#[derive(Clone, PartialEq)]
struct Party {
    provider: AiProvider,
    executable: Option<PathBuf>,
    model: Option<String>,
    effort: Option<String>,
    server: ToolServer,
}

struct Ask {
    turn: Uuid,
    config: AiConfig,
    server: ToolServer,
    prompt: String,
}

struct State {
    pending: Option<Ask>,
    /// The request being answered.
    active: Option<Uuid>,
    /// Stop the request being answered and end the conversation.
    stop: bool,
    /// End the conversation once no request is being answered.
    end: bool,
    stopping: bool,
    events: VecDeque<TurnEvent>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// Raised to kill the provider at once, from any thread.
    kill: Mutex<Arc<AtomicBool>>,
}

pub struct Conversation {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Conversation {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, AiError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: None,
                active: None,
                stop: false,
                end: false,
                stopping: false,
                events: VecDeque::new(),
            }),
            changed: Condvar::new(),
            kill: Mutex::new(Arc::new(AtomicBool::new(false))),
        });
        let owned = shared.clone();
        let worker = thread::Builder::new()
            .name("aster-agent".into())
            .spawn(move || run(owned, wake))
            .map_err(|_| AiError::WorkerStopped)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    /// Sends `prompt` in the conversation, starting one if there is none or
    /// if the provider, model or server changed.
    pub fn ask(
        &self,
        config: AiConfig,
        server: ToolServer,
        prompt: String,
    ) -> Result<Uuid, AiError> {
        config.validate()?;
        if prompt.trim().is_empty() || prompt.len() > MOST_PROMPT_BYTES || prompt.contains('\0') {
            return Err(AiError::Invalid("Enter a request of 1–65536 bytes".into()));
        }
        let mut state = self.shared.lock();
        if state.stopping {
            return Err(AiError::WorkerStopped);
        }
        if state.active.is_some() || state.pending.is_some() {
            return Err(AiError::Busy);
        }
        let turn = Uuid::new_v4();
        state.active = Some(turn);
        state.pending = Some(Ask {
            turn,
            config,
            server,
            prompt,
        });
        self.shared.changed.notify_one();
        Ok(turn)
    }

    /// Stops the request being answered. Its provider is stopped at once, so
    /// the conversation ends with it; edits it already made stay, each its
    /// own step of the history.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        if state.active.is_some() {
            state.stop = true;
            self.shared
                .kill
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .store(true, Ordering::Release);
        }
        self.shared.changed.notify_one();
    }

    /// Ends the conversation: the next request starts a new one, which
    /// remembers nothing of this one. A request being answered is stopped.
    pub fn end(&self) {
        self.stop();
        let mut state = self.shared.lock();
        state.end = true;
        self.shared.changed.notify_one();
    }

    pub fn poll(&self) -> Option<TurnEvent> {
        self.shared.lock().events.pop_front()
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tell(&self, event: TurnEvent) {
        let mut state = self.lock();
        if state.stopping {
            return;
        }
        if state.events.len() >= MOST_EVENTS
            && let Some(at) = state.events.iter().position(|event| !event.ends())
        {
            state.events.remove(at);
        }
        if event.ends() && state.active == Some(event.turn()) {
            state.active = None;
            state.stop = false;
        }
        state.events.push_back(event);
    }
}

/// The provider's process, and who it is.
struct Live {
    party: Party,
    process: provider::Process,
    agent: provider::Agent,
}

fn run(shared: Arc<Shared>, wake: impl Fn()) {
    let mut live: Option<Live> = None;
    let mut answering: Option<Uuid> = None;
    let mut heard_at = Instant::now();
    loop {
        // What the person asked for since the last look.
        let (ask, stop, end) = {
            let mut state = shared.lock();
            if live.is_none() && answering.is_none() {
                while state.pending.is_none() && !state.end && !state.stopping {
                    state = shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner());
                }
            }
            if state.stopping {
                return;
            }
            // An end waits for the request being answered to stop.
            let end = answering.is_none() && std::mem::take(&mut state.end);
            (state.pending.take(), state.stop, end)
        };
        if stop && let Some(turn) = answering.take() {
            live = None;
            shared.tell(TurnEvent::Cancelled { turn });
            wake();
        }
        if end {
            live = None;
        }
        if let Some(ask) = ask {
            if stop {
                // Stopped before it was sent.
                shared.tell(TurnEvent::Cancelled { turn: ask.turn });
                wake();
                continue;
            }
            match begin(&shared, &mut live, ask) {
                Ok((turn, fresh)) => {
                    answering = Some(turn);
                    heard_at = Instant::now();
                    shared.tell(TurnEvent::Started { turn, fresh });
                }
                Err((turn, error)) => {
                    live = None;
                    shared.tell(ended(turn, error));
                }
            }
            wake();
        }
        let Some(current) = live.as_mut() else {
            continue;
        };
        // Between requests the process is only watched, to notice its end.
        let wait = if answering.is_some() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(250)
        };
        match current.process.poll(wait) {
            Ok(Some(frame)) => {
                heard_at = Instant::now();
                match current.agent.receive(&mut current.process, &frame) {
                    Ok(heard) => {
                        let Some(turn) = answering else {
                            continue;
                        };
                        for heard in heard {
                            if let Some(event) = told(turn, heard) {
                                if event.ends() {
                                    answering = None;
                                }
                                shared.tell(event);
                            }
                        }
                        wake();
                    }
                    Err(error) => {
                        live = None;
                        if let Some(turn) = answering.take() {
                            shared.tell(ended(turn, error));
                            wake();
                        }
                    }
                }
            }
            Ok(None) => {
                if let Some(turn) = answering
                    && heard_at.elapsed() > SILENCE
                {
                    live = None;
                    answering = None;
                    shared.tell(TurnEvent::Failed {
                        turn,
                        message: "The agent said nothing for ten minutes and was stopped".into(),
                    });
                    wake();
                }
            }
            Err(error) => {
                live = None;
                if let Some(turn) = answering.take() {
                    shared.tell(ended(turn, error));
                    wake();
                }
            }
        }
    }
}

/// Sends `ask` in the live conversation, or in a new one. Answers the turn
/// and whether its conversation is new.
fn begin(
    shared: &Shared,
    live: &mut Option<Live>,
    ask: Ask,
) -> Result<(Uuid, bool), (Uuid, AiError)> {
    let party = Party {
        provider: ask.config.provider,
        executable: ask.config.executable.clone(),
        model: ask.config.model.clone(),
        effort: ask.config.effort.clone(),
        server: ask.server.clone(),
    };
    if live.as_ref().is_some_and(|live| live.party != party) {
        *live = None;
    }
    let fresh = live.is_none();
    if fresh {
        let kill = Arc::new(AtomicBool::new(false));
        *shared.kill.lock().unwrap_or_else(|e| e.into_inner()) = kill.clone();
        // A stop that came while the provider was being looked for.
        if shared.lock().stop {
            return Err((ask.turn, AiError::Cancelled));
        }
        let (process, agent) = provider::converse(&ask.config, &ask.server, kill, LIFETIME_SECONDS)
            .map_err(|error| (ask.turn, error))?;
        *live = Some(Live {
            party,
            process,
            agent,
        });
    }
    let Some(current) = live.as_mut() else {
        return Err((ask.turn, AiError::WorkerStopped));
    };
    current
        .agent
        .ask(&mut current.process, &ask.prompt)
        .map_err(|error| (ask.turn, error))?;
    Ok((ask.turn, fresh))
}

fn told(turn: Uuid, heard: provider::Heard) -> Option<TurnEvent> {
    use provider::Heard;
    Some(match heard {
        Heard::Said(text) => TurnEvent::Said { turn, text },
        Heard::Called { call, tool, op } => TurnEvent::Called {
            turn,
            call,
            tool,
            op,
        },
        Heard::Returned { call, ok, detail } => TurnEvent::Returned {
            turn,
            call,
            ok,
            detail,
        },
        Heard::Finished { model } => TurnEvent::Finished { turn, model },
        Heard::Send(_) => return None,
    })
}

fn ended(turn: Uuid, error: AiError) -> TurnEvent {
    match error {
        AiError::Cancelled => TurnEvent::Cancelled { turn },
        error => TurnEvent::Failed {
            turn,
            message: error.to_string(),
        },
    }
}

impl Drop for Conversation {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.stopping = true;
            self.shared
                .kill
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .store(true, Ordering::Release);
            self.shared.changed.notify_one();
        }
        // The provider is killed by its watchdog; a worker still unwinding
        // is not waited for.
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
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for `claude` that answers each request on its input with
    /// one tool call and one reply, in Claude's stream JSON.
    fn fake_claude(directory: &std::path::Path) -> PathBuf {
        let path = directory.join("claude");
        std::fs::write(
            &path,
            r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","model":"fake","mcp_servers":[{"name":"aster","status":"connected"}]}'
n=0
while read -r line; do
  n=$((n+1))
  case "$line" in *never*) sleep 30;; esac
  printf '%s\n' "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"t$n\",\"name\":\"mcp__aster__clip_edit\",\"input\":{\"op\":\"trim\"}}]}}"
  printf '%s\n' "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t$n\",\"content\":\"{\\\"label\\\":\\\"Trim clip\\\",\\\"changed\\\":true}\"}]}}"
  printf '%s\n' "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Answer $n\"}]}}"
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"ok","modelUsage":{"fake":{}}}'
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn config(executable: PathBuf) -> AiConfig {
        AiConfig {
            provider: AiProvider::Claude,
            executable: Some(executable),
            ..AiConfig::default()
        }
    }

    fn server() -> ToolServer {
        ToolServer {
            command: PathBuf::from("/opt/aster/aster-mcp"),
            args: vec!["--window".into(), "/tmp/window.json".into()],
        }
    }

    /// The events of `turn`, up to its end.
    fn answer(conversation: &Conversation, turn: Uuid) -> Vec<TurnEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        loop {
            if let Some(event) = conversation.poll() {
                assert_eq!(event.turn(), turn);
                let ends = event.ends();
                events.push(event);
                if ends {
                    return events;
                }
            }
            assert!(Instant::now() < deadline, "no end: {events:?}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_conversation_keeps_its_provider_between_requests_and_reports_each_step() {
        let directory = tempfile::tempdir().unwrap();
        let claude = fake_claude(directory.path());
        let conversation = Conversation::new(|| {}).unwrap();
        let first = conversation
            .ask(config(claude.clone()), server(), "Trim the start".into())
            .unwrap();
        assert!(matches!(
            conversation.ask(config(claude.clone()), server(), "again".into()),
            Err(AiError::Busy)
        ));
        let events = answer(&conversation, first);
        assert_eq!(
            events,
            [
                TurnEvent::Started {
                    turn: first,
                    fresh: true
                },
                TurnEvent::Called {
                    turn: first,
                    call: "t1".into(),
                    tool: "clip_edit".into(),
                    op: Some("trim".into())
                },
                TurnEvent::Returned {
                    turn: first,
                    call: "t1".into(),
                    ok: true,
                    detail: Some("Trim clip".into())
                },
                TurnEvent::Said {
                    turn: first,
                    text: "Answer 1".into()
                },
                TurnEvent::Finished {
                    turn: first,
                    model: Some("fake".into())
                },
            ]
        );
        // The same process answers the next request: it counts on.
        let second = conversation
            .ask(config(claude.clone()), server(), "And the end".into())
            .unwrap();
        let events = answer(&conversation, second);
        assert_eq!(
            events[0],
            TurnEvent::Started {
                turn: second,
                fresh: false
            }
        );
        assert!(events.contains(&TurnEvent::Said {
            turn: second,
            text: "Answer 2".into()
        }));
        // Ending forgets it: the next request starts over.
        conversation.end();
        let third = conversation
            .ask(config(claude), server(), "Once more".into())
            .unwrap();
        let events = answer(&conversation, third);
        assert_eq!(
            events[0],
            TurnEvent::Started {
                turn: third,
                fresh: true
            }
        );
        assert!(events.contains(&TurnEvent::Said {
            turn: third,
            text: "Answer 1".into()
        }));
    }

    #[test]
    fn stopping_a_request_ends_it_at_once_and_the_next_starts_fresh() {
        let directory = tempfile::tempdir().unwrap();
        let claude = fake_claude(directory.path());
        let conversation = Conversation::new(|| {}).unwrap();
        let turn = conversation
            .ask(config(claude.clone()), server(), "never answer".into())
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        conversation.stop();
        let events = answer(&conversation, turn);
        assert_eq!(events.last(), Some(&TurnEvent::Cancelled { turn }));
        assert!(started.elapsed() < Duration::from_secs(3));
        let next = conversation
            .ask(config(claude), server(), "Trim".into())
            .unwrap();
        assert_eq!(
            answer(&conversation, next)[0],
            TurnEvent::Started {
                turn: next,
                fresh: true
            }
        );
    }

    #[test]
    fn a_provider_that_cannot_start_fails_its_request() {
        let conversation = Conversation::new(|| {}).unwrap();
        let turn = conversation
            .ask(
                config(PathBuf::from("/aster-test/no-such-claude")),
                server(),
                "Trim".into(),
            )
            .unwrap();
        let events = answer(&conversation, turn);
        assert!(matches!(events.last(), Some(TurnEvent::Failed { .. })));
    }
}
