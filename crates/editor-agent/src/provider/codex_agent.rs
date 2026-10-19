//! Codex app-server as a conversation: one thread whose only tools are the
//! Neptune Video tool server's, approved for that server alone, and one turn per
//! request. Shell, file changes, web search and every other server stay off;
//! any of them, or any request for approval, ends the conversation.
use super::codex::{identity, turn_failure, valid_identity};
use super::{Heard, conversation_disabled, summary};
use crate::{AiConfig, AiError, ToolServer, conversation::INSTRUCTIONS};
use serde_json::{Map, Value, json};
use std::{collections::HashSet, path::Path};

/// The name the Neptune Video server has in this thread: one an inherited server of
/// the person's cannot share, so none of its settings merge into ours.
pub(super) const SERVER: &str = "neptune_video_window";
const MAX_SERVERS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Initialize,
    Account,
    Config,
    Thread,
    Ready,
}

pub(super) struct CodexAgent {
    phase: Phase,
    directory: String,
    server: ToolServer,
    requested_model: Option<String>,
    effort: Option<String>,
    model: Option<String>,
    thread: Option<String>,
    /// The turn being answered, once Codex has named it.
    turn: Option<String>,
    /// A turn was asked for and has not completed.
    active: bool,
    turns: u64,
    /// A request asked before the thread was open.
    queued: Option<String>,
    /// Tool calls of this server, which alone may report progress.
    calls: HashSet<String>,
}

impl CodexAgent {
    pub(super) fn new(config: &AiConfig, server: &ToolServer, directory: &Path) -> Self {
        Self {
            phase: Phase::Initialize,
            directory: directory.to_string_lossy().into_owned(),
            server: server.clone(),
            requested_model: config.model.clone(),
            effort: config.effort.clone(),
            model: None,
            thread: None,
            turn: None,
            active: false,
            turns: 0,
            queued: None,
            calls: HashSet::new(),
        }
    }

    pub(super) fn initialize() -> Value {
        json!({"id":"init","method":"initialize","params":{"clientInfo":{
            "name":"neptune-video", "title":"Neptune Video", "version":env!("CARGO_PKG_VERSION")
        }}})
    }

    /// The messages that ask `prompt`: none until the thread is open.
    pub(super) fn ask(&mut self, prompt: &str) -> Vec<Value> {
        if self.phase != Phase::Ready || self.active {
            self.queued = Some(prompt.to_owned());
            return Vec::new();
        }
        vec![self.start_turn(prompt)]
    }

    pub(super) fn receive(&mut self, frame: &Value) -> Result<Vec<Heard>, AiError> {
        if !frame.is_object() {
            return Err(provider("returned an invalid protocol frame"));
        }
        if let Some(method) = frame["method"].as_str() {
            if frame.get("id").is_some() {
                // Nothing here needs the person's answer: our tools are
                // approved in the thread's configuration, and nothing else
                // may run.
                return Err(provider(&format!(
                    "asked for {} during the conversation, which Neptune Video does not grant",
                    asked_for(method)
                )));
            }
            return self.notification(method, &frame["params"]);
        }
        let Some(id) = frame["id"].as_str() else {
            return Err(provider("returned a response without its request identity"));
        };
        if !frame["error"].is_null() {
            return Err(provider("refused the request"));
        }
        let result = &frame["result"];
        let mut heard = Vec::new();
        match (self.phase, id) {
            (Phase::Initialize, "init") => {
                self.phase = Phase::Account;
                heard.push(Heard::Send(json!({"method":"initialized"})));
                heard.push(Heard::Send(
                    json!({"id":"account","method":"account/read","params":{}}),
                ));
            }
            (Phase::Account, "account") => {
                if result["requiresOpenaiAuth"] == true && result["account"].is_null() {
                    return Err(provider(
                        "is not signed in; sign in with the Codex CLI first",
                    ));
                }
                self.phase = Phase::Config;
                heard.push(Heard::Send(
                    json!({"id":"config","method":"config/read","params":{
                        "cwd":self.directory, "includeLayers":false
                    }}),
                ));
            }
            (Phase::Config, "config") => {
                heard.push(Heard::Send(self.open_thread(result)?));
                self.phase = Phase::Thread;
            }
            (Phase::Thread, "thread") => {
                let thread = identity(&result["thread"]["id"])?.to_owned();
                if result
                    .get("instructionSources")
                    .is_some_and(|sources| !sources.as_array().is_some_and(Vec::is_empty))
                {
                    return Err(provider("loaded external instructions"));
                }
                if result["approvalPolicy"] != "never"
                    || result["sandbox"]["type"] != "readOnly"
                    || !matches!(
                        result["sandbox"].get("networkAccess"),
                        None | Some(Value::Bool(false))
                    )
                {
                    return Err(provider("did not keep the conversation's read-only policy"));
                }
                self.model = result["model"]
                    .as_str()
                    .filter(|model| valid_identity(model))
                    .map(str::to_owned);
                self.thread = Some(thread);
                self.phase = Phase::Ready;
                if let Some(prompt) = self.queued.take() {
                    heard.push(Heard::Send(self.start_turn(&prompt)));
                }
            }
            (Phase::Ready, id) if id == self.turn_request() => {
                let turn = &result["turn"];
                self.name_turn(identity(&turn["id"])?)?;
                match turn["status"].as_str() {
                    Some("inProgress" | "completed") if turn["error"].is_null() => {}
                    Some("interrupted") => return Err(AiError::Cancelled),
                    Some("failed") => return Err(turn_failure(&turn["error"])),
                    _ => return Err(provider("returned an invalid turn status")),
                }
            }
            _ => {}
        }
        Ok(heard)
    }

    fn turn_request(&self) -> String {
        format!("turn-{}", self.turns)
    }

    fn open_thread(&self, result: &Value) -> Result<Value, AiError> {
        let config = result["config"]
            .as_object()
            .ok_or_else(|| provider("could not establish its effective tool configuration"))?;
        let mut servers = Map::new();
        if let Some(value) = config.get("mcp_servers").filter(|value| !value.is_null()) {
            let inherited = value
                .as_object()
                .ok_or_else(|| provider("returned an invalid tool-server configuration"))?;
            if inherited.len() > MAX_SERVERS {
                return Err(provider(
                    "has too many inherited tool servers to isolate safely",
                ));
            }
            for name in inherited.keys() {
                if !valid_identity(name) {
                    return Err(provider("returned an invalid tool-server name"));
                }
                // Names only: their settings may hold credentials.
                servers.insert(name.clone(), json!({"enabled":false}));
            }
        }
        servers.insert(
            SERVER.to_owned(),
            json!({
                "command": self.server.command,
                "args": self.server.args,
                "enabled": true,
                "startup_timeout_sec": 30,
                // Exports and transcription answer within 50 s.
                "tool_timeout_sec": 300,
                // `never` asks nobody: these tools run because they are
                // approved here, and every other tool is denied.
                "default_tools_approval_mode": "approve"
            }),
        );
        let features: Map<String, Value> = conversation_disabled()
            .map(|name| (name.to_owned(), Value::Bool(false)))
            .collect();
        let mut config = json!({
            "features":features,
            "mcp_servers":servers,
            "web_search":"disabled",
            "project_doc_max_bytes":0,
            "project_doc_fallback_filenames":[],
            "notify":[]
        });
        if let Some(effort) = &self.effort {
            config["model_reasoning_effort"] = effort.as_str().into();
        }
        let mut params = json!({
            "cwd":self.directory,
            "approvalPolicy":"never",
            "approvalsReviewer":"user",
            "sandbox":"read-only",
            "ephemeral":true,
            "baseInstructions":INSTRUCTIONS,
            "developerInstructions":INSTRUCTIONS,
            "config":config
        });
        if let Some(model) = &self.requested_model {
            params["model"] = model.as_str().into();
        }
        Ok(json!({"id":"thread","method":"thread/start","params":params}))
    }

    fn start_turn(&mut self, prompt: &str) -> Value {
        self.turns += 1;
        self.turn = None;
        self.active = true;
        self.calls.clear();
        let mut params = json!({
            "threadId":self.thread,
            "input":[{"type":"text","text":prompt}],
            "approvalPolicy":"never",
            "approvalsReviewer":"user",
            "sandboxPolicy":{"type":"readOnly","networkAccess":false}
        });
        if let Some(model) = &self.requested_model {
            params["model"] = model.as_str().into();
        }
        if let Some(effort) = &self.effort {
            params["effort"] = effort.as_str().into();
        }
        json!({"id":self.turn_request(),"method":"turn/start","params":params})
    }

    fn name_turn(&mut self, id: &str) -> Result<(), AiError> {
        if self.turn.as_deref().is_some_and(|known| known != id) {
            return Err(provider("returned inconsistent turn identities"));
        }
        self.turn = Some(id.to_owned());
        Ok(())
    }

    /// Whether `params` is about the turn being answered in our thread.
    fn ours(&mut self, params: &Value) -> Result<bool, AiError> {
        if !self.active || params["threadId"].as_str() != self.thread.as_deref() {
            return Ok(false);
        }
        let Some(id) = params["turnId"]
            .as_str()
            .or_else(|| params["turn"]["id"].as_str())
        else {
            return Ok(false);
        };
        if self.turn.is_none() {
            self.name_turn(id)?;
        }
        Ok(self.turn.as_deref() == Some(id))
    }

    fn notification(&mut self, method: &str, params: &Value) -> Result<Vec<Heard>, AiError> {
        if method == "item/mcpToolCall/progress" {
            let item = params["itemId"].as_str().unwrap_or_default();
            if !self.calls.contains(item) {
                return Err(provider("ran a tool Neptune Video does not grant"));
            }
            return Ok(Vec::new());
        }
        if method.starts_with("item/commandExecution/")
            || method.starts_with("item/fileChange/")
            || method.starts_with("item/mcpToolCall/")
            || method.starts_with("item/webSearch/")
            || method.starts_with("item/tool/")
        {
            return Err(provider("ran a tool Neptune Video does not grant"));
        }
        if method == "mcpServer/startupStatus/updated"
            && params["name"] == SERVER
            && matches!(params["status"].as_str(), Some("failed" | "cancelled"))
        {
            return Err(provider("could not start Neptune Video's editing tools"));
        }
        let mut heard = Vec::new();
        match method {
            "item/started" | "item/completed" => {
                let item = &params["item"];
                self.allowed(item)?;
                if !self.ours(params)? {
                    return Ok(heard);
                }
                match (method, item["type"].as_str()) {
                    ("item/started", Some("mcpToolCall")) => {
                        let call = item["id"].as_str().unwrap_or_default().to_owned();
                        self.calls.insert(call.clone());
                        heard.push(Heard::Called {
                            call,
                            tool: item["tool"].as_str().unwrap_or_default().to_owned(),
                            op: item["arguments"]["op"].as_str().map(str::to_owned),
                        });
                    }
                    ("item/completed", Some("mcpToolCall")) => {
                        let result = &item["result"];
                        let (refused, detail) = if result.is_null() {
                            (true, item["error"]["message"].as_str().map(short))
                        } else if result["structuredContent"].is_object() {
                            summary(&Value::String(result["structuredContent"].to_string()))
                        } else {
                            summary(&result["content"])
                        };
                        heard.push(Heard::Returned {
                            call: item["id"].as_str().unwrap_or_default().to_owned(),
                            ok: item["status"] == "completed" && !refused,
                            detail,
                        });
                    }
                    ("item/completed", Some("agentMessage")) => {
                        if let Some(text) = item["text"].as_str()
                            && !text.trim().is_empty()
                            && item["phase"] != "partial_answer"
                        {
                            heard.push(Heard::Said(text.trim().to_owned()));
                        }
                    }
                    _ => {}
                }
            }
            "turn/started" => {
                self.ours(params)?;
            }
            "error" => {
                if self.ours(params)? && params["willRetry"] != true {
                    return Err(turn_failure(&params["error"]));
                }
            }
            "turn/completed" => {
                if !self.ours(params)? {
                    return Ok(heard);
                }
                let turn = &params["turn"];
                match turn["status"].as_str() {
                    Some("completed") if turn["error"].is_null() => {}
                    Some("interrupted") => return Err(AiError::Cancelled),
                    Some("failed") => return Err(turn_failure(&turn["error"])),
                    _ => return Err(provider("returned an invalid completed-turn status")),
                }
                self.active = false;
                heard.push(Heard::Finished {
                    model: self.model.clone(),
                });
                if let Some(prompt) = self.queued.take() {
                    heard.push(Heard::Send(self.start_turn(&prompt)));
                }
            }
            _ => {}
        }
        Ok(heard)
    }

    /// Items a conversation may hold: messages, reasoning and the Neptune Video
    /// server's tool calls, in any thread.
    fn allowed(&self, item: &Value) -> Result<(), AiError> {
        match item["type"].as_str() {
            Some("userMessage" | "agentMessage" | "reasoning" | "plan" | "contextCompaction") => {
                Ok(())
            }
            Some("mcpToolCall") if item["server"] == SERVER => Ok(()),
            _ => Err(provider("ran a tool Neptune Video does not grant")),
        }
    }
}

fn short(text: &str) -> String {
    text.chars().take(240).collect()
}

fn asked_for(method: &str) -> &'static str {
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => "a command",
        "item/fileChange/requestApproval" | "applyPatchApproval" => "a file change",
        "mcpServer/elicitation/request" => "an answer from the person",
        "item/permissions/requestApproval" => "permissions",
        _ => "something",
    }
}

fn provider(message: &str) -> AiError {
    AiError::Provider(format!("Codex {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn agent() -> CodexAgent {
        CodexAgent::new(
            &AiConfig::default(),
            &ToolServer {
                command: PathBuf::from("/opt/neptune-video/neptune-video-mcp"),
                args: vec!["--window".into(), "/tmp/w.json".into()],
            },
            Path::new("/private"),
        )
    }

    fn sent(heard: Vec<Heard>) -> Vec<Value> {
        heard
            .into_iter()
            .filter_map(|heard| match heard {
                Heard::Send(value) => Some(value),
                _ => None,
            })
            .collect()
    }

    /// Opens the thread with `inherited` servers in the person's config and
    /// answers what was asked of it.
    fn opened(inherited: Value) -> (CodexAgent, Value) {
        let mut agent = agent();
        assert!(agent.ask("Tighten the opening").is_empty());
        agent.receive(&json!({"id":"init","result":{}})).unwrap();
        agent
            .receive(&json!({"id":"account","result":{"account":{"type":"chatgpt"}}}))
            .unwrap();
        let thread = sent(
            agent
                .receive(&json!({"id":"config","result":{"config":{"mcp_servers":inherited}}}))
                .unwrap(),
        );
        let turn = sent(
            agent
                .receive(
                    &json!({"id":"thread","result":{"thread":{"id":"th"},"model":"gpt-6",
                    "approvalPolicy":"never","sandbox":{"type":"readOnly"}}}),
                )
                .unwrap(),
        );
        assert_eq!(turn[0]["method"], "turn/start");
        assert!(turn[0]["params"].get("outputSchema").is_none());
        (agent, thread[0].clone())
    }

    #[test]
    fn only_the_neptune_video_server_is_enabled_and_its_tools_need_nobody() {
        let (_, thread) =
            opened(json!({"private":{"env":{"TOKEN":"secret"}},"neptune-video":{"url":"x"}}));
        let servers = &thread["params"]["config"]["mcp_servers"];
        assert_eq!(servers["private"], json!({"enabled":false}));
        assert_eq!(servers["neptune-video"], json!({"enabled":false}));
        assert_eq!(
            servers[SERVER]["command"],
            "/opt/neptune-video/neptune-video-mcp"
        );
        assert_eq!(servers[SERVER]["default_tools_approval_mode"], "approve");
        assert!(!thread.to_string().contains("secret"));
        assert_eq!(thread["params"]["approvalPolicy"], "never");
        assert_eq!(thread["params"]["sandbox"], "read-only");
        // The shell stays off; the host that calls MCP tools stays on.
        let features = &thread["params"]["config"]["features"];
        assert_eq!(features["shell_tool"], false);
        assert!(features.get("code_mode_host").is_none());
    }

    #[test]
    fn a_turn_reports_its_calls_words_and_end_then_takes_the_next_request() {
        let (mut agent, _) = opened(json!({}));
        let call = json!({"type":"mcpToolCall","id":"c1","server":SERVER,"tool":"clip_edit",
            "arguments":{"op":"split"},"status":"inProgress"});
        let mut heard = agent
            .receive(
                &json!({"method":"item/started","params":{"threadId":"th","turnId":"tu",
                "item":call}}),
            )
            .unwrap();
        heard.extend(
            agent
                .receive(
                    &json!({"method":"item/completed","params":{"threadId":"th","turnId":"tu",
                    "item":{"type":"mcpToolCall","id":"c1","server":SERVER,"tool":"clip_edit",
                    "arguments":{},"status":"completed","result":{"content":[],
                    "structuredContent":{"label":"Split clip","changed":true}}}}}),
                )
                .unwrap(),
        );
        heard.extend(
            agent
                .receive(&json!({"method":"item/completed","params":{"threadId":"th","turnId":"tu",
                    "item":{"type":"agentMessage","id":"m","text":"Split it.","phase":"final_answer"}}}))
                .unwrap(),
        );
        heard.extend(
            agent
                .receive(&json!({"method":"turn/completed","params":{"threadId":"th",
                    "turn":{"id":"tu","status":"completed","items":[]}}}))
                .unwrap(),
        );
        assert_eq!(
            heard,
            [
                Heard::Called {
                    call: "c1".into(),
                    tool: "clip_edit".into(),
                    op: Some("split".into())
                },
                Heard::Returned {
                    call: "c1".into(),
                    ok: true,
                    detail: Some("Split clip".into())
                },
                Heard::Said("Split it.".into()),
                Heard::Finished {
                    model: Some("gpt-6".into())
                },
            ]
        );
        let next = agent.ask("Now add a title");
        assert_eq!(next[0]["id"], "turn-2");
        assert_eq!(next[0]["params"]["threadId"], "th");
    }

    #[test]
    fn another_servers_tool_a_shell_or_an_approval_request_ends_it() {
        let (mut agent, _) = opened(json!({}));
        assert!(
            agent
                .receive(
                    &json!({"method":"item/started","params":{"threadId":"th","turnId":"tu",
                    "item":{"type":"mcpToolCall","id":"c","server":"private","tool":"x",
                    "arguments":{},"status":"inProgress"}}})
                )
                .is_err()
        );
        let (mut agent, _) = opened(json!({}));
        assert!(
            agent
                .receive(
                    &json!({"method":"item/started","params":{"threadId":"th","turnId":"tu",
                    "item":{"type":"commandExecution","id":"c"}}})
                )
                .is_err()
        );
        let (mut agent, _) = opened(json!({}));
        assert!(
            agent
                .receive(
                    &json!({"id":7,"method":"item/commandExecution/requestApproval",
                    "params":{}})
                )
                .is_err()
        );
        let (mut agent, _) = opened(json!({}));
        assert!(
            agent
                .receive(&json!({"method":"mcpServer/startupStatus/updated",
                    "params":{"name":SERVER,"status":"failed"}}))
                .is_err()
        );
    }
}
