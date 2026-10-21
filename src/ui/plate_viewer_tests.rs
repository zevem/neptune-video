//! A clean plate from its file to the canvas, and a key colour from the
//! decoded frame to the edit: stills imported and decoded by the real media
//! worker, and the viewer painted as the application paints it.
//!
//! As in `compose::gpu_tests`, a test that needs an adapter and has none
//! prints `SKIP:` and passes.
use super::{Placed, ViewerInput, ViewerState, sampled, show};
use crate::{
    compose::gpu_tests::render_state,
    textures::{Textures, Viewing},
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, ColourPick, Selection},
};
use editor_core::{
    Asset, Clip, ClipId, Command, Editor, Effect, EffectKind, ParamKey, ParamValue, Project, Time,
    TrackKind, Transform,
};
use editor_media::{Frame, MediaEvent, MediaService};
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const GREEN: [u8; 4] = [40, 200, 60, 255];
const RED: [u8; 4] = [220, 40, 40, 255];
/// The window the viewer is painted in, and where its canvas of 16 by 9
/// comes to lie in it: the quarters of its middle row.
const WINDOW: [f32; 2] = [1000.0, 700.0];
// The middle row moved down 2 points with the 44-point transport (§3.4).
const LEFT: Pos2 = pos2(259.0, 328.0);
const RIGHT: Pos2 = pos2(741.0, 328.0);

/// A directory of the test's own, removed with whatever is in it when the
/// test ends, however it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A shot whose left half is its green backdrop and whose right half is
/// red, and the backdrop alone, imported by the worker.
struct Library {
    _folder: Scratch,
    service: MediaService,
    shot: Asset,
    plate: Asset,
}

fn imported() -> Library {
    let folder =
        Scratch(std::env::temp_dir().join(format!("neptune-video-test-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&folder.0).expect("FIXTURE: a folder for the test's files");
    let shot = folder.0.join("shot.png");
    image::RgbaImage::from_fn(64, 36, |x, _| image::Rgba(if x < 32 { GREEN } else { RED }))
        .save(&shot)
        .expect("FIXTURE: the shot is written");
    let plate = folder.0.join("plate.png");
    image::RgbaImage::from_pixel(64, 36, image::Rgba(GREEN))
        .save(&plate)
        .expect("FIXTURE: the plate is written");
    let service = MediaService::new(|| {}).expect("FIXTURE: the media workers start");
    service
        .import(vec![shot, plate])
        .expect("FIXTURE: the import is taken");
    let until = Instant::now() + Duration::from_secs(10);
    let assets = loop {
        match service.poll() {
            Some(MediaEvent::Imported { assets, errors, .. }) => {
                assert!(errors.is_empty(), "FIXTURE: {errors:?}");
                break assets;
            }
            Some(other) => panic!("FIXTURE: {other:?} came before the import"),
            None => {
                assert!(Instant::now() < until, "FIXTURE: the import timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    let named = |name: &str| {
        (assets
            .iter()
            .find(|asset: &&Asset| asset.name.starts_with(name)))
        .cloned()
        .expect("FIXTURE: both files were imported")
    };
    Library {
        _folder: folder,
        service,
        shot: named("shot"),
        plate: named("plate"),
    }
}

/// A project with the shot over the whole canvas and `effect` on it.
fn project(library: &Library, effect: Effect) -> (Editor, ClipId) {
    let mut editor = Editor::new(Project::new("Keys")).expect("a new project is valid");
    let track = (editor.project().tracks.iter())
        .find(|track| track.kind == TrackKind::Video)
        .map(|track| track.id)
        .expect("a new project has a video track");
    let clip = Clip::from_asset(&library.shot, Time::ZERO);
    let clip_id = clip.id;
    for command in [
        Command::AddAssets {
            assets: vec![library.shot.clone(), library.plate.clone()],
        },
        Command::AddClip {
            track_id: track,
            clip,
        },
        Command::SetEffects {
            clip_id,
            effects: vec![effect],
        },
    ] {
        let label = command.label().to_owned();
        if let Err(error) = editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }
    (editor, clip_id)
}

struct Stage {
    ctx: egui::Context,
    textures: Textures,
    viewer: ViewerState,
    palette: Palette,
    selection: Selection,
}

impl Stage {
    fn new(viewer: ViewerState) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        Self {
            ctx,
            textures: Textures::default(),
            viewer,
            palette,
            selection: Selection::default(),
        }
    }

    /// One frame of the application with `events` in it: what the worker
    /// published is handed over, and the viewer is painted. Returns what
    /// the viewer asked for.
    fn frame(
        &mut self,
        service: &MediaService,
        project: &Project,
        events: Vec<Event>,
    ) -> Vec<Action> {
        let generation = service.generation();
        while let Some(event) = service.poll() {
            match event {
                MediaEvent::FrameReady {
                    generation: from,
                    asset_id,
                    time,
                    size,
                    consumer,
                    frame,
                } => {
                    if from == generation
                        && let Some(asset) = project.assets.get(&asset_id)
                    {
                        (self.textures).accept(
                            &self.ctx,
                            Some(asset),
                            consumer,
                            asset_id,
                            time,
                            size,
                            &frame,
                        );
                    }
                }
                other => panic!("FIXTURE: the worker published {other:?}"),
            }
        }
        self.textures.begin_frame();
        self.viewer.begin_frame();
        let ctx = self.ctx.clone();
        let mut actions: Vec<Action> = Vec::new();
        let mut draft: Option<ClipDraft> = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW.into())),
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
                &mut draft,
                &mut self.textures,
                &ViewerInput {
                    p: &self.palette,
                    project,
                    query: None,
                    playhead: Time::ZERO,
                    playing: false,
                    speed: 1.0,
                    looping: false,
                    media: Some(service),
                    record: Default::default(),
                    library: true,
                    quality: Default::default(),
                    cards: &[],
                },
                &mut actions,
            );
        });
        // No renderer takes the textures; what they hold is not read here.
        output.textures_delta.clear();
        assert!(draft.is_none(), "the viewer held a value: {draft:?}");
        actions
    }

    /// The canvas's pixel a part of the way across its middle row.
    fn across(&self, part: f32) -> Option<[u8; 4]> {
        let ([width, height], bytes) = self.viewer.composer.read(0)?;
        let x = (width as f32 * part) as u32;
        let from = ((height / 2 * width + x) * 4) as usize;
        Some([
            bytes[from],
            bytes[from + 1],
            bytes[from + 2],
            bytes[from + 3],
        ])
    }

    /// Paints frames until `done`, or panics after ten seconds with `what`.
    fn until(
        &mut self,
        service: &MediaService,
        project: &Project,
        what: &str,
        done: impl Fn(&Self) -> bool,
    ) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(service, project, Vec::new());
            if done(self) {
                return;
            }
            assert!(Instant::now() < until, "never: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn click(&mut self, service: &MediaService, project: &Project, pos: Pos2) -> Vec<Action> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut actions = self.frame(service, project, vec![Event::PointerMoved(pos)]);
        actions.extend(self.frame(service, project, vec![button(true)]));
        actions.extend(self.frame(service, project, vec![button(false)]));
        actions
    }
}

fn near(actual: Option<[u8; 4]>, expected: [u8; 4]) -> bool {
    actual.is_some_and(|at| at.iter().zip(&expected).all(|(a, e)| a.abs_diff(*e) <= 2))
}

#[test]
fn a_difference_matte_clears_what_its_decoded_plate_shows_too() {
    let Some(state) = render_state() else { return };
    let library = imported();
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(library.plate.id);
    matte.params = vec![ParamValue {
        key: ParamKey::Softness,
        value: 0.0,
    }];
    let (mut editor, clip) = project(&library, matte.clone());
    let project = editor.snapshot();
    let background = project.settings.background;
    let plate = Viewing::Plate(clip, matte.id);
    let mut stage = Stage::new(ViewerState::with_device(Some((*state).clone())));

    // The plate is asked for with the picture. Where the two agree the
    // canvas's colour shows through; where they differ the shot is kept.
    stage.frame(&library.service, &project, Vec::new());
    assert!(
        stage.textures.has_layer(plate),
        "the plate is a layer asked for"
    );
    stage.until(
        &library.service,
        &project,
        "the backdrop is cleared",
        |stage| near(stage.across(0.25), background) && near(stage.across(0.75), RED),
    );
    assert!(stage.textures.input(plate).is_some());
    assert_eq!(
        stage.textures.layer_usage().pictures,
        0,
        "no quad is drawn of a plate or of the layer a canvas shows: neither has a texture"
    );

    // Paused: nothing more is composited, decoded or asked for.
    stage.frame(&library.service, &project, Vec::new());
    let composed = stage.viewer.composer.counters().composed;
    for _ in 0..5 {
        stage.frame(&library.service, &project, Vec::new());
    }
    assert_eq!(stage.viewer.composer.counters().composed, composed);
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));
    assert!(
        library.service.poll().is_none(),
        "nothing more was asked for"
    );

    // Hidden, the matte asks for no plate, and the shot is whole.
    matte.enabled = false;
    let mut vignette = Effect::new(EffectKind::Vignette);
    vignette.params = vec![ParamValue {
        key: ParamKey::Amount,
        value: 0.0,
    }];
    editor
        .dispatch(Command::SetEffects {
            clip_id: clip,
            effects: vec![matte.clone(), vignette],
        })
        .expect("FIXTURE: the effects are valid");
    let hidden = editor.snapshot();
    stage.until(&library.service, &hidden, "the shot is whole", |stage| {
        near(stage.across(0.25), GREEN) && near(stage.across(0.75), RED)
    });

    // Shown again while paused: drawn with its plate once more.
    matte.enabled = true;
    editor
        .dispatch(Command::SetEffects {
            clip_id: clip,
            effects: vec![matte],
        })
        .expect("FIXTURE: the effects are valid");
    let shown = editor.snapshot();
    stage.until(&library.service, &shown, "cleared again", |stage| {
        near(stage.across(0.25), background) && near(stage.across(0.75), RED)
    });
}

#[test]
fn a_matte_without_a_compositor_asks_for_no_plate() {
    let library = imported();
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(library.plate.id);
    let (editor, clip) = project(&library, matte.clone());
    let project = editor.snapshot();
    let mut stage = Stage::new(ViewerState::default());
    stage.until(&library.service, &project, "the shot is shown", |stage| {
        stage.textures.picture(Viewing::Clip(clip)).is_some()
    });
    assert!(!stage.textures.has_layer(Viewing::Plate(clip, matte.id)));
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));
}

#[test]
fn a_click_on_the_picture_picks_the_colour_of_its_decoded_frame() {
    let library = imported();
    let key = Effect::new(EffectKind::ChromaKey);
    let (mut editor, clip) = project(&library, key.clone());
    let project = editor.snapshot();
    // No device: nothing composites, and the frame is kept for the pick.
    let mut stage = Stage::new(ViewerState::default());
    stage.selection.only(clip);
    stage.until(&library.service, &project, "the shot is shown", |stage| {
        stage.textures.picture(Viewing::Clip(clip)).is_some()
    });
    assert!(stage.textures.input(Viewing::Clip(clip)).is_none());

    stage.viewer.pick_colour(Some(ColourPick {
        clip,
        effect: key.id,
    }));
    stage.until(&library.service, &project, "its frame is kept", |stage| {
        stage.textures.input(Viewing::Clip(clip)).is_some()
    });

    // Off the picture there is nothing to pick, and the pick goes on.
    let actions = stage.click(&library.service, &project, pos2(6.0, 6.0));
    assert!(actions.is_empty(), "{actions:?}");
    assert!(!stage.viewer.take_picked());
    assert_eq!(
        stage.selection.single(),
        Some(clip),
        "nothing was deselected"
    );

    // On it: the colour there, as one edit of the effect it was begun for.
    let mut actions = stage.click(&library.service, &project, RIGHT);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let Some(Action::Edit(command)) = actions.pop() else {
        panic!("the click made no edit");
    };
    assert_eq!(command.label(), "Pick key colour");
    assert!(stage.viewer.take_picked());
    let before = editor.history().count();
    editor.dispatch(command).expect("the colour is accepted");
    assert_eq!(editor.history().count(), before + 1);
    let stored = editor.project().clip(clip).expect("the clip").1.effects[0].clone();
    assert_eq!(stored.id, key.id);
    for (key, byte) in [ParamKey::KeyR, ParamKey::KeyG, ParamKey::KeyB]
        .into_iter()
        .zip(RED)
    {
        let value = stored.param(key).expect("a key colour");
        assert!(
            (value - f32::from(byte) / 255.0).abs() < 0.01,
            "{key:?} is {value}"
        );
    }
    assert!(editor.undo());

    // The other half is the other colour: it is the frame that is read.
    let picked = editor.snapshot();
    let mut actions = stage.click(&library.service, &picked, LEFT);
    let Some(Action::Edit(command)) = actions.pop() else {
        panic!("the click made no edit");
    };
    editor.dispatch(command).expect("the colour is accepted");
    let green = editor.project().clip(clip).expect("the clip").1.effects[0]
        .param(ParamKey::KeyG)
        .expect("a key colour");
    assert!(
        (green - f32::from(GREEN[1]) / 255.0).abs() < 0.01,
        "{green}"
    );

    // The pick over, a press is the viewer's own again and the frame is
    // let go.
    assert!(stage.viewer.take_picked());
    stage.viewer.pick_colour(None);
    stage.frame(&library.service, &picked, Vec::new());
    stage.frame(&library.service, &picked, Vec::new());
    assert!(stage.textures.input(Viewing::Clip(clip)).is_none());
    stage.click(&library.service, &picked, pos2(6.0, 6.0));
    assert_eq!(stage.selection.single(), None);
}

/// A frame of four columns and two rows, each texel a colour of its own.
fn chequered() -> Frame {
    let rgba: Vec<u8> = (0..8u8)
        .flat_map(|texel| [texel * 30, 255 - texel * 30, 100, 255])
        .collect();
    Frame {
        width: 4,
        height: 2,
        rgba: rgba.into(),
    }
}

#[test]
fn a_place_on_screen_is_read_through_the_layers_turn_crop_and_flips() {
    // One texel is read alone where it has no neighbours: a frame as wide
    // as it is sampled is the mean of all it has.
    let texel = |frame: &Frame, x: usize, y: usize| {
        let at = (y * frame.width as usize + x) * 4;
        [0, 1, 2].map(|c| f32::from(frame.rgba[at + c]) / 255.0)
    };
    let flat = Frame {
        width: 1,
        height: 1,
        rgba: vec![10, 20, 30, 255].into(),
    };
    let place = Placed {
        clip: ClipId::new_v4(),
        center: pos2(200.0, 100.0),
        half: vec2(100.0, 50.0),
        angle: 0.0,
        locked: false,
    };
    let plain = Transform::default();
    assert_eq!(
        sampled(&flat, &place, &plain, pos2(200.0, 100.0)),
        Some(texel(&flat, 0, 0))
    );
    assert_eq!(sampled(&flat, &place, &plain, pos2(301.0, 100.0)), None);
    assert_eq!(sampled(&flat, &place, &plain, pos2(200.0, 40.0)), None);

    // A tall frame of single texels a row, so the mean of a neighbourhood
    // is of one column: which column says where the place was read.
    let columns = Frame {
        width: 4,
        height: 1,
        rgba: chequered().rgba[..16].to_vec().into(),
    };
    let mean = |from: usize, to: usize| {
        let mut sum = [0.0f32; 3];
        for x in from..=to {
            for (sum, value) in sum.iter_mut().zip(texel(&columns, x, 0)) {
                *sum += value;
            }
        }
        sum.map(|sum| sum / (to - from + 1) as f32)
    };
    let close = |actual: Option<[f32; 3]>, expected: [f32; 3]| {
        actual
            .is_some_and(|actual| (actual.iter().zip(expected)).all(|(a, e)| (a - e).abs() < 1e-5))
    };
    // The left edge reads the first two columns, the right edge the last.
    let left = pos2(105.0, 100.0);
    let right = pos2(295.0, 100.0);
    assert!(close(sampled(&columns, &place, &plain, left), mean(0, 1)));
    assert!(close(sampled(&columns, &place, &plain, right), mean(2, 3)));
    // Flipped, and turned half way round, the edges change places.
    let flipped = Transform {
        flip_x: true,
        ..Transform::default()
    };
    assert!(close(sampled(&columns, &place, &flipped, left), mean(2, 3)));
    let turned = Placed {
        angle: std::f32::consts::PI,
        ..place
    };
    assert!(close(sampled(&columns, &turned, &plain, left), mean(2, 3)));
    // Cropped to its right half, the left edge of the box is the middle.
    let cropped = Transform {
        crop: [0.5, 0.0, 0.0, 0.0],
        ..Transform::default()
    };
    assert!(close(sampled(&columns, &place, &cropped, left), mean(1, 3)));
}

#[test]
fn a_plate_given_up_while_the_viewer_is_not_drawn_asks_for_no_frame() {
    let Some(state) = render_state() else { return };
    let library = imported();
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(library.plate.id);
    let (editor, clip) = project(&library, matte.clone());
    let project = editor.snapshot();
    let plate = Viewing::Plate(clip, matte.id);
    let mut stage = Stage::new(ViewerState::with_device(Some((*state).clone())));
    stage.until(&library.service, &project, "the plate has come", |stage| {
        stage.textures.input(plate).is_some()
    });
    stage.frame(&library.service, &project, Vec::new());
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));

    // The source monitor is up: the viewer is not drawn, and what was kept
    // for its canvases is given up on the clock. A plate that is no longer
    // there is not one a canvas waits for, in every frame from then on.
    stage.textures.keep_frames(false);
    assert!(stage.textures.input(plate).is_none());
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));
}
