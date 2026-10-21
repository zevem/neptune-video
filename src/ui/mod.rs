//! Widgets and panels. They read the document and emit [`Action`]s; nothing
//! here mutates the project or touches a worker.
pub mod agent;
pub mod browser;
pub mod captions_tab;
pub mod controls;
pub mod export_sheet;
pub mod fades;
pub mod inspector;
pub mod keyframes;
pub mod kit;
pub mod library;
pub mod panel;
pub mod render_queue;
pub mod sheets;
pub mod text_tab;
pub mod timeline;
pub mod toolbar;
pub mod tracks;
pub mod transitions;
pub mod viewer;
pub mod widgets;
pub mod window;

use crate::{
    ai::{Effort, Provider},
    theme::{Accent, Appearance, Tone},
};
use editor_core::{
    AnimTrack, AssetId, BlendMode, Clip, ClipContent, ClipId, ClipOverride, Clipboard, Command,
    Ease, Effect, Follow, Keyframe, Mask, MediaKind, ParamKey, Project, Retime, Time, TrackId,
    TrackKind, Transform, Transition, TransitionEdge, TransitionKind,
};
use std::path::PathBuf;
use uuid::Uuid;

/// A change to the person's presets: kept looks and brand colours. They
/// are not the project's and are not undone.
#[derive(Debug, Clone)]
pub enum PresetChange {
    /// Keep a title's look or a graphic under a name, and where it is.
    Keep {
        name: String,
        content: Box<ClipContent>,
        position: Option<[f32; 2]>,
    },
    Rename(Uuid, String),
    Remove(Uuid),
    AddColor([u8; 4]),
    RemoveColor([u8; 4]),
}

/// What a widget asks the application to do. Every target is captured when
/// the action is created, so a later change of selection cannot retarget it.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "an action is made and consumed within a frame; boxing Edit would change every site"
)]
pub enum Action {
    NewProject,
    OpenProject,
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    /// Save a copy with an identity of its own and carry on in it.
    DuplicateProject,
    DuplicateTo(PathBuf),
    OpenCheckpoints,
    /// Keep a checkpoint with this name.
    SaveCheckpoint(String),
    /// Go back to the newest checkpoint with this name.
    RestoreCheckpoint(String),
    ImportFiles,
    ImportFolder,
    /// Import font files only.
    ImportFonts,
    /// Read subtitle files, each onto a caption track of its own.
    ImportCaptions,
    /// Write a caption track to a subtitle file.
    ExportCaptions {
        track: TrackId,
        format: editor_core::captions::SubtitleFormat,
    },
    /// Show Preferences at the pane it was last left on.
    OpenSettings,
    /// Show Preferences at this pane.
    OpenPreferences(PreferencesPane),
    /// Open the export sheet, starting from the settings last used.
    OpenExport,
    /// Show the render queue.
    OpenRenderQueue,

    /// One document change, undone as one step.
    Edit(Command),
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    /// Clips that arrived from the system clipboard.
    PasteClips(Clipboard),
    Duplicate,
    Delete {
        ripple: bool,
    },
    SplitAtPlayhead,
    /// Put this picture clip's sound on an audio track as a linked clip of
    /// its own, as one step.
    DetachAudio(ClipId),
    /// Put a transition on each of these edges, as one step: `kind`, or the
    /// default one. The clips and their edges were captured when this was
    /// made; the application adds what the person saved as that kind's
    /// default.
    AddTransitions {
        places: Vec<(ClipId, TransitionEdge)>,
        kind: Option<TransitionKind>,
    },
    /// Keep this transition's length, easing and values as the default of
    /// its kind.
    SaveTransitionDefault(Transition),
    SelectAll,
    AddMarker,
    AddTrack(TrackKind),
    AddText,
    /// Add a clip of a look of the Text tab: on `track` from `at` when that
    /// is a text track, else at `at` (the playhead when not given) on a
    /// text track with room.
    AddLook {
        look: std::sync::Arc<text_tab::Look>,
        track: Option<TrackId>,
        at: Option<Time>,
    },
    /// A change to the person's presets, which are not the project's.
    Presets(PresetChange),
    /// Put an asset on the timeline. Without a place it goes after the last
    /// clip of the first track that can hold it.
    InsertAsset {
        asset: AssetId,
        place: Option<(TrackId, Time)>,
    },
    /// Lay this sound under the whole project on a new "Music" track, as one
    /// step.
    AddBackgroundMusic(AssetId),
    /// Measure the loudness of each of these clips and set its volume so
    /// that it reaches the target, one step each as its figures come. The
    /// clips were captured when this was made.
    Normalize(Vec<ClipId>),
    /// Lower each of these clips by `depth` wherever the other clips are
    /// heard, as volume keyframes in one step, once the others' peaks are
    /// read. The clips were captured when this was made.
    Duck {
        clips: Vec<ClipId>,
        depth: f32,
    },
    /// Give up the duck under way. Nothing was written yet.
    CancelDuck,
    /// Show or hide, for the session, the beats of this media on its clips
    /// on audio tracks. Nothing is edited: the beats are found in its peaks.
    ShowBeats {
        assets: Vec<AssetId>,
        shown: bool,
    },
    /// Put a marker on the beats of this clip, as one step, once its
    /// media's beats are held. The clip was captured when this was made.
    MarkersFromBeats(ClipId),
    /// Ask where to save this media's sound, or the span of it this clip
    /// plays, as a WAV file, and have the sound lane write it. Nothing is
    /// edited. The media or the clip was captured when this was made.
    SaveAudio(crate::extract::Source),
    /// Give up the file being written as this request of the sound lane.
    CancelSaveAudio(u64),
    /// Import the file written as this request, by the ordinary import.
    AddSavedAudio(u64),
    /// Stop offering the file written as this request.
    DismissSavedAudio(u64),
    /// Have the stem separator split the whole sound of this clip's media
    /// into stems in the project's media folder, and place them under the
    /// clip as one step when they come. The clip was captured when this was
    /// made.
    SeparateStems(ClipId),
    /// Give up the separation under way as this request of the sound lane.
    CancelSeparate(u64),
    RemoveAsset(AssetId),
    /// Show an asset in the source monitor. The timeline is not touched.
    PreviewAsset(AssetId),
    CloseSource,
    /// The source monitor's own transport; never the timeline's.
    Source(Transport),

    TogglePlay,
    Seek(Time),
    Step(i64),
    JumpStart,
    JumpEnd,
    SetSpeed(f64),
    ToggleLoop,
    /// Record a voiceover take from the default microphone while the
    /// timeline plays from its playhead. Only the timeline records.
    Record,
    /// End the take under way; it is placed where it was said.
    StopRecord,

    ZoomTimeline(f32),
    FitTimeline,
    /// Scale the whole interface to this size, 1 being as designed.
    ScaleInterface(f32),
    ToggleSnapping,
    ToggleLibrary,
    ToggleInspector,
    /// Show the agent with its field taking the keyboard: asking the model
    /// when `ask`, running a command otherwise. The panel comes forward as
    /// the one asked for last.
    OpenAgent {
        ask: bool,
    },
    /// Put the panels back as a fresh window has them: both shown, at their
    /// first widths, over a timeline of its first height. Not an edit, so
    /// not undone.
    ResetLayout,
    /// Show the library with this part of it in view.
    ShowLibrary(library::Tab),
    /// The keyframe lane of the selected clip, and the graph editor.
    ToggleKeyLane,
    ToggleGraph,
    SetAppearance(Appearance),
    SetAccent(Accent),
    /// Eased changes land at once.
    SetReduceMotion(bool),
    SetAutosave(bool),
    /// How much of the pictures the preview decodes and composites.
    SetPreviewQuality(crate::settings::PreviewQuality),
    /// When the preview reads smaller copies of videos in their place.
    SetProxyMode(crate::settings::ProxyMode),
    /// How much the proxy cache may hold, in GiB.
    SetProxyBudget(u32),
    /// Make a proxy of this video now, whatever the mode.
    CreateProxy(AssetId),
    /// Take this video's proxy out of the cache.
    RemoveProxy(AssetId),
    /// Empty the proxy cache.
    ClearProxies,

    /// Run one line of the command language. No model is asked.
    AgentSubmit(String),
    /// Ask the model for a proposal. Nothing is edited until it is applied.
    AiSubmit(String),
    AiCancel,
    /// Edits come back as proposals to review, or are made as it works.
    SetAiReview(bool),
    /// Lets the agent use one more folder, chosen in a dialog.
    AddAgentFolder,
    /// Takes back a folder the person had let the agent use.
    RemoveAgentFolder(usize),
    /// End the agent's conversation; the next request starts a new one.
    AiNewConversation,
    /// Apply or close the proposal a request brought back, by that request.
    AiApply(uuid::Uuid),
    AiDiscard(uuid::Uuid),
    /// Whether the agent panel's field asks the model or runs a command.
    SetAgentAsk(bool),
    SetAiProvider(Provider),
    SetAiEffort(Effort),
    SetAiModel(String),
    Notify(String),
    /// Put away the notice with this id.
    DismissNotice(u64),
    /// Follow a region of a clip's picture through its source, on the
    /// analysis worker. Nothing is edited until the path comes back.
    Track(TrackRequest),
    /// Give up the tracking under way, or the path that waits to be added.
    CancelTracking,
    /// Add the tracked path that waits: it came while adding it by itself
    /// would have given up what was undone or a plan under review.
    AddTracked,
    /// Measure the camera's path through this video, on the analysis
    /// worker. Nothing is edited: the path is derived data, kept in a cache.
    Stabilize(AssetId),
    /// Give up the measuring under way.
    CancelStabilize,
    /// Make captions from the timeline's sound on the speech worker, after
    /// downloading the model when it is not there. Nothing is edited until
    /// the words come back.
    GenerateCaptions(crate::speech::SpeechRequest),
    /// Stop the download or transcription under way.
    CancelSpeech,
    /// Add the captions that came and wait, or give them up.
    AddHeardCaptions,
    DiscardHeardCaptions,
    /// The person's choices for captions from speech.
    SetSpeech(crate::speech::SpeechSettings),
    /// Choose a Whisper model file by hand.
    PickSpeechModel,
    /// Look at which models are downloaded, on a worker.
    LookAtModels,
    /// Captions from speech over the whole timeline, as last chosen.
    CaptionsFromSpeech,
    /// Ask the AI provider for a copy of a caption track in another
    /// language: one proposal, reviewed before it is applied.
    TranslateCaptions {
        track_id: editor_core::TrackId,
        language: String,
    },
}

/// What a transport asks of the clock it belongs to: the timeline's, or the
/// source monitor's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transport {
    TogglePlay,
    Seek(Time),
    Step(i64),
    JumpStart,
    JumpEnd,
    SetSpeed(f64),
    ToggleLoop,
}

impl Transport {
    /// The same request, made of the timeline.
    pub fn timeline(self) -> Action {
        match self {
            Self::TogglePlay => Action::TogglePlay,
            Self::Seek(time) => Action::Seek(time),
            Self::Step(frames) => Action::Step(frames),
            Self::JumpStart => Action::JumpStart,
            Self::JumpEnd => Action::JumpEnd,
            Self::SetSpeed(speed) => Action::SetSpeed(speed),
            Self::ToggleLoop => Action::ToggleLoop,
        }
    }
}

/// The parts of Preferences, in the order its source list gives them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreferencesPane {
    #[default]
    Appearance,
    Editing,
    Preview,
    Agent,
    Keyboard,
}

impl PreferencesPane {
    pub const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Editing,
        Self::Preview,
        Self::Agent,
        Self::Keyboard,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editing => "Editing",
            Self::Preview => "Preview",
            Self::Agent => "Agent",
            Self::Keyboard => "Keyboard",
        }
    }

    /// Where the pane in view is kept: for the window's session only, so a
    /// fresh window opens Preferences at Appearance.
    fn id() -> eframe::egui::Id {
        eframe::egui::Id::new("preferences-pane")
    }

    /// The pane Preferences shows.
    pub fn shown(ctx: &eframe::egui::Context) -> Self {
        ctx.data(|data| data.get_temp(Self::id()))
            .unwrap_or_default()
    }

    /// Makes this the pane Preferences shows.
    pub fn show(self, ctx: &eframe::egui::Context) {
        ctx.data_mut(|data| data.insert_temp(Self::id(), self));
    }
}

/// Where a region is drawn in this frame, and where it rests once the side
/// panels stop sliding. Work sized by the region, such as the viewer's
/// decoding, is sized by where it rests, so a panel that slides asks for it
/// once rather than in every frame of the slide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub drawn: eframe::egui::Rect,
    pub settled: eframe::egui::Rect,
}

impl Placement {
    fn viewer_id() -> eframe::egui::Id {
        eframe::egui::Id::new("viewer-placement")
    }

    /// Where the viewer is in this frame, as the window laid it out.
    pub fn of_viewer(ctx: &eframe::egui::Context) -> Option<Self> {
        ctx.data(|data| data.get_temp(Self::viewer_id()))
    }

    /// Records where the viewer is, before it is drawn.
    pub fn keep_for_viewer(self, ctx: &eframe::egui::Context) {
        ctx.data_mut(|data| data.insert_temp(Self::viewer_id(), self));
    }
}

/// Left and Right are a choice's own while a cell of one has the keyboard:
/// a tab of a strip, or an option of a segmented control. They move the
/// keyboard among its cells (and, where the strip asks [`arrow_choice`],
/// the choice with it), and do not also step the playhead. Up and Down,
/// Delete and the rest are still the application's.
///
/// A cell is known to be one as it gains the keyboard from Tab, when it says
/// what it is. egui says nothing of the cell an arrow moves the keyboard to,
/// so the keyboard is followed there: it is still in the choice.
#[derive(Clone, Debug, Default)]
pub struct ChoiceArrows {
    /// The cell of a choice that has the keyboard.
    focused: Option<eframe::egui::Id>,
    /// The pass in which an arrow was pressed on it.
    stepped: Option<u64>,
}

impl ChoiceArrows {
    fn id() -> eframe::egui::Id {
        eframe::egui::Id::new("choice-arrows")
    }

    fn read(ctx: &eframe::egui::Context) -> Self {
        ctx.data(|data| data.get_temp(Self::id()))
            .unwrap_or_default()
    }

    /// Called before the frame's widgets are made, in every frame. Says
    /// whether a choice has the keyboard, so that Left and Right are its own.
    pub fn begin(ctx: &eframe::egui::Context) -> bool {
        use eframe::egui::{Event, Key};
        let mut state = Self::read(ctx);
        let focused = ctx.memory(|memory| memory.focused());
        let pass = ctx.cumulative_pass_nr();
        // Where the arrow of the last pass took the keyboard.
        if state.focused.is_some() && state.stepped.is_some_and(|stepped| stepped + 1 == pass) {
            state.focused = focused;
        }
        let owned = focused.is_some() && focused == state.focused;
        let arrow = ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        // Up and Down take the keyboard out of the choice.
                        key: Key::ArrowLeft | Key::ArrowRight,
                        pressed: true,
                        ..
                    }
                )
            })
        });
        state.stepped = (owned && arrow).then_some(pass);
        ctx.data_mut(|data| data.insert_temp(Self::id(), state));
        owned
    }

    /// Called once the frame's widgets are made: notes whether the widget
    /// that gained the keyboard in it is the cell of a choice.
    pub fn end(ctx: &eframe::egui::Context) {
        use eframe::egui::{WidgetType, output::OutputEvent};
        let mut state = Self::read(ctx);
        let gained = ctx.output(|output| {
            output.events.iter().find_map(|event| match event {
                OutputEvent::FocusGained(info) => Some(info.typ),
                _ => None,
            })
        });
        let focused = ctx.memory(|memory| memory.focused());
        match gained {
            Some(WidgetType::RadioButton) => state.focused = focused,
            Some(_) => state.focused = None,
            // Taken by a click elsewhere, or by Escape.
            None if focused != state.focused => state.focused = None,
            None => {}
        }
        ctx.data_mut(|data| data.insert_temp(Self::id(), state));
    }
}

/// Left and Right among `cells`, the cells of one choice in reading order
/// with `selected` the chosen one: the cell the keyboard moves to, which is
/// chosen with it, as in Terminal. Called before the cells are made, so the
/// choice is drawn where it went in the same frame. `None` without such an
/// arrow, or at the end the arrow points past.
pub fn arrow_choice(
    ctx: &eframe::egui::Context,
    cells: &[eframe::egui::Id],
    selected: usize,
) -> Option<usize> {
    use eframe::egui::{Event, Key};
    let focused = ctx.memory(|memory| memory.focused())?;
    let at = cells.iter().position(|cell| *cell == focused)?;
    let step = ctx.input(|input| {
        input.events.iter().fold(0_i32, |step, event| match event {
            Event::Key {
                key: Key::ArrowLeft,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none() => step - 1,
            Event::Key {
                key: Key::ArrowRight,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none() => step + 1,
            _ => step,
        })
    });
    if step == 0 {
        return None;
    }
    let to = (at as i32 + step).clamp(0, cells.len() as i32 - 1) as usize;
    ctx.memory_mut(|memory| {
        // The keyboard goes to the next cell of this choice, not to
        // whatever egui finds nearest that way.
        memory.move_focus(eframe::egui::FocusDirection::None);
        memory.request_focus(cells[to]);
    });
    (to != selected).then_some(to)
}

/// What is selected. Clips and a library item are selected independently: the
/// inspector shows the clip when there is one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub clips: Vec<ClipId>,
    pub asset: Option<AssetId>,
}

impl Selection {
    pub fn contains(&self, clip: ClipId) -> bool {
        self.clips.contains(&clip)
    }
    pub fn only(&mut self, clip: ClipId) {
        self.clips.clear();
        self.clips.push(clip);
        self.asset = None;
    }
    pub fn toggle(&mut self, clip: ClipId) {
        match self.clips.iter().position(|id| *id == clip) {
            Some(index) => {
                self.clips.remove(index);
            }
            None => self.clips.push(clip),
        }
        self.asset = None;
    }
    pub fn clear(&mut self) {
        self.clips.clear();
        self.asset = None;
    }
    /// The clip the inspector edits: the only selected one.
    pub fn single(&self) -> Option<ClipId> {
        match self.clips.as_slice() {
            [clip] => Some(*clip),
            _ => None,
        }
    }
}

/// A key colour being picked off the canvas: the next click on the picture
/// of `clip` gives its colour there to the chroma key `effect`. Both were
/// captured when the pick was begun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourPick {
    pub clip: ClipId,
    pub effect: Uuid,
}

/// A box being chosen on a clip's picture for a tracker to follow. It is no
/// part of the project: Escape gives it up, and Track hands it to the
/// analysis worker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackRegion {
    /// The clip whose picture it was drawn on, and the media that clip
    /// shows: both captured when the region was begun.
    pub clip: ClipId,
    pub asset: AssetId,
    /// Fractions of the source frame, origin top left.
    pub center: [f32; 2],
    pub size: [f32; 2],
}

impl TrackRegion {
    /// The smallest side a region is drawn with, as a fraction of the frame.
    pub const MIN_SIDE: f32 = editor_media::Region::MIN_SIDE;

    /// A region in the middle of the picture, to be moved to what it is to
    /// follow.
    pub fn new(clip: ClipId, asset: AssetId) -> Self {
        Self {
            clip,
            asset,
            center: [0.5, 0.5],
            size: [0.2, 0.2],
        }
    }

    /// The region held inside the frame, no smaller than it may be.
    pub fn held(mut self) -> Self {
        let held = editor_media::Region {
            center: self.center,
            size: self.size,
        }
        .held();
        (self.center, self.size) = (held.center, held.size);
        self
    }
}

/// A region to follow, with everything it is about captured when Track was
/// pressed.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackRequest {
    pub region: TrackRegion,
    /// Source times: the frame under the playhead, which the region was
    /// drawn on, and the end of what the clip shows of its source.
    pub from: Time,
    pub until: Time,
    /// What the tracker is called.
    pub name: String,
}

/// What is known of the camera path of the selected clip's video, as the
/// application last said.
#[derive(Clone, Debug, PartialEq)]
pub struct SteadyShown {
    pub clip: ClipId,
    pub asset: AssetId,
    pub status: crate::steady::Status,
    /// The least crop that hides the borders of the clip's corrections,
    /// when its path is held.
    pub suggested: Option<f32>,
}

/// The tracking under way, as the inspector shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackProgress {
    pub clip: ClipId,
    pub asset: AssetId,
    pub name: String,
    /// `0..=1`.
    pub done: f32,
    /// The path has come and waits to be added, and why it was not added
    /// by itself, in words.
    pub waits: Option<String>,
}

/// The duck under way, as the inspector shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckProgress {
    /// The clips it ducks.
    pub clips: Vec<ClipId>,
    /// The media whose sound is being read, when it is still there.
    pub reading: Option<String>,
    /// The media taken so far, of all there are.
    pub taken: usize,
    pub total: usize,
}

/// A keyframe being written while a value keyframes drive is dragged or
/// typed: the track's key at `at` takes `value`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyDraft {
    /// The effect or mask the track addresses; `None` is the clip.
    pub target: Option<Uuid>,
    pub key: ParamKey,
    /// From the clip's start, captured when the edit began.
    pub at: Time,
    pub value: f32,
}

/// Values being changed by a held drag or a focused field. The viewer and the
/// inspector draw from it; the application commits it as one undo step when
/// the interaction ends.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipDraft {
    pub clip: ClipId,
    pub transform: Transform,
    pub gain: f32,
    /// How long the clip's sound fades in from its start and out to its end.
    pub fade_in: Time,
    pub fade_out: Time,
    pub rate: f64,
    /// What a title or graphic says and how it looks; none for media.
    pub content: Option<ClipContent>,
    pub blend: BlendMode,
    /// The effect stack and the masks, while they differ from the stored
    /// ones. An untouched list is not copied.
    pub effects: Option<Vec<Effect>>,
    pub masks: Option<Vec<Mask>>,
    pub keys: Vec<KeyDraft>,
    /// The time map a held point of the speed curve makes, in source time,
    /// and the clip's length with it. The canvas does not draw from it: the
    /// clip plays as stored until the point is let go.
    pub retime: Option<(Retime, Time)>,
    /// The transitions at the clip's start and its end, while either
    /// differs from the stored one. The canvas does not draw from it.
    pub transitions: Option<[Option<Transition>; 2]>,
    /// The tracker the clip follows and how far from it.
    pub follow: Option<Follow>,
}

impl ClipDraft {
    pub fn of(clip: &Clip) -> Self {
        Self {
            clip: clip.id,
            transform: clip.transform.clone(),
            gain: clip.gain,
            fade_in: clip.fade_in,
            fade_out: clip.fade_out,
            rate: clip.playback_rate,
            content: clip.content.is_generated().then(|| clip.content.clone()),
            blend: clip.blend,
            effects: None,
            masks: None,
            keys: Vec::new(),
            retime: None,
            transitions: None,
            follow: clip.follow.clone(),
        }
    }

    /// The key a keyframe draft holds for one parameter.
    pub fn key(&self, target: Option<Uuid>, key: ParamKey) -> Option<&KeyDraft> {
        (self.keys.iter()).find(|held| held.target == target && held.key == key)
    }

    /// Holds `value` for the key at `at` of one parameter's track.
    pub fn set_key(&mut self, target: Option<Uuid>, key: ParamKey, at: Time, value: f32) {
        match (self.keys.iter_mut()).find(|held| held.target == target && held.key == key) {
            Some(held) => held.value = value,
            None => self.keys.push(KeyDraft {
                target,
                key,
                at,
                value,
            }),
        }
    }

    /// The clip's animation with the held keys written into it.
    fn animation(&self, clip: &Clip) -> Vec<AnimTrack> {
        let mut tracks = clip.animation.clone();
        for held in &self.keys {
            if let Some(track) = (tracks.iter_mut())
                .find(|track| track.target == held.target && track.key == held.key)
            {
                track.keys = with_key(&track.keys, held.at, held.value);
            }
        }
        tracks
    }

    /// What the canvas draws in the stored values' place while this is held.
    pub fn overriding(&self, project: &Project) -> ClipOverride {
        let clip = project.clip(self.clip).map(|(_, clip)| clip);
        let mut transform = self.transform.clone();
        // An offset from a tracker being changed is drawn as the same move
        // of the layer: the plan adds the stored offset to its position.
        if let (Some(held), Some(stored)) = (&self.follow, clip.and_then(|c| c.follow.as_ref()))
            && held.tracker_id == stored.tracker_id
            && held.position
            && stored.position
        {
            for axis in 0..2 {
                transform.position[axis] += held.offset[axis] - stored.offset[axis];
            }
        }
        ClipOverride {
            clip: self.clip,
            transform: Some(transform),
            effects: self.effects.clone(),
            masks: self.masks.clone(),
            blend: Some(self.blend),
            animation: clip
                .filter(|_| !self.keys.is_empty())
                .map(|clip| self.animation(clip)),
            reading: None,
            content: self.content.clone(),
        }
    }

    /// The one command that makes the project what this holds, so that it is
    /// one undo step; none when it holds what is stored.
    pub fn command(&self, project: &Project) -> Option<Command> {
        let mut commands = self.commands(project);
        match commands.len() {
            0 => None,
            1 => commands.pop(),
            _ => Some(Command::Batch {
                label: "Edit clip".into(),
                commands,
            }),
        }
    }

    /// The commands that make the project what this holds. A value that is
    /// the stored one asks for nothing.
    pub fn commands(&self, project: &Project) -> Vec<Command> {
        let Some((track, clip)) = project.clip(self.clip) else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        if self.transform != clip.transform {
            commands.push(Command::SetTransform {
                clip_id: clip.id,
                transform: self.transform.clone(),
            });
        }
        if self.gain != clip.gain {
            commands.push(Command::SetGain {
                clip_id: clip.id,
                gain: self.gain,
            });
        }
        // Before a change of speed: the fades are held to the clip as it is,
        // and a clip made shorter after them has them cut to fit.
        if (self.fade_in, self.fade_out) != (clip.fade_in, clip.fade_out) {
            commands.push(Command::SetFades {
                clip_id: clip.id,
                fade_in: self.fade_in,
                fade_out: self.fade_out,
            });
        }
        if self.rate != clip.playback_rate {
            commands.push(Command::SetPlaybackRate {
                clip_id: clip.id,
                rate: self.rate,
            });
            // A slower clip is longer. Where the next clip is in the way it
            // is cut to the room there is, rather than laid over it.
            let length = clip.duration.scaled(clip.playback_rate / self.rate);
            let room = crate::speed::room(track, clip);
            if let Some(room) = room.filter(|room| *room < length) {
                commands.push(Command::TrimClip {
                    clip_id: clip.id,
                    source_in: clip.source_in,
                    duration: room,
                    ripple: false,
                });
            }
        }
        if let Some((retime, duration)) = &self.retime
            && (clip.retime.as_ref() != Some(retime) || clip.duration != *duration)
        {
            commands.extend(crate::speed::retime_commands(
                track, clip, retime, *duration,
            ));
        }
        if let Some(held) = &self.content {
            commands.extend(content_commands(clip, held));
        }
        if self.blend != clip.blend {
            commands.push(Command::SetBlendMode {
                clip_id: clip.id,
                mode: self.blend,
            });
        }
        if let Some(effects) = (self.effects.as_ref()).filter(|held| **held != clip.effects) {
            commands.push(Command::SetEffects {
                clip_id: clip.id,
                effects: effects.clone(),
            });
        }
        if let Some(masks) = (self.masks.as_ref()).filter(|held| **held != clip.masks) {
            commands.push(Command::SetMasks {
                clip_id: clip.id,
                masks: masks.clone(),
            });
        }
        if let Some(held) = &self.transitions {
            let stored = [&clip.transition_in, &clip.transition_out];
            let edges = [TransitionEdge::In, TransitionEdge::Out];
            for ((held, stored), edge) in held.iter().zip(stored).zip(edges) {
                if held != stored {
                    commands.push(Command::SetTransition {
                        clip_id: clip.id,
                        edge,
                        transition: held.clone(),
                    });
                }
            }
        }
        if self.follow != clip.follow {
            commands.push(Command::SetFollow {
                clip_id: clip.id,
                follow: self.follow.clone(),
            });
        }
        for held in &self.keys {
            let Some(track) = (clip.animation.iter())
                .find(|track| track.target == held.target && track.key == held.key)
            else {
                continue;
            };
            let keys = with_key(&track.keys, held.at, held.value);
            if keys != track.keys {
                commands.push(Command::SetKeyframes {
                    clip_id: clip.id,
                    target: held.target,
                    key: held.key,
                    keys,
                });
            }
        }
        commands
    }
}

/// The commands that make a title or graphic what `held` is, field by field
/// so each part stays its own validated change: none for what is unchanged
/// or of another kind. Word times are not held: a new text carries them
/// over by the words it shares with the old.
fn content_commands(clip: &Clip, held: &ClipContent) -> Vec<Command> {
    let mut commands = Vec::new();
    match (&clip.content, held) {
        (
            ClipContent::Text {
                text,
                font_size,
                color,
                style,
                enter,
                exit,
                ..
            },
            ClipContent::Text {
                text: new_text,
                font_size: new_size,
                color: new_color,
                style: new_style,
                enter: new_enter,
                exit: new_exit,
                ..
            },
        ) => {
            if (text, font_size, color) != (new_text, new_size, new_color) {
                commands.push(Command::SetText {
                    clip_id: clip.id,
                    text: new_text.clone(),
                    font_size: *new_size,
                    color: *new_color,
                });
            }
            if style != new_style {
                commands.push(Command::SetTextStyle {
                    clip_id: clip.id,
                    style: new_style.clone(),
                });
            }
            for (edge, stored, animation) in [
                (TransitionEdge::In, enter, new_enter),
                (TransitionEdge::Out, exit, new_exit),
            ] {
                if stored != animation {
                    commands.push(Command::SetTextAnimation {
                        clip_id: clip.id,
                        edge,
                        animation: *animation,
                    });
                }
            }
        }
        (
            ClipContent::Graphic { size, elements },
            ClipContent::Graphic {
                size: new_size,
                elements: new_elements,
            },
        ) if (size, elements) != (new_size, new_elements) => {
            commands.push(Command::SetGraphic {
                clip_id: clip.id,
                size: *new_size,
                elements: new_elements.clone(),
            });
        }
        _ => {}
    }
    commands
}

/// `keys` with `value` at `at`: the key that is there keeps its easing, and a
/// new one leaves in a straight line.
pub fn with_key(keys: &[Keyframe], at: Time, value: f32) -> Vec<Keyframe> {
    let mut keys = keys.to_vec();
    let index = keys.partition_point(|key| key.at < at);
    match keys.get_mut(index).filter(|key| key.at == at) {
        Some(key) => key.value = value,
        None => keys.insert(
            index,
            Keyframe {
                at,
                value,
                ease: Ease::Linear,
            },
        ),
    }
    keys
}

/// The easings a keyframe is offered by name. A key's easing shapes the
/// segment that leaves it.
pub const EASINGS: [(&str, Ease); 6] = [
    ("Linear", Ease::Linear),
    ("Hold", Ease::Hold),
    ("Ease in", bezier(0.42, 0.0, 1.0, 1.0)),
    ("Ease out", bezier(0.0, 0.0, 0.58, 1.0)),
    ("Ease in and out", bezier(0.42, 0.0, 0.58, 1.0)),
    ("Overshoot", bezier(0.34, 1.56, 0.64, 1.0)),
];

const fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Ease {
    Ease::Bezier { x1, y1, x2, y2 }
}

pub fn tone_of_media(kind: MediaKind) -> Tone {
    match kind {
        MediaKind::Video => Tone::Video,
        MediaKind::Audio => Tone::Audio,
        // A table changes pictures, and is filed with them.
        MediaKind::Image | MediaKind::Gif | MediaKind::Lut => Tone::Image,
        // A font is what titles are set in.
        MediaKind::Font => Tone::Text,
    }
}

pub fn tone_of_track(kind: TrackKind) -> Tone {
    match kind {
        TrackKind::Video => Tone::Video,
        TrackKind::Audio => Tone::Audio,
        TrackKind::Text => Tone::Text,
    }
}

pub fn icon_of_media(kind: MediaKind) -> crate::icons::Icon {
    use crate::icons::Icon;
    match kind {
        MediaKind::Video => Icon::Film,
        MediaKind::Audio => Icon::Music,
        MediaKind::Image | MediaKind::Gif => Icon::Image,
        MediaKind::Lut => Icon::Cube,
        MediaKind::Font => Icon::Text,
    }
}

pub fn icon_of_track(kind: TrackKind) -> crate::icons::Icon {
    use crate::icons::Icon;
    match kind {
        TrackKind::Video => Icon::Film,
        TrackKind::Audio => Icon::Music,
        TrackKind::Text => Icon::Text,
    }
}

/// The platform's name for the command modifier, for tooltips and menus.
/// The largest interface scale this window has room for: see
/// [`crate::settings::largest_scale`].
pub fn largest_scale(ctx: &eframe::egui::Context) -> f32 {
    // The window's size at 100%, whatever the scale now.
    let size = ctx.content_rect().size() * ctx.zoom_factor();
    crate::settings::largest_scale([size.x, size.y])
}

pub fn shortcut(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.replace("Cmd+", "⌘")
            .replace("Shift+", "⇧")
            .replace("Alt+", "⌥")
    } else {
        keys.replace("Cmd+", "Ctrl+")
    }
}

/// Says whether the interaction that made a draft is still going, so that a
/// drag or a typed value becomes one command when it ends and not before.
///
/// A value being typed is held for as long as its field has the keyboard,
/// however long the pauses between the keys: Return or leaving the field
/// commits it and Escape gives it up. A draft the pointer made is held while
/// the pointer holds it, and no longer because a field elsewhere still has
/// the keyboard (a drag does not take it away).
#[derive(Default)]
pub struct DraftHold {
    seen: Option<ClipDraft>,
    project_seen: inspector::ProjectDraft,
    /// Whether the drafts were last changed with a pointer button down.
    by_pointer: bool,
}

impl DraftHold {
    /// Whether the drafts are still being made. Asked once a frame, after
    /// the frame was drawn.
    pub fn holds(
        &mut self,
        ctx: &eframe::egui::Context,
        draft: &Option<ClipDraft>,
        project: &inspector::ProjectDraft,
    ) -> bool {
        if *draft != self.seen || *project != self.project_seen {
            self.by_pointer = ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
            self.seen = draft.clone();
            self.project_seen = project.clone();
        }
        ctx.egui_is_using_pointer() || (ctx.text_edit_focused() && !self.by_pointer)
    }

    /// The drafts were committed or dropped: the next are new.
    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::PreferencesPane;
    use eframe::egui;

    #[test]
    fn preferences_open_at_the_pane_they_were_asked_for() {
        // §5.9.2: an entry point names the pane, kept for the window's
        // session under "preferences-pane"; a fresh window opens at
        // Appearance.
        let ctx = egui::Context::default();
        assert_eq!(PreferencesPane::shown(&ctx), PreferencesPane::Appearance);
        PreferencesPane::Agent.show(&ctx);
        assert_eq!(PreferencesPane::shown(&ctx), PreferencesPane::Agent);
        assert_eq!(
            ctx.data(|data| data.get_temp(egui::Id::new("preferences-pane"))),
            Some(PreferencesPane::Agent)
        );
    }
}
