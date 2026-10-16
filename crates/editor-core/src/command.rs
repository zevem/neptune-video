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

/// What a shorter clip keeps of its existing clip-relative animation.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnimationTrim {
    /// Keep the remaining curve, synthesizing an interpolated end key. Fit
    /// existing words, time maps, text animations, transitions and fades too.
    #[default]
    Crop,
    /// Leave existing timed data unchanged; reject it if it no longer fits.
    Preserve,
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
        gain: Option<f32>,
        solo: Option<bool>,
    },
    AddClip {
        track_id: TrackId,
        clip: Clip,
    },
    /// Merge fields into exactly one existing clip, retaining its identity.
    /// Objects merge recursively; arrays replace. Null removes an optional
    /// field. Times are integer microseconds. Identity and relationships are
    /// changed only by their own commands. Explicit timed data is validated
    /// as supplied, after the trim policy fits the existing data.
    PatchClip {
        clip_id: ClipId,
        patch: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        trim_animation: AnimationTrim,
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
    // Both fades of one clip, as timeline lengths from its edges; linked clips
    // keep their own. A plain comment: the planner text describes fades.
    SetFades {
        clip_id: ClipId,
        fade_in: Time,
        fade_out: Time,
    },
    /// Set a title's text, size and colour. Its style and animations stay;
    /// word times follow the words the old and new text share.
    SetText {
        clip_id: ClipId,
        text: String,
        font_size: f32,
        color: [u8; 4],
    },
    SetTextStyle {
        clip_id: ClipId,
        style: TextStyle,
    },
    /// Set or remove a title's entrance (`in`) or exit (`out`).
    SetTextAnimation {
        clip_id: ClipId,
        edge: TransitionEdge,
        animation: Option<TextAnimation>,
    },
    /// Replace a title's word times: one per word of its text, or none.
    SetWords {
        clip_id: ClipId,
        words: Vec<WordTime>,
    },
    /// Replace a graphic's design box and elements.
    SetGraphic {
        clip_id: ClipId,
        size: [f32; 2],
        elements: Vec<Element>,
    },
    /// Make a text track a caption track in `language`, or a plain text
    /// track with `null`.
    SetTrackLanguage {
        track_id: TrackId,
        language: Option<String>,
    },
    /// A new caption track `track_id` directly above `from_track_id`, its
    /// titles copied in order with `texts` in place of theirs: a
    /// translation. The source track is hidden.
    CopyCaptions {
        from_track_id: TrackId,
        track_id: TrackId,
        name: String,
        language: String,
        texts: Vec<String>,
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
            Self::PatchClip { .. } => "Patch clip",
            Self::DeleteClips { ripple: true, .. } => "Ripple delete",
            Self::DeleteClips { .. } => "Delete clips",
            Self::MoveClips { .. } => "Move clips",
            Self::TrimClip { ripple: true, .. } => "Ripple trim",
            Self::TrimClip { .. } => "Trim clip",
            Self::SplitClip { .. } => "Split clip",
            Self::SetTransform { .. } => "Transform clip",
            Self::SetPlaybackRate { .. } => "Change clip speed",
            Self::SetGain { .. } => "Change clip volume",
            Self::SetFades { .. } => "Change fades",
            Self::SetText { .. } => "Edit text",
            Self::SetTextStyle { .. } => "Style text",
            Self::SetTextAnimation {
                animation: None, ..
            } => "Remove text animation",
            Self::SetTextAnimation { .. } => "Animate text",
            Self::SetWords { .. } => "Time words",
            Self::SetGraphic { .. } => "Edit graphic",
            Self::SetTrackLanguage { .. } => "Set caption language",
            Self::CopyCaptions { .. } => "Translate captions",
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
