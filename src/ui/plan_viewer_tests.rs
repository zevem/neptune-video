//! The viewer draws the plan: what `editor_core::resolve` makes of the
//! project at the playhead, and not the clips as stored.
//!
//! These paint [`show`] in a real egui pass with no device, so every layer
//! takes the mesh path: what is asked of the media service and where the
//! quads and handles go is all the plan's. What the compositor makes of a
//! plan is tested where it is, in `compose`.
use super::{COMPOSE_IDLE, ViewerInput, ViewerState, show, snapped, source_frame};
use crate::{
    compose::gpu_tests::render_state,
    textures::{Textures, Viewing},
    theme::{self, Accent, Appearance, Palette},
    titles::Titles,
    ui::{Action, ClipDraft, Selection},
};
use editor_core::{
    Asset, BlendMode, Clip, ClipId, Command, Ease, Editor, Element, ElementKind, FrameRate,
    Keyframe, MediaKind, ParamKey, Project, Time, TrackId, TrackKind, Transform, Transition,
    TransitionEdge, TransitionKind, resolve,
};
use editor_media::{
    Frame,
    text::{FontBook, TextService},
};
use eframe::egui::{self, Color32, Pos2, Rect, Shape, TextureId, Vec2, vec2};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const NAME: &str = "Effects not shown";

fn window() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))
}

fn video(path: &str) -> Asset {
    let mut asset = Asset::new(
        PathBuf::from(path),
        MediaKind::Video,
        Time::from_seconds(10.0),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = 1000;
    asset
}

struct Scene {
    editor: Editor,
    track: TrackId,
}

impl Scene {
    fn new(assets: &[&Asset]) -> Self {
        let editor = Editor::new(Project::new("Plan")).expect("a new project is valid");
        let track = (editor.project().tracks.iter())
            .find(|track| track.kind == TrackKind::Video)
            .map(|track| track.id)
            .expect("a new project has a video track");
        let mut scene = Self { editor, track };
        if !assets.is_empty() {
            scene.edit(Command::AddAssets {
                assets: assets.iter().map(|asset| (*asset).clone()).collect(),
            });
        }
        scene
    }

    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }

    /// A clip of `asset` at a quarter of the canvas, so it has edges on it.
    fn clip(&mut self, asset: &Asset, start: f64, duration: f64) -> ClipId {
        let mut clip = Clip::from_asset(asset, Time::from_seconds(start));
        clip.duration = Time::from_seconds(duration);
        clip.transform = Transform {
            scale: [0.25, 0.25],
            ..Transform::default()
        };
        let id = clip.id;
        self.edit(Command::AddClip {
            track_id: self.track,
            clip,
        });
        id
    }

    fn project(&self) -> Arc<Project> {
        self.editor.snapshot()
    }
}

/// What one pass painted.
struct Painted {
    /// Each textured quad: its texture, its centre and its first corner's
    /// tint.
    quads: Vec<(TextureId, Pos2, Color32)>,
    /// The centre of each closed outline: a selected layer's frame.
    frames: Vec<Pos2>,
    text: Vec<String>,
    /// The names accessibility is given.
    names: Vec<String>,
}

fn centre(points: impl Iterator<Item = Pos2>) -> Pos2 {
    let (sum, count) = points.fold((Vec2::ZERO, 0.0), |(sum, count), point| {
        (sum + point.to_vec2(), count + 1.0)
    });
    (sum / f32::max(count, 1.0)).to_pos2()
}

fn collect(shape: &Shape, painted: &mut Painted) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, painted)),
        Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => painted.quads.push((
            mesh.texture_id,
            centre(mesh.vertices.iter().map(|vertex| vertex.pos)),
            mesh.vertices[0].color,
        )),
        Shape::Path(path) if path.closed && path.points.len() == 4 => {
            painted.frames.push(centre(path.points.iter().copied()));
        }
        Shape::Text(text) => painted.text.push(text.galley.text().to_owned()),
        _ => {}
    }
}

struct Stage {
    ctx: egui::Context,
    textures: Textures,
    viewer: ViewerState,
    selection: Selection,
    draft: Option<ClipDraft>,
    palette: Palette,
}

impl Stage {
    fn new() -> Self {
        Self::with(ViewerState::default())
    }

    fn with(viewer: ViewerState) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        Self {
            ctx,
            textures: Textures::default(),
            viewer,
            selection: Selection::default(),
            draft: None,
            palette,
        }
    }

    fn paint(&mut self, project: &Project, playhead: Time) -> Painted {
        self.paint_with(project, playhead, Vec::new())
    }

    /// Paints a frame in which the pointer did what `events` say.
    fn paint_with(
        &mut self,
        project: &Project,
        playhead: Time,
        events: Vec<egui::Event>,
    ) -> Painted {
        self.textures.begin_frame();
        self.viewer.begin_frame();
        let ctx = self.ctx.clone();
        let mut actions: Vec<Action> = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(window()),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            show(
                ui,
                rect,
                &mut self.viewer,
                &mut self.selection,
                &mut self.draft,
                &mut self.textures,
                &ViewerInput {
                    p: &self.palette,
                    project,
                    query: None,
                    playhead,
                    playing: false,
                    speed: 1.0,
                    looping: false,
                    media: None,
                },
                &mut actions,
            );
        });
        // No renderer takes the textures; what they hold is not read here.
        output.textures_delta.clear();
        let mut painted = Painted {
            quads: Vec::new(),
            frames: Vec::new(),
            text: Vec::new(),
            names: Vec::new(),
        };
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut painted);
        }
        if let Some(update) = &output.platform_output.accesskit_update {
            painted.names = (update.nodes.iter())
                // A label's name is its value; a control's is its label.
                .filter_map(|(_, node)| node.label().or_else(|| node.value()))
                .map(str::to_owned)
                .collect();
        }
        painted
    }

    /// A decoded frame for `clip`, which was painted and so asked for one.
    fn deliver(&mut self, asset: &Asset, clip: ClipId) {
        let (consumer, size) = (self.textures.asked(Viewing::Clip(clip)))
            .expect("FIXTURE: the clip was painted and asked for a frame");
        let time = self.textures.admitted(consumer);
        let frame = Frame {
            width: 4,
            height: 4,
            rgba: [220, 40, 40, 255].repeat(16).into(),
        };
        let taken = self.textures.accept(
            &self.ctx,
            Some(asset),
            Some(consumer),
            asset.id,
            time,
            size,
            &frame,
        );
        assert!(taken, "FIXTURE: the clip's layer took the frame");
    }

    /// Paints until `clip` shows a picture, and returns that paint.
    fn shown(&mut self, project: &Project, playhead: Time, asset: &Asset, clip: ClipId) -> Painted {
        self.paint(project, playhead);
        self.deliver(asset, clip);
        self.paint(project, playhead)
    }
}

fn fade(seconds: f64) -> Transition {
    Transition {
        kind: TransitionKind::Fade,
        duration: Time::from_seconds(seconds),
        ease: Ease::Linear,
        params: Vec::new(),
    }
}

fn key(seconds: f64, value: f32) -> Keyframe {
    Keyframe {
        at: Time::from_seconds(seconds),
        value,
        ease: Ease::Linear,
    }
}

#[test]
fn a_transition_asks_for_the_clip_it_leads_out_of_past_its_end() {
    let (out, incoming) = (video("/media/out.mp4"), video("/media/in.mp4"));
    let mut scene = Scene::new(&[&out, &incoming]);
    let first = scene.clip(&out, 0.0, 2.0);
    let second = scene.clip(&incoming, 2.0, 2.0);
    scene.edit(Command::SetTransition {
        clip_id: second,
        edge: TransitionEdge::In,
        transition: Some(fade(1.0)),
    });
    let project = scene.project();
    let wanted = |playhead: f64| {
        let mut stage = Stage::new();
        stage.paint(&project, Time::from_seconds(playhead));
        [first, second].map(|clip| stage.textures.asked(Viewing::Clip(clip)).is_some())
    };
    assert_eq!(wanted(1.5), [true, false], "before the cut");
    assert_eq!(wanted(2.0), [true, true], "as the transition starts");
    assert_eq!(wanted(2.5), [true, true], "in the transition");
    assert_eq!(wanted(3.0), [false, true], "once it is over");

    // The clip that is over goes on through its source, as far as that has
    // frames.
    let (_, clip) = project.clip(first).expect("the clip is in the project");
    let rate = FrameRate::default();
    assert_eq!(
        source_frame(&project, &out, clip, Time::from_seconds(2.5)),
        rate.frame_time(rate.frame_at(Time::from_seconds(2.5)))
    );
    assert_eq!(
        source_frame(&project, &out, clip, Time::from_seconds(60.0)),
        rate.frame_time(rate.frame_at(Time::from_seconds(10.0)) - 1),
        "held on the source's last frame"
    );
}

#[test]
fn a_clip_held_for_a_transition_is_drawn_and_has_no_handles() {
    let (out, incoming) = (video("/media/out.mp4"), video("/media/in.mp4"));
    let mut scene = Scene::new(&[&out, &incoming]);
    let first = scene.clip(&out, 0.0, 2.0);
    let second = scene.clip(&incoming, 2.0, 2.0);
    scene.edit(Command::SetTransition {
        clip_id: second,
        edge: TransitionEdge::In,
        transition: Some(fade(1.0)),
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let playhead = Time::from_seconds(2.5);
    stage.paint(&project, playhead);
    stage.deliver(&out, first);
    stage.deliver(&incoming, second);
    stage.selection.only(first);
    let painted = stage.paint(&project, playhead);
    assert_eq!(painted.quads.len(), 2, "both clips of the transition");
    assert_ne!(
        painted.quads[0].0, painted.quads[1].0,
        "each its own picture"
    );
    assert!(
        painted.frames.is_empty(),
        "a clip that is over is not to be taken hold of"
    );
    assert!(
        painted.text.iter().any(|text| text == NAME),
        "a transition with no device to draw it is said to be left out"
    );

    stage.selection.only(second);
    let painted = stage.paint(&project, playhead);
    assert_eq!(painted.frames.len(), 1, "the clip on screen is");
}

#[test]
fn keyframes_move_the_quad_and_its_handles_and_fade_it() {
    let asset = video("/media/keyed.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 4.0);
    scene.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::PositionX,
        keys: vec![key(0.0, -640.0), key(2.0, 640.0)],
    });
    scene.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::Opacity,
        keys: vec![key(0.0, 1.0), key(2.0, 0.0)],
    });
    let project = scene.project();
    let mut stage = Stage::new();
    stage.selection.only(clip);
    let middle = window().center().x;
    // The canvas is 1920 wide; how wide it is drawn is the viewer's matter.
    let mut seen = Vec::new();
    for seconds in [0.0, 1.0, 2.0] {
        let painted = stage.shown(&project, Time::from_seconds(seconds), &asset, clip);
        let [(_, quad, tint)] = painted.quads[..] else {
            panic!("one quad is painted for the clip at {seconds} s");
        };
        let [frame] = painted.frames[..] else {
            panic!("the selected clip has one frame at {seconds} s");
        };
        assert!(
            (frame - quad).length() < 0.5,
            "the handles are where the picture is at {seconds} s: {frame:?} and {quad:?}"
        );
        assert!(
            !painted.text.iter().any(|text| text == NAME),
            "keyframes of the transform are drawn without a device"
        );
        seen.push((quad.x - middle, tint.a()));
    }
    let [(left, opaque), (centre, half), (right, clear)] = seen[..] else {
        unreachable!()
    };
    assert!(left < -100.0 && right > 100.0, "{left} then {right}");
    assert!((left + right).abs() < 0.5 && centre.abs() < 0.5, "{centre}");
    assert_eq!((opaque, half, clear), (255, 128, 0));
}

#[test]
fn a_value_being_dragged_takes_the_place_of_the_stored_one() {
    let asset = video("/media/dragged.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 4.0);
    let project = scene.project();
    let mut stage = Stage::new();
    let at_rest = stage.shown(&project, Time::ZERO, &asset, clip).quads[0].1;
    let (_, stored) = project.clip(clip).expect("the clip is in the project");
    let mut draft = ClipDraft::of(stored);
    draft.transform.position = [480.0, 0.0];
    stage.draft = Some(draft);
    let dragged = stage.paint(&project, Time::ZERO).quads[0].1;
    assert!(dragged.x > at_rest.x + 50.0, "{at_rest:?} then {dragged:?}");
    assert!((dragged.y - at_rest.y).abs() < 0.5);
}

#[test]
fn without_a_device_a_blended_layer_is_a_quad_and_the_viewer_says_so() {
    let asset = video("/media/blended.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 4.0);
    let mut stage = Stage::new();
    let plain = scene.project();
    let painted = stage.shown(&plain, Time::ZERO, &asset, clip);
    assert_eq!(painted.quads.len(), 1);
    assert!(
        !painted.text.iter().any(|text| text == NAME)
            && !painted.names.iter().any(|name| name == NAME),
        "a plain plan is drawn whole, and nothing is said"
    );
    assert!(
        stage.textures.input(Viewing::Clip(clip)).is_none(),
        "no decoded frame is kept for a compositor that is not there"
    );

    scene.edit(Command::SetBlendMode {
        clip_id: clip,
        mode: BlendMode::Multiply,
    });
    let blended = scene.project();
    for _ in 0..3 {
        let painted = stage.paint(&blended, Time::ZERO);
        assert_eq!(
            painted.quads.len(),
            1,
            "the layer keeps the mesh path, with its transform and opacity"
        );
        assert!(
            matches!(painted.quads[0].0, TextureId::Managed(_)),
            "drawn from its own picture, not from a canvas"
        );
        assert!(
            painted.text.iter().any(|text| text == NAME),
            "said in words"
        );
        assert!(
            painted.names.iter().any(|name| name == NAME),
            "and to accessibility, by name: {:?}",
            painted.names
        );
    }
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));

    // Said for as long as it is so, and no longer.
    scene.edit(Command::SetBlendMode {
        clip_id: clip,
        mode: BlendMode::Normal,
    });
    let painted = stage.paint(&scene.project(), Time::ZERO);
    assert!(!painted.text.iter().any(|text| text == NAME));
}

/// One clip of `asset` over four seconds, cut at two: the halves of a cut,
/// which carry on from one another. Returns the two.
fn cut(scene: &mut Scene, asset: &Asset) -> (ClipId, ClipId) {
    let first = scene.clip(asset, 0.0, 4.0);
    let at = Time::from_seconds(2.0);
    scene.edit(Command::SplitClip { clip_id: first, at });
    let project = scene.project();
    let second = (project.tracks.iter())
        .flat_map(|track| &track.clips)
        .find(|clip| clip.start == at)
        .map(|clip| clip.id)
        .expect("FIXTURE: the split made a clip starting at the cut");
    (first, second)
}

fn transition(kind: TransitionKind, seconds: f64) -> Transition {
    Transition {
        kind,
        ..fade(seconds)
    }
}

impl Stage {
    /// The consumer `clip`'s frames are asked under, when it has a layer.
    fn consumer(&self, clip: ClipId) -> Option<editor_media::PreviewConsumer> {
        (self.textures.asked(Viewing::Clip(clip))).map(|(consumer, _)| consumer)
    }
}

#[test]
fn a_cut_without_a_transition_hands_its_layer_over_as_before() {
    let asset = video("/media/cut.mp4");
    let mut scene = Scene::new(&[&asset]);
    let (first, second) = cut(&mut scene, &asset);
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.shown(&project, Time::from_seconds(1.9), &asset, first);
    let outgoing = stage.consumer(first).expect("the first half is drawn");

    let after = stage.paint(&project, Time::from_seconds(2.0));
    assert_eq!(
        stage.consumer(second),
        Some(outgoing),
        "the incoming half decodes on under the consumer the outgoing one had"
    );
    assert_eq!(stage.consumer(first), None, "which has none any more");
    assert_eq!(
        (after.quads.len(), after.quads[0].0),
        (1, before.quads[0].0),
        "and is painted at once with the picture that one held"
    );
}

#[test]
fn the_clips_of_a_transition_each_keep_a_consumer_of_their_own() {
    let asset = video("/media/cut.mp4");
    let mut scene = Scene::new(&[&asset]);
    let (first, second) = cut(&mut scene, &asset);
    scene.edit(Command::SetTransition {
        clip_id: second,
        edge: TransitionEdge::In,
        transition: Some(transition(TransitionKind::Dissolve, 1.0)),
    });
    let project = scene.project();
    let (_, outgoing_clip) = project.clip(first).expect("the clip is in the project");
    let mut stage = Stage::new();
    let before = stage.shown(&project, Time::from_seconds(1.9), &asset, first);
    let outgoing = stage.consumer(first).expect("the first half is drawn");
    let picture = before.quads[0].0;

    // Stepped across the cut: the two are on screen together, so the half
    // that is over keeps its layer and the incoming one starts its own.
    let rate = FrameRate::default();
    let mut incoming = None;
    for seconds in [2.0, 2.2, 2.5, 2.9] {
        let playhead = Time::from_seconds(seconds);
        let painted = stage.paint(&project, playhead);
        assert_eq!(
            stage.consumer(first),
            Some(outgoing),
            "the outgoing half keeps its consumer at {seconds} s"
        );
        let consumer = stage.consumer(second);
        assert!(
            consumer.is_some() && consumer != Some(outgoing),
            "the incoming half has one of its own at {seconds} s"
        );
        assert_eq!(
            *incoming.get_or_insert(consumer),
            consumer,
            "and keeps it through the transition"
        );
        assert_eq!(
            (painted.quads.len(), painted.quads[0].0),
            (1, picture),
            "the outgoing half shows its own picture, and the incoming one \
             nothing until a frame of its own arrives"
        );
        // The half that is over reads on through the source past its end:
        // the very frames the incoming half shows.
        assert_eq!(
            source_frame(&project, &asset, outgoing_clip, playhead),
            rate.frame_time(rate.frame_at(playhead)),
        );
    }
    stage.deliver(&asset, second);
    let painted = stage.paint(&project, Time::from_seconds(2.9));
    assert_eq!(painted.quads.len(), 2);
    assert_eq!(painted.quads[0].0, picture, "the outgoing half under");
    assert_ne!(painted.quads[1].0, picture, "the incoming half over it");

    // Once it is over the outgoing half is drawn no more, and the incoming
    // one goes on as it was.
    let painted = stage.paint(&project, Time::from_seconds(3.0));
    assert_eq!(painted.quads.len(), 1);
    assert_eq!(stage.consumer(second), incoming.flatten());
    assert!(
        !painted.text.iter().any(|text| text == NAME),
        "past the transition the plan is plain again"
    );
}

#[test]
fn a_seek_into_a_transition_takes_no_layer_from_the_clip_it_is_with() {
    let asset = video("/media/cut.mp4");
    let mut scene = Scene::new(&[&asset]);
    let (first, second) = cut(&mut scene, &asset);
    scene.edit(Command::SetTransition {
        clip_id: second,
        edge: TransitionEdge::In,
        transition: Some(transition(TransitionKind::WipeLeft, 1.0)),
    });
    let project = scene.project();

    // From the body of the incoming clip back into the transition: the
    // outgoing one starts a layer, and takes none.
    let mut stage = Stage::new();
    stage.shown(&project, Time::from_seconds(3.5), &asset, second);
    let incoming = stage.consumer(second).expect("the second half is drawn");
    let painted = stage.paint(&project, Time::from_seconds(2.5));
    assert_eq!(stage.consumer(second), Some(incoming));
    let outgoing = stage.consumer(first);
    assert!(outgoing.is_some() && outgoing != Some(incoming));
    assert_eq!(
        painted.quads.len(),
        1,
        "the outgoing clip is not painted with the incoming one's picture"
    );

    // From the body of the outgoing clip to past the transition, the two
    // are never on screen together: that is a cut, handed over as ever.
    let mut stage = Stage::new();
    let before = stage.shown(&project, Time::from_seconds(1.0), &asset, first);
    let outgoing = stage.consumer(first).expect("the first half is drawn");
    let after = stage.paint(&project, Time::from_seconds(3.5));
    assert_eq!(stage.consumer(second), Some(outgoing));
    assert_eq!(after.quads[0].0, before.quads[0].0);
}

#[test]
fn without_a_device_every_kind_of_transition_is_a_cross_fade() {
    let (out, incoming) = (video("/media/out.mp4"), video("/media/in.mp4"));
    let playhead = Time::from_seconds(2.25);
    for kind in [
        TransitionKind::Fade,
        TransitionKind::Dissolve,
        TransitionKind::SlideLeft,
        TransitionKind::ZoomIn,
        TransitionKind::WipeUp,
        TransitionKind::Glitch,
        TransitionKind::Blur,
    ] {
        let mut scene = Scene::new(&[&out, &incoming]);
        let first = scene.clip(&out, 0.0, 2.0);
        let second = scene.clip(&incoming, 2.0, 2.0);
        scene.edit(Command::SetTransition {
            clip_id: second,
            edge: TransitionEdge::In,
            transition: Some(transition(kind, 1.0)),
        });
        let project = scene.project();
        let mut stage = Stage::new();
        stage.paint(&project, playhead);
        // Until the incoming clip has a picture the outgoing one fades by
        // itself.
        stage.deliver(&out, first);
        let painted = stage.paint(&project, playhead);
        let [(_, _, alone)] = painted.quads[..] else {
            panic!("{kind:?}: one quad is painted before the second picture");
        };
        assert_eq!(alone.a(), 191, "{kind:?}: three quarters of it is left");
        // Both on one box: the outgoing clip is left whole under the
        // incoming one, which hides a quarter of it. The two sum to a
        // cross-fade, with nothing of the canvas between them.
        stage.deliver(&incoming, second);
        let painted = stage.paint(&project, playhead);
        let [(_, under, below), (_, over, above)] = painted.quads[..] else {
            panic!("{kind:?}: both clips are painted");
        };
        assert!((under - over).length() < 0.5, "{kind:?}: on one box");
        assert_eq!((below.a(), above.a()), (255, 64), "{kind:?}");
        assert!(
            painted.text.iter().any(|text| text == NAME),
            "{kind:?}: and the viewer says the transition itself is left out"
        );
    }
}

#[test]
fn clips_of_a_transition_on_boxes_of_their_own_each_fade() {
    let (out, incoming) = (video("/media/out.mp4"), video("/media/in.mp4"));
    let mut scene = Scene::new(&[&out, &incoming]);
    let first = scene.clip(&out, 0.0, 2.0);
    let second = scene.clip(&incoming, 2.0, 2.0);
    scene.edit(Command::SetTransform {
        clip_id: second,
        transform: Transform {
            position: [480.0, 0.0],
            scale: [0.25, 0.25],
            ..Transform::default()
        },
    });
    scene.edit(Command::SetTransition {
        clip_id: second,
        edge: TransitionEdge::In,
        transition: Some(transition(TransitionKind::SlideRight, 1.0)),
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let playhead = Time::from_seconds(2.25);
    stage.paint(&project, playhead);
    stage.deliver(&out, first);
    stage.deliver(&incoming, second);
    let painted = stage.paint(&project, playhead);
    let [(_, _, below), (_, _, above)] = painted.quads[..] else {
        panic!("both clips are painted");
    };
    assert_eq!((below.a(), above.a()), (191, 64));
}

#[test]
fn a_transition_in_with_no_clip_before_it_fades_in_from_nothing() {
    let asset = video("/media/alone.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 1.0, 3.0);
    scene.edit(Command::SetTransition {
        clip_id: clip,
        edge: TransitionEdge::In,
        transition: Some(transition(TransitionKind::ZoomIn, 1.0)),
    });
    let project = scene.project();
    let mut stage = Stage::new();
    stage.selection.only(clip);
    let mut seen = Vec::new();
    for seconds in [1.0, 1.5, 2.0] {
        let painted = stage.shown(&project, Time::from_seconds(seconds), &asset, clip);
        let [(_, _, tint)] = painted.quads[..] else {
            panic!("one quad, and no clip held under it, at {seconds} s");
        };
        assert_eq!(
            painted.frames.len(),
            1,
            "the clip is on screen and can be taken hold of"
        );
        seen.push((
            tint.a(),
            painted.text.iter().any(|text| text == NAME),
            stage.textures.layers_kept(),
        ));
    }
    assert_eq!(
        seen,
        [(0, true, 1), (128, true, 1), (255, false, 1)],
        "clear, then half in, then whole and plain; one layer throughout"
    );
}

#[test]
fn a_transition_out_fades_to_nothing_and_only_when_nothing_follows() {
    let (asset, next) = (video("/media/last.mp4"), video("/media/next.mp4"));
    let mut scene = Scene::new(&[&asset, &next]);
    let clip = scene.clip(&asset, 0.0, 2.0);
    scene.edit(Command::SetTransition {
        clip_id: clip,
        edge: TransitionEdge::Out,
        transition: Some(transition(TransitionKind::WipeDown, 1.0)),
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let mut seen = Vec::new();
    for seconds in [0.5, 1.0, 1.5] {
        let painted = stage.shown(&project, Time::from_seconds(seconds), &asset, clip);
        let [(_, _, tint)] = painted.quads[..] else {
            panic!("one quad at {seconds} s");
        };
        seen.push((tint.a(), painted.text.iter().any(|text| text == NAME)));
    }
    assert_eq!(seen, [(255, false), (255, true), (128, true)]);
    assert_eq!(stage.textures.layers_kept(), 1);
    let painted = stage.paint(&project, Time::from_seconds(2.0));
    assert!(painted.quads.is_empty(), "past its end it is not held on");

    // A clip that abuts it takes the way out away: that edge is a cut.
    let following = scene.clip(&next, 2.0, 2.0);
    let project = scene.project();
    let mut stage = Stage::new();
    let painted = stage.shown(&project, Time::from_seconds(1.5), &asset, clip);
    assert_eq!(painted.quads[0].2.a(), 255);
    assert!(!painted.text.iter().any(|text| text == NAME));
    stage.paint(&project, Time::from_seconds(2.0));
    assert!(stage.consumer(following).is_some());
}

#[test]
fn a_layer_still_waiting_for_its_picture_is_not_said_to_be_drawn_without_effects() {
    let Some(state) = render_state() else { return };
    let asset = video("/media/blended.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 4.0);
    scene.edit(Command::SetBlendMode {
        clip_id: clip,
        mode: BlendMode::Multiply,
    });
    let project = scene.project();
    let mut stage = Stage::with(ViewerState::with_device(Some((*state).clone())));
    let said = |painted: &Painted| {
        painted.text.iter().any(|text| text == NAME)
            || painted.names.iter().any(|name| name == NAME)
    };

    // Nothing is decoded yet: nothing is drawn, so nothing is drawn wrongly.
    for _ in 0..3 {
        let painted = stage.paint(&project, Time::ZERO);
        assert!(painted.quads.is_empty());
        assert!(
            !said(&painted),
            "a compositor that refused nothing is not said to have"
        );
    }
    // The picture comes and is composited: still nothing to say.
    stage.deliver(&asset, clip);
    let painted = stage.paint(&project, Time::ZERO);
    assert_eq!(stage.viewer.composer.counters().composed, 1);
    assert!(
        painted.quads.is_empty(),
        "the canvas, and no quad of its own"
    );
    assert!(!said(&painted));
}

#[test]
fn a_keyed_zoom_asks_for_its_pictures_at_one_size() {
    let asset = video("/media/zoomed.mp4");
    let mut scene = Scene::new(&[&asset]);
    let small = scene.clip(&asset, 0.0, 4.0);
    let project = scene.project();
    let mut stage = Stage::new();
    stage.paint(&project, Time::ZERO);
    let (_, at_rest) = (stage.textures.asked(Viewing::Clip(small))).expect("the clip is drawn");

    for side in [ParamKey::ScaleX, ParamKey::ScaleY] {
        scene.edit(Command::SetKeyframes {
            clip_id: small,
            target: None,
            key: side,
            keys: vec![key(0.0, 0.1), key(4.0, 1.0)],
        });
    }
    let project = scene.project();
    let mut stage = Stage::new();
    let mut boxes = Vec::new();
    for seconds in [0.0, 1.0, 2.0, 3.0, 3.9] {
        stage.paint(&project, Time::from_seconds(seconds));
        boxes.push(
            (stage.textures.asked(Viewing::Clip(small)))
                .expect("the clip is drawn")
                .1,
        );
    }
    assert!(
        boxes.iter().all(|size| *size == boxes[0]),
        "a zoom asks at one box from end to end, and starts no decoder over: {boxes:?}"
    );
    assert!(
        boxes[0][0] > at_rest[0],
        "the box covers the zoom at its largest: {:?} over {at_rest:?}",
        boxes[0]
    );
}

/// The pointer pressed on `at` and then dragged by `by`, a frame for each.
fn drag(stage: &mut Stage, project: &Project, at: Pos2, by: Vec2) {
    let pressed = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    stage.paint_with(project, Time::ZERO, vec![egui::Event::PointerMoved(at)]);
    stage.paint_with(project, Time::ZERO, vec![pressed(true)]);
    stage.paint_with(
        project,
        Time::ZERO,
        vec![egui::Event::PointerMoved(at + by * 0.5)],
    );
    stage.paint_with(
        project,
        Time::ZERO,
        vec![egui::Event::PointerMoved(at + by)],
    );
}

#[test]
fn a_drag_of_a_value_its_keyframes_drive_writes_a_keyframe_at_the_playhead() {
    let asset = video("/media/keyed.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 4.0);

    // Nothing keyed: a drag moves the layer, as a value held to be committed.
    let project = scene.project();
    let mut stage = Stage::new();
    let at = stage.shown(&project, Time::ZERO, &asset, clip).quads[0].1;
    drag(&mut stage, &project, at, vec2(60.0, 40.0));
    let draft = stage.draft.as_ref().expect("the drag holds a value");
    assert!(draft.transform.position[0] > 50.0, "{:?}", draft.transform);
    assert!(draft.keys.is_empty(), "and no keyframe");

    // Its place across keyed: the keyframes say where it is, so the stored
    // value is not what a drag changes. It holds a keyframe at the playhead
    // for the value that is keyed, and the stored one for the value that is
    // not.
    scene.edit(Command::SetKeyframes {
        clip_id: clip,
        target: None,
        key: ParamKey::PositionX,
        keys: vec![key(0.0, 0.0), key(2.0, 640.0)],
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let playhead = Time::from_seconds(1.0);
    let at = stage.shown(&project, playhead, &asset, clip).quads[0].1;
    let pressed = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    stage.paint_with(&project, playhead, vec![egui::Event::PointerMoved(at)]);
    stage.paint_with(&project, playhead, vec![pressed(true)]);
    for step in [0.5, 1.0] {
        let to = at + vec2(60.0, 40.0) * step;
        stage.paint_with(&project, playhead, vec![egui::Event::PointerMoved(to)]);
    }
    assert_eq!(stage.selection.single(), Some(clip));
    let draft = stage.draft.clone().expect("the drag holds a value");
    let stored = &project.clip(clip).expect("the clip").1.transform;
    assert_eq!(
        draft.transform.position[0], stored.position[0],
        "the stored value keyframes hide is left as it is"
    );
    assert!(draft.transform.position[1] > 0.0, "{:?}", draft.transform);
    let held = draft.key(None, ParamKey::PositionX).expect("a keyframe");
    assert_eq!(held.at, playhead);
    // From where it was shown, half way between its keys, and no further
    // than the pointer went.
    assert!(held.value > 320.0, "{held:?}");

    // What the canvas draws is what is held, and letting go writes it as
    // the one keyframe, which an undo takes back.
    let shown = resolve(&project, playhead, &[draft.overriding(&project)]);
    assert_eq!(shown.layers[0].transform.position[0], held.value);
    let command = draft.command(&project).expect("something to commit");
    scene.edit(command);
    let after = scene.project();
    let track = &after.clip(clip).expect("the clip").1.animation[0];
    assert_eq!(
        track.keys,
        vec![key(0.0, 0.0), key(1.0, held.value), key(2.0, 640.0)]
    );
}

#[test]
fn the_centre_holds_a_layer_where_it_is_shown_not_where_it_is_stored() {
    // Stored where it is shown: within reach of the centre it is put on it.
    assert_eq!(snapped(4.0, 0.0, 6.0), Some(0.0));
    assert_eq!(snapped(-6.0, 0.0, 6.0), Some(0.0));
    assert_eq!(snapped(7.0, 0.0, 6.0), None);
    // Shown 200 to the right of where it is stored, as a layer that follows
    // a tracker is: stored near nought it is nowhere near the centre.
    assert_eq!(snapped(3.0, 200.0, 6.0), None);
    // And it is on the centre when stored 200 to the left, which is the
    // value that holds it there.
    assert_eq!(snapped(-197.0, 200.0, 6.0), Some(-200.0));
    assert_eq!(snapped(-204.0, 200.0, 6.0), Some(-200.0));
    assert_eq!(snapped(-207.0, 200.0, 6.0), None);
}

#[test]
fn what_is_held_for_the_compositor_is_given_up_on_the_clock_once_nothing_asks_for_it() {
    let Some(state) = render_state() else { return };
    let asset = video("/media/blended.mp4");
    let mut scene = Scene::new(&[&asset]);
    let clip = scene.clip(&asset, 0.0, 2.0);
    scene.edit(Command::SetBlendMode {
        clip_id: clip,
        mode: BlendMode::Multiply,
    });
    let project = scene.project();
    let mut stage = Stage::with(ViewerState::with_device(Some((*state).clone())));
    let start = std::time::Instant::now();
    let second = std::time::Duration::from_secs(1);

    stage.shown(&project, Time::ZERO, &asset, clip);
    assert!(stage.viewer.composer.read(0).is_some(), "a canvas is drawn");
    assert!(stage.textures.input(Viewing::Clip(clip)).is_some());
    assert_eq!(
        stage.viewer.rest(&mut stage.textures, start),
        None,
        "nothing is idle while the viewer composites, and no wake is asked for"
    );
    assert_eq!(
        stage.viewer.rest(&mut stage.textures, start + 10 * second),
        None,
        "however long a paused frame is left on screen"
    );
    assert!(stage.viewer.composer.read(0).is_some());

    // The playhead rests past the clip: a few frames are painted, and then
    // none. The canvases are asked to be looked at again, once, when due.
    let rested = start + 20 * second;
    stage.paint(&project, Time::from_seconds(3.0));
    assert_eq!(
        stage.viewer.rest(&mut stage.textures, rested),
        Some(COMPOSE_IDLE)
    );
    assert_eq!(
        stage.viewer.rest(&mut stage.textures, rested + second),
        Some(COMPOSE_IDLE - second),
        "a frame painted meanwhile asks for no more than what is left"
    );
    assert!(stage.viewer.composer.read(0).is_some(), "kept until then");
    // The wake: no frame was painted in between, so none was counted.
    assert_eq!(
        stage
            .viewer
            .rest(&mut stage.textures, rested + COMPOSE_IDLE),
        None
    );
    assert!(stage.viewer.composer.read(0).is_none(), "the canvases");
    assert!(!stage.viewer.composer.holds(), "and their pictures");
    assert!(!stage.textures.keeps_frames(), "and the decoded frames");
    assert_eq!(
        stage
            .viewer
            .rest(&mut stage.textures, rested + 2 * COMPOSE_IDLE),
        None,
        "and nothing wakes the editor after that"
    );

    // Looked at again, it is composited again.
    stage.shown(&project, Time::ZERO, &asset, clip);
    assert!(stage.viewer.composer.read(0).is_some());
}

// ---- Titles and graphics --------------------------------------------------

/// A stage whose titles are drawn with the bundled faces alone, so what is
/// measured is the same on every host.
fn titled() -> Stage {
    let book = || FontBook::new(&crate::titles::bundled_fonts()).with_system_roots(Vec::new());
    let service = TextService::with_book(book, Arc::new(|| {})).expect("a title lane starts");
    Stage::with(ViewerState::default().with_titles(Titles::new(Some(service))))
}

/// Paints until the lane has drawn what `clip` last asked for.
fn titled_paint(stage: &mut Stage, project: &Project, playhead: Time, clip: ClipId) -> Painted {
    stage.paint(project, playhead);
    let deadline = Instant::now() + Duration::from_secs(30);
    while stage.viewer.titles.behind(clip) {
        assert!(Instant::now() < deadline, "the title lane drew nothing");
        std::thread::sleep(Duration::from_millis(2));
        stage.viewer.titles.poll(&stage.ctx);
    }
    stage.paint(project, playhead)
}

/// The text track a new project has.
fn text_track(project: &Project) -> TrackId {
    (project.tracks.iter())
        .find(|track| track.kind == TrackKind::Text)
        .map(|track| track.id)
        .expect("a new project has a text track")
}

/// The canvas centre and its scale on screen in [`window`].
fn canvas_view(project: &Project) -> (Pos2, f32) {
    let stage = Rect::from_min_max(Pos2::ZERO, egui::pos2(1000.0, 700.0 - super::TRANSPORT));
    let room = stage.shrink(18.0);
    let size = vec2(
        project.settings.width as f32,
        project.settings.height as f32,
    );
    (
        room.center(),
        (room.width() / size.x).min(room.height() / size.y),
    )
}

#[test]
fn a_title_is_drawn_from_its_picture_with_handles_on_its_text_box() {
    let mut scene = Scene::new(&[]);
    let mut clip = Clip::text("Hello", Time::ZERO, Time::from_seconds(4.0));
    clip.transform.position = [200.0, -100.0];
    let id = clip.id;
    let track = text_track(scene.editor.project());
    scene.edit(Command::AddClip {
        track_id: track,
        clip,
    });
    let project = scene.project();
    let (centre, view) = canvas_view(&project);
    let anchor = centre + vec2(200.0, -100.0) * view;

    let mut stage = titled();
    stage.selection.only(id);
    let first = stage.paint(&project, Time::from_seconds(1.0));
    assert!(
        first.quads.is_empty() && first.frames.is_empty(),
        "nothing is drawn or held before the picture comes"
    );
    assert!(
        first.text.iter().all(|text| text != "Hello"),
        "and the interface's own text does not stand in for it"
    );

    let painted = titled_paint(&mut stage, &project, Time::from_seconds(1.0), id);
    let (picture, texture) = stage.viewer.titles.picture(id).expect("the picture");
    assert_eq!(
        picture.scale,
        editor_media::text::quantize_scale(view),
        "drawn as many pixels as it covers on screen"
    );
    assert_eq!(painted.quads.len(), 1);
    let (quad, middle, tint) = painted.quads[0];
    assert_eq!(quad, texture);
    assert_eq!(tint, Color32::WHITE, "opaque");
    let [left, top] = picture.origin;
    let [width, height] = picture.size();
    let expected = anchor + vec2(left + width / 2.0, top + height / 2.0) * view;
    assert!(
        (middle - expected).length() < 0.5,
        "{middle:?} {expected:?}"
    );
    assert_eq!(painted.frames.len(), 1, "the selected title's frame");
    assert!(
        (painted.frames[0] - anchor).length() < 0.5,
        "about its text box, which is centred on its place: {:?} {anchor:?}",
        painted.frames[0]
    );
    let [box_left, box_top, box_right, box_bottom] = picture.bounds;
    assert!(box_right - box_left > 64.0 && box_bottom - box_top > 32.0);
    assert!((box_left + box_right).abs() < 0.01 && (box_top + box_bottom).abs() < 0.01);

    // A paint that changes nothing asks for nothing.
    stage.paint(&project, Time::from_seconds(1.0));
    assert!(!stage.viewer.titles.behind(id));
}

#[test]
fn a_title_being_typed_asks_for_its_own_picture() {
    let mut scene = Scene::new(&[]);
    let clip = Clip::text("Hi", Time::ZERO, Time::from_seconds(4.0));
    let id = clip.id;
    let track = text_track(scene.editor.project());
    scene.edit(Command::AddClip {
        track_id: track,
        clip: clip.clone(),
    });
    let project = scene.project();
    let mut stage = titled();
    titled_paint(&mut stage, &project, Time::ZERO, id);
    let stored = stage
        .viewer
        .titles
        .picture(id)
        .expect("the picture")
        .0
        .bounds;

    let mut draft = ClipDraft::of(&clip);
    if let Some(editor_core::ClipContent::Text { text, .. }) = &mut draft.content {
        *text = "Hi there, everyone".to_owned();
    }
    stage.draft = Some(draft);
    titled_paint(&mut stage, &project, Time::ZERO, id);
    let typed = stage
        .viewer
        .titles
        .picture(id)
        .expect("the picture")
        .0
        .bounds;
    assert!(
        typed[2] - typed[0] > 2.0 * (stored[2] - stored[0]),
        "{stored:?} {typed:?}"
    );

    // Let go without a commit, the stored title is drawn again.
    stage.draft = None;
    titled_paint(&mut stage, &project, Time::ZERO, id);
    assert_eq!(
        stage
            .viewer
            .titles
            .picture(id)
            .expect("the picture")
            .0
            .bounds,
        stored
    );
}

#[test]
fn a_graphic_is_drawn_from_its_picture_and_held_by_its_design_box() {
    let mut scene = Scene::new(&[]);
    let bar = Element::new(
        "Bar",
        ElementKind::Rect {
            color: [20, 120, 220, 255],
            radius: 0.0,
            stroke: None,
        },
        [0.0, 0.0, 400.0, 120.0],
    );
    let mut clip = Clip::graphic(
        "Lower third",
        [400.0, 120.0],
        vec![bar],
        Time::ZERO,
        Time::from_seconds(4.0),
    );
    clip.transform.position = [-300.0, 250.0];
    clip.transform.scale = [2.0, 2.0];
    let id = clip.id;
    let track = text_track(scene.editor.project());
    scene.edit(Command::AddClip {
        track_id: track,
        clip,
    });
    let project = scene.project();
    let (centre, view) = canvas_view(&project);
    let anchor = centre + vec2(-300.0, 250.0) * view;

    let mut stage = titled();
    stage.selection.only(id);
    let painted = titled_paint(&mut stage, &project, Time::from_seconds(1.0), id);
    let (picture, _) = stage.viewer.titles.picture(id).expect("the picture");
    assert_eq!(picture.bounds, [-200.0, -60.0, 200.0, 60.0]);
    assert_eq!(
        picture.scale,
        editor_media::text::quantize_scale(2.0 * view),
        "at its scale on screen"
    );
    assert_eq!(painted.quads.len(), 1);
    assert!((painted.quads[0].1 - anchor).length() < 1.0);
    assert_eq!(painted.frames.len(), 1);
    assert!((painted.frames[0] - anchor).length() < 0.5);
}

#[test]
fn a_title_turns_and_mirrors_about_its_place() {
    let anchor = egui::pos2(100.0, 100.0);
    let mut transform = Transform {
        scale: [2.0, 1.0],
        ..Transform::default()
    };
    let at = |transform: &Transform| super::title_point(anchor, 0.5, transform, 10.0, 4.0);
    assert_eq!(at(&transform), egui::pos2(110.0, 102.0));
    transform.flip_x = true;
    assert_eq!(at(&transform), egui::pos2(90.0, 102.0));
    transform.flip_x = false;
    transform.rotation = 90.0;
    let turned = at(&transform);
    assert!(
        (turned - egui::pos2(98.0, 110.0)).length() < 1e-3,
        "{turned:?}"
    );
    let place = super::title_place(
        ClipId::new_v4(),
        anchor,
        0.5,
        &transform,
        [-10.0, -4.0, 10.0, 4.0],
        false,
    );
    assert_eq!(place.center, anchor);
    // Twenty wide at twice and half a view; eight high at once is four,
    // and a frame is never thinner than that.
    assert_eq!(place.half, vec2(10.0, 4.0));
}
