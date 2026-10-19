//! The `job` tool: long work and exports, with their progress, waited for
//! with progress notifications, and cancelled.
use crate::mcp::{
    errors::ToolError,
    jobs::{JobInfo, JobState},
    server::NeptuneVideoServer,
};
use editor_agent::{AgentOperation, AgentResult};
use editor_core::export::{RenderJobInfo, RenderState};
use rmcp::{
    Json, RoleServer, handler::server::wrapper::Parameters, model::ProgressNotificationParam,
    service::RequestContext, tool, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The longest one wait lasts: under the minute clients give a call.
const LONGEST_WAIT: u64 = 50;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct JobParams {
    /// `list`, `status`, `wait` (until it ends or `timeout_seconds`
    /// passes), `cancel`; for exports also `pause`, `resume`, `retry` and
    /// `remove`.
    pub op: String,
    #[serde(default)]
    pub job_id: Option<Uuid>,
    /// `wait`: 20 by default, at most 50. Wait again if it is still running.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Leave results out of `list`, which can be long.
    #[serde(default)]
    pub brief: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct JobList {
    pub jobs: Vec<JobInfo>,
}

fn export_info(job: &RenderJobInfo) -> JobInfo {
    let state = match job.state {
        RenderState::Done => JobState::Done,
        RenderState::Failed => JobState::Failed,
        RenderState::Cancelled => JobState::Cancelled,
        _ => JobState::Running,
    };
    let phase = serde_json::to_value(job.state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    JobInfo {
        job_id: job.id,
        kind: "export".into(),
        state,
        progress: job.progress,
        message: match &job.message {
            Some(message) if !message.is_empty() => format!("{phase}: {message}"),
            _ => phase,
        },
        result: Some(match state {
            JobState::Done => serde_json::json!({
                "destination": job.destination,
                "output_bytes": job.output_bytes,
                "frames": job.frames_total,
                "encoder": job.encoder,
                "revision": job.revision,
            }),
            _ => serde_json::json!({
                "destination": job.destination,
                "frames_done": job.frames_done,
                "frames_total": job.frames_total,
                "speed_fps": job.speed_fps,
                "eta_seconds": job.eta_seconds,
            }),
        }),
        error: (state == JobState::Failed).then(|| {
            ToolError::new(
                crate::mcp::errors::codes::JOB_FAILED,
                job.message
                    .clone()
                    .unwrap_or_else(|| "The export failed".into()),
            )
        }),
        elapsed_seconds: job.elapsed_seconds,
        started_at: None,
    }
}

/// Every job: the server's own and the exports of the project's queue.
pub async fn all(server: &NeptuneVideoServer) -> JobList {
    let mut jobs = server.shared.jobs.list();
    if let Ok(exports) = exports(server).await {
        jobs.extend(exports.iter().map(export_info));
    }
    JobList { jobs }
}

async fn exports(server: &NeptuneVideoServer) -> Result<Vec<RenderJobInfo>, ToolError> {
    server
        .on(|workspace| {
            let response = workspace.send(AgentOperation::RenderJobs, None)?;
            match workspace.answer(response)? {
                (_, AgentResult::RenderJobs { jobs }) => Ok(jobs),
                _ => Err(ToolError::unavailable(
                    "The render queue answered something else",
                )),
            }
        })
        .await
}

async fn find(server: &NeptuneVideoServer, id: Uuid) -> Result<JobInfo, ToolError> {
    if let Some(job) = server.shared.jobs.get(id) {
        return Ok(job);
    }
    exports(server)
        .await?
        .iter()
        .find(|job| job.id == id)
        .map(export_info)
        .ok_or_else(|| ToolError::not_found(format!("No job {id}")))
}

async fn render_op(
    server: &NeptuneVideoServer,
    operation: AgentOperation,
) -> Result<JobInfo, ToolError> {
    server
        .on(move |workspace| {
            let response = workspace.send(operation, None)?;
            match workspace.answer(response)? {
                (_, AgentResult::RenderJob { job }) => Ok(export_info(&job)),
                _ => Err(ToolError::unavailable(
                    "The render queue answered something else",
                )),
            }
        })
        .await
}

#[tool_router(router = job_tools, vis = "pub(crate)")]
impl NeptuneVideoServer {
    #[tool(
        description = "Jobs (transcription, model installs, scenes, loudness, exports): \
                          list, status, wait for one with progress, cancel; pause, resume, \
                          retry or remove an export."
    )]
    async fn job(
        &self,
        Parameters(params): Parameters<JobParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let mut out = serde_json::Map::new();
        if params.op == "list" {
            let mut list = all(self).await;
            if params.brief {
                for job in &mut list.jobs {
                    job.result = None;
                }
            }
            out.insert(
                "jobs".into(),
                serde_json::to_value(list.jobs).unwrap_or_default(),
            );
            return Ok(Json(out));
        }
        let id = params
            .job_id
            .ok_or_else(|| ToolError::invalid("Give job_id"))?;
        let info = match params.op.as_str() {
            "status" => find(self, id).await?,
            "wait" => {
                let limit = Duration::from_secs(
                    params.timeout_seconds.unwrap_or(20).clamp(1, LONGEST_WAIT),
                );
                let until = Instant::now() + limit;
                let token = context.meta.get_progress_token();
                let mut last_told = -1.0_f32;
                loop {
                    let info = find(self, id).await?;
                    if info.state.finished() || Instant::now() >= until || context.ct.is_cancelled()
                    {
                        break info;
                    }
                    if let Some(token) = &token
                        && info.progress > last_told
                    {
                        last_told = info.progress;
                        let _ = context
                            .peer
                            .notify_progress(
                                ProgressNotificationParam::new(
                                    token.clone(),
                                    f64::from(info.progress),
                                )
                                .with_total(1.0)
                                .with_message(info.message.clone()),
                            )
                            .await;
                    }
                    let left = until.saturating_duration_since(Instant::now());
                    // A server job says when it changes; an export is read
                    // twice a second.
                    tokio::select! {
                        _ = self.shared.jobs.changed() => {}
                        _ = tokio::time::sleep(left.min(Duration::from_millis(500))) => {}
                        _ = context.ct.cancelled() => {}
                    }
                }
            }
            "cancel" => {
                let mut info = match self.shared.jobs.cancel(id) {
                    Some(info) => info,
                    None => render_op(self, AgentOperation::CancelRender { job: id }).await?,
                };
                // A running job stops within a moment; answer how it ended.
                for _ in 0..20 {
                    if info.state.finished() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    info = find(self, id).await?;
                }
                info
            }
            "pause" => render_op(self, AgentOperation::PauseRender { job: id }).await?,
            "resume" => render_op(self, AgentOperation::ResumeRender { job: id }).await?,
            "retry" => render_op(self, AgentOperation::RetryRender { job: id }).await?,
            "remove" => render_op(self, AgentOperation::RemoveRender { job: id }).await?,
            other => {
                return Err(ToolError::invalid(format!(
                    "op is list, status, wait, cancel, pause, resume, retry or remove, not {other}"
                )));
            }
        };
        out.insert("job".into(), serde_json::to_value(info).unwrap_or_default());
        Ok(Json(out))
    }
}
