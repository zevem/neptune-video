use editor_core::{
    AnimTrack, Asset, AssetId, BlendMode, Clip, ClipId, ClipOverride, Ease, Effect, EffectKind,
    Follow, Keyframe, LayerSource, Mask, MaskShape, MediaKind, ParamKey, ParamValue, Project,
    Reading, RenderPlan, Time, Track, TrackKey, TrackKind, Tracker, Transform, Transition,
    TransitionKind, gain_at, render, resolve,
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn asset(project: &mut Project, name: &str, duration: f64) -> Asset {
    let mut asset = Asset::new(PathBuf::from(name), MediaKind::Video, seconds(duration));
    asset.has_audio = true;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    asset
}

/// A clip of `asset` from its first frame.
fn clip(asset: &Asset, start: f64, duration: f64) -> Clip {
    let mut clip = Clip::from_asset(asset, seconds(start));
    clip.duration = seconds(duration);
    clip
}

fn push(project: &mut Project, track: usize, clip: Clip) -> ClipId {
    let id = clip.id;
    Arc::make_mut(&mut project.tracks[track]).clips.push(clip);
    id
}

fn clip_mut(project: &mut Project, id: ClipId) -> &mut Clip {
    project
        .tracks
        .iter_mut()
        .find(|track| track.clips.iter().any(|clip| clip.id == id))
        .map(Arc::make_mut)
        .and_then(|track| track.clips.iter_mut().find(|clip| clip.id == id))
        .unwrap()
}

fn key(at: f64, value: f32, ease: Ease) -> Keyframe {
    Keyframe {
        at: seconds(at),
        value,
        ease,
    }
}

fn track(target: Option<Uuid>, key: ParamKey, keys: Vec<Keyframe>) -> AnimTrack {
    AnimTrack { target, key, keys }
}

fn transition(kind: TransitionKind, duration: f64) -> Transition {
    Transition {
        kind,
        duration: seconds(duration),
        ease: Ease::Linear,
        params: Vec::new(),
    }
}

fn plan(project: &Project, at: f64) -> RenderPlan {
    project.validate().unwrap();
    resolve(project, seconds(at), &[])
}

fn clips(plan: &RenderPlan) -> Vec<ClipId> {
    plan.layers.iter().map(|layer| layer.clip).collect()
}

fn source(plan: &RenderPlan, index: usize) -> (AssetId, Time, f64) {
    match plan.layers[index].source {
        LayerSource::Media {
            asset_id,
            source_time,
            speed,
        } => (asset_id, source_time, speed),
        LayerSource::Text { .. } => panic!("layer {index} is text"),
    }
}

/// Two video tracks with overlapping clips, sound and a title.
fn layered() -> (Project, [ClipId; 5]) {
    let mut project = Project::new("Layers");
    let media = asset(&mut project, "a.mp4", 60.0);
    project
        .tracks
        .insert(1, Arc::new(Track::new("Video 2", TrackKind::Video)));
    let ids = [
        push(&mut project, 0, clip(&media, 0.0, 6.0)),
        push(&mut project, 0, clip(&media, 4.0, 6.0)),
        push(&mut project, 1, clip(&media, 2.0, 6.0)),
        push(&mut project, 2, clip(&media, 0.0, 10.0)),
        push(
            &mut project,
            3,
            Clip::text("Title", seconds(1.0), seconds(8.0)),
        ),
    ];
    (project, ids)
}

#[test]
fn layer_order_matches_the_active_clips_of_visual_tracks() {
    let (project, ids) = layered();
    for at in [0.0, 1.5, 3.0, 5.0, 7.0, 9.5, 12.0] {
        let expected: Vec<_> = project
            .active_clips(seconds(at))
            .filter(|(track, _)| track.kind != TrackKind::Audio)
            .map(|(track, clip)| (track.id, clip.id))
            .collect();
        let plan = plan(&project, at);
        let layers: Vec<_> = plan.layers.iter().map(|l| (l.track, l.clip)).collect();
        assert_eq!(layers, expected, "at {at}");
    }
    let plan = plan(&project, 5.0);
    assert_eq!(clips(&plan), [ids[0], ids[1], ids[2], ids[4]]);
    assert_eq!(plan.canvas, [1920, 1080]);
    assert_eq!(plan.background, project.settings.background);
    assert!(matches!(
        &plan.layers[3].source,
        LayerSource::Text { text, font_size, color }
            if text == "Title" && *font_size == 64.0 && *color == [255; 4]
    ));
}

#[test]
fn hidden_and_audio_tracks_produce_no_layer() {
    let (mut project, ids) = layered();
    assert!(!clips(&plan(&project, 5.0)).contains(&ids[3]));
    Arc::make_mut(&mut project.tracks[1]).hidden = true;
    assert_eq!(clips(&plan(&project, 5.0)), [ids[0], ids[1], ids[4]]);
}

#[test]
fn media_layers_carry_the_source_position_and_speed() {
    let mut project = Project::new("Source");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut fast = clip(&media, 2.0, 4.0);
    fast.source_in = seconds(5.0);
    fast.playback_rate = 2.0;
    push(&mut project, 0, fast);
    let plan = plan(&project, 3.0);
    assert_eq!(source(&plan, 0), (media.id, seconds(7.0), 2.0));
}

#[test]
fn keyframes_resolve_at_and_between_keys() {
    let mut project = Project::new("Keys");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut animated = clip(&media, 2.0, 10.0);
    let blur = Effect::new(EffectKind::Blur);
    let mut mask = Mask::new(MaskShape::Ellipse);
    mask.size = [0.3, 0.4];
    animated.animation = vec![
        track(
            None,
            ParamKey::PositionX,
            vec![
                key(2.0, 100.0, Ease::Linear),
                key(6.0, 300.0, Ease::Hold),
                key(8.0, -50.0, Ease::Linear),
            ],
        ),
        track(
            None,
            ParamKey::Gain,
            vec![key(0.0, 0.0, Ease::Linear), key(10.0, 2.0, Ease::Linear)],
        ),
        track(
            Some(blur.id),
            ParamKey::Radius,
            vec![key(0.0, 0.0, Ease::Linear), key(4.0, 0.2, Ease::Linear)],
        ),
        track(
            Some(mask.id),
            ParamKey::Feather,
            vec![key(0.0, 0.0, Ease::Linear), key(4.0, 1.0, Ease::Linear)],
        ),
    ];
    animated.transform.position = [7.0, 9.0];
    animated.effects = vec![blur, Effect::new(EffectKind::Vignette)];
    animated.masks = vec![mask];
    let id = push(&mut project, 0, animated);

    let x = |at: f64| plan(&project, at).layers[0].transform.position[0];
    // Timeline 2 s is clip-relative 0: the first key holds before its time.
    assert_eq!(x(2.0), 100.0);
    assert_eq!(x(4.0), 100.0);
    assert!((x(6.0) - 200.0).abs() < 1e-3);
    assert_eq!(x(8.0), 300.0);
    assert_eq!(x(9.0), 300.0);
    assert_eq!(x(10.0), -50.0);
    assert_eq!(x(11.5), -50.0);

    let layer = &plan(&project, 4.0).layers[0];
    // Parameters without a track keep their stored value.
    assert_eq!(layer.transform.position[1], 9.0);
    assert_eq!(layer.transform.scale, [1.0, 1.0]);
    assert!((layer.effects[0].param(ParamKey::Radius).unwrap() - 0.1).abs() < 1e-6);
    assert!((layer.masks[0].feather - 0.5).abs() < 1e-6);
    assert_eq!(layer.masks[0].size, [0.3, 0.4]);
    // Defaults are filled for every parameter of the kind.
    let vignette = &layer.effects[1];
    assert_eq!(vignette.params.len(), 3);
    assert_eq!(vignette.param(ParamKey::Amount), Some(0.4));
    assert_eq!(vignette.param(ParamKey::Softness), Some(0.5));
    assert_eq!(vignette.param(ParamKey::Hue), None);

    let stored = project.clip(id).unwrap().1;
    assert!((gain_at(stored, seconds(7.0)) - 1.0).abs() < 1e-6);
    assert_eq!(gain_at(stored, seconds(12.0)), 2.0);
    let mut plain = stored.clone();
    plain.animation.clear();
    plain.gain = 0.25;
    assert_eq!(gain_at(&plain, seconds(7.0)), 0.25);
}

#[test]
fn animated_values_are_clamped_into_the_transform_ranges() {
    let mut project = Project::new("Clamp");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut animated = clip(&media, 0.0, 10.0);
    // The curve overshoots far past both keys between them.
    let overshoot = Ease::Bezier {
        x1: 0.3,
        y1: 4.0,
        x2: 0.7,
        y2: 4.0,
    };
    animated.animation = vec![
        track(
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.0, overshoot), key(10.0, 1.0, Ease::Linear)],
        ),
        track(
            None,
            ParamKey::CropLeft,
            vec![key(0.0, 0.9, Ease::Linear), key(10.0, 0.9, Ease::Linear)],
        ),
        track(
            None,
            ParamKey::CropRight,
            vec![key(0.0, 0.9, Ease::Linear), key(10.0, 0.9, Ease::Linear)],
        ),
    ];
    push(&mut project, 0, animated);
    for at in [0.0, 2.5, 5.0, 7.5, 9.99] {
        let transform = plan(&project, at).layers[0].transform.clone();
        assert!(transform.validate(), "at {at}: {transform:?}");
    }
    assert_eq!(plan(&project, 5.0).layers[0].transform.opacity, 1.0);
}

#[test]
fn a_draft_replaces_stored_values_and_animation_plays_over_it() {
    let mut project = Project::new("Draft");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut edited = clip(&media, 0.0, 10.0);
    edited.effects = vec![Effect::new(EffectKind::Blur)];
    edited.animation = vec![track(
        None,
        ParamKey::Rotation,
        vec![key(0.0, 0.0, Ease::Linear), key(10.0, 90.0, Ease::Linear)],
    )];
    let id = push(&mut project, 0, edited);
    let other = push(&mut project, 0, clip(&media, 0.0, 10.0));
    project.validate().unwrap();

    let mut grade = Effect::new(EffectKind::Grade);
    grade.params = vec![ParamValue {
        key: ParamKey::Exposure,
        value: 1.5,
    }];
    let draft = ClipOverride {
        clip: id,
        transform: Some(Transform {
            position: [40.0, -20.0],
            rotation: 10.0,
            opacity: f32::NAN,
            ..Transform::default()
        }),
        effects: Some(vec![grade.clone()]),
        masks: Some(vec![Mask::new(MaskShape::Rectangle)]),
        blend: Some(BlendMode::Screen),
        animation: None,
        reading: None,
    };
    let plan = resolve(&project, seconds(5.0), std::slice::from_ref(&draft));
    let layer = &plan.layers[0];
    assert_eq!(layer.clip, id);
    assert_eq!(layer.transform.position, [40.0, -20.0]);
    // The rotation track still wins over the drafted base value.
    assert!((layer.transform.rotation - 45.0).abs() < 1e-3);
    // An unvalidated draft cannot carry a NaN into the plan.
    assert_eq!(layer.transform.opacity, 1.0);
    assert_eq!(layer.blend, BlendMode::Screen);
    assert_eq!(layer.effects.len(), 1);
    assert_eq!(layer.effects[0].id, grade.id);
    assert_eq!(layer.effects[0].param(ParamKey::Exposure), Some(1.5));
    assert_eq!(layer.masks.len(), 1);
    // Other clips and the stored project are untouched.
    assert_eq!(plan.layers[1].clip, other);
    assert!(plan.layers[1].is_plain());
    assert_eq!(project.clip(id).unwrap().1.blend, BlendMode::Normal);

    let partial = ClipOverride {
        clip: id,
        blend: Some(BlendMode::Add),
        ..ClipOverride::default()
    };
    let layer = &resolve(&project, seconds(5.0), &[partial]).layers[0];
    assert_eq!(layer.blend, BlendMode::Add);
    assert_eq!(layer.effects[0].kind, EffectKind::Blur);
    assert_eq!(layer.transform.position, [0.0, 0.0]);

    // A keyframe being edited stands in for the stored tracks: the value
    // keyframes drive shows the draft, and the project keeps its own.
    let keyed = ClipOverride {
        clip: id,
        animation: Some(vec![track(
            None,
            ParamKey::Rotation,
            vec![key(0.0, 0.0, Ease::Linear), key(5.0, 120.0, Ease::Linear)],
        )]),
        ..ClipOverride::default()
    };
    let layer = &resolve(&project, seconds(5.0), &[keyed]).layers[0];
    assert!((layer.transform.rotation - 120.0).abs() < 1e-3);
    let stored = &resolve(&project, seconds(5.0), &[]).layers[0];
    assert!((stored.transform.rotation - 45.0).abs() < 1e-3);
}

#[test]
fn disabled_and_zero_mix_effects_are_dropped() {
    let mut project = Project::new("Stack");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut stacked = clip(&media, 0.0, 10.0);
    let mut disabled = Effect::new(EffectKind::Glow);
    disabled.enabled = false;
    let mut silent = Effect::new(EffectKind::Grain);
    silent.mix = 0.0;
    let mut kept = Effect::new(EffectKind::CurveRed);
    kept.mix = 0.5;
    kept.points = vec![[0.0, 0.1], [1.0, 0.9]];
    stacked.effects = vec![disabled, kept.clone(), silent];
    push(&mut project, 0, stacked);
    let plan = plan(&project, 1.0);
    let effects = &plan.layers[0].effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(
        (effects[0].id, effects[0].mix, &effects[0].points),
        (kept.id, 0.5, &kept.points)
    );
    assert!(effects[0].params.is_empty());
    assert!(!plan.is_plain());
}

#[test]
fn a_transition_in_pairs_with_the_abutting_clip() {
    let mut project = Project::new("Pair");
    let media = asset(&mut project, "a.mp4", 60.0);
    let below = {
        project
            .tracks
            .insert(0, Arc::new(Track::new("Video 0", TrackKind::Video)));
        push(&mut project, 0, clip(&media, 0.0, 10.0))
    };
    let mut out = clip(&media, 0.0, 2.0);
    // Ignored: another clip abuts this one.
    out.transition_out = Some(transition(TransitionKind::Fade, 1.0));
    let a = push(&mut project, 1, out);
    let mut incoming = clip(&media, 2.0, 4.0);
    incoming.transition_in = Some(Transition {
        params: vec![ParamValue {
            key: ParamKey::Softness,
            value: 0.2,
        }],
        ..transition(TransitionKind::WipeLeft, 1.0)
    });
    let b = push(&mut project, 1, incoming);

    let before = plan(&project, 1.5);
    assert_eq!(clips(&before), [below, a]);
    assert!(before.is_plain());

    let during = plan(&project, 2.25);
    // The outgoing clip sits directly below the clip it leads into.
    assert_eq!(clips(&during), [below, a, b]);
    assert!(during.layers[0].is_plain());
    let (outgoing, entering) = (
        during.layers[1].transition.as_ref().unwrap(),
        during.layers[2].transition.as_ref().unwrap(),
    );
    assert!(outgoing.outgoing && !entering.outgoing);
    assert_eq!((outgoing.partner, entering.partner), (Some(b), Some(a)));
    assert_eq!(outgoing.kind, TransitionKind::WipeLeft);
    assert_eq!(outgoing.progress, entering.progress);
    assert!((entering.progress - 0.25).abs() < 1e-6);
    assert_eq!(entering.params, outgoing.params);
    assert_eq!(
        entering.params,
        [ParamValue {
            key: ParamKey::Softness,
            value: 0.2
        }]
    );
    // The outgoing source keeps running into its handle.
    assert_eq!(source(&during, 1), (media.id, seconds(2.25), 1.0));
    assert_eq!(source(&during, 2), (media.id, seconds(0.25), 1.0));
    assert_eq!(during.layers[1].track, during.layers[2].track);

    let start = plan(&project, 2.0);
    assert_eq!(start.layers[2].transition.as_ref().unwrap().progress, 0.0);
    let after = plan(&project, 3.0);
    assert_eq!(clips(&after), [below, b]);
    assert!(after.is_plain());
}

#[test]
fn an_eased_transition_reports_eased_progress() {
    let mut project = Project::new("Eased");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut incoming = clip(&media, 2.0, 4.0);
    incoming.transition_in = Some(Transition {
        ease: Ease::Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        },
        ..transition(TransitionKind::ZoomIn, 2.0)
    });
    push(&mut project, 0, incoming);
    let layer = &plan(&project, 2.5).layers[0];
    let entering = layer.transition.as_ref().unwrap();
    assert!(entering.progress > 0.0 && entering.progress < 0.25);
    assert_eq!(
        entering.params,
        [ParamValue {
            key: ParamKey::Amount,
            value: 0.5
        }]
    );
}

#[test]
fn a_transition_in_without_a_partner_comes_from_transparency() {
    let mut project = Project::new("Alone");
    let media = asset(&mut project, "a.mp4", 60.0);
    // A gap of one microsecond is not abutting.
    let mut near = clip(&media, 0.0, 2.0);
    near.duration = Time(1_999_999);
    push(&mut project, 0, near);
    let mut incoming = clip(&media, 2.0, 4.0);
    incoming.transition_in = Some(transition(TransitionKind::Dissolve, 1.0));
    let b = push(&mut project, 0, incoming);
    let plan = plan(&project, 2.5);
    assert_eq!(clips(&plan), [b]);
    let entering = plan.layers[0].transition.as_ref().unwrap();
    assert!(!entering.outgoing);
    assert_eq!(entering.partner, None);
    assert!((entering.progress - 0.5).abs() < 1e-6);
}

#[test]
fn a_transition_out_applies_only_when_nothing_abuts() {
    let mut project = Project::new("Out");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut leaving = clip(&media, 0.0, 4.0);
    leaving.transition_out = Some(transition(TransitionKind::Fade, 1.0));
    let a = push(&mut project, 0, leaving);

    assert!(plan(&project, 2.9).is_plain());
    let during = plan(&project, 3.75);
    assert_eq!(clips(&during), [a]);
    let out = during.layers[0].transition.as_ref().unwrap();
    assert!(out.outgoing);
    assert_eq!(out.partner, None);
    assert!((out.progress - 0.75).abs() < 1e-6);
    assert!(plan(&project, 4.0).layers.is_empty());

    push(&mut project, 0, clip(&media, 4.0, 2.0));
    assert!(plan(&project, 3.75).is_plain());
}

#[test]
fn an_outgoing_clip_without_a_handle_holds_its_last_frame() {
    let mut project = Project::new("Held");
    let short = asset(&mut project, "short.mp4", 4.0);
    let long = asset(&mut project, "long.mp4", 60.0);
    let a = push(&mut project, 0, clip(&short, 0.0, 4.0));
    let mut incoming = clip(&long, 4.0, 4.0);
    incoming.transition_in = Some(transition(TransitionKind::Fade, 1.0));
    let b = push(&mut project, 0, incoming);
    let plan = plan(&project, 4.5);
    assert_eq!(clips(&plan), [a, b]);
    assert_eq!(source(&plan, 0), (short.id, seconds(4.0), 0.0));
    assert_eq!(source(&plan, 1), (long.id, seconds(0.5), 1.0));
}

/// A tracked 60 s video on the lower track from 2 s and a title following it.
fn followed() -> (Project, ClipId, ClipId) {
    let mut project = Project::new("Follow");
    let media = asset(&mut project, "a.mp4", 60.0);
    let mut shown = clip(&media, 2.0, 6.0);
    shown.source_in = seconds(10.0);
    let shown = push(&mut project, 0, shown);
    let tracker = Tracker {
        id: Uuid::new_v4(),
        name: "Face".into(),
        asset_id: media.id,
        keys: vec![
            TrackKey {
                source: seconds(12.0),
                center: [0.25, 0.5],
                scale: 1.0,
                rotation: 0.0,
            },
            TrackKey {
                source: seconds(14.0),
                center: [0.75, 1.0],
                scale: 2.0,
                rotation: 30.0,
            },
        ],
    };
    let mut title = Clip::text("Name", seconds(0.0), seconds(20.0));
    title.transform.position = [10.0, 20.0];
    title.follow = Some(Follow {
        tracker_id: tracker.id,
        position: true,
        scale: true,
        rotation: true,
        offset: [5.0, -5.0],
    });
    let title = push(&mut project, 2, title);
    Arc::make_mut(&mut project.trackers).insert(tracker.id, Arc::new(tracker));
    (project, shown, title)
}

#[test]
fn follow_moves_the_layer_with_its_tracker_in_canvas_pixels() {
    let (mut project, shown, title) = followed();
    let transform = |project: &Project, at: f64| {
        let plan = plan(project, at);
        assert_eq!(plan.layers.last().unwrap().clip, title);
        plan.layers.last().unwrap().transform.clone()
    };
    // Timeline 5 s shows source 13 s, halfway between the keys.
    let mid = transform(&project, 5.0);
    assert!((mid.position[0] - 15.0).abs() < 1e-2, "{mid:?}");
    assert!((mid.position[1] - (20.0 + 0.25 * 1080.0 - 5.0)).abs() < 1e-2);
    assert!((mid.scale[0] - 1.5).abs() < 1e-4 && (mid.scale[1] - 1.5).abs() < 1e-4);
    assert!((mid.rotation - 15.0).abs() < 1e-3);
    // Before the first key the path holds its first point.
    let early = transform(&project, 2.5);
    assert!((early.position[0] - (10.0 - 0.25 * 1920.0 + 5.0)).abs() < 1e-2);
    assert_eq!((early.scale, early.rotation), ([1.0, 1.0], 0.0));
    assert!(plan(&project, 5.0).is_plain());

    let follow = clip_mut(&mut project, title).follow.as_mut().unwrap();
    (follow.scale, follow.rotation) = (false, false);
    let position_only = transform(&project, 5.0);
    assert_eq!(position_only.position, mid.position);
    assert_eq!(
        (position_only.scale, position_only.rotation),
        ([1.0; 2], 0.0)
    );

    // The followed clip's own layer is unaffected.
    assert_eq!(plan(&project, 5.0).layers[0].clip, shown);
    assert_eq!(
        plan(&project, 5.0).layers[0].transform,
        Transform::default()
    );
}

/// A 4:3 video tracked at one place of its frame, under a title that follows
/// it from the canvas centre.
fn followed_at(center: [f32; 2]) -> (Project, ClipId, ClipId) {
    let mut project = Project::new("Follow");
    let mut media = Asset::new(PathBuf::from("a.mp4"), MediaKind::Video, seconds(60.0));
    (media.width, media.height) = (1440, 1080);
    Arc::make_mut(&mut project.assets).insert(media.id, media.clone());
    let shown = push(&mut project, 0, clip(&media, 0.0, 10.0));
    let tracker = Tracker {
        id: Uuid::new_v4(),
        name: "Face".into(),
        asset_id: media.id,
        keys: vec![TrackKey {
            source: seconds(1.0),
            center,
            scale: 1.0,
            rotation: 0.0,
        }],
    };
    let mut title = Clip::text("Name", seconds(0.0), seconds(10.0));
    title.follow = Some(Follow {
        tracker_id: tracker.id,
        position: true,
        scale: false,
        rotation: false,
        offset: [0.0, 0.0],
    });
    let title = push(&mut project, 2, title);
    Arc::make_mut(&mut project.trackers).insert(tracker.id, Arc::new(tracker));
    (project, shown, title)
}

#[test]
fn a_follower_is_put_where_the_tracked_clip_draws_the_point() {
    let (mut project, shown, title) = followed_at([0.9, 0.25]);
    let place = |project: &Project, overrides: &[ClipOverride]| {
        let plan = resolve(project, seconds(5.0), overrides);
        assert_eq!(plan.layers.last().unwrap().clip, title);
        plan.layers.last().unwrap().transform.position
    };
    let near = |got: [f32; 2], want: [f32; 2]| {
        assert!(
            (got[0] - want[0]).abs() < 1e-2 && (got[1] - want[1]).abs() < 1e-2,
            "{got:?} is not {want:?}"
        );
    };
    // The picture is 1440 wide in a canvas of 1920: the point is a fraction
    // of the picture, not of the canvas.
    near(place(&project, &[]), [0.4 * 1440.0, -0.25 * 1080.0]);

    // Made a quarter of its size in a corner, the point goes with it.
    let inset = Transform {
        position: [600.0, -300.0],
        scale: [0.25, 0.25],
        ..Transform::default()
    };
    clip_mut(&mut project, shown).transform = inset.clone();
    near(place(&project, &[]), [600.0 + 144.0, -300.0 - 67.5]);

    // Cropped, flipped and turned a quarter: what is left of the picture is
    // 0.8 of its width, the point an eighth in from the edge shown on the
    // left, and across is down.
    clip_mut(&mut project, shown).transform = Transform {
        position: [100.0, 50.0],
        scale: [0.5, 0.5],
        rotation: 90.0,
        crop: [0.2, 0.0, 0.0, 0.0],
        flip_x: true,
        ..Transform::default()
    };
    near(
        place(&project, &[]),
        [100.0 + 0.25 * 1080.0 * 0.5, 50.0 - 0.375 * 1152.0 * 0.5],
    );

    // A value keyframes drive, and one held in a draft, place the point too.
    clip_mut(&mut project, shown).transform = Transform::default();
    clip_mut(&mut project, shown).animation = vec![track(
        None,
        ParamKey::PositionX,
        vec![key(0.0, 0.0, Ease::Linear), key(10.0, 200.0, Ease::Linear)],
    )];
    near(place(&project, &[]), [100.0 + 576.0, -270.0]);
    let held = ClipOverride {
        clip: shown,
        transform: Some(inset),
        ..ClipOverride::default()
    };
    near(place(&project, &[held]), [100.0 + 144.0, -300.0 - 67.5]);
    clip_mut(&mut project, shown).animation.clear();

    // A picture read another way (a stabilized one, enlarged 1.25 times and
    // moved) shows the point where that reading does.
    let reading = Reading {
        matrix: [0.8, 0.0, 0.0, 0.8],
        offset: [0.02, -0.01],
    };
    let steadied = ClipOverride {
        clip: shown,
        reading: Some(reading),
        ..ClipOverride::default()
    };
    let at = reading.shown([0.9, 0.25]);
    assert!(
        (at[0] - 0.975).abs() < 1e-5 && (at[1] - 0.2).abs() < 1e-5,
        "{at:?}"
    );
    let back = reading.read(at);
    assert!((back[0] - 0.9).abs() < 1e-5 && (back[1] - 0.25).abs() < 1e-5);
    near(
        place(&project, std::slice::from_ref(&steadied)),
        [0.475 * 1440.0, -0.3 * 1080.0],
    );
    // And the place asked for outside a plan is the one the plan uses.
    let tracker = project.trackers.values().next().unwrap().clone();
    let asked = render::tracked_place(&project, seconds(5.0), title, &tracker, &[steadied]);
    near(asked.unwrap(), [0.475 * 1440.0, -0.3 * 1080.0]);
    assert_eq!(
        render::tracked_place(&project, seconds(11.0), title, &tracker, &[]),
        None
    );
}

#[test]
fn clips_that_follow_each_other_are_placed_and_stay_in_bounds() {
    // Two clips of one tracked video, each following the point on the other.
    let (mut project, shown, _) = followed_at([0.75, 0.5]);
    let media = project.clip(shown).unwrap().1.asset_id().unwrap();
    let media = project.assets[&media].clone();
    let tracker = project.trackers.values().next().unwrap().id;
    let follow = Follow {
        tracker_id: tracker,
        position: true,
        scale: false,
        rotation: false,
        offset: [0.0, 0.0],
    };
    project
        .tracks
        .insert(1, Arc::new(Track::new("Video 2", TrackKind::Video)));
    let mut other = clip(&media, 0.0, 10.0);
    other.follow = Some(follow.clone());
    push(&mut project, 1, other);
    clip_mut(&mut project, shown).follow = Some(follow);
    let plan = resolve(&project, seconds(5.0), &[]);
    for layer in &plan.layers {
        assert!(
            layer
                .transform
                .position
                .iter()
                .all(|value| value.is_finite())
        );
    }
    assert_eq!(plan, resolve(&project, seconds(5.0), &[]));
}

#[test]
fn follow_is_inert_while_the_tracked_asset_is_not_on_screen() {
    let (mut project, shown, title) = followed();
    let stored = project.clip(title).unwrap().1.transform.clone();
    let transform = |project: &Project, at: f64| {
        let plan = plan(project, at);
        plan.layers.last().unwrap().transform.clone()
    };
    assert_eq!(transform(&project, 1.0), stored);
    assert_eq!(transform(&project, 8.0), stored);
    assert_ne!(transform(&project, 5.0), stored);
    Arc::make_mut(&mut project.tracks[0]).hidden = true;
    assert_eq!(transform(&project, 5.0), stored);
    Arc::make_mut(&mut project.tracks[0]).hidden = false;

    // A copy of the asset on a higher track is the one that is read.
    let media = project.clip(shown).unwrap().1.asset_id().unwrap();
    let media = project.assets[&media].clone();
    project
        .tracks
        .insert(1, Arc::new(Track::new("Video 2", TrackKind::Video)));
    let mut upper = clip(&media, 4.0, 2.0);
    upper.source_in = seconds(14.0);
    push(&mut project, 1, upper);
    let top = transform(&project, 5.0);
    assert!((top.scale[0] - 2.0).abs() < 1e-4, "{top:?}");
    assert!((top.rotation - 30.0).abs() < 1e-3);
}

#[test]
fn a_layers_time_is_its_frames_start_from_the_clips_start() {
    let (project, _) = layered();
    let times = |at: Time| -> Vec<(Time, Time)> {
        resolve(&project, at, &[])
            .layers
            .iter()
            .map(|layer| {
                let clip = project
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter())
                    .find(|clip| clip.id == layer.clip)
                    .unwrap();
                (layer.time, clip.start)
            })
            .collect()
    };
    // 30 fps: frame 150 spans 5.000000 s up to 5.033333 s.
    let frame = times(seconds(5.0));
    assert!(!frame.is_empty());
    for (time, start) in &frame {
        assert_eq!(*time, seconds(5.0) - *start);
    }
    assert_eq!(frame, times(Time(5_033_332)));
    let next = times(Time(5_033_334));
    for ((before, _), (after, _)) in frame.iter().zip(&next) {
        assert_eq!(*after - *before, Time(33_333));
    }
}

#[test]
fn the_seed_is_stable_within_a_frame_and_changes_across_frames() {
    let (project, _) = layered();
    let seeds = |at: Time| -> Vec<u64> {
        resolve(&project, at, &[])
            .layers
            .iter()
            .map(|layer| layer.seed)
            .collect()
    };
    // 30 fps: frame 150 spans 5.000000 s up to 5.033333 s.
    let frame = seeds(seconds(5.0));
    assert_eq!(frame, seeds(Time(5_033_332)));
    assert_eq!(frame, seeds(seconds(5.0)));
    let next = seeds(Time(5_033_334));
    assert_eq!(frame.len(), next.len());
    assert!(frame.iter().zip(&next).all(|(a, b)| a != b));
    // Clips differ within a frame.
    assert!(
        frame
            .iter()
            .enumerate()
            .all(|(i, a)| frame[..i].iter().all(|b| a != b))
    );
    assert_eq!(
        resolve(&project, seconds(5.0), &[]),
        resolve(&project, seconds(5.0), &[])
    );
    // Time is not quantized: source positions move inside a frame.
    assert_ne!(
        resolve(&project, seconds(5.0), &[]),
        resolve(&project, Time(5_020_000), &[])
    );
}

#[test]
fn an_untouched_project_resolves_to_a_plain_plan() {
    let (project, _) = layered();
    let plan = plan(&project, 5.0);
    assert_eq!(plan.layers.len(), 4);
    assert!(plan.is_plain());
    for layer in &plan.layers {
        assert!(layer.is_plain());
        assert_eq!(layer.transform, Transform::default());
        assert_eq!(layer.blend, BlendMode::Normal);
    }
    let mut blended = project.clone();
    let id = plan.layers[0].clip;
    clip_mut(&mut blended, id).blend = BlendMode::Multiply;
    let plan = resolve(&blended, seconds(5.0), &[]);
    assert!(!plan.layers[0].is_plain() && !plan.is_plain());
    assert!(plan.layers[1].is_plain());
}

#[test]
fn resolve_never_panics_at_the_edges_of_time() {
    let empty = Project::new("Empty");
    for at in [Time::ZERO, Time::MAX, Time(i64::MAX), Time(i64::MIN)] {
        let plan = resolve(&empty, at, &[]);
        assert!(plan.layers.is_empty() && plan.is_plain());
    }
    let bare = Project {
        tracks: Vec::new(),
        ..Project::new("Bare")
    };
    assert!(resolve(&bare, Time::MAX, &[]).layers.is_empty());

    let (mut project, title, _) = {
        let (project, _, title) = followed();
        (project, title, ())
    };
    let media = project.assets.values().next().unwrap().clone();
    let mut late = clip(&media, 0.0, 4.0);
    late.start = Time::MAX - seconds(4.0);
    late.transition_in = Some(transition(TransitionKind::Glitch, 1.0));
    late.transition_out = Some(transition(TransitionKind::Blur, 1.0));
    push(&mut project, 0, late);
    clip_mut(&mut project, title).duration = Time::MAX;
    project.validate().unwrap();
    for at in [
        Time::ZERO,
        Time::MAX - seconds(4.0),
        Time::MAX - Time(1),
        Time::MAX,
        Time(i64::MAX),
        Time(i64::MIN),
    ] {
        let plan = resolve(&project, at, &[]);
        assert!(plan.layers.iter().all(|layer| layer.transform.validate()));
    }
}
