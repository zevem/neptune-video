//! A LUT from its file to the canvas: a `.cube` fixture imported by the real
//! import worker, its table read by that worker when a layer first looks
//! colours up in it, and the viewer's canvas read back from a real device.
//!
//! As in `compose::gpu_tests`, a test without an adapter prints `SKIP:` and
//! passes.
use super::{ViewerInput, ViewerState, show};
use crate::{
    compose::{gpu_tests::render_state, lut::Taken},
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, Selection},
};
use editor_core::{
    Asset, Clip, ClipId, Command, Editor, Effect, EffectKind, MediaKind, Project, Time, TrackKind,
};
use editor_media::{MediaEvent, MediaService};
use eframe::egui::{self, Pos2, Rect, vec2};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const NAME: &str = "Effects not shown";
const RED: [u8; 4] = [220, 40, 40, 255];

/// The text of a `.cube` file of two points an edge that swaps red and blue.
const SWAP: &str = "TITLE \"swap\"\n# red and blue change places\nLUT_3D_SIZE 2\n\
                    0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";

/// A directory of the test's own, removed with whatever is in it when the
/// test ends, however it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A still and a LUT in a folder of the test's own, imported by the worker.
struct Library {
    _folder: Scratch,
    service: MediaService,
    still: Asset,
    lut: Asset,
}

fn imported() -> Library {
    let folder = Scratch(std::env::temp_dir().join(format!("aster-test-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&folder.0).expect("FIXTURE: a folder for the test's files");
    let still = folder.0.join("red.png");
    image::RgbaImage::from_pixel(64, 36, image::Rgba(RED))
        .save(&still)
        .expect("FIXTURE: the still is written");
    let lut = folder.0.join("swap.cube");
    std::fs::write(&lut, SWAP).expect("FIXTURE: the table is written");
    let service = MediaService::new(|| {}).expect("FIXTURE: the media workers start");
    service
        .import(vec![still, lut])
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
    let of = |kind| {
        (assets.iter().find(|asset: &&Asset| asset.kind == kind))
            .cloned()
            .expect("FIXTURE: both files were imported")
    };
    Library {
        _folder: folder,
        service,
        still: of(MediaKind::Image),
        lut: of(MediaKind::Lut),
    }
}

/// A project with the still over the whole canvas, looked up in the LUT.
fn graded(library: &Library) -> (Editor, ClipId) {
    let mut editor = Editor::new(Project::new("Looks")).expect("a new project is valid");
    let track = (editor.project().tracks.iter())
        .find(|track| track.kind == TrackKind::Video)
        .map(|track| track.id)
        .expect("a new project has a video track");
    let clip = Clip::from_asset(&library.still, Time::ZERO);
    let clip_id = clip.id;
    let mut lookup = Effect::new(EffectKind::Lut);
    lookup.asset_id = Some(library.lut.id);
    for command in [
        Command::AddAssets {
            assets: vec![library.still.clone(), library.lut.clone()],
        },
        Command::AddClip {
            track_id: track,
            clip,
        },
        Command::SetEffects {
            clip_id,
            effects: vec![lookup],
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
    /// What the worker said of tables, as the application would hear it.
    failures: Vec<String>,
}

impl Stage {
    fn new(viewer: ViewerState) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        Self {
            ctx,
            textures: Textures::default(),
            viewer,
            palette,
            failures: Vec::new(),
        }
    }

    /// One frame of the application: what the worker published is handed
    /// over as `App::take_media` hands it, and the viewer is painted.
    /// Returns the names accessibility is given.
    fn frame(&mut self, service: &MediaService, project: &Project) -> Vec<String> {
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
                        self.textures.accept(
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
                MediaEvent::LutLoaded {
                    generation: from,
                    asset_id,
                    path,
                    file_size,
                    table,
                } => {
                    if from != generation {
                        continue;
                    }
                    let asset = project.assets.get(&asset_id);
                    let taken =
                        (self.viewer).take_lut(asset, asset_id, from, &path, file_size, table);
                    if let Taken::Failed(why) = taken {
                        self.failures.push(why);
                    }
                }
                other => panic!("FIXTURE: the worker published {other:?}"),
            }
        }
        self.textures.begin_frame();
        self.viewer.begin_frame();
        let ctx = self.ctx.clone();
        let mut actions: Vec<Action> = Vec::new();
        let mut selection = Selection::default();
        let mut draft: Option<ClipDraft> = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            show(
                ui,
                rect,
                &mut self.viewer,
                &mut selection,
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
                },
                &mut actions,
            );
        });
        // No renderer takes the textures; what they hold is not read here.
        output.textures_delta.clear();
        (output.platform_output.accesskit_update.iter())
            .flat_map(|update| update.nodes.iter())
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .map(str::to_owned)
            .collect()
    }

    /// The middle pixel of the viewer's canvas, once one is composited.
    fn middle(&self) -> Option<[u8; 4]> {
        let ([width, height], bytes) = self.viewer.composer.read(0)?;
        let from = ((height / 2 * width + width / 2) * 4) as usize;
        Some([
            bytes[from],
            bytes[from + 1],
            bytes[from + 2],
            bytes[from + 3],
        ])
    }

    /// Paints frames until the canvas's middle is near `colour`, and returns
    /// what the last frame named. Panics after ten seconds.
    fn until(&mut self, service: &MediaService, project: &Project, colour: [u8; 4]) -> Vec<String> {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let names = self.frame(service, project);
            let middle = self.middle();
            if middle.is_some_and(|at| at.iter().zip(&colour).all(|(a, e)| a.abs_diff(*e) <= 2)) {
                return names;
            }
            assert!(
                Instant::now() < until,
                "the canvas's middle is {middle:?} and never became {colour:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn near(actual: Option<[u8; 4]>, expected: [u8; 4]) -> bool {
    actual.is_some_and(|at| at.iter().zip(&expected).all(|(a, e)| a.abs_diff(*e) <= 2))
}

#[test]
fn a_cube_file_grades_the_canvas_once_the_worker_has_read_it() {
    let Some(state) = render_state() else { return };
    let library = imported();
    assert_eq!(library.lut.width, 2);
    let (mut editor, clip) = graded(&library);
    let project = editor.snapshot();
    let mut stage = Stage::new(ViewerState::with_device(Some((*state).clone())));
    let [r, g, b, a] = RED;

    // The first frame asks for the picture and the table and waits for
    // neither; the canvas is graded when both have come.
    stage.frame(&library.service, &project);
    assert!(stage.viewer.luts.get(&library.lut).is_none());
    let names = stage.until(&library.service, &project, [b, g, r, a]);
    assert!(!names.iter().any(|name| name == NAME), "{names:?}");
    assert!(stage.failures.is_empty(), "{:?}", stage.failures);
    assert!(stage.viewer.luts.get(&library.lut).is_some());

    // Paused: the canvas is not drawn again, and the table not read again.
    let composed = stage.viewer.composer.counters().composed;
    for _ in 0..5 {
        stage.frame(&library.service, &project);
    }
    assert_eq!(stage.viewer.composer.counters().composed, composed);
    assert!(!stage.viewer.canvas_is_stale(&stage.textures));
    assert!(
        library.service.poll().is_none(),
        "nothing more was asked for"
    );

    // The effect mixed half in is half way, in linear light, and is drawn
    // with the table that is kept.
    let mut effects = project.clip(clip).expect("the clip").1.effects.clone();
    effects[0].mix = 0.5;
    editor
        .dispatch(Command::SetEffects {
            clip_id: clip,
            effects,
        })
        .expect("the mix is in range");
    let linear = |byte: u8| {
        let c = f64::from(byte) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let between = |a: u8, b: u8| {
        let light = 0.5 * linear(a) + 0.5 * linear(b);
        let c = if light <= 0.003_130_8 {
            light * 12.92
        } else {
            1.055 * light.powf(1.0 / 2.4) - 0.055
        };
        (c * 255.0).round() as u8
    };
    let half = [between(r, b), g, between(b, r), a];
    stage.until(&library.service, &editor.snapshot(), half);
    assert!(
        library.service.poll().is_none(),
        "the table was not read again"
    );

    // Another project: every table is given up with the canvases.
    stage.viewer.release_canvases();
    assert!(stage.viewer.luts.get(&library.lut).is_none());
    assert!(stage.middle().is_none());
}

#[test]
fn a_lut_whose_file_is_gone_leaves_the_picture_as_it_is_and_the_viewer_says_so() {
    let Some(state) = render_state() else { return };
    let library = imported();
    let (editor, _) = graded(&library);
    let project = editor.snapshot();
    std::fs::remove_file(&library.lut.path).expect("FIXTURE: the table's file is removed");
    let mut stage = Stage::new(ViewerState::with_device(Some((*state).clone())));

    // The picture comes and is shown ungraded; the failure is heard once.
    let until = Instant::now() + Duration::from_secs(10);
    let names = loop {
        let names = stage.frame(&library.service, &project);
        if !stage.failures.is_empty() && near(stage.middle(), RED) {
            // The frame after the failure was taken says so.
            break stage.frame(&library.service, &project);
        }
        assert!(Instant::now() < until, "the failure never came: {names:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(near(stage.middle(), RED), "{:?}", stage.middle());
    assert_eq!(stage.failures.len(), 1, "{:?}", stage.failures);
    assert!(stage.failures[0].starts_with("swap.cube: "));
    assert!(names.iter().any(|name| name == NAME), "{names:?}");
    // It is not asked for again frame after frame.
    for _ in 0..5 {
        stage.frame(&library.service, &project);
    }
    std::thread::sleep(Duration::from_millis(50));
    stage.frame(&library.service, &project);
    assert_eq!(stage.failures.len(), 1);

    // The asset now of a file that is there: the old failure is not its.
    let mut relinked = library.lut.clone();
    relinked.path = Path::new(&library.still.path).with_file_name("again.cube");
    std::fs::write(&relinked.path, SWAP).expect("FIXTURE: the table is written again");
    relinked.file_size = SWAP.len() as u64;
    let mut moved = (*project).clone();
    std::sync::Arc::make_mut(&mut moved.assets).insert(relinked.id, relinked);
    moved
        .validate()
        .expect("FIXTURE: the relinked project is valid");
    // As `App::media_changed` does for an asset of another file.
    stage.viewer.forget_lut(library.lut.id);
    let [r, g, b, a] = RED;
    let names = stage.until(&library.service, &moved, [b, g, r, a]);
    assert!(!names.iter().any(|name| name == NAME), "{names:?}");
    assert_eq!(stage.failures.len(), 1);
}

#[test]
fn a_decoded_frame_is_kept_only_while_a_canvas_is_drawn_from_it() {
    let Some(state) = render_state() else { return };
    let library = imported();
    let (mut editor, clip) = graded(&library);
    let viewing = crate::textures::Viewing::Clip(clip);
    let effects = |editor: &mut Editor, effects: Vec<Effect>| {
        editor
            .dispatch(Command::SetEffects {
                clip_id: clip,
                effects,
            })
            .expect("FIXTURE: the effects are valid");
    };
    // A plain project, though the window has a device to composite on.
    effects(&mut editor, Vec::new());
    let plain = editor.snapshot();
    let mut stage = Stage::new(ViewerState::with_device(Some((*state).clone())));
    let until = Instant::now() + Duration::from_secs(10);
    while stage.textures.picture(viewing).is_none() {
        stage.frame(&library.service, &plain);
        assert!(Instant::now() < until, "the still never came");
        std::thread::sleep(Duration::from_millis(5));
    }
    stage.frame(&library.service, &plain);
    assert!(
        stage.textures.input(viewing).is_none(),
        "nothing reads the decoded frame of a plain plan, so none is kept"
    );
    assert_eq!(stage.viewer.composer.counters().composed, 0);

    // It gains an effect while paused: its layer asks for the frame again,
    // and the canvas is drawn from that alone.
    let mut vignette = Effect::new(EffectKind::Vignette);
    vignette.mix = 1.0;
    effects(&mut editor, vec![vignette]);
    let fancy = editor.snapshot();
    stage.until(&library.service, &fancy, RED);
    stage.frame(&library.service, &fancy);
    assert!(stage.textures.input(viewing).is_some());
    assert_eq!(
        stage.textures.layer_usage().pictures,
        0,
        "a layer a canvas shows is uploaded once, by the compositor: no \
         texture of its own is kept beside that"
    );

    // Plain again: the layer is a quad of its own picture at once, made
    // from the frame that was kept, and the frame is let go.
    effects(&mut editor, Vec::new());
    stage.frame(&library.service, &editor.snapshot());
    assert!(stage.textures.picture(viewing).is_some());
    assert!(stage.textures.input(viewing).is_none());
    assert!(
        library.service.poll().is_none(),
        "and nothing was decoded for that"
    );
}
