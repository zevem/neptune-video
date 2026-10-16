//! The timeline: tracks, clips and markers, with the window's placement
//! rules. A clip never lands on another one unless the agent asks.
use super::{Guard, batch, clip, refused, track};
use crate::{
    mcp::{errors::ToolError, server::AsterServer, time::TimeSpec, workspace::Edited},
    rules::{self, Edge},
};
use editor_core::{Command, Editor, Marker, Time, Track, TrackKind, Transform};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrackParams {
    /// `add`, `remove`, `reorder` or `set`.
    pub op: String,
    /// For everything but `add`.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    /// For `add`: `video`, `audio` or `text`.
    #[serde(default)]
    pub kind: Option<TrackKind>,
    #[serde(default)]
    pub name: Option<String>,
    /// For `reorder`: the new place, 0 the first (bottom) track.
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default)]
    pub solo: Option<bool>,
    /// The track's volume, 0 to 8 (1 is as recorded).
    #[serde(default)]
    pub gain: Option<f32>,
    /// A language tag (`en`, `pt-BR`) makes a text track a caption track;
    /// an empty string makes it a plain text track again.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClipAddParams {
    /// The media UUID or unique hexadecimal prefix (at least four digits).
    pub asset_id: String,
    /// The track; the first usable one of the media's kind by default, or
    /// a new one.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    /// Where it starts. Without it, after the last clip on the track. With
    /// it, at the first free time from there unless `exact`.
    #[serde(default)]
    pub at: Option<TimeSpec>,
    /// Refuse instead of moving later when `at` is taken.
    #[serde(default)]
    pub exact: bool,
    /// Where in the media the clip begins.
    #[serde(default)]
    pub source_in: Option<TimeSpec>,
    /// How long the clip is; to the end of the media by default.
    #[serde(default)]
    pub duration: Option<TimeSpec>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TransformSpec {
    /// Canvas pixels from the canvas's centre, y down.
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    #[serde(default)]
    pub scale: Option<[f32; 2]>,
    /// Degrees.
    #[serde(default)]
    pub rotation: Option<f32>,
    /// 0 to 1.
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    pub flip_x: Option<bool>,
    #[serde(default)]
    pub flip_y: Option<bool>,
    /// Amounts cut from the left, top, right and bottom, 0 to 1.
    #[serde(default)]
    pub crop: Option<[f32; 4]>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClipEditParams {
    /// `move`, `trim`, `split`, `delete`, `speed`, `transform`, `link`,
    /// `unlink`, `group`, `ungroup` or `duplicate`.
    pub op: String,
    /// The clips. Linked and grouped clips follow as in the window.
    #[serde(default)]
    pub clip_ids: Vec<Uuid>,
    /// `move`: the new start of the earliest clip. `split`: where to cut.
    /// `duplicate`: where the copies start.
    #[serde(default)]
    pub at: Option<TimeSpec>,
    /// `move`, `duplicate`: the track the named clips go to.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    /// `move`: land at the nearest free place when `at` is taken, as a drag
    /// does, instead of refusing.
    #[serde(default)]
    pub nearest: bool,
    /// `trim`: `start` or `end`.
    #[serde(default)]
    pub edge: Option<String>,
    /// `trim`: the timeline time the edge moves to.
    #[serde(default)]
    pub to: Option<TimeSpec>,
    /// `trim`: stop at the furthest the edge may go instead of refusing.
    #[serde(default)]
    pub clamp: bool,
    /// `delete`, `trim`: close the gap (or open room) by moving later
    /// clips. A ripple trim keeps the clip's start: `to` on its start edge
    /// says how much of its head goes.
    #[serde(default)]
    pub ripple: bool,
    /// `speed`: 0.05 to 16; the clip's length changes with it.
    #[serde(default)]
    pub rate: Option<f64>,
    /// `transform`: the fields to change.
    #[serde(default)]
    pub transform: Option<TransformSpec>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarkerParams {
    /// `add` or `remove`.
    pub op: String,
    #[serde(default)]
    pub at: Option<TimeSpec>,
    #[serde(default)]
    pub label: Option<String>,
    /// RGBA, 0 to 255 each.
    #[serde(default)]
    pub color: Option<[u8; 4]>,
    #[serde(default)]
    pub marker_id: Option<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

fn edge(given: Option<&str>) -> Result<Edge, ToolError> {
    match given {
        Some("start") => Ok(Edge::Start),
        Some("end") => Ok(Edge::End),
        _ => Err(ToolError::invalid("edge is start or end")),
    }
}

/// The AddClip inside a placement edit.
fn added_clip(command: &mut Command) -> Option<&mut editor_core::Clip> {
    match command {
        Command::AddClip { clip, .. } => Some(clip),
        Command::Batch { commands, .. } => commands.iter_mut().find_map(added_clip),
        _ => None,
    }
}

fn need_clips(ids: &[Uuid]) -> Result<(), ToolError> {
    if ids.is_empty() {
        return Err(ToolError::invalid("Name the clips in clip_ids"));
    }
    Ok(())
}

#[tool_router(router = timeline_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Tracks: add one (video, audio or text), remove an empty or unlocked \
                          one, reorder, or set its name, lock, hide, mute, solo, volume or \
                          caption language."
    )]
    async fn track_edit(
        &self,
        Parameters(params): Parameters<TrackParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let id = || {
                params
                    .track_id
                    .ok_or_else(|| ToolError::invalid("Name the track_id"))
            };
            match params.op.as_str() {
                "add" => {
                    let kind = params
                        .kind
                        .ok_or_else(|| ToolError::invalid("kind is video, audio or text"))?;
                    let name = params.name.clone().unwrap_or_else(|| {
                        let count = project.tracks.iter().filter(|t| t.kind == kind).count();
                        format!("{} {}", rules::kind_name(kind), count + 1)
                    });
                    let mut track = Track::new(name, kind);
                    if let Some(language) = params.language.as_deref().filter(|l| !l.is_empty()) {
                        if kind != TrackKind::Text {
                            return Err(refused("Only a text track carries captions"));
                        }
                        track.language = Some(language.to_owned());
                    }
                    let mut commands = vec![Command::AddTrack {
                        track: track.clone(),
                    }];
                    if let Some(index) = params.index {
                        commands.push(Command::ReorderTrack {
                            track_id: track.id,
                            index,
                        });
                    }
                    batch("Add track", commands)
                }
                "remove" => {
                    let track = track(project, id()?)?;
                    Ok(Command::RemoveTrack { track_id: track.id })
                }
                "reorder" => {
                    let track = track(project, id()?)?;
                    let index = params
                        .index
                        .ok_or_else(|| ToolError::invalid("Give index"))?;
                    Ok(Command::ReorderTrack {
                        track_id: track.id,
                        index,
                    })
                }
                "set" => {
                    let track = track(project, id()?)?;
                    let mut commands = Vec::new();
                    if params.locked.is_some()
                        || params.hidden.is_some()
                        || params.muted.is_some()
                        || params.name.is_some()
                        || params.gain.is_some()
                        || params.solo.is_some()
                    {
                        commands.push(Command::SetTrackState {
                            track_id: track.id,
                            locked: params.locked,
                            hidden: params.hidden,
                            muted: params.muted,
                            name: params.name.clone(),
                            gain: params.gain,
                            solo: params.solo,
                        });
                    }
                    if let Some(language) = &params.language {
                        commands.push(Command::SetTrackLanguage {
                            track_id: track.id,
                            language: (!language.is_empty()).then(|| language.clone()),
                        });
                    }
                    batch("Change track", commands)
                }
                other => Err(refused(format!(
                    "op is add, remove, reorder or set, not {other}"
                ))),
            }
        })
        .await
    }

    #[tool(
        description = "Place media on the timeline as a clip, optionally a part of it \
                          (source_in, duration), at a time and track. Answers the new clip's \
                          ID in `created`."
    )]
    async fn clip_add(
        &self,
        Parameters(params): Parameters<ClipAddParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            let asset_id = super::resolve_asset_id(project, &params.asset_id)?;
            let asset = &project.assets[&asset_id];
            let at = params.at.as_ref().map(|at| at.at(rate)).transpose()?;
            let source_in = params
                .source_in
                .as_ref()
                .map(|t| t.exact(rate))
                .transpose()?;
            let duration = params
                .duration
                .as_ref()
                .map(|t| t.length(rate))
                .transpose()?;
            let place = match (params.track_id, at) {
                (Some(track), at) => Some((
                    track,
                    at.unwrap_or_else(|| {
                        project
                            .track(track)
                            .and_then(|t| t.clips.iter().map(|c| c.end()).max())
                            .unwrap_or(Time::ZERO)
                    }),
                )),
                (None, Some(at)) => {
                    let home = rules::home_track(asset);
                    project
                        .tracks
                        .iter()
                        .find(|t| {
                            t.kind == home && !t.locked && rules::track_accepts(t.kind, asset)
                        })
                        .map(|t| (t.id, at))
                }
                (None, None) => None,
            };
            if let Some(track_id) = params.track_id {
                let track = track(project, track_id)?;
                if track.locked {
                    return Err(refused("That track is locked"));
                }
                if !rules::track_accepts(track.kind, asset) {
                    return Err(refused(format!(
                        "A {} track does not take {}",
                        rules::kind_name(track.kind).to_lowercase(),
                        asset.name
                    )));
                }
            }
            let (mut command, _) = rules::place_asset(project, asset.id, place).map_err(refused)?;
            let clip = added_clip(&mut command)
                .ok_or_else(|| ToolError::unavailable("No clip was made"))?;
            if source_in.is_some() || duration.is_some() {
                let source_in = source_in.unwrap_or(clip.source_in);
                if source_in >= asset.duration && asset.kind != editor_core::MediaKind::Image {
                    return Err(refused("source_in is past the end of the media"));
                }
                clip.source_in = source_in;
                clip.duration = match duration {
                    Some(duration) => duration,
                    None => asset.duration - source_in,
                };
                clip.name = asset.name.clone();
            }
            // The free place for the clip as it is now. A track the edit
            // adds is empty: the clip goes where it was asked to.
            let target = match &command {
                Command::AddClip { track_id, .. } => Some(*track_id),
                Command::Batch { commands, .. } => {
                    commands.iter().find_map(|command| match command {
                        Command::AddClip { track_id, .. } => Some(*track_id),
                        _ => None,
                    })
                }
                _ => None,
            };
            let fresh = target.is_some_and(|id| project.track(id).is_none());
            if fresh {
                if let Some(clip) = added_clip(&mut command) {
                    clip.start = at.unwrap_or(Time::ZERO);
                }
            } else if let (Some(track_id), Some(clip)) = (target, added_clip(&mut command)) {
                let track = track(project, track_id)?;
                let wanted = at.unwrap_or(clip.start);
                let free = rules::free_start(track, wanted, clip.duration, &[]);
                if params.exact && at.is_some() && free != wanted {
                    return Err(ToolError::new(
                        crate::mcp::errors::codes::OVERLAP,
                        "Another clip is there; leave exact out to place it at the next free time",
                    ));
                }
                clip.start = free;
            }
            Ok(command)
        })
        .await
    }

    #[tool(
        description = "Edit clips: move, trim an edge, split at a time, delete (ripple), \
                          change speed, transform (position, scale, rotation, opacity, crop, \
                          flip), link, unlink, group, ungroup, or duplicate to a time."
    )]
    async fn clip_edit(
        &self,
        Parameters(params): Parameters<ClipEditParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            let ids = params.clip_ids.clone();
            for id in &ids {
                clip(project, *id)?;
            }
            match params.op.as_str() {
                "move" => {
                    need_clips(&ids)?;
                    let earliest = ids
                        .iter()
                        .filter_map(|id| project.clip(*id))
                        .map(|(_, clip)| clip.start)
                        .min()
                        .unwrap_or_default();
                    let to = match &params.at {
                        Some(at) => at.at(rate)?,
                        None if params.track_id.is_some() => earliest,
                        None => return Err(ToolError::invalid("Give at, track_id or both")),
                    };
                    if let Some(target) = params.track_id {
                        let target = track(project, target)?;
                        if target.locked {
                            return Err(refused("That track is locked"));
                        }
                        // As a drag does: clips of one track, to a track of
                        // the same kind.
                        let mut from = ids.iter().filter_map(|id| project.clip(*id)).map(|(t, _)| t);
                        let first = from.next().expect("checked above");
                        if from.any(|other| other.id != first.id) {
                            return Err(refused("Move clips of one track at a time to another track"));
                        }
                        if first.kind != target.kind {
                            return Err(refused(format!(
                                "Clips move to another track of the same kind; that is a {} track",
                                rules::kind_name(target.kind).to_lowercase()
                            )));
                        }
                    }
                    let wanted = Time(to.0 - earliest.0);
                    let landed = rules::resolve_move(project, &ids, wanted, params.track_id)
                        .ok_or_else(|| refused("There is no room to move those clips there"))?;
                    if landed != wanted && !params.nearest {
                        return Err(ToolError::new(
                            crate::mcp::errors::codes::OVERLAP,
                            format!(
                                "Other clips are there; the nearest free place starts at {:.3} s \
                                 (pass nearest: true to take it)",
                                (earliest.0 + landed.0) as f64 / 1e6
                            ),
                        ));
                    }
                    Ok(Command::MoveClips {
                        clip_ids: ids,
                        delta: landed,
                        target_track: params.track_id,
                    })
                }
                "trim" => {
                    let [id] = ids.as_slice() else {
                        return Err(ToolError::invalid("Trim one clip at a time"));
                    };
                    let (track, trimmed) = clip(project, *id)?;
                    let edge = edge(params.edge.as_deref())?;
                    let to = params
                        .to
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give `to`: where the edge goes"))?
                        .at(rate)?;
                    let from = match edge {
                        Edge::Start => trimmed.start,
                        Edge::End => trimmed.end(),
                    };
                    let wanted = Time(to.0 - from.0);
                    // A ripple trim keeps the clip's start and moves what
                    // follows, so only its source bounds it.
                    let (delta, (least, most)) = if params.ripple {
                        (
                            rules::ripple_trim_delta(project, track, trimmed, edge, wanted),
                            rules::ripple_trim_range(project, track, trimmed, edge),
                        )
                    } else {
                        (
                            rules::trim_delta(project, track, trimmed, edge, wanted),
                            rules::trim_range(project, track, trimmed, edge),
                        )
                    };
                    if delta != wanted && !params.clamp {
                        return Err(refused(format!(
                            "That edge can go from {:.3} s to {:.3} s; pass clamp: true to stop there",
                            (from.0 + least.0) as f64 / 1e6,
                            (from.0 + most.0) as f64 / 1e6
                        )));
                    }
                    let mut command = crate::ui::timeline::trim_command(trimmed, edge, delta);
                    if let Command::TrimClip { ripple, .. } = &mut command {
                        *ripple = params.ripple;
                    }
                    Ok(command)
                }
                "split" => {
                    let at = params
                        .at
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give `at`: where to cut"))?
                        .at(rate)?;
                    if !ids.is_empty()
                        && !ids.iter().any(|id| {
                            project.clip(*id).is_some_and(|(_, c)| c.start < at && at < c.end())
                        })
                    {
                        return Err(refused("None of those clips is under that time"));
                    }
                    rules::split_command(project, at, &ids).map_err(|_| refused("No clip is under that time"))
                }
                "delete" => {
                    need_clips(&ids)?;
                    Ok(Command::DeleteClips {
                        clip_ids: ids,
                        ripple: params.ripple,
                    })
                }
                "speed" => {
                    let [id] = ids.as_slice() else {
                        return Err(ToolError::invalid("Change one clip's speed at a time"));
                    };
                    let rate = params.rate.ok_or_else(|| ToolError::invalid("Give rate"))?;
                    if !(crate::speed::SLOWEST..=crate::speed::FASTEST).contains(&rate) {
                        return Err(ToolError::invalid("rate is from 0.05 to 16"));
                    }
                    let (track, changed) = clip(project, *id)?;
                    if !crate::speed::timed(project, track, changed) {
                        return Err(refused("Only a clip of moving media has a speed"));
                    }
                    crate::speed::constant_command(project, *id, rate)
                        .ok_or_else(|| refused("That clip's speed cannot change"))
                }
                "transform" => {
                    need_clips(&ids)?;
                    let spec = params.transform.as_ref().ok_or_else(|| ToolError::invalid("Give transform"))?;
                    let commands = ids
                        .iter()
                        .map(|id| {
                            let (_, changed) = clip(project, *id)?;
                            let old = changed.transform.clone();
                            let transform = Transform {
                                position: spec.position.unwrap_or(old.position),
                                scale: spec.scale.unwrap_or(old.scale),
                                rotation: spec.rotation.unwrap_or(old.rotation),
                                opacity: spec.opacity.unwrap_or(old.opacity),
                                flip_x: spec.flip_x.unwrap_or(old.flip_x),
                                flip_y: spec.flip_y.unwrap_or(old.flip_y),
                                crop: spec.crop.unwrap_or(old.crop),
                            };
                            Ok(Command::SetTransform { clip_id: *id, transform })
                        })
                        .collect::<Result<Vec<_>, ToolError>>()?;
                    batch("Transform clips", commands)
                }
                "link" | "unlink" | "group" | "ungroup" => {
                    need_clips(&ids)?;
                    Ok(match params.op.as_str() {
                        "link" => Command::LinkClips { clip_ids: ids },
                        "unlink" => Command::UnlinkClips { clip_ids: ids },
                        "group" => Command::GroupClips { clip_ids: ids },
                        _ => Command::UngroupClips { clip_ids: ids },
                    })
                }
                "duplicate" => {
                    need_clips(&ids)?;
                    let at = params
                        .at
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give `at`: where the copies start"))?
                        .at(rate)?;
                    let editor = Editor::new((**project).clone())
                        .map_err(|error| ToolError::unavailable(error.to_string()))?;
                    let clipboard = editor
                        .copy(&ids)
                        .map_err(|error| refused(error.to_string()))?;
                    let command = editor
                        .paste_command(&clipboard, at, params.track_id)
                        .map_err(|error| refused(error.to_string()))?;
                    Ok(match command {
                        Command::Batch { commands, .. } => Command::Batch {
                            label: "Duplicate clips".into(),
                            commands,
                        },
                        other => other,
                    })
                }
                other => Err(refused(format!(
                    "op is move, trim, split, delete, speed, transform, link, unlink, group, \
                     ungroup or duplicate, not {other}"
                ))),
            }
        })
        .await
    }

    #[tool(
        description = "Markers on the timeline: add one at a time with a label and colour, \
                          or remove one."
    )]
    async fn marker_edit(
        &self,
        Parameters(params): Parameters<MarkerParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            match params.op.as_str() {
                "add" => {
                    let at = params
                        .at
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give `at`"))?
                        .at(project.settings.frame_rate)?;
                    let Command::AddMarker { mut marker } = crate::ui::timeline::marker_at(at)
                    else {
                        return Err(ToolError::unavailable("No marker was made"));
                    };
                    if let Some(label) = &params.label {
                        marker.label = label.clone();
                    }
                    if let Some(color) = params.color {
                        marker.color = color;
                    }
                    Ok(Command::AddMarker { marker })
                }
                "remove" => {
                    let id = params
                        .marker_id
                        .ok_or_else(|| ToolError::invalid("Give marker_id"))?;
                    if !project
                        .markers
                        .iter()
                        .any(|marker: &Marker| marker.id == id)
                    {
                        return Err(ToolError::not_found(format!("No marker {id}")));
                    }
                    Ok(Command::RemoveMarker { marker_id: id })
                }
                other => Err(refused(format!("op is add or remove, not {other}"))),
            }
        })
        .await
    }
}
