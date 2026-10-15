//! Sound: clip volume and fades, track mix, background music, detaching a
//! video's sound, noise reduction, and normalizing to a measured loudness.
use super::{Guard, batch, clip, refused};
use crate::{
    mcp::{errors::ToolError, server::AsterServer, time::TimeSpec, workspace::Edited},
    normalize,
    ui::fades::{self, Asked, FadeEdge},
};
use editor_core::Command;
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AudioParams {
    /// `volume` (clips), `fades` (clips), `track` (a track's mix),
    /// `music` (lay media under the whole project on a Music track),
    /// `detach` (a video's sound onto an audio track, linked), `noise`
    /// (reduce noise on audio-track clips) or `normalize` (to -14 LUFS from
    /// a finished loudness job).
    pub op: String,
    #[serde(default)]
    pub clip_ids: Vec<Uuid>,
    /// `volume`: 0 to 8, 1 as recorded.
    #[serde(default)]
    pub gain: Option<f32>,
    /// `volume`: in decibels, instead of gain.
    #[serde(default)]
    pub db: Option<f32>,
    /// `fades`: the fade in's length; zero removes it.
    #[serde(default)]
    pub fade_in: Option<TimeSpec>,
    /// `fades`: the fade out's length; zero removes it.
    #[serde(default)]
    pub fade_out: Option<TimeSpec>,
    /// `track`.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default)]
    pub solo: Option<bool>,
    /// `music`: the sound to lay under the project.
    #[serde(default)]
    pub asset_id: Option<Uuid>,
    /// `noise`: 0 to 1.
    #[serde(default)]
    pub amount: Option<f32>,
    /// `normalize`: a finished loudness_start job.
    #[serde(default)]
    pub job_id: Option<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[tool_router(router = audio_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Sound: set clip volume (gain or dB), fades, a track's volume, mute \
                          or solo, lay background music, detach a video's sound, reduce noise, \
                          or normalize clips from a loudness job. Audio effects are in \
                          effect_edit."
    )]
    async fn audio_edit(
        &self,
        Parameters(params): Parameters<AudioParams>,
    ) -> Result<Json<Edited>, ToolError> {
        let measured = match (params.op.as_str(), params.job_id) {
            ("normalize", Some(job)) => Some(self.shared.jobs.result(job, "loudness")?),
            ("normalize", None) => return Err(ToolError::invalid("Give the loudness job_id")),
            _ => None,
        };
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            for id in &params.clip_ids {
                clip(project, *id)?;
            }
            let need_clips = || {
                if params.clip_ids.is_empty() {
                    Err(ToolError::invalid("Name the clips in clip_ids"))
                } else {
                    Ok(())
                }
            };
            match params.op.as_str() {
                "volume" => {
                    need_clips()?;
                    let gain = match (params.gain, params.db) {
                        (Some(gain), None) => gain,
                        (None, Some(db)) => 10_f32.powf(db / 20.0),
                        _ => return Err(ToolError::invalid("Give gain or db")),
                    };
                    if !(0.0..=8.0).contains(&gain) {
                        return Err(ToolError::invalid("Volume is from 0 to 8 (+18 dB)"));
                    }
                    let mut commands = Vec::new();
                    for id in &params.clip_ids {
                        let (_, target) = clip(project, *id)?;
                        if !fades::has_sound(project, target) {
                            return Err(refused("That clip has no sound"));
                        }
                        if editor_core::sound::owned(project, target) {
                            return Err(refused(format!("{}: change its volume there", fades::OWNED)));
                        }
                        if let Some(command) = normalize::command(target, gain).map_err(refused)? {
                            commands.push(command);
                        }
                    }
                    batch("Change clip volume", commands)
                }
                "fades" => {
                    need_clips()?;
                    let fade_in = params.fade_in.as_ref().map(|t| t.at(rate)).transpose()?;
                    let fade_out = params.fade_out.as_ref().map(|t| t.at(rate)).transpose()?;
                    match (fade_in, fade_out) {
                        (None, None) => Err(ToolError::invalid("Give fade_in, fade_out or both")),
                        // One edge: the window's own rule, which fits it in
                        // the room the other fade leaves.
                        (Some(length), None) | (None, Some(length)) => {
                            let edge = if fade_in.is_some() { FadeEdge::In } else { FadeEdge::Out };
                            fades::command(project, &params.clip_ids, Asked::Set(edge, length)).map_err(refused)
                        }
                        // Both edges at once: the pair must fit the clip.
                        (Some(fade_in), Some(fade_out)) => {
                            let mut commands = Vec::new();
                            for id in &params.clip_ids {
                                if let Some(reason) = fades::refusal(project, *id) {
                                    return Err(refused(reason));
                                }
                                let (_, target) = clip(project, *id)?;
                                if fade_in < editor_core::Time::ZERO || fade_out < editor_core::Time::ZERO || fade_in + fade_out > target.duration {
                                    return Err(refused(format!(
                                        "The fades together must fit the clip, which is {:.3} s long",
                                        target.duration.as_seconds()
                                    )));
                                }
                                if (fade_in, fade_out) != (target.fade_in, target.fade_out) {
                                    commands.push(Command::SetFades { clip_id: *id, fade_in, fade_out });
                                }
                            }
                            if commands.is_empty() {
                                return Err(refused("Those fades are there already"));
                            }
                            batch("Change fades", commands)
                        }
                    }
                }
                "track" => {
                    let track = super::track(project, params.track_id.ok_or_else(|| ToolError::invalid("Give track_id"))?)?;
                    let gain = match (params.gain, params.db) {
                        (Some(gain), None) => Some(gain),
                        (None, Some(db)) => Some(10_f32.powf(db / 20.0)),
                        (None, None) => None,
                        _ => return Err(ToolError::invalid("Give gain or db, not both")),
                    };
                    Ok(Command::SetTrackState {
                        track_id: track.id,
                        locked: None,
                        hidden: None,
                        muted: params.muted,
                        name: None,
                        gain,
                        solo: params.solo,
                    })
                }
                "music" => {
                    let asset = params.asset_id.ok_or_else(|| ToolError::invalid("Give asset_id"))?;
                    crate::rules::background_music(project, asset).map_err(refused)
                }
                "detach" => {
                    let [id] = params.clip_ids.as_slice() else {
                        return Err(ToolError::invalid("Detach one clip at a time"));
                    };
                    crate::rules::detach_audio(project, *id).map_err(refused)
                }
                "noise" => {
                    need_clips()?;
                    let amount = params.amount.unwrap_or(0.5);
                    crate::audio::noise_command(project, &params.clip_ids, amount).map_err(refused)
                }
                "normalize" => {
                    let measured = measured.expect("read above");
                    if measured["project_id"].as_str() != Some(project.id.to_string().as_str()) {
                        return Err(ToolError::new(
                            crate::mcp::errors::codes::PROJECT_CHANGED,
                            "That measurement is of another project; measure again",
                        ));
                    }
                    let mut commands = Vec::new();
                    let mut skipped = Vec::new();
                    let mut changed = Vec::new();
                    for entry in measured["clips"].as_array().into_iter().flatten() {
                        let Some(id) = entry["clip_id"].as_str().and_then(|id| Uuid::parse_str(id).ok()) else {
                            continue;
                        };
                        if !params.clip_ids.is_empty() && !params.clip_ids.contains(&id) {
                            continue;
                        }
                        let (Some(integrated), Some(peak)) = (
                            entry["integrated_lufs"].as_f64(),
                            entry["true_peak_dbfs"].as_f64(),
                        ) else {
                            skipped.push(id);
                            continue;
                        };
                        let Some((_, target)) = project.clip(id) else {
                            skipped.push(id);
                            continue;
                        };
                        // A clip trimmed, given other media or other audio
                        // effects since is no longer what was measured.
                        let now = crate::normalize::sound_key(project, id).map(|key| key.to_string());
                        if entry["sound_key"].as_str() != now.as_deref() {
                            changed.push(id);
                            continue;
                        }
                        if let Some(reason) = normalize::refusal(project, id) {
                            return Err(refused(reason));
                        }
                        let Some(volume) = normalize::volume(integrated as f32, peak as f32) else {
                            skipped.push(id);
                            continue;
                        };
                        if let Some(command) = normalize::command(target, volume.gain).map_err(refused)? {
                            commands.push(command);
                        }
                    }
                    if !changed.is_empty() {
                        return Err(refused(format!(
                            "{} changed since it was measured (trimmed, other media or other \
                             audio effects); measure again with loudness_start",
                            changed.iter().map(Uuid::to_string).collect::<Vec<_>>().join(", ")
                        )));
                    }
                    if commands.is_empty() {
                        return Err(refused(if skipped.is_empty() {
                            "Those clips are at -14 LUFS already"
                        } else {
                            "None of the measured clips can be normalized (silent, too short, or gone)"
                        }));
                    }
                    batch(normalize::LABEL, commands)
                }
                other => Err(refused(format!(
                    "op is volume, fades, track, music, detach, noise or normalize, not {other}"
                ))),
            }
        })
        .await
    }
}
