//! Native provider adapters share bounded process ownership and no GUI work.
mod claude_agent;
mod codex;
mod codex_agent;
mod process;
#[cfg(windows)]
mod windows;
use crate::{AiConfig, AiContext, AiError, AiProvider, ToolServer, conversation, planner};
use claude_agent::ClaudeAgent;
use codex_agent::CodexAgent;
use process::Limits;
pub(crate) use process::Process;
use serde_json::Value;
use std::{
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) struct ProviderOutput {
    pub bytes: Vec<u8>,
    pub model: Option<String>,
}

pub(crate) fn plan(
    config: &AiConfig,
    context: &AiContext,
    request: &str,
    cancel: Arc<AtomicBool>,
) -> Result<ProviderOutput, AiError> {
    let prompt = planner::prompt(context, request)?;
    let schema = planner::proposal_schema();
    let executable = resolve_executable(config)?;
    let mut command = Command::new(executable);
    match config.provider {
        AiProvider::Claude => {
            let schema = serde_json::to_string(&schema)
                .map_err(|_| AiError::Invalid("Cannot encode proposal schema".into()))?;
            command_line_schema(schema.len())?;
            command
                .args([
                    "--print",
                    "--safe-mode",
                    "--output-format",
                    "json",
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                    "--permission-mode",
                    "dontAsk",
                    "--setting-sources",
                    "",
                    "--disable-slash-commands",
                    "--no-session-persistence",
                    "--json-schema",
                ])
                .arg(schema)
                .arg("--system-prompt")
                .arg(planner::SYSTEM);
            if let Some(model) = &config.model {
                command.arg("--model").arg(model);
            }
            if let Some(effort) = &config.effort {
                command.arg("--effort").arg(effort);
            }
            // An enclosing Claude session is not this app's planning session.
            command
                .env_remove("CLAUDECODE")
                .env_remove("CLAUDE_CODE_SESSION_ID");
        }
        AiProvider::Codex => {
            command.arg("app-server");
            for feature in DISABLED_FEATURES {
                command.arg("-c").arg(format!("features.{feature}=false"));
            }
            command.arg("-c").arg("web_search=\"disabled\"");
            // A private cwd does not disable global AGENTS.md or the separate
            // turn-complete notifier. Isolate these before creating a thread.
            for setting in [
                "project_doc_max_bytes=0",
                "project_doc_fallback_filenames=[]",
                "notify=[]",
            ] {
                command.arg("-c").arg(setting);
            }
        }
    }
    let mut process = match config.provider {
        AiProvider::Claude => Process::spawn(command, cancel, config.timeout_seconds)?,
        AiProvider::Codex => Process::spawn_with(
            command,
            cancel,
            config.timeout_seconds,
            |command, directory| configure_codex_instructions(command, directory, planner::SYSTEM),
        )?,
    };
    match config.provider {
        AiProvider::Claude => {
            process.send_text(&prompt)?;
            process.close_input();
            claude_result(process.next()?, config)
        }
        AiProvider::Codex => codex::plan(&mut process, config, &prompt, &schema),
    }
}

/// What a conversation's provider said or did, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Heard {
    /// Words for the person.
    Said(String),
    /// A tool call began. `op` is its `op` argument, when it has one.
    Called {
        call: String,
        tool: String,
        op: Option<String>,
    },
    /// A tool call ended. `detail` is the edit's label, or why it was refused.
    Returned {
        call: String,
        ok: bool,
        detail: Option<String>,
    },
    /// The request is answered; the conversation waits for the next one.
    Finished { model: Option<String> },
    /// A protocol message for the provider, sent by `Agent::receive`.
    Send(Value),
}

/// A tool result's content, read as the Neptune Video server writes it: whether it
/// is a refusal, and the label of the edit it made or the refusal's reason.
fn summary(content: &Value) -> (bool, Option<String>) {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let short = |text: &str| text.chars().take(240).collect::<String>();
    match serde_json::from_str::<Value>(&text) {
        Ok(value) if value["code"].is_string() && value["message"].is_string() => {
            (true, value["message"].as_str().map(short))
        }
        Ok(value) if value["changed"] == true => (false, value["label"].as_str().map(short)),
        _ => (false, None),
    }
}

/// A conversation with a provider whose only tools are one Neptune Video server's.
pub(crate) struct Agent(Party);

enum Party {
    Claude(ClaudeAgent),
    Codex(Box<CodexAgent>),
}

impl Agent {
    /// Asks `prompt`, now or once the provider is ready for it.
    pub(crate) fn ask(&mut self, process: &mut Process, prompt: &str) -> Result<(), AiError> {
        match &mut self.0 {
            Party::Claude(agent) => process.send(&agent.ask(prompt)),
            Party::Codex(agent) => {
                for message in agent.ask(prompt) {
                    process.send(&message)?;
                }
                Ok(())
            }
        }
    }

    /// What one message from the provider means. Replies it needs are sent.
    pub(crate) fn receive(
        &mut self,
        process: &mut Process,
        frame: &Value,
    ) -> Result<Vec<Heard>, AiError> {
        let heard = match &mut self.0 {
            Party::Claude(agent) => agent.receive(frame)?,
            Party::Codex(agent) => agent.receive(frame)?,
        };
        let mut told = Vec::with_capacity(heard.len());
        for heard in heard {
            match heard {
                Heard::Send(message) => process.send(&message)?,
                other => told.push(other),
            }
        }
        Ok(told)
    }
}

/// Starts a conversation: the provider runs with `server` as its only tools,
/// approved for that server alone, in a private folder of its own. The
/// process lives until it is dropped, `deadline_seconds` pass or `cancel`.
pub(crate) fn converse(
    config: &AiConfig,
    server: &ToolServer,
    cancel: Arc<AtomicBool>,
    deadline_seconds: u64,
) -> Result<(Process, Agent), AiError> {
    for text in
        std::iter::once(server.command.to_str()).chain(server.args.iter().map(|a| Some(a.as_str())))
    {
        if text.is_none_or(|text| text.contains('\0')) {
            return Err(AiError::Invalid(
                "The editing tools' command cannot be passed to the provider".into(),
            ));
        }
    }
    let executable = resolve_executable(config)?;
    let mut command = Command::new(executable);
    match config.provider {
        AiProvider::Claude => {
            // Not --safe-mode: it turns off every MCP server, ours included.
            // The person's settings, instructions, hooks and commands stay
            // out through the options below.
            command
                .args([
                    "--print",
                    "--verbose",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--allowedTools",
                    "mcp__neptune-video__*",
                    "--permission-mode",
                    "dontAsk",
                    "--setting-sources",
                    "",
                    "--disable-slash-commands",
                    "--no-session-persistence",
                    "--max-turns",
                    "400",
                ])
                .arg("--system-prompt")
                .arg(conversation::INSTRUCTIONS);
            if let Some(model) = &config.model {
                command.arg("--model").arg(model);
            }
            if let Some(effort) = &config.effort {
                command.arg("--effort").arg(effort);
            }
            command
                .env_remove("CLAUDECODE")
                .env_remove("CLAUDE_CODE_SESSION_ID");
            let process = Process::spawn_limited(
                command,
                cancel,
                deadline_seconds,
                Limits::CONVERSATION,
                |command, directory| configure_claude_server(command, directory, server),
            )?;
            Ok((process, Agent(Party::Claude(ClaudeAgent::default()))))
        }
        AiProvider::Codex => {
            command.arg("app-server");
            for feature in conversation_disabled() {
                command.arg("-c").arg(format!("features.{feature}=false"));
            }
            command.arg("-c").arg("web_search=\"disabled\"");
            for setting in [
                "project_doc_max_bytes=0",
                "project_doc_fallback_filenames=[]",
                "notify=[]",
            ] {
                command.arg("-c").arg(setting);
            }
            let mut process = Process::spawn_limited(
                command,
                cancel,
                deadline_seconds,
                Limits::CONVERSATION,
                |command, directory| {
                    configure_codex_instructions(command, directory, conversation::INSTRUCTIONS)
                },
            )?;
            let agent = CodexAgent::new(config, server, process.directory());
            process.send(&CodexAgent::initialize())?;
            Ok((process, Agent(Party::Codex(Box::new(agent)))))
        }
    }
}

/// The server goes to Claude in a private file rather than on its command
/// line, so no path is shown to other users of the computer.
fn configure_claude_server(
    command: &mut Command,
    directory: &std::path::Path,
    server: &ToolServer,
) -> Result<(), AiError> {
    use std::{fs::OpenOptions, io::Write};
    let path = std::path::absolute(directory.join("neptune-video-tools.json"))
        .map_err(|_| AiError::Provider("Cannot resolve the private tool configuration".into()))?;
    let config = serde_json::json!({"mcpServers":{"neptune-video":{
        "type":"stdio",
        "command":server.command,
        "args":server.args,
        "env":{}
    }}});
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| AiError::Provider("Cannot create the private tool configuration".into()))?;
    file.write_all(config.to_string().as_bytes())
        .map_err(|_| AiError::Provider("Cannot write the private tool configuration".into()))?;
    command.arg("--mcp-config").arg(path);
    Ok(())
}

/// The encoded schema is an argument beside the planner instructions, so it
/// is held under its part of their shared command line.
fn command_line_schema(bytes: usize) -> Result<(), AiError> {
    if bytes >= planner::MAX_SCHEMA_BYTES {
        return Err(AiError::Invalid(
            "Proposal schema exceeds native command-line budget".into(),
        ));
    }
    Ok(())
}

fn configure_codex_instructions(
    command: &mut Command,
    directory: &std::path::Path,
    instructions: &str,
) -> Result<(), AiError> {
    use std::{fs::OpenOptions, io::Write};
    let path = std::path::absolute(directory.join("neptune-video-model-instructions.txt"))
        .map_err(|_| {
            AiError::Provider("Cannot resolve the private Codex instruction path".into())
        })?;
    let encoded = path.to_str().ok_or_else(|| {
        AiError::Provider("Cannot encode the private Codex instruction path".into())
    })?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|_| {
        AiError::Provider("Cannot create the private Codex instruction file".into())
    })?;
    // Codex rejects an empty/whitespace-only configured instruction file.
    // This file contains only our source-controlled prompt, never user data.
    file.write_all(instructions.as_bytes())
        .map_err(|_| AiError::Provider("Cannot write the private Codex instruction file".into()))?;
    // CLI overrides apply before Codex loads its config. Explicit baseInstructions
    // alone still makes Codex read an inherited model_instructions_file first.
    // Command arguments are passed directly, never interpolated into a shell.
    command.arg("-c").arg(format!(
        "model_instructions_file={}",
        toml_basic_string(encoded)
    ));
    Ok(())
}

/// Encode one Unicode path as a TOML basic string, including DEL/C1 controls
/// that JSON string serialization can leave unescaped. No lossy path conversion.
fn toml_basic_string(value: &str) -> String {
    use std::fmt::Write as _;
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{08}' => quoted.push_str("\\b"),
            '\u{0c}' => quoted.push_str("\\f"),
            control if control.is_control() => {
                // Formatting into String is infallible; all Unicode controls
                // fit a four-digit TOML Unicode escape.
                let _ = write!(quoted, "\\u{:04X}", control as u32);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// Resolve on the planning worker before entering its private cwd. Passing an
/// explicit path also avoids launcher/proxy ambiguity in embedded app hosts.
fn resolve_executable(config: &AiConfig) -> Result<std::path::PathBuf, AiError> {
    use std::path::{Path, PathBuf};
    let requested = config
        .executable
        .as_deref()
        .unwrap_or_else(|| Path::new(config.provider.executable()));
    if requested.is_absolute() || requested.components().count() > 1 {
        #[cfg(windows)]
        validate_windows_launcher(config.provider, requested)?;
        return std::path::absolute(requested)
            .map_err(|_| AiError::Provider("Cannot resolve native AI executable".into()));
    }
    let paths = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let mut batch_refused = false;
    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|ext| ext.starts_with('.') && ext.len() <= 16)
        .map(str::to_owned)
        .collect();
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(requested);
        #[cfg(unix)]
        let mut candidates: Vec<PathBuf> = vec![candidate];
        #[cfg(windows)]
        let mut candidates: Vec<PathBuf> = if requested.extension().is_some() {
            vec![candidate]
        } else {
            // npm installs an extensionless POSIX shim alongside its Windows
            // launchers. Follow PATHEXT instead of mistaking it for a native
            // executable before discovering a usable .exe or .cmd.
            Vec::new()
        };
        #[cfg(windows)]
        if requested.extension().is_none() {
            for extension in &extensions {
                let mut name = requested.as_os_str().to_os_string();
                name.push(extension);
                candidates.push(directory.join(name));
            }
        }
        for path in candidates.drain(..) {
            if !is_executable(&path) {
                continue;
            }
            #[cfg(windows)]
            if validate_windows_launcher(config.provider, &path).is_err() {
                // A native executable later in PATH remains usable even if an
                // earlier npm directory exposes only a batch wrapper.
                batch_refused = true;
                continue;
            }
            return std::path::absolute(path)
                .map_err(|_| AiError::Provider("Cannot resolve native AI executable".into()));
        }
    }
    #[cfg(windows)]
    if batch_refused {
        return Err(claude_batch_error());
    }
    Err(AiError::Provider(format!(
        "Cannot find {:?}; install its CLI or choose its executable in AI settings",
        config.provider
    )))
}

#[cfg(any(windows, test))]
fn validate_windows_launcher(provider: AiProvider, path: &std::path::Path) -> Result<(), AiError> {
    let batch = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    if provider == AiProvider::Claude && batch {
        return Err(claude_batch_error());
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn claude_batch_error() -> AiError {
    AiError::Provider(
        "Claude needs the native Windows CLI: install it and choose claude.exe in AI settings. .cmd and .bat launchers cannot pass Neptune Video's schema and instructions safely.".into(),
    )
}

fn is_executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        true
    }
}

pub(super) const DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "code_mode_host",
    "apps",
    "multi_agent",
    "view_image",
    "image_generation",
    "plugins",
    "hooks",
    "tool_suggest",
    "sleep_tool",
    "goals",
    "browser_use",
    "computer_use",
    "skill_search",
    "in_app_browser",
    "memories",
];

/// Codex calls MCP tools from its code-mode host, which reaches only the
/// tools a thread enables. A conversation keeps it; a planner, with no
/// tools at all, does not.
pub(super) const CONVERSATION_FEATURES: &[&str] = &["code_mode_host"];

/// The features a conversation turns off.
pub(super) fn conversation_disabled() -> impl Iterator<Item = &'static str> {
    DISABLED_FEATURES
        .iter()
        .copied()
        .filter(|feature| !CONVERSATION_FEATURES.contains(feature))
}

fn claude_result(value: Value, config: &AiConfig) -> Result<ProviderOutput, AiError> {
    if value.get("type").and_then(Value::as_str) != Some("result")
        || value.get("is_error") == Some(&Value::Bool(true))
        || value.get("subtype").and_then(Value::as_str) != Some("success")
    {
        return Err(AiError::Provider("Claude could not complete the request; check its sign-in, model access and usage limits".into()));
    }
    let structured = value
        .get("structured_output")
        .filter(|value| value.is_object())
        .ok_or_else(|| AiError::Provider("Claude returned no structured edit proposal".into()))?;
    let bytes = serde_json::to_vec(structured)
        .map_err(|_| AiError::Provider("Claude returned invalid JSON".into()))?;
    let model = value
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .filter(|model| model.len() <= 128)
        .cloned()
        .or_else(|| config.model.clone());
    Ok(ProviderOutput { bytes, model })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_requires_successful_structured_output() {
        let cfg = AiConfig::default();
        let output = claude_result(json!({"type":"result","subtype":"success","is_error":false,"structured_output":{"summary":"Rename","commands":[]},"modelUsage":{"actual-model":{}}}), &cfg).unwrap();
        assert_eq!(output.model.as_deref(), Some("actual-model"));
        assert!(claude_result(json!({"type":"result","subtype":"error_max_turns","structured_output":{"summary":"Not completed","commands":[]}}), &cfg).is_err());
        assert!(
            claude_result(
                json!({"type":"result","subtype":"success","result":"A text promise"}),
                &cfg
            )
            .is_err()
        );
    }

    #[test]
    fn a_schema_at_its_part_of_the_command_line_is_refused_before_launch() {
        assert_eq!(planner::MAX_SCHEMA_BYTES, 23 * 1024);
        command_line_schema(planner::MAX_SCHEMA_BYTES - 1).unwrap();
        for bytes in [planner::MAX_SCHEMA_BYTES, 24 * 1024] {
            assert!(matches!(
                command_line_schema(bytes),
                Err(AiError::Invalid(message)) if message.contains("command-line budget")
            ));
        }
        // The schema the planner sends passes, with the instructions under theirs.
        let schema = serde_json::to_string(&planner::proposal_schema()).unwrap();
        command_line_schema(schema.len()).unwrap();
        assert!(planner::SYSTEM.len() < planner::MAX_SYSTEM_BYTES);
    }

    #[test]
    fn explicit_relative_executable_keeps_launch_directory() {
        let config = AiConfig {
            executable: Some("./tools/provider".into()),
            ..AiConfig::default()
        };
        assert_eq!(
            resolve_executable(&config).unwrap(),
            std::env::current_dir().unwrap().join("tools/provider")
        );
    }

    #[test]
    fn windows_claude_batch_launchers_are_refused_with_native_installation_guidance() {
        for path in ["claude.cmd", "CLAUDE.CMD", "claude.bat", "Claude.BaT"] {
            let error = validate_windows_launcher(AiProvider::Claude, std::path::Path::new(path))
                .unwrap_err();
            assert!(error.to_string().contains("claude.exe"));
            assert!(error.to_string().contains("native Windows CLI"));
        }
        assert!(
            validate_windows_launcher(AiProvider::Claude, std::path::Path::new("claude.exe"))
                .is_ok()
        );
        // Codex sends the schema and metadata through stdin, not batch argv.
        assert!(
            validate_windows_launcher(AiProvider::Codex, std::path::Path::new("codex.cmd")).is_ok()
        );
    }

    #[test]
    fn private_codex_instruction_file_contains_only_neptune_video_instructions_and_preserves_unicode_path()
     {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("Unicode 日本語 folder");
        std::fs::create_dir(&directory).unwrap();
        let mut command = Command::new("owned-provider");
        configure_codex_instructions(&mut command, &directory, planner::SYSTEM).unwrap();
        let path = directory.join("neptune-video-model-instructions.txt");
        assert_eq!(std::fs::read(&path).unwrap(), planner::SYSTEM.as_bytes());
        let arguments: Vec<_> = command
            .get_args()
            .map(|value| value.to_str().unwrap())
            .collect();
        let setting = format!(
            "model_instructions_file={}",
            toml_basic_string(path.to_str().unwrap())
        );
        assert_eq!(arguments, vec!["-c", setting.as_str()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn toml_instruction_paths_escape_quotes_backslashes_and_controls_without_losing_unicode() {
        assert_eq!(
            toml_basic_string("C:\\owned\\日本語 \"quoted\"\\prompt.txt"),
            "\"C:\\\\owned\\\\日本語 \\\"quoted\\\"\\\\prompt.txt\""
        );
        assert_eq!(
            toml_basic_string("line\n\t\r\u{08}\u{0c}\u{1b}\u{7f}\u{85}"),
            "\"line\\n\\t\\r\\b\\f\\u001B\\u007F\\u0085\""
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unrepresentable_native_path_is_refused_before_creating_an_instruction_file() {
        use std::os::unix::ffi::OsStrExt;
        let parent = tempfile::tempdir().unwrap();
        let directory = parent
            .path()
            .join(std::ffi::OsStr::from_bytes(b"non-utf8-\xff"));
        std::fs::create_dir(&directory).unwrap();
        let mut command = Command::new("owned-provider");
        assert!(configure_codex_instructions(&mut command, &directory, planner::SYSTEM).is_err());
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
        assert_eq!(command.get_args().count(), 0);
    }
}
