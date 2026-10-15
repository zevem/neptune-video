//! The tools, one module per namespace. Each typed tool builds its command
//! with the same builders the window uses, from a fresh snapshot, and
//! applies it through the workspace; `edit_apply` reaches every core
//! command directly.
pub mod analysis;
pub mod audio;
pub mod edit;
pub mod effects;
pub mod history;
pub mod jobs;
pub mod media;
pub mod project;
pub mod render;
pub mod text;
pub mod timeline;

use super::{errors::ToolError, time::Stamp};
use editor_core::{Clip, ClipContent, FrameRate, Project, Time, Track};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

/// The options every editing tool takes.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
pub struct Guard {
    /// Refuse the edit unless the project is still at this revision.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    /// Check the edit and say what it would do, without applying it.
    #[serde(default)]
    pub dry_run: bool,
    /// Allow clips to overlap on a track, which the window never does.
    #[serde(default)]
    pub allow_overlap: bool,
}

/// A refusal from one of the window's builders.
pub fn refused(message: impl Into<String>) -> ToolError {
    ToolError::new("invalid_edit", message)
}

pub fn clip(project: &Project, id: Uuid) -> Result<(&Track, &Clip), ToolError> {
    project
        .clip(id)
        .ok_or_else(|| ToolError::not_found(format!("No clip {id} in the project")))
}

pub fn track(project: &Project, id: Uuid) -> Result<&Track, ToolError> {
    project
        .track(id)
        .ok_or_else(|| ToolError::not_found(format!("No track {id} in the project")))
}

pub fn rate(project: &Project) -> FrameRate {
    project.settings.frame_rate
}

pub fn stamp(time: Time, project: &Project) -> Stamp {
    Stamp::new(time, rate(project))
}

/// Several commands as one history entry, or the one alone.
pub fn batch(
    label: &str,
    mut commands: Vec<editor_core::Command>,
) -> Result<editor_core::Command, ToolError> {
    match commands.len() {
        0 => Err(ToolError::invalid("There is nothing to change")),
        // A builder's own one-command batch takes this tool's label.
        1 => Ok(match commands.remove(0) {
            editor_core::Command::Batch { commands, .. } => editor_core::Command::Batch {
                label: label.to_owned(),
                commands,
            },
            command => command,
        }),
        _ => Ok(editor_core::Command::Batch {
            label: label.to_owned(),
            commands,
        }),
    }
}

/// What kind of clip this is, in a word.
pub fn content_kind(clip: &Clip) -> &'static str {
    match clip.content {
        ClipContent::Media { .. } => "media",
        ClipContent::Text { .. } => "text",
        ClipContent::Graphic { .. } => "graphic",
    }
}

/// A clip as tools report it.
pub fn clip_view(project: &Project, track: &Track, clip: &Clip) -> serde_json::Value {
    let mut view = serde_json::json!({
        "clip_id": clip.id,
        "track_id": track.id,
        "name": clip.name,
        "kind": content_kind(clip),
        "start": stamp(clip.start, project),
        "end": stamp(clip.end(), project),
        "duration": stamp(clip.duration, project),
        "source_in": clip.source_in.0,
        "speed": clip.playback_rate,
    });
    let object = view.as_object_mut().expect("an object");
    if let Some(asset) = clip.asset_id() {
        object.insert("asset_id".into(), serde_json::json!(asset));
    }
    if let ClipContent::Text { text, .. } = &clip.content {
        object.insert("text".into(), text.clone().into());
    }
    if clip.gain != 1.0 {
        object.insert("gain".into(), clip.gain.into());
    }
    if !clip.effects.is_empty() {
        object.insert(
            "effects".into(),
            clip.effects
                .iter()
                .map(|effect| serde_json::json!({"effect_id": effect.id, "kind": effect.kind, "enabled": effect.enabled}))
                .collect(),
        );
    }
    if !clip.masks.is_empty() {
        object.insert("masks".into(), clip.masks.len().into());
    }
    if !clip.animation.is_empty() {
        object.insert(
            "animated".into(),
            clip.animation
                .iter()
                .map(|track| serde_json::json!(track.key))
                .collect(),
        );
    }
    for (name, transition) in [
        ("transition_in", &clip.transition_in),
        ("transition_out", &clip.transition_out),
    ] {
        if let Some(transition) = transition {
            object.insert(
                name.into(),
                serde_json::json!({"kind": transition.kind, "duration": stamp(transition.duration, project)}),
            );
        }
    }
    if clip.fade_in > Time::ZERO || clip.fade_out > Time::ZERO {
        object.insert(
            "fades".into(),
            serde_json::json!({"in": stamp(clip.fade_in, project), "out": stamp(clip.fade_out, project)}),
        );
    }
    if let Some(link) = clip.linked_id {
        object.insert("linked_id".into(), serde_json::json!(link));
    }
    if let Some(group) = clip.group_id {
        object.insert("group_id".into(), serde_json::json!(group));
    }
    if clip.retime.is_some() {
        object.insert("retimed".into(), true.into());
    }
    view
}

/// A frame rate from frames a second: the broadcast rates exactly.
pub fn frame_rate(fps: f64) -> Result<FrameRate, ToolError> {
    if !(fps.is_finite() && (1.0..=240.0).contains(&fps)) {
        return Err(ToolError::invalid(
            "A frame rate is from 1 to 240 frames a second",
        ));
    }
    let ntsc = [
        (23.976, 24_000),
        (29.97, 30_000),
        (59.94, 60_000),
        (119.88, 120_000),
    ];
    for (rate, numerator) in ntsc {
        if (fps - rate).abs() < 0.005 {
            return Ok(FrameRate {
                numerator,
                denominator: 1001,
            });
        }
    }
    if (fps - fps.round()).abs() < 1e-6 {
        return Ok(FrameRate {
            numerator: fps.round() as u32,
            denominator: 1,
        });
    }
    Ok(FrameRate {
        numerator: (fps * 1000.0).round() as u32,
        denominator: 1000,
    })
}

/// Runs blocking `work` off the runtime. The client cancelling the call
/// raises the flag `work` is given.
pub async fn blocking<T: Send + 'static>(
    ct: tokio_util::sync::CancellationToken,
    work: impl FnOnce(&std::sync::atomic::AtomicBool) -> Result<T, ToolError> + Send + 'static,
) -> Result<T, ToolError> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let watcher = tokio::spawn(async move {
        ct.cancelled().await;
        flag.store(true, Ordering::Release);
    });
    let flag = cancel.clone();
    let result = tokio::task::spawn_blocking(move || work(&flag)).await;
    watcher.abort();
    result.map_err(|_| ToolError::unavailable("The work failed inside the server"))?
}

/// The media the project's clips and effects use, and its fonts (every one
/// is loaded to draw titles), that this server may not read: a project
/// from elsewhere can point anywhere.
pub fn unreadable(project: &Project, roots: &crate::mcp::paths::Roots) -> Vec<String> {
    let mut used = std::collections::BTreeSet::new();
    used.extend(
        (project.assets.values())
            .filter(|asset| asset.kind == editor_core::MediaKind::Font)
            .map(|asset| asset.id),
    );
    for track in &project.tracks {
        for clip in &track.clips {
            used.extend(clip.asset_id());
            used.extend(clip.effects.iter().filter_map(|effect| effect.asset_id));
        }
    }
    used.into_iter()
        .filter_map(|id| project.assets.get(&id))
        .filter(|asset| !roots.readable(&asset.path))
        .map(|asset| asset.path.display().to_string())
        .collect()
}

/// Refuses work that would read media outside the roots.
pub fn check_readable(
    project: &Project,
    roots: &crate::mcp::paths::Roots,
) -> Result<(), ToolError> {
    let outside = unreadable(project, roots);
    if outside.is_empty() {
        return Ok(());
    }
    Err(ToolError::new(
        crate::mcp::errors::codes::OUTSIDE_ROOTS,
        format!(
            "The project uses {} media file(s) outside the folders this server may read, or \
             missing; start the server with --root for their folder",
            outside.len()
        ),
    )
    .with_detail(serde_json::json!({ "paths": outside })))
}
