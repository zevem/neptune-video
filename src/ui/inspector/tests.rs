//! The clip inspector, driven as a person drives it.
//!
//! [`Bench`] paints [`show`] in a real egui pass and hands it pointer and key
//! events. A control is found by the name accessibility is given for it, so a
//! control with no name cannot be reached here either. The bench ends a frame
//! as the application does: an [`Action::Edit`] commits the draft and is then
//! dispatched, and a draft nothing holds any longer ([`DraftHold::holds`]) is
//! committed as its one command ([`ClipDraft::command`]). Every edit below is
//! therefore what `Editor::dispatch` accepted, and its undo is the editor's.
use super::{
    InspectorInput, InspectorState, ProjectDraft, Section,
    clip::{Layout, sections},
    look::{addable, effect_name, family, presented},
    show,
};
use crate::{
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, DraftHold, EASINGS, Selection, with_key},
};
use editor_core::{
    AnimTarget, Asset, BlendMode, Clip, ClipId, Command, Ease, Editor, Effect, EffectKind,
    FrameRate, Keyframe, LookPreset, Mask, MaskOp, MaskShape, MediaKind, ParamKey, Project, Time,
    TrackId, TrackKind, Transform, look, resolve,
};
use eframe::egui::{
    self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2, accesskit::Role, vec2,
};
use std::{path::PathBuf, time::Duration};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn media(name: &str, kind: MediaKind) -> Asset {
    let mut asset = Asset::new(PathBuf::from(format!("/media/{name}")), kind, seconds(10.0));
    asset.name = name.to_owned();
    if kind != MediaKind::Audio {
        asset.width = 1920;
        asset.height = 1080;
    }
    if kind == MediaKind::Video {
        asset.frame_rate = Some(FrameRate::default());
        asset.has_audio = true;
    }
    asset.file_size = 1000;
    asset
}

fn key(at: f64, value: f32) -> Keyframe {
    Keyframe {
        at: seconds(at),
        value,
        ease: Ease::Linear,
    }
}

/// A control accessibility was told of in the last frame.
struct Seen {
    node: egui::accesskit::NodeId,
    name: String,
    /// What a field holds, as it is read out.
    value: Option<String>,
    rect: Rect,
    role: Role,
    disabled: bool,
    /// Whether a control that is one of several is the chosen one.
    selected: Option<bool>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    textures: Textures,
    editor: Editor,
    state: InspectorState,
    selection: Selection,
    draft: Option<ClipDraft>,
    project_draft: ProjectDraft,
    /// What says a draft is still held, as the application asks it.
    hold: DraftHold,
    playhead: Time,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// What the editor refused, in the words of the refusal.
    refused: Vec<String>,
    /// How long the last frame said it could wait before the next.
    wait: Duration,
    /// How often the inspector asked for the file picker of an import.
    imports: usize,
    /// What the inspector said in a notice.
    said: Vec<String>,
    /// The regions the inspector asked the analysis worker to follow, and
    /// how often it asked for the tracking under way to be given up.
    tracks: Vec<crate::ui::TrackRequest>,
    /// The videos it asked to have analysed for stabilization.
    analyses: Vec<editor_core::AssetId>,
    cancels: usize,
}

impl Bench {
    /// A project with one video clip of four seconds from the start, which
    /// is selected, and the playhead a second into it.
    fn new() -> (Self, ClipId) {
        let asset = media("shot.mp4", MediaKind::Video);
        let mut bench = Self::empty();
        bench.edit(Command::AddAssets {
            assets: vec![asset.clone()],
        });
        let clip = bench.add(TrackKind::Video, {
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.duration = seconds(4.0);
            clip
        });
        bench.selection.only(clip);
        bench.playhead = seconds(1.0);
        (bench, clip)
    }

    fn empty() -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        Self {
            ctx,
            palette,
            textures: Textures::default(),
            editor: Editor::new(Project::new("Inspector")).expect("a new project is valid"),
            state: InspectorState::default(),
            selection: Selection::default(),
            draft: None,
            project_draft: ProjectDraft::default(),
            hold: DraftHold::default(),
            playhead: Time::ZERO,
            // Tall, so that every row of a section is on screen.
            size: vec2(300.0, 2400.0),
            time: 0.0,
            seen: Vec::new(),
            refused: Vec::new(),
            wait: Duration::ZERO,
            imports: 0,
            said: Vec::new(),
            tracks: Vec::new(),
            analyses: Vec::new(),
            cancels: 0,
        }
    }

    fn track(&self, kind: TrackKind) -> TrackId {
        (self.editor.project().tracks.iter())
            .find(|track| track.kind == kind)
            .map(|track| track.id)
            .expect("FIXTURE: a new project has a track of each kind")
    }

    fn add(&mut self, kind: TrackKind, clip: Clip) -> ClipId {
        let id = clip.id;
        let track_id = self.track(kind);
        self.edit(Command::AddClip { track_id, clip });
        id
    }

    /// A fixture's own edit: not one the inspector made.
    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }

    fn clip(&self, id: ClipId) -> &Clip {
        self.editor.project().clip(id).expect("the clip exists").1
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn last(&self) -> String {
        self.editor.undo_label().unwrap_or_default().to_owned()
    }

    fn commit(&mut self) {
        // The canvas's values are committed with the clip's, as
        // `App::commit_drafts` does.
        if let Some(settings) = std::mem::take(&mut self.project_draft).canvas
            && settings != self.editor.project().settings
            && let Err(error) = self.editor.dispatch(Command::SetSettings { settings })
        {
            self.refused.push(error.to_string());
        }
        let Some(draft) = self.draft.take() else {
            return;
        };
        if let Some(command) = draft.command(self.editor.project())
            && let Err(error) = self.editor.dispatch(command)
        {
            self.refused.push(error.to_string());
        }
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let mut actions: Vec<Action> = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            // The application hears an Escape before it draws the frame.
            if ui.input(|i| i.key_pressed(Key::Escape))
                && (InspectorState::holds(ui.ctx(), &self.draft) || self.state.typed(ui.ctx()))
            {
                self.state.cancel(&mut self.draft);
            }
            let rect = ui.max_rect();
            show(
                ui,
                rect,
                &mut self.state,
                &self.selection,
                &mut self.draft,
                &mut self.project_draft,
                &InspectorInput {
                    p: &self.palette,
                    project: &project,
                    textures: &self.textures,
                    playhead: self.playhead,
                    transition: None,
                },
                &mut actions,
            );
        });
        // No renderer takes the textures.
        output.textures_delta.clear();
        self.wait = (output.viewport_output.values())
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(Duration::MAX);
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(id, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        node: *id,
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        value: node.value().map(str::to_owned),
                        rect: Rect::from_min_max(
                            Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                            Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        disabled: node.is_disabled(),
                        selected: (node.toggled())
                            .map(|toggled| toggled == egui::accesskit::Toggled::True),
                    })
                })
                .collect();
        }
        // The end of the application's frame.
        for action in actions {
            match action {
                Action::Edit(command) => {
                    self.commit();
                    if let Err(error) = self.editor.dispatch(command) {
                        self.refused.push(error.to_string());
                    }
                }
                Action::Seek(time) => self.playhead = time,
                Action::ImportFiles => self.imports += 1,
                Action::Notify(text) => self.said.push(text),
                Action::Track(request) => self.tracks.push(request),
                Action::CancelTracking => self.cancels += 1,
                Action::AddTracked => self.said.push("add tracked".to_owned()),
                Action::Stabilize(asset) => self.analyses.push(asset),
                Action::CancelStabilize => self.cancels += 1,
                other => panic!("FIXTURE: the inspector asked for {other:?}"),
            }
        }
        if !(self.hold).holds(&ctx, &self.draft, &self.project_draft) {
            self.hold.forget();
            self.commit();
        }
    }

    /// Frames in which nothing is done, for this many seconds.
    fn rest(&mut self, seconds: f64) {
        let frames = (seconds / 0.25).ceil() as usize;
        for _ in 0..frames {
            self.time += 0.2;
            self.frame(Vec::new());
        }
    }

    /// The control with this name; a label that only says the name is not it.
    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter()).find(|seen| seen.name == name && !Self::is_text(seen))
    }

    /// A label, or a run of its text.
    fn is_text(seen: &Seen) -> bool {
        matches!(seen.role, Role::Label | Role::TextRun)
    }

    fn has(&self, name: &str) -> bool {
        self.seen.iter().any(|seen| seen.name == name)
    }

    fn at(&self, name: &str) -> Pos2 {
        match self.find(name) {
            Some(seen) => seen.rect.center(),
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            }
        }
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn release(&mut self, pos: Pos2) {
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        // The frame after the release shows what it committed.
        self.frame(Vec::new());
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.press(pos);
        self.release(pos);
    }

    /// Gives the control the keyboard focus, as a screen reader does.
    fn focus(&mut self, name: &str) {
        self.frame(Vec::new());
        let target_node = match self.find(name) {
            Some(seen) => seen.node,
            None => panic!("FIXTURE: no control is named \"{name}\""),
        };
        self.frame(vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Focus,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node,
                data: None,
            },
        )]);
        self.frame(Vec::new());
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
    }

    fn enter(&mut self, modifiers: Modifiers) {
        self.key(Key::Enter, modifiers);
        self.frame(Vec::new());
    }

    fn secondary_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
    }

    /// Takes hold of the control and moves it `by`, without letting go.
    fn drag(&mut self, name: &str, by: Vec2) -> Pos2 {
        self.frame(Vec::new());
        let from = self.at(name);
        self.press(from);
        for step in 1..=4 {
            self.move_to(from + by * (step as f32 / 4.0));
        }
        from + by
    }

    /// The layer the canvas would draw for `clip` now, draft and all.
    fn layer(&self, clip: ClipId) -> editor_core::Layer {
        let project = self.editor.project();
        let overrides: Vec<_> = (self.draft.iter())
            .map(|draft| draft.overriding(project))
            .collect();
        (resolve(project, self.playhead, &overrides)
            .layers
            .into_iter())
        .find(|layer| layer.clip == clip)
        .expect("the clip is on screen at the playhead")
    }
}

/// A bench whose clip has `effect`, open in the Look section.
fn with_effect(kind: EffectKind) -> (Bench, ClipId, Effect) {
    let (mut bench, clip) = Bench::new();
    let effect = Effect::new(kind);
    bench.edit(Command::SetEffects {
        clip_id: clip,
        effects: vec![effect.clone()],
    });
    bench.state.section = Section::Look;
    bench.state.effect = Some(effect.id);
    (bench, clip, effect)
}

fn opacity_track(bench: &mut Bench, clip: ClipId) {
    bench.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::Opacity,
        keys: vec![key(0.0, 0.0), key(2.0, 1.0)],
    });
}

fn opacity_keys(bench: &Bench, clip: ClipId) -> Vec<Keyframe> {
    (bench.clip(clip).animation.iter())
        .find(|track| track.target.is_none() && track.key == ParamKey::Opacity)
        .map(|track| track.keys.clone())
        .unwrap_or_default()
}

// ---- Drafts: one gesture, one step --------------------------------------

#[test]
fn dragging_an_effects_value_previews_it_and_commits_one_step_on_release() {
    let (mut bench, clip, effect) = with_effect(EffectKind::Glow);
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let end = bench.drag("Amount", vec2(60.0, 0.0));

    // Held: the project is as it was, and the canvas shows the draft.
    assert_eq!(bench.steps(), before, "a held drag is no step");
    assert_eq!(*bench.editor.project(), *stored);
    let held = bench.layer(clip).effects[0].param(ParamKey::Amount);
    assert!(
        held.is_some_and(|amount| amount > 0.5),
        "previewed: {held:?}"
    );

    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "the whole drag is one step");
    assert_eq!(bench.last(), "Change effects");
    assert!(bench.draft.is_none());
    let committed = bench.clip(clip).effects[0].param(ParamKey::Amount);
    assert_eq!(committed, held, "what was previewed is what was committed");
    assert_eq!(bench.clip(clip).effects[0].id, effect.id);

    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects, vec![effect]);
}

#[test]
fn an_escape_gives_up_a_held_drag_and_leaves_the_project_unchanged() {
    let (mut bench, clip, effect) = with_effect(EffectKind::Glow);
    let before = bench.steps();
    let end = bench.drag("Amount", vec2(60.0, 0.0));
    assert!(bench.draft.is_some());

    bench.key(Key::Escape, Modifiers::NONE);
    assert!(bench.draft.is_none(), "the Escape dropped the draft");
    // The pointer is still down and still moving: nothing it writes is kept.
    bench.move_to(end + vec2(30.0, 0.0));
    assert!(bench.draft.is_none(), "the cancelled drag writes no draft");
    assert_eq!(
        bench.layer(clip).effects[0].param(ParamKey::Amount),
        Some(0.5)
    );
    bench.release(end + vec2(30.0, 0.0));
    assert_eq!(bench.steps(), before, "a cancelled drag is no step");
    assert_eq!(bench.clip(clip).effects, vec![effect]);

    // The next drag is its own, and is kept.
    let end = bench.drag("Amount", vec2(60.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
}

#[test]
fn an_escape_in_a_field_being_typed_in_gives_up_what_was_typed() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    bench.focus("Rotation");
    bench.frame(vec![Event::Text("45".to_owned())]);
    let held = bench.draft.as_ref().map(|draft| draft.transform.rotation);
    assert_eq!(held, Some(45.0), "what is typed is held as it is typed");
    assert_eq!(bench.steps(), before, "and is no step yet");

    // No pointer is down. egui takes the keyboard from the field before the
    // frame with the Escape in it is drawn, so it is the frame before that
    // says a field was being typed in.
    bench.key(Key::Escape, Modifiers::NONE);
    assert!(bench.draft.is_none(), "the Escape dropped what was typed");
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before, "a value given up is no step");
    assert_eq!(bench.clip(clip).transform.rotation, 0.0);

    // What is typed next is kept when the field is left with Return.
    bench.focus("Rotation");
    bench.frame(vec![Event::Text("30".to_owned())]);
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.clip(clip).transform.rotation, 30.0);
}

#[test]
fn a_transform_drag_is_one_step_and_undoes() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    let end = bench.drag("Rotation", vec2(40.0, 0.0));
    let held = bench.layer(clip).transform.rotation;
    assert!(held > 0.0);
    assert_eq!(bench.clip(clip).transform.rotation, 0.0);
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Transform clip");
    assert_eq!(bench.clip(clip).transform.rotation, held);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).transform, Transform::default());
}

#[test]
fn a_mix_slider_scrub_is_one_step() {
    let (mut bench, clip, _) = with_effect(EffectKind::Blur);
    let before = bench.steps();
    bench.frame(Vec::new());
    let track = bench
        .find("Blur mix")
        .expect("the mix slider is named")
        .rect;
    let from = Pos2::new(track.right() - 8.0, track.center().y);
    bench.press(from);
    for step in 1..=4 {
        bench.move_to(from - vec2(15.0 * step as f32, 0.0));
    }
    let held = bench.layer(clip).effects[0].mix;
    assert!(held < 0.9 && held > 0.0, "previewed mix: {held}");
    assert_eq!(bench.clip(clip).effects[0].mix, 1.0);
    bench.release(from - vec2(60.0, 0.0));
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.clip(clip).effects[0].mix, held);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects[0].mix, 1.0);
}

// ---- The effect stack ----------------------------------------------------

#[test]
fn an_effect_is_added_from_the_menu_reordered_hidden_and_removed() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Look;
    let kinds = |bench: &Bench| -> Vec<EffectKind> {
        (bench.clip(clip).effects.iter())
            .map(|effect| effect.kind)
            .collect()
    };
    let before = bench.steps();

    bench.click("Add effect");
    bench.click("Glow");
    assert_eq!(kinds(&bench), [EffectKind::Glow]);
    assert_eq!(bench.steps(), before + 1);
    let glow = bench.clip(clip).effects[0].clone();
    assert_eq!(glow, {
        let mut fresh = Effect::new(EffectKind::Glow);
        fresh.id = glow.id;
        fresh
    });
    // The effect that was added is the one whose rows are open.
    assert_eq!(bench.state.effect, Some(glow.id));

    bench.click("Add effect");
    bench.click("Blur");
    assert_eq!(kinds(&bench), [EffectKind::Glow, EffectKind::Blur]);

    bench.click("Move Blur up");
    assert_eq!(kinds(&bench), [EffectKind::Blur, EffectKind::Glow]);
    assert_eq!(bench.last(), "Change effects");
    bench.click("Move Blur down");
    assert_eq!(kinds(&bench), [EffectKind::Glow, EffectKind::Blur]);
    // The ends of the stack have nowhere further to go.
    bench.frame(Vec::new());
    assert!(bench.find("Move Glow up").is_some_and(|seen| seen.disabled));
    assert!(
        bench
            .find("Move Blur down")
            .is_some_and(|seen| seen.disabled)
    );

    bench.click("Show Glow");
    assert!(!bench.clip(clip).effects[0].enabled);
    assert!(
        bench
            .layer(clip)
            .effects
            .iter()
            .all(|e| e.kind != EffectKind::Glow),
        "a hidden effect is not drawn"
    );

    let steps = bench.steps();
    bench.click("Remove Glow");
    assert_eq!(kinds(&bench), [EffectKind::Blur]);
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.editor.undo());
    assert_eq!(kinds(&bench), [EffectKind::Glow, EffectKind::Blur]);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_value_off_its_default_has_a_reset() {
    let (mut bench, clip, mut effect) = with_effect(EffectKind::Vignette);
    bench.frame(Vec::new());
    assert!(bench.find("Reset amount").is_none(), "nothing to reset yet");
    effect.params = vec![editor_core::ParamValue {
        key: ParamKey::Amount,
        value: 0.9,
    }];
    bench.edit(Command::SetEffects {
        clip_id: clip,
        effects: vec![effect],
    });
    let before = bench.steps();
    bench.click("Reset amount");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(
        bench.clip(clip).effects[0].param(ParamKey::Amount),
        Some(0.4)
    );
}

#[test]
fn a_look_is_applied_to_the_clip_it_was_asked_for_at_the_strength_set() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Look;
    bench.state.strength = 0.5;
    let before = bench.steps();
    bench.click("Noir");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Apply look");
    let expected = look::preset_effects(LookPreset::Noir, 0.5);
    let applied = &bench.clip(clip).effects;
    assert_eq!(applied.len(), expected.len());
    for (applied, expected) in applied.iter().zip(&expected) {
        assert_eq!(
            (applied.kind, &applied.params),
            (expected.kind, &expected.params)
        );
    }
    assert!(bench.editor.undo());
    assert!(bench.clip(clip).effects.is_empty());
}

#[test]
fn another_look_takes_the_place_of_the_one_given_and_its_strength_is_one_step() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Look;
    let kinds = |bench: &Bench| -> Vec<EffectKind> {
        (bench.clip(clip).effects.iter())
            .map(|effect| effect.kind)
            .collect()
    };
    let of = |preset| -> Vec<EffectKind> {
        (look::preset_effects(preset, 1.0).iter())
            .map(|effect| effect.kind)
            .collect()
    };
    let before = bench.steps();
    // The same look again, and then another: one look, not three stacked.
    bench.click("Cinematic");
    bench.click("Cinematic");
    assert_eq!(kinds(&bench), of(LookPreset::Cinematic));
    bench.click("Noir");
    assert_eq!(kinds(&bench), of(LookPreset::Noir));
    // The look the clip already has is not given again: no empty step.
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.last(), "Apply look");
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    let full = bench.clip(clip).effects.clone();
    let contrast = |effects: &[Effect]| effects[0].param(ParamKey::Contrast);
    assert_eq!(contrast(&full), Some(0.35));

    // The strength tones the look the clip has: previewed while it is
    // held, and one step when it is let go.
    let steps = bench.steps();
    bench.frame(Vec::new());
    let track = bench.find("Look strength").expect("named").rect;
    let from = Pos2::new(track.right() - 8.0, track.center().y);
    bench.press(from);
    for step in 1..=4 {
        bench.move_to(from - vec2(12.0 * step as f32, 0.0));
    }
    assert_eq!(bench.clip(clip).effects, full, "held, not written");
    let held = bench.layer(clip).effects[0].param(ParamKey::Contrast);
    assert!(
        held.is_some_and(|held| held > 0.0 && held < 0.3),
        "{held:?}"
    );
    bench.release(from - vec2(48.0, 0.0));
    assert_eq!(bench.steps(), steps + 1, "the whole scrub is one step");
    let toned = bench.clip(clip).effects.clone();
    assert_eq!(contrast(&toned), held);
    let ids = |effects: &[Effect]| effects.iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids(&toned), ids(&full), "the same effects, made again");
    assert_eq!(
        contrast(&toned),
        contrast(&look::preset_effects(
            LookPreset::Noir,
            look::applied_look(&toned).expect("a look").strength
        ))
    );
    // The strength kept for a clip that has no look is not this look's.
    assert_eq!(bench.state.strength, 1.0);

    // An Escape gives a held strength up.
    bench.press(from - vec2(48.0, 0.0));
    bench.move_to(from - vec2(80.0, 0.0));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.release(from - vec2(80.0, 0.0));
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.clip(clip).effects, toned);

    // An effect of the clip's own stays when the look goes.
    let mut with = toned.clone();
    with.push(Effect::new(EffectKind::Glow));
    bench.edit(Command::SetEffects {
        clip_id: clip,
        effects: with,
    });
    bench.click("Remove look");
    assert_eq!(bench.last(), "Remove look");
    assert_eq!(kinds(&bench), [EffectKind::Glow]);
    assert!(!bench.has("Remove look"));
    // Undone, the look is the clip's again and can be removed again.
    assert!(bench.editor.undo());
    bench.frame(Vec::new());
    assert!(bench.has("Remove look"));
}

#[test]
fn the_look_a_clip_has_is_read_from_its_effects_whoever_gave_it() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Look;
    // As the library and the agent give one: the command, at a strength.
    bench.edit(Command::ApplyPreset {
        clip_ids: vec![clip],
        preset: LookPreset::TealOrange,
        strength: 0.5,
    });
    bench.frame(Vec::new());
    let chosen = |bench: &Bench, name: &str| bench.find(name).expect("named").selected;
    assert_eq!(chosen(&bench, "Teal & orange"), Some(true));
    assert_eq!(chosen(&bench, "Noir"), Some(false));
    assert!(bench.has("Remove look"));
    // Another look from here takes its place, as strong as it was.
    bench.click("Noir");
    let held = look::applied_look(&bench.clip(clip).effects).expect("a look");
    assert_eq!((held.preset, held.at), (LookPreset::Noir, 0));
    assert_eq!(held.len, bench.clip(clip).effects.len());
    assert!((held.strength - 0.5).abs() < 1e-3, "{held:?}");
    // Undone twice the clip has no look, and nothing says it has.
    assert!(bench.editor.undo() && bench.editor.undo());
    bench.frame(Vec::new());
    assert_eq!(chosen(&bench, "Noir"), Some(false));
    assert!(!bench.has("Remove look"));
}

#[test]
fn a_field_of_many_turns_is_dragged_half_a_degree_at_a_time() {
    for (target, key) in [
        (AnimTarget::Mask, ParamKey::Rotation),
        (AnimTarget::Effect(EffectKind::Distort), ParamKey::OffsetX),
    ] {
        let spec = look::param_spec(target, key).expect("FIXTURE: the tables have it");
        let (name, field) = presented(target, spec);
        assert!(field.speed <= 1.0, "{name}: {} a point", field.speed);
    }
    let turn = look::param_spec(AnimTarget::Mask, ParamKey::Rotation).expect("FIXTURE");
    assert_eq!(presented(AnimTarget::Mask, turn).1.speed, 0.5);
    // A narrow range is still crossed in three hundred points.
    let feather = look::param_spec(AnimTarget::Mask, ParamKey::Feather).expect("FIXTURE");
    let field = presented(AnimTarget::Mask, feather).1;
    assert!(field.speed < 0.5 && field.speed >= 0.001, "{}", field.speed);
}

#[test]
fn a_key_colour_is_a_swatch_and_a_table_a_list_of_tables() {
    let (mut bench, _, _) = with_effect(EffectKind::ChromaKey);
    bench.frame(Vec::new());
    assert!(bench.find("Key colour").is_some());
    for hidden in ["Key red", "Key green", "Key blue"] {
        assert!(!bench.has(hidden), "{hidden} is part of the swatch");
    }
    assert!(bench.find("Tolerance").is_some());

    // A lookup's table is chosen among the project's tables.
    let (mut bench, clip) = Bench::new();
    let (first, second) = (
        media("warm.cube", MediaKind::Lut),
        media("cold.cube", MediaKind::Lut),
    );
    let table = |mut asset: Asset| {
        asset.width = 17;
        asset.height = 0;
        asset
    };
    let (first, second) = (table(first), table(second));
    bench.edit(Command::AddAssets {
        assets: vec![
            first.clone(),
            second.clone(),
            media("still.png", MediaKind::Image),
        ],
    });
    let mut lut = Effect::new(EffectKind::Lut);
    lut.asset_id = Some(first.id);
    bench.edit(Command::SetEffects {
        clip_id: clip,
        effects: vec![lut.clone()],
    });
    bench.state.section = Section::Look;
    bench.state.effect = Some(lut.id);
    let before = bench.steps();
    bench.click("Table");
    assert!(bench.has("warm.cube") && bench.has("cold.cube"));
    assert!(!bench.has("still.png"), "only tables are offered");
    bench.click("cold.cube");
    assert_eq!(bench.clip(clip).effects[0].asset_id, Some(second.id));
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- Masks and blend -----------------------------------------------------

#[test]
fn a_mask_is_added_edited_and_removed_as_whole_lists() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    bench.click("Add mask");
    bench.click("Ellipse");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change masks");
    let mask = bench.clip(clip).masks[0].clone();
    assert_eq!(mask.shape, MaskShape::Ellipse);
    assert_eq!(bench.state.mask, Some(mask.id), "the new mask is open");
    assert_eq!(bench.state.outlined_mask(), Some(mask.id));

    bench.click("Subtract");
    assert_eq!(bench.clip(clip).masks[0].op, MaskOp::Subtract);
    bench.click("Invert Ellipse 1");
    assert!(bench.clip(clip).masks[0].invert);
    bench.click("Rectangle");
    assert_eq!(bench.clip(clip).masks[0].shape, MaskShape::Rectangle);

    // A drag of a mask's value is previewed and is one step.
    let steps = bench.steps();
    let end = bench.drag("Feather", vec2(45.0, 0.0));
    let held = bench.layer(clip).masks[0].feather;
    assert!(held > 0.0, "previewed feather: {held}");
    assert_eq!(bench.clip(clip).masks[0].feather, 0.0);
    bench.release(end);
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.clip(clip).masks[0].feather, held);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).masks[0].feather, 0.0);

    bench.click("Remove Rectangle 1");
    assert!(bench.clip(clip).masks.is_empty());
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).masks.len(), 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_blend_mode_is_chosen_from_its_list() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    bench.click("Blend mode");
    bench.click("Screen");
    assert_eq!(bench.clip(clip).blend, BlendMode::Screen);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change blend mode");
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).blend, BlendMode::Normal);
}

// ---- Layouts -------------------------------------------------------------

#[test]
fn a_layout_is_one_transform_command() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    bench.click("Inset, bottom right");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Transform clip");
    let t = bench.clip(clip).transform.clone();
    assert_eq!(t.scale, [0.3, 0.3]);
    // A 576×324 picture, 43 pixels from the right and bottom of 1920×1080.
    assert_eq!(t.position, [629.0, 335.0]);
    bench.frame(Vec::new());
    assert!(bench.has("Inset, bottom right"));
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).transform, Transform::default());
}

#[test]
fn layouts_place_a_picture_in_the_canvas() {
    let canvas = [1920.0, 1080.0];
    let from = Transform {
        rotation: 12.0,
        opacity: 0.5,
        position: [300.0, -40.0],
        scale: [2.0, 2.0],
        crop: [0.1, 0.0, 0.0, 0.0],
        ..Transform::default()
    };
    let fill = Layout::Fill.apply(&from, [1920.0, 1080.0], canvas);
    assert_eq!(
        (fill.position, fill.scale, fill.crop),
        ([0.0, 0.0], [1.0, 1.0], [0.0; 4])
    );
    // The turn and the opacity are not a layout's.
    assert_eq!((fill.rotation, fill.opacity), (12.0, 0.5));

    // Half the canvas is filled: a wide picture loses its sides.
    let left = Layout::Left.apply(&from, [1920.0, 1080.0], canvas);
    assert_eq!(left.position, [-480.0, 0.0]);
    assert_eq!(left.scale, [1.0, 1.0]);
    assert!((left.crop[0] - 0.25).abs() < 1e-5 && left.crop[0] == left.crop[2]);
    assert_eq!((left.crop[1], left.crop[3]), (0.0, 0.0));
    // And a tall one its top and bottom.
    let right = Layout::Right.apply(&from, [540.0, 1920.0], canvas);
    assert_eq!(right.position, [480.0, 0.0]);
    assert!(right.crop[1] > 0.0 && right.crop[1] == right.crop[3]);
    assert_eq!((right.crop[0], right.crop[2]), (0.0, 0.0));

    // Every layout is a transform the project accepts.
    for layout in Layout::ALL {
        for source in [[1920.0, 1080.0], [1080.0, 1920.0], [4000.0, 100.0]] {
            let laid = layout.apply(&from, source, canvas);
            assert!(laid.validate(), "{layout:?} of {source:?}: {laid:?}");
        }
    }
    let corner = Layout::TopLeft.apply(&from, [1920.0, 1080.0], canvas);
    assert!(corner.position[0] < 0.0 && corner.position[1] < 0.0);
}

#[test]
fn a_layout_is_not_offered_over_keyframes_it_would_not_show_through() {
    let (mut bench, clip) = Bench::new();
    bench.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::PositionX,
        keys: vec![key(0.0, 0.0), key(2.0, 400.0)],
    });
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(bench.find("Left half").is_some_and(|seen| seen.disabled));
    bench.click("Left half");
    assert_eq!(bench.steps(), before);
}

// ---- Keyframes -----------------------------------------------------------

#[test]
fn the_diamond_makes_a_keyframe_at_the_playhead_holding_the_value() {
    let (mut bench, clip) = Bench::new();
    bench.edit(Command::SetTransform {
        clip_id: clip,
        transform: Transform {
            opacity: 0.4,
            ..Transform::default()
        },
    });
    let before = bench.steps();
    bench.click("Animate opacity");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change keyframes");
    assert_eq!(opacity_keys(&bench, clip), [key(1.0, 0.4)]);

    // The row now steps between keyframes and removes the one it is on.
    bench.frame(Vec::new());
    assert!(bench.find("Remove opacity keyframe").is_some());
    assert!(
        bench
            .find("Previous opacity keyframe")
            .is_some_and(|seen| seen.disabled)
    );
    assert!(bench.editor.undo());
    assert!(bench.clip(clip).animation.is_empty());

    // A row of two values makes both of its tracks in one step.
    bench.click("Animate position");
    assert_eq!(bench.steps(), before + 1);
    let animated: Vec<ParamKey> = (bench.clip(clip).animation.iter())
        .map(|track| track.key)
        .collect();
    assert_eq!(animated, [ParamKey::PositionX, ParamKey::PositionY]);
    assert!(bench.editor.undo());
    assert!(bench.clip(clip).animation.is_empty());
}

#[test]
fn a_keyframe_is_made_for_the_clip_and_playhead_of_the_click() {
    let (mut bench, clip) = Bench::new();
    bench.frame(Vec::new());
    let pos = bench.at("Animate rotation");
    bench.press(pos);
    // The click ends in a frame whose action is taken after the selection
    // and the playhead have both gone elsewhere.
    let ctx = bench.ctx.clone();
    let project = bench.editor.snapshot();
    let mut actions: Vec<Action> = Vec::new();
    bench.time += 0.05;
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, bench.size)),
        time: Some(bench.time),
        events: vec![Bench::button(pos, PointerButton::Primary, false)],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let rect = ui.max_rect();
        show(
            ui,
            rect,
            &mut bench.state,
            &bench.selection,
            &mut bench.draft,
            &mut bench.project_draft,
            &InspectorInput {
                p: &bench.palette,
                project: &project,
                textures: &bench.textures,
                playhead: bench.playhead,
                transition: None,
            },
            &mut actions,
        );
    });
    output.textures_delta.clear();
    bench.selection.clear();
    bench.playhead = seconds(3.0);
    let [Action::Edit(command)] = actions.as_slice() else {
        panic!("one edit was asked for: {actions:?}");
    };
    let Command::SetKeyframes {
        clip_id, key, keys, ..
    } = command
    else {
        panic!("a keyframe was asked for: {command:?}");
    };
    assert_eq!(*clip_id, clip);
    assert_eq!(*key, ParamKey::Rotation);
    assert_eq!(keys.as_slice(), [key_at_one_second()]);
}

fn key_at_one_second() -> Keyframe {
    key(1.0, 0.0)
}

#[test]
fn an_animated_row_shows_and_edits_the_value_at_the_playhead() {
    let (mut bench, clip) = Bench::new();
    opacity_track(&mut bench, clip);
    let before = bench.steps();
    bench.frame(Vec::new());
    // Half way between a key of 0 and a key of 1.
    let shown = bench.find("Opacity").and_then(|seen| seen.value.clone());
    assert_eq!(shown.as_deref(), Some("50 %"), "the value at the playhead");
    assert!(bench.find("Add opacity keyframe").is_some());

    let end = bench.drag("Opacity", vec2(40.0, 0.0));
    // Held: the keyframes are as stored, and the canvas shows the draft.
    assert_eq!(opacity_keys(&bench, clip), [key(0.0, 0.0), key(2.0, 1.0)]);
    let held = bench.layer(clip).transform.opacity;
    assert!(
        held > 0.55,
        "a keyed value previews while it is dragged: {held}"
    );
    assert_eq!(bench.clip(clip).transform.opacity, 1.0);

    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "one step for the drag");
    assert_eq!(bench.last(), "Change keyframes");
    assert_eq!(
        opacity_keys(&bench, clip),
        [key(0.0, 0.0), key(1.0, held), key(2.0, 1.0)]
    );
    // The stored value under the keyframes is not what was edited.
    assert_eq!(bench.clip(clip).transform.opacity, 1.0);
    assert!(bench.editor.undo());
    assert_eq!(opacity_keys(&bench, clip), [key(0.0, 0.0), key(2.0, 1.0)]);

    // On a keyframe, the edit replaces it and makes no other.
    bench.playhead = seconds(2.0);
    let end = bench.drag("Opacity", vec2(-40.0, 0.0));
    bench.release(end);
    let keys = opacity_keys(&bench, clip);
    assert_eq!(keys.len(), 2);
    assert!(
        keys[1].at == seconds(2.0) && keys[1].value < 1.0,
        "{keys:?}"
    );
}

#[test]
fn an_escape_gives_up_a_keyframe_being_dragged() {
    let (mut bench, clip) = Bench::new();
    opacity_track(&mut bench, clip);
    let before = bench.steps();
    let end = bench.drag("Opacity", vec2(40.0, 0.0));
    assert!(
        bench
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.keys.is_empty())
    );
    bench.key(Key::Escape, Modifiers::NONE);
    bench.move_to(end + vec2(10.0, 0.0));
    assert!((bench.layer(clip).transform.opacity - 0.5).abs() < 1e-4);
    bench.release(end + vec2(10.0, 0.0));
    assert_eq!(bench.steps(), before);
    assert_eq!(opacity_keys(&bench, clip), [key(0.0, 0.0), key(2.0, 1.0)]);
}

#[test]
fn the_steps_beside_the_diamond_move_the_playhead_between_keyframes() {
    let (mut bench, clip) = Bench::new();
    opacity_track(&mut bench, clip);
    // The clip does not start where the timeline does.
    bench.edit(Command::MoveClips {
        clip_ids: vec![clip],
        delta: seconds(5.0),
        target_track: None,
    });
    bench.playhead = seconds(6.0);
    let before = bench.steps();
    bench.click("Next opacity keyframe");
    assert_eq!(bench.playhead, seconds(7.0));
    bench.frame(Vec::new());
    assert!(
        bench.find("Remove opacity keyframe").is_some(),
        "on the key"
    );
    assert!(
        bench
            .find("Next opacity keyframe")
            .is_some_and(|seen| seen.disabled)
    );
    bench.click("Previous opacity keyframe");
    assert_eq!(bench.playhead, seconds(5.0));
    assert_eq!(bench.steps(), before, "moving the playhead edits nothing");

    // On a keyframe the diamond takes it away; the last one takes the track.
    bench.click("Remove opacity keyframe");
    assert_eq!(opacity_keys(&bench, clip), [key(2.0, 1.0)]);
    assert_eq!(bench.steps(), before + 1);
    // Off a keyframe it adds one holding the value shown there.
    bench.click("Add opacity keyframe");
    assert_eq!(opacity_keys(&bench, clip), [key(0.0, 1.0), key(2.0, 1.0)]);
}

#[test]
fn the_diamonds_menu_sets_the_easing_and_removes_the_animation() {
    let (mut bench, clip) = Bench::new();
    opacity_track(&mut bench, clip);
    let before = bench.steps();
    bench.secondary_click("Add opacity keyframe");
    assert!(bench.has("Remove animation"), "the menu is open");
    assert_eq!(bench.steps(), before, "opening the menu edits nothing");
    bench.click("Ease in");
    let (_, ease_in) = EASINGS[2];
    // The key the playhead has passed shapes the stretch it is in.
    assert_eq!(opacity_keys(&bench, clip)[0].ease, ease_in);
    assert_eq!(opacity_keys(&bench, clip)[1].ease, Ease::Linear);
    assert_eq!(bench.steps(), before + 1);
    assert!(Ease::validate(&ease_in));

    // From the keyboard: Enter on the focused diamond is its click, and
    // Shift+Enter opens its menu.
    bench.focus("Add opacity keyframe");
    bench.enter(Modifiers::NONE);
    assert_eq!(opacity_keys(&bench, clip).len(), 3, "a key at the playhead");
    bench.enter(Modifiers::SHIFT);
    assert!(bench.has("Remove animation"), "Shift+Enter opens the menu");
    assert_eq!(opacity_keys(&bench, clip).len(), 3, "and is not a click");
    let steps = bench.steps();
    bench.click("Remove animation");
    assert!(bench.clip(clip).animation.is_empty());
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.editor.undo());
    assert_eq!(opacity_keys(&bench, clip).len(), 3);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn an_effects_value_is_keyframed_under_the_effect() {
    let (mut bench, clip, effect) = with_effect(EffectKind::Blur);
    bench.click("Animate radius");
    let track = &bench.clip(clip).animation[0];
    assert_eq!(
        (track.target, track.key),
        (Some(effect.id), ParamKey::Radius)
    );
    assert_eq!(track.keys, [key(1.0, 0.01)]);

    // Its field now writes the keyframe, and the effect keeps its value.
    let end = bench.drag("Radius", vec2(40.0, 0.0));
    let held = bench.layer(clip).effects[0].param(ParamKey::Radius);
    assert!(
        held.is_some_and(|radius| radius > 0.01),
        "previewed: {held:?}"
    );
    bench.release(end);
    assert_eq!(bench.last(), "Change keyframes");
    assert_eq!(bench.clip(clip).effects, vec![effect]);
    assert_eq!(Some(bench.clip(clip).animation[0].keys[0].value), held);
}

// ---- What a clip is offered ---------------------------------------------

#[test]
fn a_title_and_a_sound_are_offered_only_what_the_project_accepts() {
    let mut bench = Bench::empty();
    let sound = media("voice.wav", MediaKind::Audio);
    bench.edit(Command::AddAssets {
        assets: vec![sound.clone()],
    });
    let title = bench.add(
        TrackKind::Text,
        Clip::text("Hello", Time::ZERO, seconds(3.0)),
    );
    let voice = bench.add(TrackKind::Audio, {
        let mut clip = Clip::from_asset(&sound, Time::ZERO);
        clip.duration = seconds(3.0);
        clip
    });
    bench.playhead = seconds(1.0);

    // A title: no look, colour or speed, and nothing that masks or blends.
    bench.selection.only(title);
    bench.state.section = Section::Look;
    bench.frame(Vec::new());
    for offered in ["Video", "Motion", "Animate opacity", "Animate position"] {
        assert!(bench.has(offered), "a title has {offered}");
    }
    for refused in [
        "Look",
        "Color",
        "Speed",
        "Add effect",
        "Add mask",
        "Blend mode",
        "Noir",
        "Left half",
        "Animate left",
    ] {
        assert!(!bench.has(refused), "a title has no {refused}");
    }
    let before = bench.steps();
    bench.click("Animate opacity");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);

    // A sound: its speed and its volume, which can be keyframed.
    bench.selection.only(voice);
    bench.frame(Vec::new());
    for refused in [
        "Video",
        "Look",
        "Motion",
        "Animate opacity",
        "Add mask",
        "Blend mode",
    ] {
        assert!(!bench.has(refused), "a sound has no {refused}");
    }
    assert!(bench.find("Speed").is_some() && bench.find("Volume").is_some());
    bench.click("Animate volume");
    assert_eq!(bench.steps(), before + 2, "{:?}", bench.refused);
    let track = &bench.clip(voice).animation[0];
    assert_eq!((track.target, track.key), (None, ParamKey::Gain));
    assert!(bench.refused.is_empty());

    let project = bench.editor.project();
    for (id, expected) in [(title, 2), (voice, 0)] {
        let (track, clip) = project.clip(id).expect("the clip exists");
        assert_eq!(sections(track, clip).len(), expected);
    }
}

#[test]
fn a_locked_track_takes_no_edit() {
    let (mut bench, clip, effect) = with_effect(EffectKind::Glow);
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
    });
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(bench.find("Amount").is_some_and(|seen| seen.disabled));
    let end = bench.drag("Amount", vec2(60.0, 0.0));
    bench.release(end);
    bench.click("Remove Glow");
    bench.click("Animate amount");
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(clip).effects, vec![effect.clone()]);

    // And the editor refuses what a draft made before the lock would ask.
    let mut draft = ClipDraft::of(bench.clip(clip));
    draft.effects = Some(Vec::new());
    let command = draft.command(bench.editor.project()).expect("a change");
    assert!(bench.editor.dispatch(command).is_err());
    assert_eq!(bench.clip(clip).effects, vec![effect]);
    assert_eq!(bench.steps(), before);
}

#[test]
fn a_refused_draft_leaves_the_project_as_it_was() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let mut draft = ClipDraft::of(bench.clip(clip));
    let mut glow = Effect::new(EffectKind::Glow);
    glow.mix = 7.0;
    draft.effects = Some(vec![glow]);
    draft.transform.rotation = 20.0;
    bench.draft = Some(draft);
    bench.commit();
    assert_eq!(bench.refused.len(), 1, "the editor said why");
    assert_eq!(
        *bench.editor.project(),
        *stored,
        "none of the draft was kept"
    );
    assert_eq!(bench.steps(), before);
}

// ---- Tables, names and room ---------------------------------------------

#[test]
fn every_parameter_of_the_tables_has_a_name_and_a_field_that_holds_its_range() {
    let targets = (EffectKind::ALL.into_iter().map(AnimTarget::Effect))
        .chain([AnimTarget::Mask, AnimTarget::Clip]);
    for target in targets {
        for spec in look::target_spec(target) {
            let (label, field) = presented(target, spec);
            assert!(!label.is_empty(), "{target:?} {:?}", spec.key);
            assert_eq!((field.min, field.max), (spec.min, spec.max));
            // The finest step the field shows is finer than its range.
            let step = 10f32.powi(-(field.decimals as i32));
            assert!(
                (spec.max - spec.min) * field.scale >= step * 20.0,
                "{label}: {field:?}"
            );
            assert!(look::animatable(target, spec.key));
        }
    }
    // Every kind has a name and a family; the menu adds only what is drawn
    // and belongs to this section.
    for kind in EffectKind::ALL {
        assert!(!effect_name(kind).is_empty());
        let _ = family(kind);
    }
    let added: Vec<EffectKind> = (EffectKind::ALL.into_iter())
        .filter(|k| addable(*k))
        .collect();
    assert_eq!(added.len(), 11);
    for left_out in [
        EffectKind::Stabilize,
        EffectKind::Grade,
        EffectKind::Lut,
        EffectKind::DifferenceMatte,
    ] {
        assert!(!added.contains(&left_out));
    }
    for (_, ease) in EASINGS {
        assert!(ease.validate());
    }
}

#[test]
fn every_control_is_named_and_fits_a_narrow_panel() {
    for width in [300.0, 240.0] {
        for section in [Section::Video, Section::Look, Section::Speed] {
            let (mut bench, clip, effect) = with_effect(EffectKind::Shadow);
            let mask = Mask::new(MaskShape::Rectangle);
            bench.edit(Command::SetMasks {
                clip_id: clip,
                masks: vec![mask.clone()],
            });
            // The widest rows: animated ones, with their steps.
            for (target, key, value) in [
                (None, ParamKey::PositionX, 0.0),
                (None, ParamKey::PositionY, 0.0),
                (Some(effect.id), ParamKey::OffsetX, 0.01),
                (Some(mask.id), ParamKey::Feather, 0.0),
            ] {
                bench.edit(Command::SetKeyframes {
                    clip_id: clip,
                    target,
                    key,
                    keys: vec![key_of(0.0, value), key_of(2.0, value)],
                });
            }
            bench.state.section = section;
            bench.state.mask = Some(mask.id);
            bench.size = vec2(width, 2400.0);
            bench.frame(Vec::new());
            bench.frame(Vec::new());
            let panel = Rect::from_min_size(Pos2::ZERO, bench.size);
            let mut controls = 0;
            for seen in bench.seen.iter().filter(|seen| !Bench::is_text(seen)) {
                // The window and the panel's own containers are not controls.
                if seen.rect.width() >= width - 1.0 {
                    continue;
                }
                controls += 1;
                assert!(
                    seen.rect.left() >= panel.left() + 11.0
                        && seen.rect.right() <= panel.right() - 11.0,
                    "{:?} at {width}: \"{}\" is at {:?}",
                    section,
                    seen.name,
                    seen.rect
                );
                assert!(!seen.name.trim().is_empty());
            }
            assert!(controls > 3, "{section:?} at {width} has controls");
        }
    }
}

fn key_of(at: f64, value: f32) -> Keyframe {
    key(at, value)
}

#[test]
fn a_keyframe_is_written_where_one_is_and_added_where_none_is() {
    let keys = [key(0.0, 1.0), key(2.0, 3.0)];
    let mut eased = keys;
    eased[0].ease = Ease::Hold;
    let replaced = with_key(&eased, Time::ZERO, 9.0);
    assert_eq!(replaced.len(), 2);
    assert_eq!((replaced[0].value, replaced[0].ease), (9.0, Ease::Hold));
    let added = with_key(&keys, seconds(1.0), 5.0);
    assert_eq!(added, [key(0.0, 1.0), key(1.0, 5.0), key(2.0, 3.0)]);
    assert_eq!(with_key(&keys, seconds(4.0), 5.0)[2], key(4.0, 5.0));
    assert_eq!(with_key(&[], seconds(1.0), 5.0), [key(1.0, 5.0)]);
}

#[test]
fn an_inspector_nothing_is_done_in_asks_for_no_frame() {
    let (mut bench, _, _) = with_effect(EffectKind::Glow);
    for section in [Section::Video, Section::Look] {
        bench.state.section = section;
        // What a first frame eases in has settled within a second.
        for _ in 0..30 {
            bench.frame(Vec::new());
        }
        assert!(
            bench.wait > Duration::from_secs(3600),
            "{section:?} asked for a frame in {:?}",
            bench.wait
        );
        assert!(bench.draft.is_none() && bench.refused.is_empty());
    }
}

#[path = "color_tests.rs"]
mod color;

#[path = "speed_tests.rs"]
mod speed;

#[path = "motion_tests.rs"]
mod motion;

#[path = "steady_tests.rs"]
mod steady;

#[path = "typing_tests.rs"]
mod typing;

/// The times of the keys of one of the clip's own values.
fn key_times(bench: &Bench, clip: ClipId, key: ParamKey) -> Vec<Time> {
    (bench.clip(clip).animation.iter())
        .find(|track| track.target.is_none() && track.key == key)
        .map(|track| track.keys.iter().map(|key| key.at).collect())
        .unwrap_or_default()
}

#[test]
fn a_value_of_a_keyed_pair_scrubbed_alone_keys_both_and_the_diamond_says_so() {
    let (mut bench, clip) = Bench::new();
    bench.click("Animate position");
    bench.playhead = seconds(3.0);
    bench.frame(Vec::new());
    assert!(bench.has("Add position keyframe"), "off a keyframe");
    let before = bench.steps();
    let end = bench.drag("Position X", vec2(30.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "one step for both values");
    let times = [seconds(1.0), seconds(3.0)];
    assert_eq!(key_times(&bench, clip, ParamKey::PositionX), times);
    assert_eq!(key_times(&bench, clip, ParamKey::PositionY), times);
    let moved = bench.clip(clip).animation[0].keys[1].value;
    assert!(moved > 0.0, "{moved}");
    assert_eq!(bench.clip(clip).animation[1].keys[1].value, 0.0);
    bench.frame(Vec::new());
    assert!(
        bench.has("Remove position keyframe"),
        "on a keyframe of both"
    );

    // Values keyed apart, by the graph or an agent: the diamond is filled
    // on a keyframe of either, and takes that one away.
    bench.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::PositionY,
        keys: vec![key(1.0, 0.0)],
    });
    bench.frame(Vec::new());
    assert!(bench.has("Remove position keyframe"));
    bench.click("Remove position keyframe");
    assert_eq!(key_times(&bench, clip, ParamKey::PositionX), [seconds(1.0)]);
    assert_eq!(key_times(&bench, clip, ParamKey::PositionY), [seconds(1.0)]);
}

#[test]
fn a_keyed_row_is_walked_in_reading_order_and_keeps_its_name_whole() {
    let (mut bench, clip) = Bench::new();
    bench.click("Animate position");
    bench.playhead = seconds(2.0);
    bench.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::PositionX,
        keys: vec![key(1.0, 0.0), key(3.0, 10.0)],
    });
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let row = [
        "Previous position keyframe",
        "Add position keyframe",
        "Next position keyframe",
        "Position X",
        "Position Y",
    ];
    // Left to right on one line, and Tab takes them in that order.
    let rects: Vec<Rect> = row
        .iter()
        .map(|name| bench.find(name).unwrap().rect)
        .collect();
    assert!(
        rects
            .windows(2)
            .all(|pair| pair[0].right() <= pair[1].left() + 0.5),
        "{rects:?}"
    );
    bench.focus(row[0]);
    for name in &row[1..] {
        bench.key(Key::Tab, Modifiers::NONE);
        bench.frame(Vec::new());
        let focused = bench.ctx.memory(|memory| memory.focused());
        let node = bench.find(name).unwrap().node;
        assert_eq!(
            focused.map(|id| id.value()),
            Some(node.0),
            "Tab reaches {name}"
        );
    }
    // The name beside the steps is whole in a panel of the usual width.
    let label = (bench.seen.iter())
        .find(|seen| seen.name == "Position" && Bench::is_text(seen))
        .expect("the row is named");
    let ctx = bench.ctx.clone();
    let whole = ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(
                "Position".to_owned(),
                theme::regular(12.5),
                egui::Color32::WHITE,
            )
            .size()
            .x
    });
    assert!(
        label.rect.width() >= whole - 0.5,
        "{} of {whole}",
        label.rect.width()
    );
    assert!(label.rect.right() <= rects[0].left() + 0.5);
}

#[test]
fn a_diamond_pressed_with_the_playhead_off_the_clip_says_so_and_keys_nothing() {
    let (mut bench, clip) = Bench::new();
    bench.playhead = seconds(9.0);
    let before = bench.steps();
    bench.click("Animate opacity");
    assert_eq!(bench.steps(), before);
    assert!(bench.clip(clip).animation.is_empty());
    assert_eq!(
        bench.said,
        ["Move the playhead onto the clip to set a keyframe."]
    );
}

#[test]
fn the_diamonds_menu_closes_on_a_choice_and_hands_the_keyboard_back() {
    let (mut bench, clip) = Bench::new();
    opacity_track(&mut bench, clip);
    let diamond = |bench: &Bench| bench.find("Add opacity keyframe").expect("named").node;
    let focused = |bench: &Bench| {
        bench
            .ctx
            .memory(|memory| memory.focused().map(|id| id.value()))
    };
    bench.focus("Add opacity keyframe");
    bench.enter(Modifiers::SHIFT);
    assert!(bench.has("Remove animation"), "the menu is open");
    bench.frame(Vec::new());
    assert_eq!(
        focused(&bench),
        Some(bench.find("Linear").expect("listed").node.0),
        "and the keyboard is on its first choice"
    );
    bench.key(Key::Escape, Modifiers::NONE);
    bench.frame(Vec::new());
    assert!(!bench.has("Remove animation"), "an Escape closes it");
    assert_eq!(
        focused(&bench),
        Some(diamond(&bench).0),
        "and the diamond has the keyboard"
    );
    assert_eq!(bench.selection.single(), Some(clip));

    // A choice made with the keyboard closes the menu too.
    bench.enter(Modifiers::SHIFT);
    bench.focus("Ease in");
    let before = bench.steps();
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1);
    assert!(!bench.has("Remove animation"), "a choice closes it");
    assert_eq!(focused(&bench), Some(diamond(&bench).0));

    // On the last keyframe there is no stretch to ease: nothing is offered
    // that would be a step with no effect.
    let last = opacity_keys(&bench, clip).last().expect("keys").at;
    bench.playhead = last;
    bench.secondary_click("Remove opacity keyframe");
    assert!(
        (bench.seen.iter()).any(|seen| seen.name == "No keyframe follows to ease to"),
        "the menu says why"
    );
    assert!(bench.find("Ease in").expect("listed").disabled);
}
