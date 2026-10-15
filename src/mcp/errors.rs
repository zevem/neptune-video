//! Refusals as an agent reads them: a tool result marked `isError` whose
//! structured content is `{code, message, ...}`. JSON-RPC errors are kept
//! for requests that are malformed or name no tool.
use rmcp::{
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult},
};
use schemars::JsonSchema;
use serde::Serialize;

/// Codes this server adds to the protocol's (`stale_revision`,
/// `invalid_edit`, `stale_context`, ...).
pub mod codes {
    pub const OUTSIDE_ROOTS: &str = "outside_roots";
    pub const NOT_FOUND: &str = "not_found";
    pub const NO_PROJECT: &str = "no_project";
    pub const PROJECT_CHANGED: &str = "project_changed";
    pub const PROJECT_BUSY: &str = "project_busy";
    pub const RECOVERY_AVAILABLE: &str = "recovery_available";
    pub const PROJECT_CONFLICTED: &str = "project_conflicted";
    pub const DESKTOP_CONFLICT: &str = "desktop_conflict";
    pub const DESKTOP_STALE: &str = "desktop_stale";
    pub const DESKTOP_CLOSED: &str = "desktop_closed";
    pub const NOT_AGENT_ENTRY: &str = "not_agent_entry";
    pub const TOO_LARGE: &str = "too_large";
    pub const BUSY: &str = "busy";
    pub const JOB_FAILED: &str = "job_failed";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const INDETERMINATE: &str = "indeterminate";
    pub const INVALID: &str = "invalid";
    pub const OVERLAP: &str = "overlap";
    pub const EXISTS: &str = "destination_exists";
    pub const STORAGE: &str = "storage";
}

/// Why a tool did nothing. `revision` is the project's revision when it was
/// refused, where one is known.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// More about the refusal: the problems found, the sessions in conflict.
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            revision: None,
            detail: serde_json::Value::Null,
        }
    }

    pub fn at(mut self, revision: u64) -> Self {
        self.revision = Some(revision);
        self
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = detail;
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(codes::UNAVAILABLE, message)
    }

    pub fn no_project() -> Self {
        Self::new(
            codes::NO_PROJECT,
            "No project is open: call project_create, project_open or desktop {op: attach}",
        )
    }
}

impl From<editor_store::StoreError> for ToolError {
    fn from(error: editor_store::StoreError) -> Self {
        use editor_store::StoreError;
        let code = match &error {
            StoreError::DocumentConflict(_) => "storage_conflict",
            StoreError::DocumentBusy(_) => "storage_busy",
            StoreError::RecoveryInUse(_) => codes::PROJECT_BUSY,
            _ => codes::STORAGE,
        };
        Self::new(code, error.to_string())
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, rmcp::ErrorData> {
        let value = serde_json::to_value(&self)
            .unwrap_or_else(|_| serde_json::json!({"code": self.code, "message": self.message}));
        Ok(CallToolResult::structured_error(value).into())
    }
}
