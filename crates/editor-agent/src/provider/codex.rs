//! Codex app-server planning: protocol state only, with no tool execution.
use super::{DISABLED_FEATURES, Process, ProviderOutput};
use crate::{AiConfig, AiError, planner};
use serde_json::{Map, Value, json};
use std::path::Path;

const MAX_SERVERS: usize = 64;
const MAX_ID_BYTES: usize = 128;

pub(super) fn plan(
    process: &mut Process,
    config: &AiConfig,
    prompt: &str,
    schema: &Value,
) -> Result<ProviderOutput, AiError> {
    let mut driver = Driver::new(config, prompt, schema, process.directory());
    process.send(&Driver::initialize())?;
    loop {
        let step = driver.receive(&process.next()?)?;
        for reply in step.replies {
            process.send(&reply)?;
        }
        if step.rejected {
            return Err(provider(
                "requested a capability unavailable to an editing planner",
            ));
        }
        if let Some(output) = step.output {
            return Ok(output);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initialize,
    Account,
    Config,
    Thread,
    Turn,
}
impl Phase {
    fn request_id(self) -> &'static str {
        match self {
            Self::Initialize => "init",
            Self::Account => "account",
            Self::Config => "config",
            Self::Thread => "thread",
            Self::Turn => "turn",
        }
    }
}

#[derive(Default)]
struct Step {
    replies: Vec<Value>,
    output: Option<ProviderOutput>,
    rejected: bool,
}

struct Driver {
    phase: Phase,
    directory: String,
    prompt: String,
    schema: Value,
    requested_model: Option<String>,
    effort: Option<String>,
    model: Option<String>,
    thread: Option<String>,
    turn: Option<String>,
    accepted: bool,
    completed: bool,
    answer: Option<String>,
    final_answer: bool,
}
impl Driver {
    fn new(config: &AiConfig, prompt: &str, schema: &Value, directory: &Path) -> Self {
        Self {
            phase: Phase::Initialize,
            directory: directory.to_string_lossy().into_owned(),
            prompt: prompt.to_owned(),
            schema: schema.clone(),
            requested_model: config.model.clone(),
            effort: config.effort.clone(),
            model: None,
            thread: None,
            turn: None,
            accepted: false,
            completed: false,
            answer: None,
            final_answer: false,
        }
    }

    fn initialize() -> Value {
        json!({"id":"init","method":"initialize","params":{"clientInfo":{
            "name":"aster", "title":"Aster", "version":env!("CARGO_PKG_VERSION")
        }}})
    }

    fn receive(&mut self, frame: &Value) -> Result<Step, AiError> {
        if !frame.is_object() {
            return Err(provider("returned an invalid protocol frame"));
        }
        if let Some(method) = frame["method"].as_str() {
            if let Some(id) = frame.get("id") {
                return Ok(Step {
                    replies: vec![denial(id, method)],
                    rejected: true,
                    ..Step::default()
                });
            }
            return self.notification(method, &frame["params"]);
        }
        let Some(id) = frame["id"].as_str() else {
            return Err(provider("returned a response without its request identity"));
        };
        if id != self.phase.request_id() {
            return Ok(Step::default());
        }
        if !frame["error"].is_null() {
            // RPC errors can repeat private configuration or account details.
            return Err(provider("refused the planning request"));
        }
        let result = &frame["result"];
        if !result.is_object() {
            return Err(provider("returned an invalid planning response"));
        }
        let replies = match self.phase {
            Phase::Initialize => {
                self.phase = Phase::Account;
                vec![
                    json!({"method":"initialized"}),
                    json!({"id":"account","method":"account/read","params":{}}),
                ]
            }
            Phase::Account => {
                if result["requiresOpenaiAuth"] == true && result["account"].is_null() {
                    return Err(provider(
                        "is not signed in; sign in with the Codex CLI first",
                    ));
                }
                // Only readiness is used; account names and credentials stay private.
                self.phase = Phase::Config;
                vec![json!({"id":"config","method":"config/read","params":{
                    "cwd":self.directory, "includeLayers":false
                }})]
            }
            Phase::Config => {
                let opening = self.open_thread(result)?;
                self.phase = Phase::Thread;
                vec![opening]
            }
            Phase::Thread => {
                let thread = identity(&result["thread"]["id"])?;
                // Current stable wire reports loaded instruction paths. Older
                // providers may omit this optional field; if supplied, it must
                // confirm that global/project customizations stayed disabled.
                if result
                    .get("instructionSources")
                    .is_some_and(|sources| !sources.as_array().is_some_and(Vec::is_empty))
                {
                    return Err(provider("loaded external instructions during planning"));
                }
                if result["approvalPolicy"] != "never"
                    || result["approvalsReviewer"] != "user"
                    || result["sandbox"]["type"] != "readOnly"
                    || !matches!(
                        result["sandbox"].get("networkAccess"),
                        None | Some(Value::Bool(false))
                    )
                {
                    return Err(provider("did not retain the planner's read-only policy"));
                }
                self.model = Some(identity(&result["model"])?.to_owned());
                self.thread = Some(thread.to_owned());
                self.phase = Phase::Turn;
                vec![self.start_turn(thread)]
            }
            Phase::Turn => {
                let turn = &result["turn"];
                self.name_turn(identity(&turn["id"])?)?;
                self.inspect_items(turn, false)?;
                match turn["status"].as_str() {
                    Some("inProgress" | "completed") if turn["error"].is_null() => {}
                    Some("interrupted") => return Err(AiError::Cancelled),
                    Some("failed") => return Err(turn_failure(&turn["error"])),
                    _ => return Err(provider("returned an invalid planning-turn status")),
                }
                self.accepted = true;
                return self.finish();
            }
        };
        Ok(Step {
            replies,
            ..Step::default()
        })
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
                // Read server names only; their settings may contain credentials.
                servers.insert(name.clone(), json!({"enabled":false}));
            }
        }
        let features: Map<String, Value> = DISABLED_FEATURES
            .iter()
            .map(|name| ((*name).to_owned(), Value::Bool(false)))
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
            "baseInstructions":planner::SYSTEM,
            "developerInstructions":planner::SYSTEM,
            "config":config
        });
        if let Some(model) = &self.requested_model {
            params["model"] = model.as_str().into();
        }
        // Inherit the configured model provider/auth; never copy account settings.
        Ok(json!({"id":"thread","method":"thread/start","params":params}))
    }

    fn start_turn(&self, thread: &str) -> Value {
        let mut params = json!({
            "threadId":thread,
            "input":[{"type":"text","text":self.prompt}],
            "approvalPolicy":"never",
            "approvalsReviewer":"user",
            "sandboxPolicy":{"type":"readOnly","networkAccess":false},
            "outputSchema":self.schema
        });
        if let Some(model) = &self.requested_model {
            params["model"] = model.as_str().into();
        }
        if let Some(effort) = &self.effort {
            params["effort"] = effort.as_str().into();
        }
        json!({"id":"turn","method":"turn/start","params":params})
    }

    fn notification(&mut self, method: &str, params: &Value) -> Result<Step, AiError> {
        // A tool may start without an approval request. Reject it on any thread.
        if method.starts_with("item/commandExecution/")
            || method.starts_with("item/fileChange/")
            || method.starts_with("item/mcpToolCall/")
            || method.starts_with("item/webSearch/")
            || method.starts_with("item/tool/")
        {
            return Err(provider("attempted tool execution during planning"));
        }
        if matches!(method, "item/started" | "item/completed") {
            safe_item(&params["item"])?;
        }
        if matches!(method, "turn/started" | "turn/completed")
            && let Some(items) = params["turn"]["items"].as_array()
        {
            for item in items {
                safe_item(item)?;
            }
        }
        if self.phase != Phase::Turn || params["threadId"].as_str() != self.thread.as_deref() {
            return Ok(Step::default());
        }
        match method {
            "error" => {
                if !self.matches_turn(identity(&params["turnId"])?)? || params["willRetry"] == true
                {
                    return Ok(Step::default());
                }
                return Err(turn_failure(&params["error"]));
            }
            "turn/started" => {
                let turn = &params["turn"];
                self.name_turn(identity(&turn["id"])?)?;
                self.inspect_items(turn, false)?;
            }
            "item/completed" => {
                if self.matches_turn(identity(&params["turnId"])?)? {
                    self.remember(&params["item"])?;
                }
            }
            "turn/completed" => {
                let turn = &params["turn"];
                if !self.matches_turn(identity(&turn["id"])?)? {
                    return Ok(Step::default());
                }
                self.inspect_items(turn, true)?;
                match turn["status"].as_str() {
                    Some("completed") if turn["error"].is_null() => self.completed = true,
                    Some("interrupted") => return Err(AiError::Cancelled),
                    Some("failed") => return Err(turn_failure(&turn["error"])),
                    _ => return Err(provider("returned an invalid completed-turn status")),
                }
            }
            _ => {}
        }
        self.finish()
    }

    fn name_turn(&mut self, id: &str) -> Result<(), AiError> {
        if self.turn.as_deref().is_some_and(|known| known != id) {
            return Err(provider("returned inconsistent planning-turn identities"));
        }
        self.turn = Some(id.to_owned());
        Ok(())
    }

    fn matches_turn(&mut self, id: &str) -> Result<bool, AiError> {
        if let Some(known) = self.turn.as_deref() {
            return Ok(known == id);
        }
        // Notifications can precede the turn/start reply. No output is returned
        // until that reply confirms this same turn and completion is observed.
        self.name_turn(id)?;
        Ok(true)
    }

    fn inspect_items(&mut self, turn: &Value, completed: bool) -> Result<(), AiError> {
        let items = turn["items"]
            .as_array()
            .ok_or_else(|| provider("returned an invalid planning-turn item list"))?;
        for item in items {
            safe_item(item)?;
            if completed {
                self.remember(item)?;
            }
        }
        Ok(())
    }

    fn remember(&mut self, item: &Value) -> Result<(), AiError> {
        if item["type"] != "agentMessage" {
            return Ok(());
        }
        if !item["delivery"].is_null()
            || item.get("questions").is_some_and(|questions| {
                !questions.is_null() && !questions.as_array().is_some_and(Vec::is_empty)
            })
        {
            return Err(provider(
                "returned asynchronous output instead of the requested proposal",
            ));
        }
        let phase = item["phase"].as_str();
        let is_final = match phase {
            Some("commentary" | "partial_answer") => return Ok(()),
            Some("final_answer") => true,
            None if item["phase"].is_null() => false,
            _ => return Err(provider("returned an unknown assistant-message phase")),
        };
        if self.final_answer && !is_final {
            return Ok(());
        }
        let text = item["text"]
            .as_str()
            .ok_or_else(|| provider("returned an invalid assistant message"))?;
        if text.len() > planner::MAX_OUTPUT_BYTES {
            return Err(AiError::Invalid(
                "Codex proposal exceeds its size limit".into(),
            ));
        }
        if is_final && text.trim().is_empty() {
            return Err(provider("returned an empty final proposal"));
        }
        if !text.trim().is_empty() {
            self.answer = Some(text.to_owned());
            self.final_answer = is_final;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<Step, AiError> {
        if !self.accepted || !self.completed {
            return Ok(Step::default());
        }
        let bytes = self
            .answer
            .take()
            .ok_or_else(|| provider("completed without an edit proposal"))?
            .into_bytes();
        Ok(Step {
            output: Some(ProviderOutput {
                bytes,
                model: self.model.take(),
            }),
            ..Step::default()
        })
    }
}

fn provider(message: &str) -> AiError {
    AiError::Provider(format!("Codex {message}"))
}

/// Current stable CodexErrorInfo discriminants only. Raw provider messages,
/// details, HTTP bodies and account/configuration fields never reach the UI.
pub(super) fn turn_failure(error: &Value) -> AiError {
    let info = &error["codexErrorInfo"];
    let code = info.as_str().or_else(|| {
        info.as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.keys().next().map(String::as_str))
    });
    provider(match code {
        Some("usageLimitExceeded") => "usage limit reached; check its usage limits before retrying",
        Some("rateLimitExceeded") => "is rate limited; retry later",
        Some("contextWindowExceeded") => {
            "context limit was exceeded; use a smaller independent selection"
        }
        Some("sessionBudgetExceeded") => "planning session budget was reached; start a new request",
        Some("serverOverloaded" | "flexUnavailable") => {
            "service is unavailable or overloaded; retry later"
        }
        Some("unauthorized") => "sign-in or provider authorization failed; sign in with its CLI",
        Some("httpConnectionFailed" | "responseStreamConnectionFailed") => {
            "could not connect to its provider; check connectivity"
        }
        Some("responseStreamDisconnected") => {
            "provider connection closed before completion; retry the request"
        }
        Some("responseTooManyFailedAttempts") => {
            "exhausted its provider connection retries; check connectivity and retry later"
        }
        Some("badRequest") => {
            "provider refused this planning request; check the model and configuration"
        }
        Some("cyberPolicy" | "misalignmentPolicyViolation") => {
            "provider policy refused the request"
        }
        Some("tooManyDenials") => "planning stopped after repeated capability denials",
        Some("sandboxError") => "could not apply the planner sandbox policy",
        Some("internalServerError" | "threadRollbackFailed") => {
            "service failed to complete the planning request; retry later"
        }
        Some("activeTurnNotSteerable") => "current turn cannot accept this planning request",
        _ => "could not complete the planning turn",
    })
}

pub(super) fn valid_identity(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

pub(super) fn identity(value: &Value) -> Result<&str, AiError> {
    value
        .as_str()
        .filter(|id| valid_identity(id))
        .ok_or_else(|| provider("returned an invalid protocol identity"))
}

fn safe_item(item: &Value) -> Result<(), AiError> {
    match item["type"].as_str() {
        Some("userMessage" | "agentMessage" | "reasoning" | "plan" | "contextCompaction") => Ok(()),
        _ => Err(provider(
            "attempted an unavailable operation during planning",
        )),
    }
}

fn denial(id: &Value, method: &str) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({"id":id,"result":{"decision":"cancel"}})
        }
        "execCommandApproval" | "applyPatchApproval" => {
            json!({"id":id,"result":{"decision":"abort"}})
        }
        "mcpServer/elicitation/request" => {
            json!({"id":id,"result":{"action":"cancel","content":null}})
        }
        "item/permissions/requestApproval" => {
            json!({"id":id,"result":{"permissions":{},"scope":"turn"}})
        }
        "item/tool/call" => json!({"id":id,"result":{"success":false,"contentItems":[]}}),
        "item/tool/requestUserInput" => json!({"id":id,"result":{"answers":{}}}),
        _ => json!({"id":id,"error":{
            "code":-32601,"message":"Not available to an editing planner"
        }}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> Driver {
        Driver::new(
            &AiConfig::default(),
            "request",
            &json!({"type":"object"}),
            Path::new("/owned"),
        )
    }

    fn configuring(driver: &mut Driver) {
        driver.receive(&json!({"id":"init","result":{}})).unwrap();
        driver
            .receive(&json!({"id":"account","result":{"requiresOpenaiAuth":false,"account":null}}))
            .unwrap();
    }

    fn running() -> Driver {
        let mut driver = driver();
        configuring(&mut driver);
        driver
            .receive(&json!({"id":"config","result":{"config":{}}}))
            .unwrap();
        driver.receive(&json!({"id":"thread","result":{
            "thread":{"id":"thread-1"},"model":"actual-model",
            "approvalPolicy":"never","approvalsReviewer":"user","sandbox":{"type":"readOnly","networkAccess":false}
        }})).unwrap();
        driver
    }

    fn accepted(driver: &mut Driver) {
        driver.receive(&json!({"id":"turn","result":{"turn":{"id":"turn-1","items":[],"status":"inProgress","error":null}}})).unwrap();
    }

    fn done(items: Value, status: &str) -> Value {
        json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{
            "id":"turn-1","items":items,"status":status,"error":null
        }}})
    }

    #[test]
    fn inherited_tool_servers_are_disabled_without_copying_private_settings() {
        let mut driver = driver();
        configuring(&mut driver);
        let step = driver
            .receive(&json!({"id":"config","result":{"config":{
                "mcp_servers":{"private":{"env":{"credential":"fixture-secret"}}},
                "model_provider":"custom"
            }}}))
            .unwrap();
        let params = &step.replies[0]["params"];
        assert_eq!(
            params["config"]["mcp_servers"]["private"],
            json!({"enabled":false})
        );
        assert_eq!(params["config"]["features"]["shell_tool"], false);
        assert_eq!(params["config"]["web_search"], "disabled");
        assert_eq!(params["config"]["project_doc_max_bytes"], 0);
        assert_eq!(
            params["config"]["project_doc_fallback_filenames"],
            json!([])
        );
        assert_eq!(params["config"]["notify"], json!([]));
        assert_eq!(params["baseInstructions"], planner::SYSTEM);
        assert_eq!(params["developerInstructions"], planner::SYSTEM);
        assert!(params.get("modelProvider").is_none());
        assert!(params.get("tools").is_none());
        assert!(!step.replies[0].to_string().contains("fixture-secret"));
    }

    #[test]
    fn inherited_instruction_and_notification_settings_are_replaced_for_the_planner() {
        let mut driver = driver();
        configuring(&mut driver);
        let step = driver
            .receive(&json!({"id":"config","result":{"config":{
                "project_doc_max_bytes":65536,
                "project_doc_fallback_filenames":["PRIVATE_GUIDE.md"],
            "notify":["owned-notify-marker", "must-not-run"],
            "model_instructions_file":"private-inherited-base-path",
            "instructions":"private-inherited-inline-prompt"
            }}}))
            .unwrap();
        let config = &step.replies[0]["params"]["config"];
        assert_eq!(config["project_doc_max_bytes"], 0);
        assert_eq!(config["project_doc_fallback_filenames"], json!([]));
        assert_eq!(config["notify"], json!([]));
        assert!(!step.replies[0].to_string().contains("PRIVATE_GUIDE"));
        assert!(!step.replies[0].to_string().contains("owned-notify-marker"));
        assert_eq!(
            step.replies[0]["params"]["baseInstructions"],
            planner::SYSTEM
        );
        assert!(!step.replies[0].to_string().contains("private-inherited"));
    }

    #[test]
    fn reported_instruction_sources_must_be_empty_but_older_missing_fields_are_supported() {
        for sources in [
            json!([]),
            json!(["/owned/AGENTS.md"]),
            json!(null),
            json!("invalid"),
        ] {
            let mut driver = driver();
            configuring(&mut driver);
            driver
                .receive(&json!({"id":"config","result":{"config":{}}}))
                .unwrap();
            let result = driver.receive(&json!({"id":"thread","result":{
                "thread":{"id":"thread-1"},"model":"actual-model",
                "approvalPolicy":"never","approvalsReviewer":"user",
                "sandbox":{"type":"readOnly","networkAccess":false},
                "instructionSources":sources
            }}));
            assert_eq!(result.is_ok(), sources == json!([]));
        }
        // The shared fixture deliberately omits the optional stable field.
        assert!(running().phase == Phase::Turn);
    }

    #[test]
    fn typed_provider_failures_never_echo_private_messages_or_details() {
        let cases = [
            (json!("usageLimitExceeded"), "usage limit reached"),
            (json!("rateLimitExceeded"), "rate limited"),
            (json!("contextWindowExceeded"), "context limit"),
            (json!("sessionBudgetExceeded"), "session budget"),
            (json!("unauthorized"), "authorization failed"),
            (json!("serverOverloaded"), "overloaded"),
            (
                json!({"httpConnectionFailed":{"httpStatusCode":503,"body":"private-http-body"}}),
                "could not connect",
            ),
            (
                json!({"responseStreamConnectionFailed":{"httpStatusCode":null}}),
                "could not connect",
            ),
            (
                json!({"responseStreamDisconnected":{"httpStatusCode":null}}),
                "connection closed",
            ),
            (
                json!({"responseTooManyFailedAttempts":{"httpStatusCode":500}}),
                "connection retries",
            ),
            (json!("sandboxError"), "sandbox policy"),
            (json!("misalignmentPolicyViolation"), "policy refused"),
            (
                json!({"private-unrecognized-discriminant":{"credential":"private-token"}}),
                "could not complete",
            ),
            (json!(null), "could not complete"),
        ];
        for (info, expected) in cases {
            let failure = turn_failure(&json!({
                "codexErrorInfo":info,"message":"private-provider-message",
                "additionalDetails":"private-provider-details", "account":"private-account"
            }))
            .to_string();
            assert!(failure.contains(expected), "{failure}");
            assert!(!failure.contains("private-"), "{failure}");
            assert!(!failure.contains("503") && !failure.contains("500"));
        }
    }

    #[test]
    fn terminal_failures_belong_to_the_matching_turn_and_retries_can_complete() {
        let mut driver = running();
        accepted(&mut driver);
        let mut notification = json!({"method":"error","params":{
            "threadId":"another-thread","turnId":"turn-1","willRetry":false,
            "error":{"codexErrorInfo":"usageLimitExceeded","message":"private-message"}
        }});
        assert!(driver.receive(&notification).unwrap().output.is_none());
        notification["params"]["threadId"] = "thread-1".into();
        notification["params"]["turnId"] = "old-turn".into();
        assert!(driver.receive(&notification).unwrap().output.is_none());
        notification["params"]["turnId"] = "turn-1".into();
        notification["params"]["willRetry"] = true.into();
        assert!(driver.receive(&notification).unwrap().output.is_none());
        assert!(
            driver
                .receive(&done(
                    json!([{"type":"agentMessage","text":"{}"}]),
                    "completed"
                ))
                .unwrap()
                .output
                .is_some()
        );

        let mut driver = running();
        accepted(&mut driver);
        notification["params"]["willRetry"] = false.into();
        let error = driver.receive(&notification).err().unwrap().to_string();
        assert!(error.contains("usage limit reached"));
        assert!(!error.contains("private-message"));
    }

    #[test]
    fn failed_turn_replies_and_completion_use_the_same_private_safe_classification() {
        let error = json!({"codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":503}},
            "message":"private-turn-message", "additionalDetails":"private-turn-details"});
        let mut driver = running();
        let failure = driver
            .receive(&json!({"id":"turn","result":{"turn":{
                "id":"turn-1","items":[],"status":"failed","error":error
            }}}))
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            failure,
            "Codex could not connect to its provider; check connectivity"
        );
        let mut driver = running();
        accepted(&mut driver);
        let mut completion = done(json!([]), "failed");
        completion["params"]["turn"]["error"] = error;
        assert_eq!(
            driver.receive(&completion).err().unwrap().to_string(),
            failure
        );
    }

    #[test]
    fn too_many_inherited_servers_fail_closed() {
        let mut driver = driver();
        configuring(&mut driver);
        let servers: Map<String, Value> = (0..65).map(|i| (format!("s{i}"), json!({}))).collect();
        assert!(
            driver
                .receive(&json!({"id":"config","result":{"config":{"mcp_servers":servers}}}))
                .is_err()
        );
    }

    #[test]
    fn signed_out_first_party_requires_sign_in_but_custom_provider_does_not() {
        let mut first_party = driver();
        first_party
            .receive(&json!({"id":"init","result":{}}))
            .unwrap();
        assert!(
            first_party
                .receive(
                    &json!({"id":"account","result":{"requiresOpenaiAuth":true,"account":null}})
                )
                .is_err()
        );
        let mut custom = driver();
        configuring(&mut custom);
        assert!(custom.phase == Phase::Config);
    }

    #[test]
    fn proposals_require_matching_accepted_completion_and_prefer_final_answer() {
        let mut driver = running();
        accepted(&mut driver);
        let step = driver.receive(&done(json!([
            {"type":"agentMessage","id":"p","text":"progress","phase":"commentary"},
            {"type":"agentMessage","id":"f","text":"{\"summary\":\"Done\",\"commands\":[]}","phase":"final_answer"},
            {"type":"agentMessage","id":"u","text":"legacy tail"}
        ]), "completed")).unwrap();
        let output = step.output.unwrap();
        assert_eq!(
            String::from_utf8(output.bytes).unwrap(),
            "{\"summary\":\"Done\",\"commands\":[]}"
        );
        assert_eq!(output.model.as_deref(), Some("actual-model"));
    }

    #[test]
    fn completion_before_start_reply_waits_for_confirmation() {
        let mut driver = running();
        assert!(
            driver
                .receive(&done(
                    json!([{"type":"agentMessage","id":"a","text":"{}"}]),
                    "completed"
                ))
                .unwrap()
                .output
                .is_none()
        );
        let step = driver.receive(&json!({"id":"turn","result":{"turn":{"id":"turn-1","items":[],"status":"inProgress","error":null}}})).unwrap();
        assert_eq!(step.output.unwrap().bytes, b"{}");
    }

    #[test]
    fn another_turn_cannot_finish_or_replace_the_proposal() {
        let mut driver = running();
        accepted(&mut driver);
        let mut stale = done(
            json!([{"type":"agentMessage","id":"a","text":"wrong"}]),
            "completed",
        );
        stale["params"]["turn"]["id"] = "old-turn".into();
        assert!(driver.receive(&stale).unwrap().output.is_none());
        assert!(driver.answer.is_none());
    }

    #[test]
    fn failed_or_interrupted_turns_never_return_proposals() {
        for status in ["failed", "interrupted"] {
            let mut driver = running();
            accepted(&mut driver);
            assert!(
                driver
                    .receive(&done(
                        json!([{"type":"agentMessage","id":"a","text":"{}"}]),
                        status
                    ))
                    .is_err()
            );
        }
    }

    #[test]
    fn tools_without_approval_requests_are_rejected() {
        for kind in [
            "commandExecution",
            "fileChange",
            "mcpToolCall",
            "webSearch",
            "dynamicToolCall",
            "hookPrompt",
            "collabAgentToolCall",
        ] {
            let mut driver = running();
            accepted(&mut driver);
            assert!(
                driver
                    .receive(&done(json!([{"type":kind,"id":"x"}]), "completed"))
                    .is_err()
            );
        }
    }

    #[test]
    fn server_requests_are_denied_before_failing_closed() {
        let mut driver = running();
        let step = driver
            .receive(&json!({"id":7,"method":"item/commandExecution/requestApproval","params":{}}))
            .unwrap();
        assert!(step.rejected);
        assert_eq!(
            step.replies,
            vec![json!({"id":7,"result":{"decision":"cancel"}})]
        );
        assert!(step.output.is_none());
    }

    #[test]
    fn an_unconfirmed_policy_or_private_rpc_error_cannot_produce_a_plan() {
        let mut driver = driver();
        configuring(&mut driver);
        driver
            .receive(&json!({"id":"config","result":{"config":{}}}))
            .unwrap();
        assert!(driver.receive(&json!({"id":"thread","result":{
            "thread":{"id":"thread-1"},"approvalPolicy":"never","sandbox":{"type":"dangerFullAccess"}
        }})).is_err());
        let error = driver
            .receive(&json!({"id":"thread","error":{"message":"fixture-private-account"}}))
            .err()
            .unwrap();
        assert!(!error.to_string().contains("fixture-private-account"));
    }

    #[test]
    fn incomplete_and_async_messages_cannot_become_proposals() {
        let mut driver = running();
        accepted(&mut driver);
        driver.receive(&json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{
            "id":"turn-1","items":[{"type":"agentMessage","id":"draft","text":"{}"}],"status":"inProgress"
        }}})).unwrap();
        assert!(driver.answer.is_none());
        assert!(
            driver
                .receive(&done(
                    json!([{"type":"agentMessage","id":"async","text":"{}","delivery":"async"}]),
                    "completed"
                ))
                .is_err()
        );
    }

    #[test]
    fn empty_explicit_final_answer_cannot_fall_back_to_earlier_text() {
        let mut driver = running();
        accepted(&mut driver);
        assert!(
            driver
                .receive(&done(
                    json!([
                        {"type":"agentMessage","id":"draft","text":"{}"},
                        {"type":"agentMessage","id":"final","phase":"final_answer","text":" "}
                    ]),
                    "completed"
                ))
                .is_err()
        );
    }

    #[test]
    fn tool_execution_in_foreign_turn_snapshots_is_still_rejected() {
        let mut driver = running();
        accepted(&mut driver);
        let mut foreign = done(
            json!([{"type":"commandExecution","id":"exec"}]),
            "completed",
        );
        foreign["params"]["threadId"] = "another-thread".into();
        assert!(driver.receive(&foreign).is_err());
    }
}
