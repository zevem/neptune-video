//! Exports: what this computer can encode, and exports or quick previews
//! queued in the render queue of whoever holds the project (the window's,
//! or this server's own).
use super::check_readable;
use crate::mcp::{
    errors::{ToolError, codes},
    server::NeptuneVideoServer,
    time::TimeSpec,
};
use editor_agent::{AgentOperation, AgentResult};
use editor_core::export::{
    EncodeSpeed, ExportRange, ExportSettings, RateControl, Resolution, apply_preset,
    builtin_presets,
};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CapabilityParams {
    /// Include the JSON schema of export settings.
    #[serde(default)]
    pub schema: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    /// The file to write, inside the roots; its extension should suit the
    /// container.
    pub destination: String,
    /// A preset id from render_capabilities: recommended (the default),
    /// high-quality, small-file, youtube-1080, youtube-shorts, prores-master,
    /// webm, audio-wav…
    #[serde(default)]
    pub preset: Option<String>,
    /// Export settings to use instead (render_capabilities {schema: true}
    /// describes them). The range below still applies.
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
    /// A quick, small, low-quality export to watch: 540 pixels on the short
    /// side, fastest encoding.
    #[serde(default)]
    pub preview: bool,
    /// Only this stretch of the timeline.
    #[serde(default)]
    pub from: Option<TimeSpec>,
    #[serde(default)]
    pub to: Option<TimeSpec>,
    /// Replace a file that is there.
    #[serde(default)]
    pub overwrite: bool,
}

/// What the authority can export, once FFmpeg's encoders have been checked
/// (the first moments after a start).
async fn capabilities(server: &NeptuneVideoServer) -> Result<AgentResult, ToolError> {
    for _ in 0..60 {
        let answer = server
            .on(|workspace| {
                let response = workspace.send(AgentOperation::ExportCapabilities, None)?;
                match response.result {
                    AgentResult::Error { code, .. } if code == "capabilities_pending" => Ok(None),
                    _ => workspace.answer(response).map(|(_, result)| Some(result)),
                }
            })
            .await?;
        if let Some(result) = answer {
            return Ok(result);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(ToolError::new(
        codes::BUSY,
        "FFmpeg's encoders are still being checked; ask again",
    ))
}

#[tool_router(router = render_tools, vis = "pub(crate)")]
impl NeptuneVideoServer {
    #[tool(
        description = "What this computer can export: containers, video and audio encoders \
                       (hardware ones too), and the export presets.",
        annotations(read_only_hint = true)
    )]
    async fn render_capabilities(
        &self,
        Parameters(params): Parameters<CapabilityParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let AgentResult::ExportCapabilities {
            capabilities,
            presets,
            settings_schema,
        } = capabilities(self).await?
        else {
            return Err(ToolError::unavailable(
                "The authority answered something else",
            ));
        };
        let mut out = serde_json::Map::new();
        out.insert(
            "capabilities".into(),
            serde_json::to_value(&capabilities).unwrap_or_default(),
        );
        out.insert(
            "presets".into(),
            presets
                .iter()
                .map(|preset| serde_json::json!({"id": preset.id, "name": preset.name, "note": preset.note, "container": preset.settings.container}))
                .collect(),
        );
        if params.schema {
            out.insert("settings_schema".into(), settings_schema);
        }
        Ok(Json(out))
    }

    #[tool(
        description = "Export the project (or a stretch of it) to a file, as a job: a \
                          preset, your own settings, or a quick preview. Poll with job {op: \
                          wait}; an export from this server ends if the server stops."
    )]
    async fn render_export_start(
        &self,
        Parameters(params): Parameters<ExportParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let destination = self
            .roots()
            .destination(std::path::Path::new(&params.destination), params.overwrite)?;
        // The queue takes an export once it knows the encoders.
        capabilities(self).await?;
        let mut settings = match (&params.settings, &params.preset) {
            (Some(_), Some(_)) => {
                return Err(ToolError::invalid("Give preset or settings, not both"));
            }
            (Some(value), None) => serde_json::from_value::<ExportSettings>(value.clone())
                .map_err(|error| {
                    ToolError::invalid(format!("Those settings are not valid: {error}"))
                })?,
            (None, preset) => {
                let id = preset.as_deref().unwrap_or(if params.preview {
                    "small-file"
                } else {
                    "recommended"
                });
                let preset = builtin_presets()
                    .into_iter()
                    .find(|preset| preset.id == id)
                    .ok_or_else(|| {
                        ToolError::invalid(format!(
                            "No preset {id}; render_capabilities lists them"
                        ))
                    })?;
                apply_preset(&ExportSettings::default(), &preset)
            }
        };
        if params.preview
            && let Some(video) = &mut settings.video
        {
            video.resolution = Resolution::Short { pixels: 540 };
            video.speed = EncodeSpeed::Faster;
            video.rate_control = RateControl::Quality {
                level: editor_core::export::QualityLevel::Lower,
            };
        }
        let roots = self.shared.roots.clone();
        let from = params.from.clone();
        let to = params.to.clone();
        let overwrite = params.overwrite;
        let preview = params.preview;
        let (revision, result, authority) = self
            .on(move |workspace| {
                let snapshot = workspace.snapshot()?;
                check_readable(&snapshot.project, &roots)?;
                let rate = snapshot.project.settings.frame_rate;
                if from.is_some() || to.is_some() {
                    let start = from
                        .as_ref()
                        .map(|t| t.at(rate))
                        .transpose()?
                        .unwrap_or_default();
                    let end = match &to {
                        Some(to) => to.at(rate)?,
                        None => snapshot.project.duration(),
                    };
                    settings.range = ExportRange::Span { start, end };
                }
                roots.create_parent(&destination)?;
                let response = workspace.send(
                    AgentOperation::QueueExport {
                        settings,
                        destination,
                        overwrite,
                    },
                    Some(snapshot.revision),
                )?;
                let (revision, result) = workspace.answer(response)?;
                Ok((revision, result, workspace.authority_name()))
            })
            .await?;
        let AgentResult::RenderJob { job } = result else {
            return Err(ToolError::unavailable(
                "The render queue answered something else",
            ));
        };
        let mut out = serde_json::Map::new();
        out.insert("job_id".into(), serde_json::json!(job.id));
        out.insert("revision".into(), revision.into());
        out.insert("preview".into(), preview.into());
        out.insert(
            "job".into(),
            serde_json::to_value(&*job).unwrap_or_default(),
        );
        if authority == "local" {
            out.insert(
                "note".into(),
                "This export runs in this server; it stops if the server stops.".into(),
            );
        }
        Ok(Json(out))
    }
}
