//! Transitions: the pass that makes one layer of two, read back from a real
//! device, and the targets and passes a job with one is counted at.
//!
//! The outgoing clip is red and the incoming one blue unless a test says
//! otherwise, both filling a 64×64 canvas over a dark one. Every kind is
//! held to its two ends, the outgoing clip alone at nought and the incoming
//! alone at one, and half way to a mix or a geometry worked out here from
//! its definition in `shaders/transition.wgsl`.
//!
//! As in `gpu_tests`, a test without an adapter prints `SKIP:` and passes.
use super::{
    ComposeBudget, Job, Ledger, Run,
    gpu::Gpu,
    gpu_tests::{
        AMBER, BLACK, BLUE, CANVAS, GREEN, Image, NIGHT, Pic, RED, WHITE, draw, encoded, gpu, job,
        linear, mask, quartered, solid,
    },
    pass::{self, Side},
    runs,
};
use editor_core::{
    MaskShape, ParamKey, ParamValue, RenderPlan, ResolvedTransition, TransitionKind, look,
};
use editor_media::Frame;

const KINDS: [TransitionKind; 14] = [
    TransitionKind::Fade,
    TransitionKind::Dissolve,
    TransitionKind::SlideLeft,
    TransitionKind::SlideRight,
    TransitionKind::SlideUp,
    TransitionKind::SlideDown,
    TransitionKind::ZoomIn,
    TransitionKind::ZoomOut,
    TransitionKind::WipeLeft,
    TransitionKind::WipeRight,
    TransitionKind::WipeUp,
    TransitionKind::WipeDown,
    TransitionKind::Glitch,
    TransitionKind::Blur,
];

/// The parameters of `kind` as `resolve` gives them: all of its table, those
/// in `set` as given.
fn params(kind: TransitionKind, set: &[(ParamKey, f32)]) -> Vec<ParamValue> {
    let spec = look::transition_spec(kind);
    for (key, value) in set {
        let known = spec.iter().find(|spec| spec.key == *key);
        assert!(
            known.is_some_and(|spec| spec.contains(*value)),
            "{kind:?} takes no {key:?} of {value}"
        );
    }
    (spec.iter())
        .map(|spec| ParamValue {
            key: spec.key,
            value: (set.iter().find(|(key, _)| *key == spec.key))
                .map_or(spec.default, |(_, value)| *value),
        })
        .collect()
}

impl Pic {
    /// The clip a transition leads out of, with no partner yet.
    fn leaving(mut self, kind: TransitionKind, progress: f32, set: &[(ParamKey, f32)]) -> Self {
        self.layer.transition = Some(ResolvedTransition {
            kind,
            progress,
            outgoing: true,
            partner: None,
            params: params(kind, set),
        });
        self
    }

    /// The clip a transition leads into, with no partner yet.
    fn arriving(mut self, kind: TransitionKind, progress: f32, set: &[(ParamKey, f32)]) -> Self {
        self.layer.transition = Some(ResolvedTransition {
            kind,
            progress,
            outgoing: false,
            partner: None,
            params: params(kind, set),
        });
        self
    }
}

/// `out` then `into` as the two layers of one transition, as `resolve` emits
/// them: on one track, each naming the other.
fn joined(
    out: Pic,
    into: Pic,
    kind: TransitionKind,
    progress: f32,
    set: &[(ParamKey, f32)],
) -> [Pic; 2] {
    let (mut out, mut into) = (
        out.leaving(kind, progress, set),
        into.arriving(kind, progress, set),
    );
    into.layer.track = out.layer.track;
    let (first, second) = (out.layer.clip, into.layer.clip);
    for (layer, partner) in [(&mut out.layer, second), (&mut into.layer, first)] {
        if let Some(transition) = &mut layer.transition {
            transition.partner = Some(partner);
        }
    }
    [out, into]
}

/// Red into blue, both filling the canvas.
fn pair(kind: TransitionKind, progress: f32, set: &[(ParamKey, f32)]) -> [Pic; 2] {
    joined(Pic::full(RED), Pic::full(BLUE), kind, progress, set)
}

/// Opaque colours summed in linear light, each by its share.
fn sum(parts: &[([u8; 4], f64)]) -> [u8; 4] {
    let channel = |c: usize| {
        encoded(
            parts
                .iter()
                .map(|(colour, share)| linear(colour[c]) * share)
                .sum(),
        )
    };
    [channel(0), channel(1), channel(2), 255]
}

/// Pixels well inside each quarter and at the centre.
const SPOTS: [(u32, u32); 5] = [(8, 8), (56, 8), (8, 56), (56, 56), (32, 32)];

#[test]
fn every_kind_is_the_outgoing_clip_at_nought_and_the_incoming_at_one() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    for kind in KINDS {
        for (progress, colour, which) in [(0.0, RED, "outgoing"), (1.0, BLUE, "incoming")] {
            let image = draw(&mut gpu, Some(NIGHT), &pair(kind, progress, &[]));
            for y in (1..64).step_by(6) {
                for x in (1..64).step_by(6) {
                    image.is(
                        x,
                        y,
                        colour,
                        &format!("{kind:?} at {progress} is the {which} clip alone"),
                    );
                }
            }
        }
    }
}

#[test]
fn a_dissolve_is_a_mix_in_linear_light() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::Dissolve, 0.5, &[]),
    );
    for (x, y) in SPOTS {
        image.is(x, y, sum(&[(RED, 0.5), (BLUE, 0.5)]), "half of each");
    }
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::Dissolve, 0.25, &[]),
    );
    image.is(32, 32, sum(&[(RED, 0.75), (BLUE, 0.25)]), "a quarter in");
}

#[test]
fn a_fade_between_two_clips_dips_to_what_is_under_them() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    for (progress, expected, what) in [
        (
            0.25,
            sum(&[(RED, 0.5), (NIGHT, 0.5)]),
            "the outgoing half gone",
        ),
        (0.5, NIGHT, "neither clip, half way"),
        (
            0.75,
            sum(&[(BLUE, 0.5), (NIGHT, 0.5)]),
            "the incoming half in",
        ),
    ] {
        let image = draw(
            &mut gpu,
            Some(NIGHT),
            &pair(TransitionKind::Fade, progress, &[]),
        );
        for (x, y) in SPOTS {
            image.is(x, y, expected, what);
        }
    }
    // Over a picture-in-picture stack what shows half way is the clip under
    // the two, not black.
    let [out, into] = pair(TransitionKind::Fade, 0.5, &[]);
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(GREEN), out, into]);
    image.is(32, 32, GREEN, "the layer under a fade shows through it");
}

#[test]
fn a_slide_pushes_the_outgoing_clip_off_the_canvas() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    // Each picture's quarters, left top, right top, left bottom, right
    // bottom: half way, half of each is on the canvas.
    let out = [RED, GREEN, AMBER, WHITE];
    let into = [BLUE, BLACK, NIGHT, [200, 60, 200, 255]];
    let half_way = |kind| {
        joined(
            Pic::of(quartered(out)),
            Pic::of(quartered(into)),
            kind,
            0.5,
            &[],
        )
    };
    // What the canvas's quarters show, in the same order.
    for (kind, expected) in [
        (
            TransitionKind::SlideLeft,
            [out[1], into[0], out[3], into[2]],
        ),
        (
            TransitionKind::SlideRight,
            [into[1], out[0], into[3], out[2]],
        ),
        (TransitionKind::SlideUp, [out[2], out[3], into[0], into[1]]),
        (
            TransitionKind::SlideDown,
            [into[2], into[3], out[0], out[1]],
        ),
    ] {
        let image = draw(&mut gpu, Some(NIGHT), &half_way(kind));
        for ((x, y), colour) in SPOTS.into_iter().zip(expected) {
            image.is(x, y, colour, &format!("{kind:?} half way"));
        }
    }
    // A quarter of the way the seam is a quarter in, and sharp.
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::SlideLeft, 0.25, &[]),
    );
    image.is(47, 32, RED, "the outgoing clip up to the seam");
    image.is(48, 32, BLUE, "the incoming clip from it");
}

#[test]
fn a_zoom_scales_both_clips_about_their_boxes_and_mixes_them() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let small = || Pic::full(RED).at([32.0, 32.0], [16.0, 16.0]);
    let both = sum(&[(RED, 0.5), (BLUE, 0.5)]);
    let amount = [(ParamKey::Amount, 1.0)];
    // In: half way the outgoing box is half as large again, 8 to 56, and
    // the incoming picture three quarters of its size, 8 to 56 as well.
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &joined(
            small(),
            Pic::full(BLUE),
            TransitionKind::ZoomIn,
            0.5,
            &amount,
        ),
    );
    image.is(32, 32, both, "both at the centre");
    image.is(
        12,
        32,
        both,
        "the outgoing box has grown past its edge at 16",
    );
    image.is(4, 32, NIGHT, "neither reaches the canvas's edge");
    // Out: the outgoing box is two thirds of its size, 21 to 43, and the
    // incoming picture a third larger than the canvas.
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &joined(
            small(),
            Pic::full(BLUE),
            TransitionKind::ZoomOut,
            0.5,
            &amount,
        ),
    );
    image.is(32, 32, both, "both at the centre");
    let incoming = sum(&[(BLUE, 0.5), (NIGHT, 0.5)]);
    image.is(18, 32, incoming, "the outgoing box has shrunk from 16");
    image.is(2, 2, incoming, "the incoming picture covers the canvas");
}

#[test]
fn a_wipe_crosses_the_canvas_the_way_it_is_named() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let sharp = [(ParamKey::Softness, 0.0)];
    // Half way a sharp edge is at the middle: the pixels either side of it,
    // the incoming clip named first.
    for (kind, incoming, outgoing) in [
        (TransitionKind::WipeLeft, (32, 20), (31, 20)),
        (TransitionKind::WipeRight, (31, 20), (32, 20)),
        (TransitionKind::WipeUp, (20, 32), (20, 31)),
        (TransitionKind::WipeDown, (20, 31), (20, 32)),
    ] {
        let image = draw(&mut gpu, Some(NIGHT), &pair(kind, 0.5, &sharp));
        image.is(
            incoming.0,
            incoming.1,
            BLUE,
            &format!("{kind:?} behind the edge"),
        );
        image.is(
            outgoing.0,
            outgoing.1,
            RED,
            &format!("{kind:?} ahead of the edge"),
        );
    }

    // A soft edge is a band: half the canvas wide here, from a quarter to
    // three quarters of the way along when the wipe is half done.
    let soft = 0.5;
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::WipeLeft, 0.5, &[(ParamKey::Softness, 0.5)]),
    );
    image.is(60, 32, BLUE, "behind the band");
    image.is(4, 32, RED, "ahead of the band");
    for x in [20u32, 31, 44] {
        let along = 1.0 - (f64::from(x) + 0.5) / 64.0;
        let ramp = ((0.5 * (1.0 + soft) - along) / soft).clamp(0.0, 1.0);
        let share = ramp * ramp * (3.0 - 2.0 * ramp);
        image.is(
            x,
            32,
            sum(&[(RED, 1.0 - share), (BLUE, share)]),
            "eased across the band",
        );
    }
}

#[test]
fn a_blur_softens_both_clips_most_half_way() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let wide = [(ParamKey::Radius, 0.25)];
    // Solid pictures blur to themselves: half way it is the even mix.
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::Blur, 0.5, &wide),
    );
    for (x, y) in SPOTS {
        image.is(x, y, sum(&[(RED, 0.5), (BLUE, 0.5)]), "half of each");
    }
    // One picture into itself shows the blur alone: white left of the
    // middle and black right of it, an edge a blur of deviation 8 spreads.
    let edge = || Pic::of(quartered([WHITE, BLACK, WHITE, BLACK]));
    let at = |gpu: &mut Gpu, progress| {
        draw(
            gpu,
            Some(NIGHT),
            &joined(edge(), edge(), TransitionKind::Blur, progress, &wide),
        )
    };
    let sharp = at(&mut gpu, 0.0);
    sharp.is(28, 32, WHITE, "no blur at the start");
    sharp.is(36, 32, BLACK, "no blur at the start");
    let soft = at(&mut gpu, 0.5);
    // The edge is at 32. A pixel centred 3.5 from it, 0.44 of a deviation,
    // has 67% of the light of the bright side, and one 4.5 past it has 29%.
    let (left, right) = (soft.at(28, 32)[0], soft.at(36, 32)[0]);
    let expected = (encoded(0.669), encoded(0.287));
    assert!(
        left.abs_diff(expected.0) <= 12 && right.abs_diff(expected.1) <= 12,
        "either side of a blurred edge: {left} and {right}, expected about {expected:?}"
    );
    soft.is(2, 32, WHITE, "far from the edge nothing changes");
}

#[test]
fn a_glitch_cuts_bands_from_one_clip_to_the_other() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let whole = [(ParamKey::Amount, 1.0)];
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::Glitch, 0.5, &whole),
    );
    // Rows are torn sideways, but a row of a solid picture is one colour
    // however far it is moved: every pixel is one clip or the other.
    let (mut red, mut blue) = (0, 0);
    for y in 0..64 {
        let first = image.at(0, y);
        let near = |colour: [u8; 4]| (first.iter().zip(&colour)).all(|(a, e)| a.abs_diff(*e) <= 2);
        assert!(
            near(RED) || near(BLUE),
            "row {y} is neither clip: {first:?}"
        );
        for x in 0..64 {
            image.is(x, y, first, "a row is one colour");
        }
        red += usize::from(near(RED));
        blue += usize::from(near(BLUE));
    }
    assert!(
        red > 8 && blue > 8,
        "half way some bands are of each clip: {red} red rows and {blue} blue"
    );
    // The same frame is the same picture, and another frame another.
    let again = draw(
        &mut gpu,
        Some(NIGHT),
        &pair(TransitionKind::Glitch, 0.5, &whole),
    );
    assert_eq!(image.0, again.0, "nothing but the plan decides the bands");
    let mut later = pair(TransitionKind::Glitch, 0.5, &whole);
    later[1].layer.seed = 8;
    assert_ne!(
        image.0,
        draw(&mut gpu, Some(NIGHT), &later).0,
        "the bands are drawn from the frame's seed"
    );
}

#[test]
fn a_glitch_tears_rows_and_parts_colours_only_between_its_ends() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    // One picture into itself, so that only the tearing shows: an edge
    // down the middle that a torn row moves.
    let edge = || Pic::of(quartered([WHITE, BLACK, WHITE, BLACK]));
    let at = |gpu: &mut Gpu, progress| {
        draw(
            gpu,
            Some(NIGHT),
            &joined(
                edge(),
                edge(),
                TransitionKind::Glitch,
                progress,
                &[(ParamKey::Amount, 1.0)],
            ),
        )
    };
    let whole = draw(&mut gpu, Some(NIGHT), &[edge()]);
    let differing = |image: &Image| {
        (image.0.iter().zip(&whole.0))
            .filter(|(a, b)| a.abs_diff(**b) > 2)
            .count()
    };
    assert_eq!(
        differing(&at(&mut gpu, 0.0)),
        0,
        "nothing is torn at the start"
    );
    assert_eq!(differing(&at(&mut gpu, 1.0)), 0, "nor at the end");
    assert!(differing(&at(&mut gpu, 0.5)) > 64, "rows are torn half way");
}

#[test]
fn a_clip_with_no_partner_comes_from_nothing_or_goes_to_it() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let half = |colour| sum(&[(colour, 0.5), (NIGHT, 0.5)]);
    // A fade with nothing to dip to is the plain fade a dissolve is.
    for kind in [TransitionKind::Fade, TransitionKind::Dissolve] {
        let image = draw(
            &mut gpu,
            Some(NIGHT),
            &[Pic::full(BLUE).arriving(kind, 0.5, &[])],
        );
        image.is(32, 32, half(BLUE), &format!("{kind:?} in from nothing"));
        let image = draw(
            &mut gpu,
            Some(NIGHT),
            &[Pic::full(RED).leaving(kind, 0.5, &[])],
        );
        image.is(32, 32, half(RED), &format!("{kind:?} out to nothing"));
    }
    for (progress, colour) in [(0.0, NIGHT), (1.0, BLUE)] {
        let incoming = Pic::full(BLUE).arriving(TransitionKind::Dissolve, progress, &[]);
        draw(&mut gpu, Some(NIGHT), &[incoming]).is(32, 32, colour, "the ends of a way in");
    }
    for (progress, colour) in [(0.0, RED), (1.0, NIGHT)] {
        let outgoing = Pic::full(RED).leaving(TransitionKind::Fade, progress, &[]);
        draw(&mut gpu, Some(NIGHT), &[outgoing]).is(32, 32, colour, "the ends of a way out");
    }
    // The geometry is the pair's, with nothing on the other side.
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(BLUE).arriving(TransitionKind::SlideLeft, 0.5, &[])],
    );
    image.is(8, 32, NIGHT, "nothing is pushed out");
    image.is(56, 32, BLUE, "the clip slides in from the right");
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(RED).leaving(TransitionKind::WipeLeft, 0.5, &[(ParamKey::Softness, 0.0)])],
    );
    image.is(8, 32, RED, "ahead of the edge the clip is still there");
    image.is(56, 32, NIGHT, "behind it there is nothing");
}

#[test]
fn a_clip_whose_partner_has_no_picture_yet_is_drawn_against_nothing() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    // The incoming clip's first frame is not decoded: the outgoing clip
    // keeps its half of the transition, and nothing stands in for the other.
    let [out, _] = pair(TransitionKind::Dissolve, 0.25, &[]);
    let image = draw(&mut gpu, Some(NIGHT), &[out]);
    image.is(
        32,
        32,
        sum(&[(RED, 0.75), (NIGHT, 0.25)]),
        "three quarters left",
    );
    // A fade is still the dip it is between two clips.
    let [_, into] = pair(TransitionKind::Fade, 0.5, &[]);
    let image = draw(&mut gpu, Some(NIGHT), &[into]);
    image.is(32, 32, NIGHT, "the incoming clip has not begun to fade in");
    let [_, into] = pair(TransitionKind::Fade, 0.75, &[]);
    let image = draw(&mut gpu, Some(NIGHT), &[into]);
    image.is(32, 32, sum(&[(BLUE, 0.5), (NIGHT, 0.5)]), "half in");
}

#[test]
fn each_clip_of_a_transition_keeps_its_opacity_masks_and_place() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    // The outgoing clip at half opacity, before the transition has moved.
    let [mut out, into] = pair(TransitionKind::Dissolve, 0.0, &[]);
    out.layer.transform.opacity = 0.5;
    let image = draw(&mut gpu, Some(NIGHT), &[out, into]);
    image.is(32, 32, sum(&[(RED, 0.5), (NIGHT, 0.5)]), "half opacity");

    // The outgoing clip masked to its left half: its chain is drawn on its
    // own targets while the incoming clip's is, and each is then mixed.
    let left = mask(MaskShape::Rectangle, [0.25, 0.5], [0.5, 1.0]);
    let [out, into] = joined(
        Pic::full(RED).masked(left),
        Pic::full(BLUE),
        TransitionKind::Dissolve,
        0.5,
        &[],
    );
    let image = draw(&mut gpu, Some(NIGHT), &[out, into]);
    image.is(8, 32, sum(&[(RED, 0.5), (BLUE, 0.5)]), "inside the mask");
    image.is(56, 32, sum(&[(BLUE, 0.5), (NIGHT, 0.5)]), "outside it");

    // Two pairs on two tracks, the upper a quarter of the canvas: each is
    // one layer over what is under it.
    let [low_out, low_in] = pair(TransitionKind::Dissolve, 0.5, &[]);
    let [high_out, high_in] = joined(
        Pic::full(GREEN).at([48.0, 16.0], [16.0, 16.0]),
        Pic::full(WHITE).at([48.0, 16.0], [16.0, 16.0]),
        TransitionKind::WipeLeft,
        0.5,
        &[(ParamKey::Softness, 0.0)],
    );
    let image = draw(&mut gpu, Some(NIGHT), &[low_out, low_in, high_out, high_in]);
    image.is(8, 56, sum(&[(RED, 0.5), (BLUE, 0.5)]), "the lower pair");
    // A wipe crosses the canvas, so half way its edge is at the canvas's
    // middle: the upper box, right of it, is all the incoming clip.
    image.is(48, 16, WHITE, "the upper pair over the lower");
}

#[test]
fn the_transition_pass_is_told_its_kind_and_its_two_sides() {
    let transition = |kind, partner: bool, set: &[(ParamKey, f32)]| ResolvedTransition {
        kind,
        progress: 0.5,
        outgoing: false,
        partner: partner.then(uuid::Uuid::new_v4),
        params: params(kind, set),
    };
    let sides = (
        Some(Side {
            opacity: 0.5,
            centre: [10.0, 20.0],
        }),
        Some(Side {
            opacity: 1.0,
            centre: [30.0, 40.0],
        }),
    );
    let words = |uniform: pass::Uniform| -> Vec<f32> {
        (uniform.bytes().chunks_exact(4))
            .map(|word| f32::from_ne_bytes([word[0], word[1], word[2], word[3]]))
            .collect()
    };
    let kind_of = |uniform: &pass::Uniform| {
        let bytes = uniform.bytes();
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    };
    let made = |kind, partner, set: &[(ParamKey, f32)]| {
        pass::transition(CANVAS, &transition(kind, partner, set), 7, sides.0, sides.1)
    };

    // A fade dips only between two clips; a blur is a dissolve of layers
    // their chains blurred.
    assert_eq!(
        kind_of(&made(TransitionKind::Fade, true, &[])),
        pass::KIND_DIP
    );
    assert_eq!(
        kind_of(&made(TransitionKind::Fade, false, &[])),
        pass::KIND_DISSOLVE
    );
    assert_ne!(
        made(TransitionKind::Fade, true, &[]),
        made(TransitionKind::Dissolve, true, &[]),
        "a fade and a dissolve between two clips are not one thing"
    );
    for kind in [TransitionKind::Dissolve, TransitionKind::Blur] {
        assert_eq!(kind_of(&made(kind, true, &[])), pass::KIND_DISSOLVE);
    }
    for (kind, expected) in [
        (TransitionKind::SlideUp, pass::KIND_SLIDE),
        (TransitionKind::ZoomOut, pass::KIND_ZOOM),
        (TransitionKind::WipeDown, pass::KIND_WIPE),
        (TransitionKind::Glitch, pass::KIND_GLITCH),
    ] {
        assert_eq!(kind_of(&made(kind, true, &[])), expected, "{kind:?}");
    }

    // Twelve words of header, then p[0], p[1] and p[2].
    let wipe = words(made(
        TransitionKind::WipeLeft,
        true,
        &[(ParamKey::Softness, 0.25)],
    ));
    assert_eq!(wipe[12..16], [0.5, 0.25, 0.5, 1.0]);
    assert_eq!(wipe[16..20], [-1.0, 0.0, 1.0, 1.0]);
    assert_eq!(wipe[20..24], [10.0, 20.0, 30.0, 40.0]);
    let zoom = words(made(
        TransitionKind::ZoomIn,
        true,
        &[(ParamKey::Amount, 1.0)],
    ));
    assert_eq!(zoom[16..20], [0.0, 0.0, 1.5, 0.75]);
    let zoom = words(made(
        TransitionKind::ZoomOut,
        true,
        &[(ParamKey::Amount, 1.0)],
    ));
    assert_eq!(zoom[16..20], [0.0, 0.0, 1.0 / 1.5, 2.0 / 1.5]);
    // A side that is not there is drawn at no opacity.
    let alone = pass::transition(
        CANVAS,
        &transition(TransitionKind::Dissolve, false, &[]),
        7,
        None,
        sides.1,
    );
    assert_eq!(words(alone)[12..16], [0.5, 0.0, 0.0, 1.0]);
}

fn frames<'a>(pics: &'a [Pic]) -> Job<'a> {
    job(Some(NIGHT), pics)
}

#[test]
fn a_transition_is_counted_in_the_targets_and_passes_of_its_job() {
    // The layers are told apart by the plan, with no device.
    let plain = [Pic::full(RED)];
    assert_eq!((frames(&plain).slots(), frames(&plain).passes()), (1, 3));

    // Two plain layers of a pair: one target each and one for the two as
    // one. Place twice, the transition, one blend, and the encoding.
    let pics = pair(TransitionKind::Dissolve, 0.5, &[]);
    assert!(pass::paired(&pics[0].layer, &pics[1].layer));
    assert!(!pass::paired(&pics[1].layer, &pics[0].layer));
    let job = frames(&pics);
    assert_eq!(job.groups().len(), 1);
    assert_eq!(job.slots(), 3);
    assert!(job.passes() >= 5, "{}", job.passes());

    // A clip by itself needs a second target for its transition.
    let alone = [Pic::full(BLUE).arriving(TransitionKind::WipeUp, 0.5, &[])];
    assert_eq!(frames(&alone).slots(), 2);
    assert!(frames(&alone).passes() >= 4);
    // So does the clip of a pair whose partner has no picture.
    let [out, _] = pair(TransitionKind::Dissolve, 0.5, &[]);
    assert_eq!(frames(&[out]).slots(), 2);

    // A blur gives each chain the passes and targets of a blur effect.
    let pics = pair(TransitionKind::Blur, 0.5, &[]);
    for pic in &pics {
        let chain = pass::chain(&pic.layer, &pic.place, [1920, 1080], &[]);
        assert!(!chain.is_empty() && chain.len() <= pass::chain_len(&pic.layer));
        assert_eq!(pass::chain_slots(&pic.layer), pass::SLOTS);
    }
    assert_eq!(frames(&pics).slots(), 2 * pass::SLOTS);
    // At its ends there is nothing to blur, and nothing is drawn for it.
    for progress in [0.0, 1.0] {
        let [out, _] = pair(TransitionKind::Blur, progress, &[]);
        assert!(pass::chain(&out.layer, &out.place, CANVAS, &[]).is_empty());
    }

    // Clips of two tracks, or two clips that do not name each other, are
    // not a pair.
    let [out, mut into] = pair(TransitionKind::Dissolve, 0.5, &[]);
    into.layer.track = uuid::Uuid::new_v4();
    assert!(!pass::paired(&out.layer, &into.layer));
    let strangers = [
        Pic::full(RED).leaving(TransitionKind::Fade, 0.5, &[]),
        Pic::full(BLUE).arriving(TransitionKind::Fade, 0.5, &[]),
    ];
    assert_eq!(frames(&strangers).groups().len(), 2);
}

#[test]
fn a_plan_with_a_transition_is_composited_and_fits_the_budget_at_full_size() {
    let pics = pair(TransitionKind::Dissolve, 0.5, &[]);
    let plan = RenderPlan {
        canvas: [1920, 1080],
        background: NIGHT,
        layers: pics.iter().map(|pic| pic.layer.clone()).collect(),
    };
    assert!(!plan.is_plain());
    assert_eq!(runs(&plan), vec![Run::Composite(0..2)]);

    // Two full pictures in, five canvases between the passes and one out.
    let full = |colour| Frame {
        width: 1920,
        height: 1080,
        ..solid(colour, [1, 1])
    };
    let mut pics = pair(TransitionKind::Dissolve, 0.5, &[]);
    pics[0].frame = full(RED);
    pics[1].frame = full(BLUE);
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    ledger.begin_frame();
    let turn = ledger.turn([1920, 1080], &[frames(&pics)]);
    assert_eq!((turn.size, turn.targets), ([1920, 1080], 5));
    assert_eq!(ledger.bytes(), 8 * 1920 * 1080 * 4);
    assert!(ledger.bytes() <= ComposeBudget::PRODUCTION.bytes);
}

#[test]
fn a_paused_transition_is_composited_once() {
    let mut pics = pair(TransitionKind::Glitch, 0.5, &[]);
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    for _ in 0..3 {
        ledger.begin_frame();
        ledger.turn(CANVAS, &[frames(&pics)]);
    }
    let counters = ledger.counters();
    assert_eq!((counters.composed, counters.skipped), (1, 2));
    // Progress is the plan's: when it moves the canvas is drawn again.
    for pic in &mut pics {
        if let Some(transition) = &mut pic.layer.transition {
            transition.progress = 0.6;
        }
    }
    ledger.begin_frame();
    ledger.turn(CANVAS, &[frames(&pics)]);
    assert_eq!(ledger.counters().composed, 2);
}
