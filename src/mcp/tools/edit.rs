//! Core commands as they are: any edit the editor knows, in a batch that is
//! one history entry, checked before it is applied. New editor features are
//! reachable here as soon as they are commands.
use super::Guard;
use crate::mcp::{
    errors::ToolError, server::NeptuneVideoServer, time::TimeSpec, workspace::Edited,
};
use editor_core::{AnimationTrim, Command, FrameRate, Time};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

/// The most commands one call applies.
const MOST_COMMANDS: usize = 10_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyParams {
    /// Core commands (edit_schema describes them), applied in order as one
    /// undoable step: all of them or none. Times are integer microseconds.
    pub commands: Vec<serde_json::Value>,
    /// The history entry's name; the command's own for one command.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SchemaParams {
    /// One command type (add_clip, set_effects, …) instead of the list.
    #[serde(default)]
    pub command: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchClipParams {
    pub clip_id: Uuid,
    #[serde(default)]
    pub start: Option<TimeSpec>,
    #[serde(default)]
    pub duration: Option<TimeSpec>,
    /// Exact source time; never snapped to project frames.
    #[serde(default)]
    pub source_in: Option<TimeSpec>,
    /// Retarget a media clip, keeping its other properties.
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Partial stored clip fields. Objects merge, arrays replace, null clears
    /// optional fields. Times here are microseconds. See edit_schema patch_clip.
    #[serde(default)]
    pub changes: serde_json::Map<String, serde_json::Value>,
    /// crop (default) fits existing timed data when shortening; preserve
    /// refuses data that no longer fits. Explicit replacements must fit.
    #[serde(default)]
    pub trim_animation: AnimationTrim,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OffsetClipsParams {
    pub clip_ids: Vec<Uuid>,
    /// Signed offset (frame, seconds, us or nonnegative timecode). Linked
    /// and grouped clips follow, as with a timeline move.
    pub delta: TimeSpec,
    #[serde(flatten)]
    pub guard: Guard,
}

fn offset_time(spec: &TimeSpec, rate: FrameRate) -> Result<Time, ToolError> {
    let negative = spec.frame.is_some_and(|value| value < 0)
        || spec.seconds.is_some_and(|value| value < 0.0)
        || spec.us.is_some_and(|value| value < 0);
    let absolute = |value: Option<i64>| {
        value
            .map(|value| {
                value
                    .checked_abs()
                    .ok_or_else(|| ToolError::invalid("delta is outside the timeline"))
            })
            .transpose()
    };
    let magnitude = TimeSpec {
        frame: absolute(spec.frame)?,
        seconds: spec.seconds.map(f64::abs),
        us: absolute(spec.us)?,
        timecode: spec.timecode.clone(),
    }
    .exact(rate)?;
    Ok(if negative {
        Time(-magnitude.0)
    } else {
        magnitude
    })
}

fn command_names(schema: &serde_json::Value) -> Vec<String> {
    schema["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            variant["properties"]["type"]["const"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

#[tool_router(router = edit_tools, vis = "pub(crate)")]
impl NeptuneVideoServer {
    #[tool(
        description = "Patch one clip in place, retaining its ID. start/duration use project \
                       frames; source_in is exact. changes merges stored fields recursively \
                       (arrays replace; null clears optional fields); use effect_edit and \
                       keyframes for individual effects/animation tracks. Shortening crops \
                       existing curves with an interpolated end key. Only this clip changes."
    )]
    async fn patch_clip(
        &self,
        Parameters(params): Parameters<PatchClipParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            super::clip(project, params.clip_id)?;
            let rate = project.settings.frame_rate;
            let mut patch = params.changes;
            for (name, value) in [
                ("start", params.start.map(|time| time.at(rate)).transpose()?),
                (
                    "duration",
                    params.duration.map(|time| time.length(rate)).transpose()?,
                ),
                (
                    "source_in",
                    params.source_in.map(|time| time.exact(rate)).transpose()?,
                ),
            ] {
                if let Some(time) = value
                    && patch.insert(name.into(), time.0.into()).is_some()
                {
                    return Err(ToolError::invalid(format!(
                        "Give {name} once, outside changes"
                    )));
                }
            }
            if let Some(asset_id) = params.asset_id {
                let asset_id = super::resolve_asset_id(project, &asset_id)?;
                let content = patch
                    .entry("content")
                    .or_insert_with(|| serde_json::json!({}));
                let content = content
                    .as_object_mut()
                    .ok_or_else(|| ToolError::invalid("changes.content must be an object"))?;
                if content
                    .insert("asset_id".into(), asset_id.to_string().into())
                    .is_some()
                {
                    return Err(ToolError::invalid("Give asset_id once, outside changes"));
                }
            }
            if patch.is_empty() {
                return Err(ToolError::invalid("Give fields to patch"));
            }
            Ok(Command::PatchClip {
                clip_id: params.clip_id,
                patch,
                trim_animation: params.trim_animation,
            })
        })
        .await
    }

    #[tool(
        description = "Shift clips by a signed delta, keeping IDs and clip-relative keys. \
                          Linked/grouped clips follow. A locked track or invalid placement \
                          refuses the whole edit; project visibility is unchanged."
    )]
    async fn offset_clips(
        &self,
        Parameters(params): Parameters<OffsetClipsParams>,
    ) -> Result<Json<Edited>, ToolError> {
        if params.clip_ids.is_empty() || params.clip_ids.len() > MOST_COMMANDS {
            return Err(ToolError::invalid(format!(
                "Give 1 to {MOST_COMMANDS} clip_ids"
            )));
        }
        self.edit(params.guard, move |snapshot| {
            let delta = offset_time(&params.delta, snapshot.project.settings.frame_rate)?;
            Ok(Command::MoveClips {
                clip_ids: params.clip_ids,
                delta,
                target_track: None,
            })
        })
        .await
    }

    #[tool(
        description = "Apply core editing commands as one undoable, all-or-nothing step: \
                          every edit the editor has, including ones the typed tools do not \
                          cover. dry_run checks them first."
    )]
    async fn edit_apply(
        &self,
        Parameters(params): Parameters<ApplyParams>,
    ) -> Result<Json<Edited>, ToolError> {
        if params.commands.is_empty() || params.commands.len() > MOST_COMMANDS {
            return Err(ToolError::invalid(format!(
                "Give 1 to {MOST_COMMANDS} commands"
            )));
        }
        let mut commands = Vec::with_capacity(params.commands.len());
        for (index, value) in params.commands.into_iter().enumerate() {
            let command: Command = serde_json::from_value(value).map_err(|error| {
                ToolError::invalid(format!(
                    "Command {index} is not a command ({error}); edit_schema describes them"
                ))
            })?;
            commands.push(command);
        }
        let command = match (commands.len(), params.label) {
            (1, None) => commands.remove(0),
            (_, label) => Command::Batch {
                label: label.unwrap_or_else(|| "Agent edit".into()),
                commands,
            },
        };
        self.edit(params.guard, move |_| Ok(command)).await
    }

    #[tool(
        description = "The JSON schema of core commands for edit_apply: the list of command \
                       types, or one command's schema with the types it uses.",
        annotations(read_only_hint = true)
    )]
    async fn edit_schema(
        &self,
        Parameters(params): Parameters<SchemaParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap_or_default();
        let mut out = serde_json::Map::new();
        match params.command {
            None => {
                out.insert("commands".into(), command_names(&schema).into());
                out.insert(
                    "note".into(),
                    "Times are integer microseconds; keyframe and word times are from the \
                     clip's start. Ask for one command to see its fields."
                        .into(),
                );
            }
            Some(name) => {
                let variant = schema["oneOf"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|variant| variant["properties"]["type"]["const"] == name.as_str())
                    .cloned()
                    .ok_or_else(|| ToolError::not_found(format!("No command {name}")))?;
                out.insert("command".into(), variant);
                out.insert("$defs".into(), schema["$defs"].clone());
                if name == "patch_clip" {
                    out.insert("note".into(), "patch takes partial fields of $defs.Clip; objects merge recursively, arrays replace, null clears optional fields. id, linked_id and group_id are protected. crop fits existing timed data before explicit replacements, so replacements must fit the new duration. Only the named clip changes; use offset_clips to move linked/grouped clips together.".into());
                }
            }
        }
        Ok(Json(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_names_every_command() {
        let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap();
        let names = command_names(&schema);
        assert!(names.contains(&"add_clip".to_owned()), "{names:?}");
        assert!(names.contains(&"patch_clip".to_owned()), "{names:?}");
        assert!(names.contains(&"batch".to_owned()));
        assert!(names.len() > 40);
    }

    #[test]
    fn offset_accepts_negative_and_exact_subframe_deltas() {
        let rate = FrameRate::default();
        for (given, expected) in [
            (serde_json::json!({"seconds": -0.5}), Time(-500_000)),
            (serde_json::json!({"us": -16_667}), Time(-16_667)),
            (serde_json::json!({"frame": -3}), Time(-100_000)),
            (serde_json::json!({"us": 16_667}), Time(16_667)),
        ] {
            let spec = serde_json::from_value(given).unwrap();
            assert_eq!(offset_time(&spec, rate).unwrap(), expected);
        }
        for given in [
            serde_json::json!({"seconds": -0.5, "us": -500_000}),
            serde_json::json!({"us": i64::MIN}),
            serde_json::json!({}),
        ] {
            let spec = serde_json::from_value(given).unwrap();
            assert!(offset_time(&spec, rate).is_err());
        }
    }
}
