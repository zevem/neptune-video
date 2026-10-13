use crate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("Object {0} does not exist")]
    Missing(Uuid),
    #[error("Track is locked")]
    Locked,
    #[error("Clip is incompatible with this track")]
    IncompatibleTrack,
    #[error("Invalid {0}")]
    Invalid(String),
    #[error("Stale revision: expected {expected}, current {actual}")]
    StaleRevision { expected: u64, actual: u64 },
}

/// The sole mutation protocol for human interaction, shortcuts and agent tools.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "boxing AddClip would change every construction site for one move"
)]
pub enum Command {
    AddAssets {
        assets: Vec<Asset>,
    },
    RemoveAssets {
        asset_ids: Vec<AssetId>,
    },
    AddFolder {
        folder: MediaFolder,
    },
    MoveAssetToFolder {
        asset_id: AssetId,
        folder_id: Option<FolderId>,
    },
    AddTrack {
        track: Track,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    ReorderTrack {
        track_id: TrackId,
        index: usize,
    },
    SetTrackState {
        track_id: TrackId,
        locked: Option<bool>,
        hidden: Option<bool>,
        muted: Option<bool>,
        name: Option<String>,
    },
    AddClip {
        track_id: TrackId,
        clip: Clip,
    },
    DeleteClips {
        clip_ids: Vec<ClipId>,
        ripple: bool,
    },
    MoveClips {
        clip_ids: Vec<ClipId>,
        delta: Time,
        target_track: Option<TrackId>,
    },
    TrimClip {
        clip_id: ClipId,
        source_in: Time,
        duration: Time,
        ripple: bool,
    },
    /// Split at an absolute timeline timestamp; linked clips split together.
    /// The editor creates right-hand clips and fresh IDs, retaining the content.
    SplitClip {
        clip_id: ClipId,
        at: Time,
    },
    SetTransform {
        clip_id: ClipId,
        transform: Transform,
    },
    SetPlaybackRate {
        clip_id: ClipId,
        rate: f64,
    },
    SetGain {
        clip_id: ClipId,
        gain: f32,
    },
    SetText {
        clip_id: ClipId,
        text: String,
        font_size: f32,
        color: [u8; 4],
    },
    LinkClips {
        clip_ids: Vec<ClipId>,
    },
    UnlinkClips {
        clip_ids: Vec<ClipId>,
    },
    GroupClips {
        clip_ids: Vec<ClipId>,
    },
    UngroupClips {
        clip_ids: Vec<ClipId>,
    },
    AddMarker {
        marker: Marker,
    },
    RemoveMarker {
        marker_id: MarkerId,
    },
    RenameProject {
        name: String,
    },
    SetSettings {
        settings: CanvasSettings,
    },
    /// Replace the effect stack; animation of a removed effect goes with it.
    SetEffects {
        clip_id: ClipId,
        effects: Vec<Effect>,
    },
    SetMasks {
        clip_id: ClipId,
        masks: Vec<Mask>,
    },
    SetBlendMode {
        clip_id: ClipId,
        mode: BlendMode,
    },
    /// Replace one animation track; no keys removes it. `target` is an effect
    /// or mask ID on the clip, or null for its transform and gain.
    SetKeyframes {
        clip_id: ClipId,
        target: Option<Uuid>,
        key: ParamKey,
        keys: Vec<Keyframe>,
    },
    /// Set or clear the time map and the clip's new duration; linked clips follow.
    SetRetime {
        clip_id: ClipId,
        retime: Option<Retime>,
        duration: Time,
    },
    SetTransition {
        clip_id: ClipId,
        edge: TransitionEdge,
        transition: Option<Transition>,
    },
    /// Give a look at `strength` in 0..=1; it replaces the look held.
    ApplyPreset {
        clip_ids: Vec<ClipId>,
        preset: LookPreset,
        strength: f32,
    },
    SetTracker {
        tracker: Tracker,
    },
    RemoveTracker {
        tracker_id: Uuid,
    },
    SetFollow {
        clip_id: ClipId,
        follow: Option<Follow>,
    },
    /// Restore a checkpoint of this project as one reversible transaction.
    RestoreProject {
        project: Box<Project>,
    },
    Batch {
        label: String,
        commands: Vec<Command>,
    },
}
impl Command {
    pub fn label(&self) -> &str {
        match self {
            Self::AddAssets { .. } => "Import media",
            Self::RemoveAssets { .. } => "Remove media",
            Self::AddFolder { .. } => "Create folder",
            Self::MoveAssetToFolder { .. } => "Move media",
            Self::AddTrack { .. } => "Add track",
            Self::RemoveTrack { .. } => "Remove track",
            Self::ReorderTrack { .. } => "Reorder track",
            Self::SetTrackState { .. } => "Change track",
            Self::AddClip { .. } => "Add clip",
            Self::DeleteClips { ripple: true, .. } => "Ripple delete",
            Self::DeleteClips { .. } => "Delete clips",
            Self::MoveClips { .. } => "Move clips",
            Self::TrimClip { ripple: true, .. } => "Ripple trim",
            Self::TrimClip { .. } => "Trim clip",
            Self::SplitClip { .. } => "Split clip",
            Self::SetTransform { .. } => "Transform clip",
            Self::SetPlaybackRate { .. } => "Change clip speed",
            Self::SetGain { .. } => "Change clip volume",
            Self::SetText { .. } => "Edit text",
            Self::LinkClips { .. } => "Link clips",
            Self::UnlinkClips { .. } => "Unlink clips",
            Self::GroupClips { .. } => "Group clips",
            Self::UngroupClips { .. } => "Ungroup clips",
            Self::AddMarker { .. } => "Add marker",
            Self::RemoveMarker { .. } => "Remove marker",
            Self::RenameProject { .. } => "Rename project",
            Self::SetSettings { .. } => "Project settings",
            Self::SetEffects { .. } => "Change effects",
            Self::SetMasks { .. } => "Change masks",
            Self::SetBlendMode { .. } => "Change blend mode",
            Self::SetKeyframes { .. } => "Change keyframes",
            Self::SetRetime { .. } => "Retime clip",
            Self::SetTransition {
                transition: None, ..
            } => "Remove transition",
            Self::SetTransition { .. } => "Set transition",
            Self::ApplyPreset { .. } => "Apply look",
            Self::SetTracker { .. } => "Set tracker",
            Self::RemoveTracker { .. } => "Remove tracker",
            Self::SetFollow { .. } => "Change tracking follow",
            Self::RestoreProject { .. } => "Restore checkpoint",
            Self::Batch { label, .. } => label,
        }
    }
}
