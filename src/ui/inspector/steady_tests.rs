//! The Stabilize card of the Motion section: the effect switched on, set and
//! removed through the clip's draft, the analysis asked for and given up as
//! actions, and what is known of the camera's path said in words.
use super::*;
use crate::{steady::Status, ui::SteadyShown};
use editor_core::{AssetId, Effect};

fn moving() -> (Bench, ClipId, AssetId) {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Motion;
    let asset = bench.clip(clip).asset_id().expect("a media clip");
    (bench, clip, asset)
}

/// Says what the application would of the clip's camera path.
fn known(bench: &mut Bench, clip: ClipId, asset: AssetId, status: Status, suggested: Option<f32>) {
    bench.state.set_steady(Some(SteadyShown {
        clip,
        asset,
        status,
        suggested,
    }));
    bench.frame(Vec::new());
}

fn stabilization(bench: &Bench, clip: ClipId) -> Option<Effect> {
    (bench.clip(clip).effects.iter())
        .find(|effect| effect.kind == EffectKind::Stabilize)
        .cloned()
}

fn value(effect: &Effect, key: ParamKey) -> f32 {
    let spec = (look::spec(EffectKind::Stabilize).iter())
        .find(|spec| spec.key == key)
        .unwrap();
    effect.param(key).unwrap_or(spec.default)
}

#[test]
fn a_clip_is_steadied_by_a_switch_and_analysed_by_a_button() {
    let (mut bench, clip, asset) = moving();
    let before = bench.steps();
    bench.frame(Vec::new());
    // Until the application has said, nothing is known.
    assert!(bench.has("Stabilization: Looking for an analysis…"));
    assert_eq!(bench.find("Stabilize").unwrap().selected, Some(false));
    assert!(!bench.has("Strength"), "no effect, no rows of it");
    known(&mut bench, clip, asset, Status::NotAnalysed, None);
    assert!(bench.has("Stabilization: Not analysed."));

    // The switch is the edit: one effect, as its table has it, in one step.
    bench.click("Stabilize");
    assert_eq!(bench.steps(), before + 1);
    let effect = stabilization(&bench, clip).expect("the clip is stabilized");
    assert!(effect.enabled);
    assert_eq!(value(&effect, ParamKey::Smoothness), 0.5);
    assert_eq!(value(&effect, ParamKey::CropZoom), 1.1);
    assert_eq!(bench.find("Stabilize").unwrap().selected, Some(true));
    // With no path the clip is not steadied yet, and the line says so.
    assert!(bench.has("Stabilization: Not analysed. The clip is shown as it is."));
    assert!(bench.find("Strength").is_some() && bench.find("Crop zoom").is_some());

    // Analysing is asked of the application for the clip's own media, and
    // edits nothing.
    bench.click("Analyse");
    assert_eq!(bench.analyses, vec![asset]);
    assert_eq!(bench.steps(), before + 1);

    // Under way: how far, and the way out of it.
    known(&mut bench, clip, asset, Status::Analysing(0.42), None);
    let progress = bench.find("Analysing the video").expect("a progress bar");
    assert_eq!(progress.role, Role::ProgressIndicator);
    assert!(!bench.has("Analyse"), "one analysis at a time is offered");
    bench.click("Cancel analysis");
    assert_eq!(bench.cancels, 1);
    assert_eq!(bench.steps(), before + 1);

    // Ready: said, and it may be done again.
    let ready = Status::Ready {
        frames: 300,
        unmeasured: 2,
    };
    known(&mut bench, clip, asset, ready, None);
    assert!(bench.has("Stabilization: Ready · 300 frames · 2 not measured"));
    bench.click("Analyse again");
    assert_eq!(bench.analyses, vec![asset, asset]);

    // Off and on again are the effect's own switch, a step each.
    bench.click("Stabilize");
    assert!(!stabilization(&bench, clip).unwrap().enabled);
    assert_eq!(bench.steps(), before + 2);
    assert!(bench.editor.undo());
    assert!(stabilization(&bench, clip).unwrap().enabled);

    // Removed, it is gone in one step, and undone it is back.
    bench.click("Remove Stabilize");
    assert!(stabilization(&bench, clip).is_none());
    assert!(!bench.has("Strength"));
    assert!(bench.editor.undo());
    assert!(stabilization(&bench, clip).is_some());
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn the_crop_that_hides_the_borders_is_offered_and_begun_with() {
    let (mut bench, clip, asset) = moving();
    let ready = Status::Ready {
        frames: 300,
        unmeasured: 0,
    };
    // A clip whose path is known begins with the crop it needs.
    known(&mut bench, clip, asset, ready.clone(), Some(1.07));
    bench.click("Stabilize");
    let effect = stabilization(&bench, clip).unwrap();
    assert_eq!(value(&effect, ParamKey::CropZoom), 1.07);
    assert!(bench.has("Borders hide at 1.07 ×"));
    assert!(bench.find("Fit crop").unwrap().disabled, "it already fits");

    // The clip is trimmed and needs more: the button sets it, in one step.
    known(&mut bench, clip, asset, ready.clone(), Some(1.18));
    assert!(bench.has("Borders hide at 1.18 ×"));
    assert!(bench.has("With less crop some frames are steadied less, so that no border shows."));
    let before = bench.steps();
    bench.click("Fit crop");
    assert_eq!(bench.steps(), before + 1);
    let effect = stabilization(&bench, clip).unwrap();
    assert_eq!(value(&effect, ParamKey::CropZoom), 1.18);
    assert_eq!(value(&effect, ParamKey::Smoothness), 0.5);
    assert!(bench.find("Fit crop").unwrap().disabled);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_strength_dragged_is_one_step_and_an_escape_gives_it_up() {
    let (mut bench, clip, asset) = moving();
    known(&mut bench, clip, asset, Status::NotAnalysed, None);
    bench.click("Stabilize");
    let before = bench.steps();

    let end = bench.drag("Strength", vec2(30.0, 0.0));
    // Held, it is a draft the canvas shows and no edit yet.
    assert_eq!(bench.steps(), before);
    let held = bench.layer(clip);
    let drawn = (held.effects.iter())
        .find(|effect| effect.kind == EffectKind::Stabilize)
        .and_then(|effect| effect.param(ParamKey::Smoothness))
        .unwrap();
    assert!(drawn > 0.5, "the draft is drawn: {drawn}");
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    let stronger = value(&stabilization(&bench, clip).unwrap(), ParamKey::Smoothness);
    assert!(stronger > 0.5 && stronger <= 1.0, "{stronger}");

    // An Escape while it is held leaves the clip as it was.
    let end = bench.drag("Crop zoom", vec2(40.0, 0.0));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    let effect = stabilization(&bench, clip).unwrap();
    assert_eq!(value(&effect, ParamKey::CropZoom), 1.1);
    assert_eq!(value(&effect, ParamKey::Smoothness), stronger);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_stale_or_failed_analysis_is_said_and_what_is_of_another_clip_is_not() {
    let (mut bench, clip, asset) = moving();
    known(&mut bench, clip, asset, Status::Stale, None);
    assert!(bench.has("Stabilization: Stale: the media changed since it was analysed."));
    assert!(bench.find("Analyse").is_some());
    let failed = Status::Failed("ffmpeg: no such file".to_owned());
    known(&mut bench, clip, asset, failed, None);
    assert!(bench.has("Stabilization: The analysis failed: ffmpeg: no such file"));
    known(&mut bench, clip, asset, Status::Reading, None);
    assert!(bench.has("Stabilization: Looking for an analysis…"));

    // What the application said of another clip, or of other media, is
    // not this clip's.
    let ready = Status::Ready {
        frames: 10,
        unmeasured: 0,
    };
    known(
        &mut bench,
        uuid::Uuid::new_v4(),
        asset,
        ready.clone(),
        Some(1.5),
    );
    assert!(bench.has("Stabilization: Looking for an analysis…"));
    known(&mut bench, clip, uuid::Uuid::new_v4(), ready, Some(1.5));
    assert!(bench.has("Stabilization: Looking for an analysis…"));

    // A still has no camera, and a title no picture.
    let still = media("card.png", MediaKind::Image);
    bench.edit(Command::AddAssets {
        assets: vec![still.clone()],
    });
    let picture = bench.add(TrackKind::Video, {
        let mut picture = Clip::from_asset(&still, seconds(6.0));
        picture.duration = seconds(2.0);
        picture
    });
    bench.selection.only(picture);
    bench.frame(Vec::new());
    assert!(bench.has("Only a video has a camera to steady."));
    assert!(bench.find("Stabilize").is_none());
    assert!(bench.find("Analyse").is_none());
}

#[test]
fn a_stabilization_is_reached_without_a_pointer_and_a_locked_track_takes_none() {
    let (mut bench, clip, asset) = moving();
    known(&mut bench, clip, asset, Status::NotAnalysed, None);
    let before = bench.steps();
    bench.focus("Stabilize");
    bench.enter(Modifiers::NONE);
    assert!(stabilization(&bench, clip).is_some());
    assert_eq!(bench.steps(), before + 1);
    bench.focus("Analyse");
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.analyses, vec![asset]);

    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
    });
    bench.frame(Vec::new());
    assert!(bench.find("Stabilize").unwrap().disabled);
    assert!(bench.find("Strength").unwrap().disabled);
    // And what would ask for it anyway is refused by the project.
    let refused = bench.editor.dispatch(Command::SetEffects {
        clip_id: clip,
        effects: Vec::new(),
    });
    assert!(refused.is_err());
    assert!(stabilization(&bench, clip).is_some());
}
