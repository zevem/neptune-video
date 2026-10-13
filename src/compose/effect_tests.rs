//! The effects' pixels, read back from a real device, and the chains that
//! draw them.
//!
//! Each effect is held to something that follows from its definition and
//! can be worked out by hand: a stop of exposure doubles the light, a blur
//! keeps the mean, the key colour is clear. Pictures are 64 pixels square,
//! one texel a pixel, so an edge in one is an edge on the canvas.
//!
//! As in `gpu_tests`, a test without an adapter prints `SKIP:` and passes.
use super::{
    ComposeBudget,
    gpu::Gpu,
    gpu_tests::{
        AMBER, BLACK, BLUE, CANVAS, GREEN, Image, NIGHT, Pic, RED, WHITE, composer, draw, encoded,
        gpu, job, linear, mask, quartered,
    },
    pass::{self, Second},
};
use editor_core::{
    EffectKind, LookPreset, MaskShape, ParamKey, ParamValue, ResolvedEffect, Time, look,
};
use editor_media::Frame;
use uuid::Uuid;

const SIDE: u32 = 64;

/// An effect with every parameter of its kind, those in `set` as given and
/// the rest as its table has them.
fn fx(kind: EffectKind, set: &[(ParamKey, f32)]) -> ResolvedEffect {
    let spec = look::spec(kind);
    for (key, value) in set {
        let known = spec.iter().find(|spec| spec.key == *key);
        assert!(
            known.is_some_and(|spec| spec.contains(*value)),
            "{kind:?} takes no {key:?} of {value}"
        );
    }
    ResolvedEffect {
        id: Uuid::new_v4(),
        kind,
        mix: 1.0,
        params: (spec.iter())
            .map(|spec| ParamValue {
                key: spec.key,
                value: (set.iter().find(|(key, _)| *key == spec.key))
                    .map_or(spec.default, |(_, value)| *value),
            })
            .collect(),
        points: Vec::new(),
        asset_id: None,
    }
}

fn curve(kind: EffectKind, points: &[[f32; 2]]) -> ResolvedEffect {
    ResolvedEffect {
        points: points.to_vec(),
        ..fx(kind, &[])
    }
}

fn mixed(mut effect: ResolvedEffect, mix: f32) -> ResolvedEffect {
    effect.mix = mix;
    effect
}

impl Pic {
    fn with(mut self, effect: ResolvedEffect) -> Self {
        self.layer.effects.push(effect);
        self
    }

    fn seeded(mut self, seed: u64) -> Self {
        self.layer.seed = seed;
        self
    }

    fn timed(mut self, time: Time) -> Self {
        self.layer.time = time;
        self
    }
}

/// A picture of the canvas's size, a texel a pixel.
fn painted(colour: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            rgba.extend_from_slice(&colour(x, y));
        }
    }
    Frame {
        width: SIDE,
        height: SIDE,
        rgba: rgba.into(),
    }
}

fn grey(code: u8) -> [u8; 4] {
    [code, code, code, 255]
}

/// The left half one colour and the right half another, with nothing
/// between.
fn halves(left: [u8; 4], right: [u8; 4]) -> Frame {
    painted(|x, _| if x < SIDE / 2 { left } else { right })
}

fn stacked(top: [u8; 4], bottom: [u8; 4]) -> Frame {
    painted(|_, y| if y < SIDE / 2 { top } else { bottom })
}

/// One channel of every colour worked out in linear light.
fn lit(colour: [u8; 4], light: impl Fn(usize, f64) -> f64) -> [u8; 4] {
    let channel = |c: usize| encoded(light(c, linear(colour[c])));
    [channel(0), channel(1), channel(2), colour[3]]
}

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

impl Image {
    /// The light of the red channel along a row, pixel by pixel.
    fn row(&self, y: u32) -> Vec<f64> {
        (0..SIDE).map(|x| linear(self.at(x, y)[0])).collect()
    }

    fn alpha(&self) -> f64 {
        self.0
            .chunks_exact(4)
            .map(|texel| f64::from(texel[3]))
            .sum()
    }

    /// How many pixels are not what they are in `other`.
    fn unlike(&self, other: &Image) -> usize {
        (self.0.chunks_exact(4).zip(other.0.chunks_exact(4)))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
            .count()
    }

    #[track_caller]
    fn same_as(&self, other: &Image, what: &str) {
        let worst = (self.0.iter().zip(&other.0))
            .map(|(a, b)| a.abs_diff(*b))
            .max();
        assert!(worst <= Some(1), "{what}: a channel is {worst:?} codes off");
    }
}

/// A picture of many colours, to hold an effect that should do nothing to.
fn swatches() -> Frame {
    painted(|x, y| {
        [
            (x * 4) as u8,
            (y * 4) as u8,
            255 - (x * 2 + y * 2) as u8,
            255,
        ]
    })
}

#[test]
fn effects_at_their_neutral_settings_change_nothing() {
    let Some(mut gpu) = gpu() else { return };
    let plain = draw(&mut gpu, Some(NIGHT), &[Pic::of(swatches())]);
    let neutral = [
        fx(EffectKind::Grade, &[]),
        fx(
            EffectKind::HslBand,
            &[(ParamKey::Hue, 120.0), (ParamKey::HueWidth, 180.0)],
        ),
        curve(EffectKind::CurveMaster, &[[0.0, 0.0], [1.0, 1.0]]),
        curve(
            EffectKind::CurveGreen,
            &[[0.0, 0.0], [0.3, 0.3], [0.8, 0.8], [1.0, 1.0]],
        ),
        // A key that takes in no colour but its own, and no spill.
        fx(
            EffectKind::ChromaKey,
            &[
                (ParamKey::Tolerance, 0.0),
                (ParamKey::Softness, 0.0),
                (ParamKey::Spill, 0.0),
            ],
        ),
        // Past the corners, nothing over the threshold, a shadow under a
        // layer with nothing clear in it, and a blur not mixed in.
        fx(EffectKind::Vignette, &[(ParamKey::Radius, 2.0)]),
        fx(EffectKind::Glow, &[(ParamKey::Threshold, 1.0)]),
        fx(
            EffectKind::Shadow,
            &[(ParamKey::Amount, 1.0), (ParamKey::Radius, 0.05)],
        ),
        mixed(fx(EffectKind::Blur, &[(ParamKey::Radius, 0.2)]), 0.0),
    ];
    for effect in neutral {
        let kind = effect.kind;
        let pic = Pic::of(swatches()).with(effect);
        assert!(
            !pass::chain(&pic.layer, &pic.place, CANVAS, &[]).is_empty(),
            "{kind:?} is drawn"
        );
        let image = draw(&mut gpu, Some(NIGHT), &[pic]);
        image.same_as(&plain, &format!("{kind:?} set to do nothing"));
    }
    // And those that are drawn by no pass when they are set to nothing.
    for kind in [
        EffectKind::Grain,
        EffectKind::Vignette,
        EffectKind::Blur,
        EffectKind::Glow,
        EffectKind::Sharpen,
        EffectKind::Shadow,
        EffectKind::MotionBlur,
        EffectKind::Glitch,
        EffectKind::Distort,
        EffectKind::Shake,
    ] {
        let key = if kind == EffectKind::Blur {
            ParamKey::Radius
        } else {
            ParamKey::Amount
        };
        let pic = Pic::of(swatches()).with(fx(kind, &[(key, 0.0)]));
        assert!(
            pass::chain(&pic.layer, &pic.place, CANVAS, &[]).is_empty(),
            "{kind:?} at nought costs no pass"
        );
    }
}

#[test]
fn a_grade_is_its_arithmetic_on_linear_light() {
    let Some(mut gpu) = gpu() else { return };
    let mut graded = |colour: [u8; 4], set: &[(ParamKey, f32)]| {
        let pic = Pic::full(colour).with(fx(EffectKind::Grade, set));
        draw(&mut gpu, Some(NIGHT), &[pic]).at(32, 32)
    };
    #[track_caller]
    fn near(actual: [u8; 4], expected: [u8; 4], what: &str) {
        assert!(
            (actual.iter().zip(&expected)).all(|(a, e)| a.abs_diff(*e) <= 2),
            "{what}: {actual:?}, expected {expected:?}"
        );
    }

    // A stop is twice the light, and a stop down half of it.
    let up = graded(grey(100), &[(ParamKey::Exposure, 1.0)]);
    near(up, lit(grey(100), |_, x| 2.0 * x), "a stop up");
    let down = graded(grey(188), &[(ParamKey::Exposure, -1.0)]);
    near(down, grey(137), "a stop down from half is a quarter");

    // Without saturation every channel is the colour's brightness.
    let light: f64 = (0..3).map(|c| LUMA[c] * linear(RED[c])).sum();
    let flat = graded(RED, &[(ParamKey::Saturation, 0.0)]);
    near(flat, lit(RED, |_, _| light), "no saturation");
    // And twice the saturation is as far again from it.
    let vivid = graded(RED, &[(ParamKey::Saturation, 2.0)]);
    near(
        vivid,
        lit(RED, |_, x| light + 2.0 * (x - light)),
        "twice the saturation",
    );

    // Contrast is a power about mid grey: 0.18 stays, and at +1 the rest
    // is squared away from it.
    let middle = encoded(0.18);
    let held = graded(grey(middle), &[(ParamKey::Contrast, 1.0)]);
    near(held, grey(middle), "mid grey under contrast");
    let apart = graded(grey(150), &[(ParamKey::Contrast, 1.0)]);
    near(
        apart,
        lit(grey(150), |_, x| 0.18 * (x / 0.18).powi(2)),
        "contrast above mid grey",
    );
    let closer = graded(grey(60), &[(ParamKey::Contrast, -1.0)]);
    near(
        closer,
        lit(grey(60), |_, x| 0.18 * (x / 0.18).sqrt()),
        "less contrast under mid grey",
    );

    // Lift raises black and leaves white, gain scales, and a gamma of two
    // is the square root: above one lifts the middle.
    let lifted = graded(BLACK, &[(ParamKey::Lift, 0.1)]);
    near(lifted, lit(BLACK, |_, _| 0.1), "black, lifted");
    let white = graded(WHITE, &[(ParamKey::Lift, 0.1)]);
    near(white, WHITE, "white, lifted");
    let halved = graded(WHITE, &[(ParamKey::GainLuma, 0.5)]);
    near(halved, grey(188), "half the gain");
    let rooted = graded(grey(100), &[(ParamKey::Gamma, 2.0)]);
    near(rooted, lit(grey(100), |_, x| x.sqrt()), "a gamma of two");
    assert!(rooted[0] > 100, "a gamma above one lifts the middle");

    // Temperature and tint are gains that leave a grey as bright as it was.
    let balance = |temperature: f64, tint: f64| {
        let gains = [
            (0.5 * temperature).exp2(),
            (-0.5 * tint).exp2(),
            (-0.5 * temperature).exp2(),
        ];
        let white: f64 = (0..3).map(|c| LUMA[c] * gains[c]).sum();
        gains.map(|gain| gain / white)
    };
    let warm = graded(grey(128), &[(ParamKey::Temperature, 1.0)]);
    let gains = balance(1.0, 0.0);
    near(warm, lit(grey(128), |c, x| x * gains[c]), "warmer");
    assert!(
        warm[0] > warm[1] && warm[1] > warm[2],
        "warm is red: {warm:?}"
    );
    let after: f64 = (0..3).map(|c| LUMA[c] * linear(warm[c])).sum();
    assert!(
        (after - linear(128)).abs() < 0.01,
        "as bright as before: {after}"
    );
    let cool = graded(grey(128), &[(ParamKey::Temperature, -1.0)]);
    assert!(
        cool[2] > cool[1] && cool[1] > cool[0],
        "cool is blue: {cool:?}"
    );
    let magenta = graded(grey(128), &[(ParamKey::Tint, 1.0)]);
    let gains = balance(0.0, 1.0);
    near(
        magenta,
        lit(grey(128), |c, x| x * gains[c]),
        "towards magenta",
    );
    assert!(magenta[1] < magenta[0] && magenta[0] == magenta[2]);
}

#[test]
fn an_effect_is_mixed_in_by_its_mix_and_stacked_in_its_order() {
    let Some(mut gpu) = gpu() else { return };
    // Half of a stop up is half way from the light to twice the light.
    let half = mixed(fx(EffectKind::Grade, &[(ParamKey::Exposure, 1.0)]), 0.5);
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(grey(100)).with(half)]);
    image.is(32, 32, lit(grey(100), |_, x| 1.5 * x), "half a stop's mix");

    // Twice the light and then a lift is not a lift and then twice.
    let stop = || fx(EffectKind::Grade, &[(ParamKey::Exposure, 1.0)]);
    let lift = || fx(EffectKind::Grade, &[(ParamKey::Lift, 0.2)]);
    let first = Pic::full(grey(100)).with(stop()).with(lift());
    let image = draw(&mut gpu, Some(NIGHT), &[first]);
    let expected = lit(grey(100), |_, x| 2.0 * x + 0.2 * (1.0 - 2.0 * x));
    image.is(32, 32, expected, "exposure, then lift");
    let second = Pic::full(grey(100)).with(lift()).with(stop());
    let image = draw(&mut gpu, Some(NIGHT), &[second]);
    let expected = lit(grey(100), |_, x| 2.0 * (x + 0.2 * (1.0 - x)));
    image.is(32, 32, expected, "lift, then exposure");

    // An effect that needs the layer again mixes over the layer too.
    let blur = || fx(EffectKind::Blur, &[(ParamKey::Radius, 0.1)]);
    let whole = Pic::of(halves(WHITE, BLACK)).with(blur());
    let whole = draw(&mut gpu, Some(NIGHT), &[whole]).row(32);
    let part = Pic::of(halves(WHITE, BLACK)).with(mixed(blur(), 0.5));
    let part = draw(&mut gpu, Some(NIGHT), &[part]).row(32);
    for x in [26, 30, 31] {
        let expected = 0.5 + 0.5 * whole[x];
        assert!(
            (part[x] - expected).abs() < 0.02,
            "half a blur at x = {x}: {} for {expected}",
            part[x]
        );
    }

    // A mask cuts what the stack made: its edge is not blurred with it.
    let rect = mask(MaskShape::Rectangle, [0.5, 0.5], [0.5, 0.5]);
    let masked = Pic::full(WHITE).with(blur()).masked(rect);
    let image = draw(&mut gpu, Some(NIGHT), &[masked]);
    image.is(16, 32, WHITE, "inside the mask's edge");
    image.is(15, 32, NIGHT, "outside the mask's edge");
}

#[test]
fn a_band_of_hues_is_shifted_and_the_rest_is_left() {
    let Some(mut gpu) = gpu() else { return };
    let mut banded = |colour: [u8; 4], set: &[(ParamKey, f32)]| {
        let pic = Pic::full(colour).with(fx(EffectKind::HslBand, set));
        draw(&mut gpu, Some(NIGHT), &[pic])
    };
    let red = [255, 0, 0, 255];
    let third = [(ParamKey::Hue, 0.0), (ParamKey::HueShift, 120.0)];
    banded(red, &third).is(32, 32, [0, 255, 0, 255], "red, a third of a turn on");
    let blue = [0, 0, 255, 255];
    banded(blue, &third).is(32, 32, blue, "blue is not in a band about red");
    banded(grey(128), &third).is(32, 32, grey(128), "a grey has no hue");

    // No saturation is the grey of its lightness: half, gamma-encoded.
    let flat = [(ParamKey::Hue, 0.0), (ParamKey::SatShift, -1.0)];
    banded(red, &flat).is(32, 32, grey(128), "red without saturation");
    // Half way to white from a lightness of a half is three quarters.
    let light = [(ParamKey::Hue, 0.0), (ParamKey::LumShift, 0.5)];
    banded(red, &light).is(32, 32, [255, 128, 128, 255], "red, lighter");
    let dark = [(ParamKey::Hue, 0.0), (ParamKey::LumShift, -0.5)];
    banded(red, &dark).is(32, 32, [128, 0, 0, 255], "red, darker");

    // A wholly soft band 60° wide is half as strong 15° from its centre:
    // an orange at 15° goes 60° on, to 75°.
    let soft = [
        (ParamKey::Hue, 0.0),
        (ParamKey::HueWidth, 60.0),
        (ParamKey::Softness, 1.0),
        (ParamKey::HueShift, 120.0),
    ];
    let image = banded([255, 64, 0, 255], &soft);
    image.is(32, 32, [192, 255, 0, 255], "half way down a soft band");
    // And with no softness it is whole up to its edge and nothing past it.
    let hard = [
        (ParamKey::Hue, 0.0),
        (ParamKey::HueWidth, 60.0),
        (ParamKey::Softness, 0.0),
        (ParamKey::HueShift, 120.0),
    ];
    banded([255, 64, 0, 255], &hard).is(32, 32, [0, 255, 64, 255], "inside a hard band");
    let yellow = [255, 255, 0, 255];
    banded(yellow, &hard).is(32, 32, yellow, "outside a hard band");
}

#[test]
fn a_curve_goes_through_its_points_on_the_channels_it_is_for() {
    let Some(mut gpu) = gpu() else { return };
    let colour = [64, 128, 200, 255];
    // Two points are a straight line: a quarter and half the value.
    let line = [[0.0, 0.25], [1.0, 0.75]];
    let on = |code: u8| (63.75 + 0.5 * f64::from(code)).round() as u8;
    let mut curved = |kind: EffectKind, points: &[[f32; 2]]| {
        let pic = Pic::full(colour).with(curve(kind, points));
        draw(&mut gpu, Some(NIGHT), &[pic])
    };
    let all = [on(64), on(128), on(200), 255];
    curved(EffectKind::CurveMaster, &line).is(32, 32, all, "every channel");
    let red = [on(64), 128, 200, 255];
    curved(EffectKind::CurveRed, &line).is(32, 32, red, "red alone");
    let green = [64, on(128), 200, 255];
    curved(EffectKind::CurveGreen, &line).is(32, 32, green, "green alone");
    let blue = [64, 128, on(200), 255];
    curved(EffectKind::CurveBlue, &line).is(32, 32, blue, "blue alone");

    // Through a point in the middle, and flat outside the points it has.
    let bent = [[0.0, 0.0], [0.502, 0.75], [1.0, 1.0]];
    let image = curved(EffectKind::CurveGreen, &bent);
    image.is(32, 32, [64, 191, 200, 255], "through its middle point");
    let short = [[0.4, 0.2], [0.6, 0.9]];
    let image = curved(EffectKind::CurveMaster, &short);
    // Between them the line from 0.2 to 0.9: 0.557 at 128.
    image.is(
        32,
        32,
        [51, 142, 230, 255],
        "flat before, a line, flat after",
    );
}

/// The curve as `effect.wgsl` draws it: the one the inspector's editor draws.
fn curve_at(points: &[[f32; 2]], v: f32) -> f32 {
    pass::curve_at(points, &pass::slopes(points), v)
}

#[test]
fn the_curve_the_editor_draws_is_the_one_the_shader_evaluates() {
    let Some(mut gpu) = gpu() else { return };
    let bent = [
        [0.0, 0.05],
        [0.2, 0.1],
        [0.45, 0.7],
        [0.8, 0.85],
        [1.0, 0.95],
    ];
    for code in [0u8, 20, 51, 80, 115, 140, 180, 204, 230, 255] {
        let pic = Pic::full([code, code, code, 255]).with(curve(EffectKind::CurveMaster, &bent));
        let drawn = curve_at(&bent, f32::from(code) / 255.0);
        let on = (f64::from(drawn) * 255.0).round().clamp(0.0, 255.0) as u8;
        draw(&mut gpu, Some(NIGHT), &[pic]).is(32, 32, [on, on, on, 255], "a grey on the curve");
    }
}

#[test]
fn a_curve_never_turns_back_between_points_that_do_not() {
    let rising = [
        [0.0, 0.0],
        [0.1, 0.02],
        [0.15, 0.6],
        [0.5, 0.61],
        [0.9, 0.95],
        [1.0, 1.0],
    ];
    let mut before = curve_at(&rising, 0.0);
    for step in 1..=1000 {
        let now = curve_at(&rising, step as f32 / 1000.0);
        assert!(
            now >= before - 1e-6,
            "falls at {step}: {now} after {before}"
        );
        before = now;
    }
    for point in rising {
        assert!((curve_at(&rising, point[0]) - point[1]).abs() < 1e-5);
    }
    // A peak is flat at its top, and the line through two points is a line.
    let peak = [[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]];
    assert_eq!(pass::slopes(&peak)[1], 0.0);
    let line = [[0.0, 0.25], [1.0, 0.75]];
    assert_eq!(pass::slopes(&line), vec![0.5, 0.5]);
    assert!((curve_at(&line, 0.3) - 0.4).abs() < 1e-6);
}

#[test]
fn a_chroma_key_clears_its_colour_and_keeps_one_far_from_it() {
    let Some(mut gpu) = gpu() else { return };
    let key = [0, 255, 0, 255];
    let keyed = |set: &[(ParamKey, f32)]| fx(EffectKind::ChromaKey, set);
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(key).with(keyed(&[]))]);
    image.is(32, 32, NIGHT, "the key colour");
    let image = draw(&mut gpu, None, &[Pic::full(key).with(keyed(&[]))]);
    image.is(32, 32, [0, 0, 0, 0], "the key colour over nothing");
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).with(keyed(&[]))]);
    image.is(32, 32, RED, "a colour far from the key");

    // A green near the key, inside the tolerance and outside a tighter one.
    let near = [30, 230, 40, 255];
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(near).with(keyed(&[]))]);
    image.is(32, 32, NIGHT, "a colour within the tolerance");
    let tight = [(ParamKey::Tolerance, 0.02), (ParamKey::Softness, 0.02)];
    let pic = Pic::full(near).with(keyed(&[tight[0], tight[1], (ParamKey::Spill, 0.0)]));
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(32, 32, near, "the same colour past a tighter one");

    // Spill takes the key's channel down to the greater of the others.
    let tinged = [120, 200, 110, 255];
    let set = |spill: f32| {
        [
            (ParamKey::Tolerance, 0.1),
            (ParamKey::Softness, 0.1),
            (ParamKey::Spill, spill),
        ]
    };
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(tinged).with(keyed(&set(1.0)))],
    );
    image.is(32, 32, [120, 120, 110, 255], "all the spill taken");
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(tinged).with(keyed(&set(0.5)))],
    );
    image.is(32, 32, [120, 160, 110, 255], "half of it");

    // Another key: blue is cleared and green is kept.
    let blue = [
        (ParamKey::KeyR, 0.0),
        (ParamKey::KeyG, 0.0),
        (ParamKey::KeyB, 1.0),
    ];
    let pic = Pic::of(halves([0, 0, 255, 255], key)).with(keyed(&blue));
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(16, 32, NIGHT, "blue under a blue key");
    image.is(48, 32, key, "green under a blue key");
}

#[test]
fn a_difference_matte_keeps_what_is_not_in_its_clean_plate() {
    let Some(mut gpu) = gpu() else { return };
    let matte = fx(EffectKind::DifferenceMatte, &[]);
    let mut pic = Pic::of(quartered([RED, BLUE, GREEN, AMBER]));
    pic.plates
        .push((matte.id, 77_001, quartered([RED, BLUE, GREEN, RED])));
    let pic = pic.with(matte.clone());
    let image = draw(&mut gpu, Some(NIGHT), std::slice::from_ref(&pic));
    image.is(16, 16, NIGHT, "what the plate has, left top");
    image.is(48, 16, NIGHT, "what the plate has, right top");
    image.is(16, 48, NIGHT, "what the plate has, left bottom");
    image.is(48, 48, AMBER, "what the plate has not");

    // The plate lies over the layer's box, wherever that is.
    let moved = Pic {
        plates: vec![(matte.id, 77_002, quartered([RED, BLUE, GREEN, RED]))],
        ..Pic::of(quartered([RED, BLUE, GREEN, AMBER]))
    }
    .at([48.0, 48.0], [16.0, 16.0])
    .with(matte.clone());
    let image = draw(&mut gpu, Some(NIGHT), &[moved]);
    image.is(40, 40, NIGHT, "the plate's own quarter, moved");
    image.is(56, 56, AMBER, "the quarter that differs, moved");

    // Without a plate the layer is left whole, and costs no pass.
    let bare = Pic::of(quartered([RED, BLUE, GREEN, AMBER])).with(matte);
    assert!(pass::chain(&bare.layer, &bare.place, CANVAS, &[]).is_empty());
    let image = draw(&mut gpu, Some(NIGHT), &[bare]);
    image.is(16, 16, RED, "no plate to differ from");
}

#[test]
fn grain_is_the_same_for_a_seed_and_another_for_another() {
    let Some(mut gpu) = gpu() else { return };
    let grain = || fx(EffectKind::Grain, &[(ParamKey::Amount, 0.4)]);
    let mut grained = |seed: u64| {
        let pic = Pic::full(grey(128)).with(grain()).seeded(seed);
        draw(&mut gpu, Some(NIGHT), &[pic])
    };
    let first = grained(7);
    assert_eq!(first.0, grained(7).0, "a seed drawn twice");
    let other = grained(8);
    let pixels = (SIDE * SIDE) as usize;
    assert!(
        first.unlike(&other) > pixels / 2,
        "another seed is another grain: {} pixels differ",
        first.unlike(&other)
    );

    // At 0.4 a grain is at most a tenth of the way to white or black, the
    // same on every channel, and as often up as down.
    let codes: Vec<f64> = (first.0.chunks_exact(4))
        .map(|texel| {
            assert!(texel[0] == texel[1] && texel[1] == texel[2], "one grain");
            assert!(texel[0].abs_diff(128) <= 27, "a grain of {}", texel[0]);
            f64::from(texel[0])
        })
        .collect();
    let mean = codes.iter().sum::<f64>() / pixels as f64;
    let spread = (codes.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / pixels as f64).sqrt();
    assert!((mean - 128.0).abs() < 2.0, "the mean stays: {mean}");
    // Even over ±25.5 codes is a deviation of 14.7.
    assert!((spread - 14.7).abs() < 2.0, "spread evenly: {spread}");

    // Larger grains: 0.02 of 64 pixels is 1.28, so cells of that side, and
    // some pixels share the cell of the one before them.
    let coarse = fx(
        EffectKind::Grain,
        &[(ParamKey::Amount, 0.4), (ParamKey::Radius, 0.02)],
    );
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(grey(128)).with(coarse)]);
    // The centres of pixels 1 and 2 are both in the cell from 1.28 to 2.56.
    for y in [0, 20, 63] {
        assert_eq!(image.at(1, y), image.at(2, y), "one cell at row {y}");
    }
    assert!(
        image.unlike(&first) > pixels / 4,
        "another size, another grain"
    );
}

#[test]
fn a_vignette_darkens_from_its_radius_out_and_leaves_the_centre() {
    let Some(mut gpu) = gpu() else { return };
    let set = [
        (ParamKey::Amount, 0.5),
        (ParamKey::Radius, 0.5),
        (ParamKey::Softness, 0.5),
    ];
    let pic = Pic::full(WHITE).with(fx(EffectKind::Vignette, &set));
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(32, 32, WHITE, "the centre");
    image.is(40, 32, WHITE, "inside the radius");
    // The corner is 1.39 from the centre: past the radius and the softness.
    image.is(0, 0, grey(188), "the corner: half the light");
    image.is(63, 63, grey(188), "the opposite corner");
    // Three quarters out along an axis is half way through the softness.
    let [edge, ..] = image.at(56, 32);
    assert!(
        edge.abs_diff(encoded(0.75)) <= 6,
        "half darkened half way: {edge}"
    );

    // It follows the layer's box, not the canvas.
    let small = Pic::full(WHITE)
        .at([16.0, 16.0], [16.0, 16.0])
        .with(fx(EffectKind::Vignette, &set));
    let image = draw(&mut gpu, Some(NIGHT), &[small]);
    image.is(16, 16, WHITE, "the centre of a smaller layer");
    image.is(31, 31, grey(188), "its corner");
}

#[test]
fn a_blur_keeps_the_mean_and_lessens_the_difference() {
    let Some(mut gpu) = gpu() else { return };
    let sharp = draw(&mut gpu, Some(NIGHT), &[Pic::of(halves(WHITE, BLACK))]).row(32);
    let variance = |row: &[f64]| {
        let mean = row.iter().sum::<f64>() / row.len() as f64;
        row.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / row.len() as f64
    };
    // A narrow one, drawn at the canvas's size, and a wide one, drawn small.
    for (radius, flat) in [(0.05, 12), (0.25, 0)] {
        let blur = fx(EffectKind::Blur, &[(ParamKey::Radius, radius)]);
        let pic = Pic::of(halves(WHITE, BLACK)).with(blur);
        let soft = draw(&mut gpu, Some(NIGHT), &[pic]).row(32);
        let (before, after): (f64, f64) = (sharp.iter().sum(), soft.iter().sum());
        assert!(
            (before - after).abs() < 0.5,
            "a blur of {radius} keeps the mean: {after} of {before}"
        );
        assert!(
            variance(&soft) < variance(&sharp) - 0.004,
            "a blur of {radius} lessens the variance: {}",
            variance(&soft)
        );
        assert!(
            soft.windows(2).all(|pair| pair[1] <= pair[0] + 0.004),
            "a blurred edge only falls: {soft:?}"
        );
        // The edge is where it was: the two pixels beside it are as far
        // from a half as each other.
        assert!(
            (soft[31] + soft[32] - 1.0).abs() < 0.03 && soft[31] > 0.5 && soft[31] < 0.99,
            "about the edge: {} and {}",
            soft[31],
            soft[32]
        );
        // Three deviations from the edge nothing has changed.
        for x in 0..flat {
            assert!(soft[x] > 0.99 && soft[63 - x] < 0.002, "far from the edge");
        }
    }

    // The deviation is half the radius: 0.1 of 64 pixels is 3.2 pixels. One
    // deviation from an edge a gaussian has let 15.9% through.
    let blur = fx(EffectKind::Blur, &[(ParamKey::Radius, 0.2)]);
    let pic = Pic::of(halves(WHITE, BLACK)).with(blur);
    let soft = draw(&mut gpu, Some(NIGHT), &[pic]).row(32);
    // 6.4 pixels of deviation: the centre of pixel 38 is 6.5 past the edge.
    assert!(
        (soft[38] - 0.155).abs() < 0.03,
        "a deviation past the edge: {}",
        soft[38]
    );

    // A layer smaller than the canvas spreads into what is clear around it,
    // and has as much alpha in all as it had.
    let square = || Pic::full(WHITE).at([32.0, 32.0], [8.0, 8.0]);
    let before = draw(&mut gpu, None, &[square()]).alpha();
    for radius in [0.05, 0.2] {
        let blur = fx(EffectKind::Blur, &[(ParamKey::Radius, radius)]);
        let image = draw(&mut gpu, None, &[square().with(blur)]);
        let after = image.alpha();
        assert!(
            (after / before - 1.0).abs() < 0.03,
            "the alpha of a blur of {radius}: {after} of {before}"
        );
        assert!(image.at(22, 32)[3] > 0, "past where the layer was");
        assert!(image.at(32, 32)[3] < 255 || radius < 0.1);
    }
}

#[test]
fn a_glow_spreads_the_light_over_its_threshold_and_no_other() {
    let Some(mut gpu) = gpu() else { return };
    let glow = |threshold: f32| {
        fx(
            EffectKind::Glow,
            &[
                (ParamKey::Amount, 1.0),
                (ParamKey::Radius, 0.1),
                (ParamKey::Threshold, threshold),
            ],
        )
    };
    let picture = || Pic::of(halves(WHITE, grey(100)));
    let image = draw(&mut gpu, Some(NIGHT), &[picture().with(glow(0.6))]);
    image.is(8, 32, WHITE, "what glows is no darker");
    image.is(60, 32, grey(100), "far from the light");
    // White is 0.4 over the threshold. Half a pixel past the edge, with a
    // deviation of 3.2 pixels, 43.8% of that has spread.
    let lit = linear(image.at(32, 32)[0]) - linear(100);
    assert!(
        (lit - 0.4 * 0.438).abs() < 0.03,
        "the light beside the edge: {lit}"
    );
    let row = image.row(32);
    assert!(
        row[32..].windows(2).all(|pair| pair[1] <= pair[0] + 0.004),
        "the glow falls off: {row:?}"
    );

    // A grey under the threshold lights nothing.
    let dim = Pic::of(halves(grey(180), grey(100))).with(glow(0.6));
    let image = draw(&mut gpu, Some(NIGHT), &[dim]);
    image.is(33, 32, grey(100), "nothing over the threshold");
    image.is(30, 32, grey(180), "nothing over the threshold");

    // A glow lights what is clear around a layer.
    let small = Pic::full(WHITE)
        .at([32.0, 32.0], [8.0, 8.0])
        .with(glow(0.0));
    let image = draw(&mut gpu, None, &[small]);
    assert!(image.at(21, 32)[3] > 20, "light past the layer's edge");
    assert_eq!(image.at(2, 2), [0, 0, 0, 0], "and none far from it");
}

#[test]
fn sharpening_leaves_what_is_flat_and_steepens_an_edge() {
    let Some(mut gpu) = gpu() else { return };
    let sharpen = fx(
        EffectKind::Sharpen,
        &[(ParamKey::Amount, 1.0), (ParamKey::Radius, 0.02)],
    );
    let pic = Pic::of(halves(grey(100), grey(180))).with(sharpen);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(8, 32, grey(100), "flat, on the dark side");
    image.is(56, 32, grey(180), "flat, on the light side");
    let ([dark, ..], [light, ..]) = (image.at(31, 32), image.at(32, 32));
    assert!(
        dark < 96 && light > 184,
        "both sides of the edge go further apart: {dark} and {light}"
    );
    // As much is taken from the dark side as is given to the light one.
    let (under, over) = (linear(100) - linear(dark), linear(light) - linear(180));
    assert!((under - over).abs() < 0.02, "{under} under and {over} over");
}

#[test]
fn a_shadow_is_the_layers_alpha_moved_and_dimmed_under_it() {
    let Some(mut gpu) = gpu() else { return };
    // 0.125 of the canvas's height is 8 pixels: the layer is at 24..40 and
    // its shadow at 32..48.
    let shadow = |radius: f32| {
        fx(
            EffectKind::Shadow,
            &[
                (ParamKey::Amount, 0.5),
                (ParamKey::Radius, radius),
                (ParamKey::OffsetX, 0.125),
                (ParamKey::OffsetY, 0.125),
            ],
        )
    };
    let square = || Pic::full(WHITE).at([32.0, 32.0], [8.0, 8.0]);
    let image = draw(&mut gpu, None, &[square().with(shadow(0.0))]);
    image.is(28, 28, WHITE, "the layer");
    image.is(36, 36, WHITE, "the layer over its shadow");
    image.is(44, 44, [0, 0, 0, 128], "the shadow alone");
    image.is(44, 36, [0, 0, 0, 128], "the shadow beside the layer");
    image.is(20, 20, [0, 0, 0, 0], "neither");
    image.is(50, 44, [0, 0, 0, 0], "past the shadow");

    // Over a canvas it darkens what is under it by its amount.
    let image = draw(&mut gpu, Some(WHITE), &[square().with(shadow(0.0))]);
    image.is(44, 44, grey(188), "half the light of white");

    // Blurred, its edge is half as dark as its middle and goes on past it.
    let image = draw(&mut gpu, None, &[square().with(shadow(0.05))]);
    let ([.., inside], [.., edge], [.., past]) =
        (image.at(44, 43), image.at(48, 44), image.at(50, 44));
    assert!(inside > 100, "the middle of a soft shadow: {inside}");
    // Half a pixel past the edge of a blur 1.6 pixels deep: 37.7% of a half.
    assert!(edge.abs_diff(48) <= 10, "its edge: {edge}");
    assert!(past > 0 && past < edge, "past its edge: {past}");
}

#[test]
fn motion_blur_streaks_along_its_angle_and_not_across_it() {
    let Some(mut gpu) = gpu() else { return };
    // An upright bar eight pixels wide.
    let bar = || Pic::full(WHITE).at([32.0, 32.0], [4.0, 32.0]);
    let streak = |amount: f32, angle: f32| {
        fx(
            EffectKind::MotionBlur,
            &[(ParamKey::Amount, amount), (ParamKey::Angle, angle)],
        )
    };
    let whole = draw(&mut gpu, None, &[bar()]).alpha();
    // Along a streak of 16 pixels the bar covers half, and of 32 a quarter:
    // one pass for the first and two for the second.
    for (amount, cover) in [(0.25, 0.5), (0.5, 0.25)] {
        let pic = bar().with(streak(amount, 0.0));
        let passes = pass::chain(&pic.layer, &pic.place, CANVAS, &[]).len();
        assert_eq!(passes, if amount > 0.3 { 2 } else { 1 });
        let image = draw(&mut gpu, None, &[pic]);
        let [.., middle] = image.at(32, 32);
        assert!(
            (f64::from(middle) / 255.0 - cover).abs() < 0.03,
            "a streak of {amount} leaves {middle} at the bar's middle"
        );
        assert!(
            (image.alpha() / whole - 1.0).abs() < 0.03,
            "and as much of the bar in all"
        );
        let reach = 4.0 + 32.0 * amount;
        let (inside, outside) = (32 + reach as u32 - 2, 32 + reach as u32 + 2);
        assert!(image.at(inside, 32)[3] > 0, "it reaches {inside}");
        assert_eq!(image.at(outside, 32)[3], 0, "and not {outside}");
    }
    // Along the bar there is nothing to streak.
    let image = draw(&mut gpu, None, &[bar().with(streak(0.25, 90.0))]);
    image.is(32, 32, WHITE, "streaked along itself");
    image.is(27, 32, [0, 0, 0, 0], "and no wider for it");
}

/// How far to the right and down the alpha of a picture is, on average.
fn centre(image: &Image) -> [f64; 2] {
    let mut sum = [0.0; 3];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let a = f64::from(image.at(x, y)[3]);
            sum = [
                sum[0] + a * f64::from(x),
                sum[1] + a * f64::from(y),
                sum[2] + a,
            ];
        }
    }
    [sum[0] / sum[2], sum[1] / sum[2]]
}

#[test]
fn a_shake_moves_the_layer_by_what_its_time_draws() {
    let Some(mut gpu) = gpu() else { return };
    let shake = || {
        fx(
            EffectKind::Shake,
            &[(ParamKey::Amount, 0.125), (ParamKey::Frequency, 8.0)],
        )
    };
    let square = || Pic::full(WHITE).at([32.0, 32.0], [8.0, 8.0]);
    let still = draw(&mut gpu, None, &[square()]);
    let mut pic = square().with(shake()).timed(Time(250_000));
    // What is drawn depends on the clip: the same one every run.
    pic.layer.clip = Uuid::from_u128(0x5eed);
    let first = draw(&mut gpu, None, std::slice::from_ref(&pic));
    // The same clip at the same time is in the same place, whatever its
    // seed, which is another every frame.
    pic.layer.seed = 99;
    assert_eq!(
        first.0,
        draw(&mut gpu, None, std::slice::from_ref(&pic)).0,
        "the same time drawn twice"
    );
    assert!(
        (first.alpha() / still.alpha() - 1.0).abs() < 0.01,
        "moved whole"
    );
    // Where the pass says: its offset is in pixels of the canvas.
    let moved = pass::chain(&pic.layer, &pic.place, CANVAS, &[])[0]
        .uniform
        .bytes();
    let float = |index: usize| {
        let mut word = [0u8; 4];
        word.copy_from_slice(&moved[index * 4..index * 4 + 4]);
        f64::from(f32::from_ne_bytes(word))
    };
    // p[6] is the tenth vector of the record.
    let offset = [float(36), float(37)];
    assert!(
        offset.iter().all(|by| by.abs() <= 8.0) && offset.iter().any(|by| by.abs() > 0.25),
        "within an eighth of the canvas: {offset:?}"
    );
    let ([x0, y0], [x1, y1]) = (centre(&still), centre(&first));
    assert!(
        (x1 - x0 - offset[0]).abs() < 0.1 && (y1 - y0 - offset[1]).abs() < 0.1,
        "moved by {offset:?}: from {x0}, {y0} to {x1}, {y1}"
    );

    // Another time is another place, and a frame later it is not far off.
    pic.layer.time = Time(1_250_000);
    let later = draw(&mut gpu, None, std::slice::from_ref(&pic));
    assert!(later.unlike(&first) > 0, "another time");
    pic.layer.time = Time(250_000 + 16_667);
    let next = centre(&draw(&mut gpu, None, std::slice::from_ref(&pic)));
    assert!(
        (next[0] - x1).abs() < 4.0 && (next[1] - y1).abs() < 4.0,
        "a frame on it has not jumped: {next:?}"
    );

    // A turn alone keeps the layer's centre where it is.
    let turn = fx(
        EffectKind::Shake,
        &[(ParamKey::Amount, 0.0), (ParamKey::Angle, 45.0)],
    );
    let mut bar = Pic::full(WHITE)
        .at([32.0, 32.0], [16.0, 4.0])
        .with(turn)
        .timed(Time(250_000));
    bar.layer.clip = Uuid::from_u128(0x5eed);
    let flat = draw(
        &mut gpu,
        None,
        &[Pic::full(WHITE).at([32.0, 32.0], [16.0, 4.0])],
    );
    let turned = draw(&mut gpu, None, &[bar]);
    assert!(turned.unlike(&flat) > 0, "turned");
    let [x, y] = centre(&turned);
    assert!((x - 31.5).abs() < 0.2 && (y - 31.5).abs() < 0.2);
}

#[test]
fn a_wave_moves_each_point_along_its_crests_by_its_sine() {
    let Some(mut gpu) = gpu() else { return };
    // A wave that runs across, a quarter of a wave on at the layer's middle
    // column, where its sine is one: 0.125 of 64 pixels is 8 pixels.
    let wave = |phase: f32| {
        fx(
            EffectKind::Distort,
            &[
                (ParamKey::Amount, 0.125),
                (ParamKey::Frequency, 1.0),
                (ParamKey::Angle, 0.0),
                (ParamKey::OffsetX, phase),
            ],
        )
    };
    let picture = || Pic::of(stacked(RED, BLUE));
    let image = draw(&mut gpu, Some(NIGHT), &[picture().with(wave(0.25))]);
    image.is(32, 20, RED, "red from eight pixels down is still red");
    image.is(32, 26, BLUE, "blue brought up eight pixels");
    image.is(32, 40, BLUE, "blue below");
    // Half a wave across, the other way: at a quarter of the box either
    // side of the middle the sine is nought, and nothing has moved.
    image.is(16, 30, RED, "where the wave crosses nought");
    image.is(16, 33, BLUE, "where the wave crosses nought");
    image.is(48, 30, RED, "and where it crosses back");
    let image = draw(&mut gpu, Some(NIGHT), &[picture().with(wave(0.75))]);
    image.is(32, 38, RED, "red brought down eight pixels");
    image.is(32, 42, BLUE, "blue below it");
    // What is brought in from past the canvas is clear.
    image.is(32, 4, NIGHT, "nothing from above the canvas");

    // Turned a quarter, the wave runs down and moves its points across.
    let down = fx(
        EffectKind::Distort,
        &[
            (ParamKey::Amount, 0.125),
            (ParamKey::Frequency, 1.0),
            (ParamKey::Angle, 90.0),
            (ParamKey::OffsetX, 0.25),
        ],
    );
    let pic = Pic::of(halves(RED, BLUE)).with(down);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(38, 32, RED, "red brought right eight pixels");
    image.is(42, 32, BLUE, "blue past it");
}

#[test]
fn a_glitch_is_the_same_for_a_stretch_of_time_and_only_moves_rows_across() {
    let Some(mut gpu) = gpu() else { return };
    let glitch = || {
        fx(
            EffectKind::Glitch,
            &[
                (ParamKey::Amount, 1.0),
                (ParamKey::Frequency, 6.0),
                (ParamKey::OffsetX, 0.05),
            ],
        )
    };
    // One clip throughout: another has a pattern of its own.
    let of = |frame: Frame| {
        let mut pic = Pic::of(frame).with(glitch());
        pic.layer.clip = Uuid::from_u128(0x5eed);
        pic
    };
    let edge = || of(halves(WHITE, BLACK));
    let plain = draw(&mut gpu, Some(NIGHT), &[Pic::of(halves(WHITE, BLACK))]);
    let first = draw(&mut gpu, Some(NIGHT), &[edge()]);
    // Six a second: the frames of one sixth of a second share a pattern,
    // whatever their seeds, and the next sixth has another.
    let same = edge().timed(Time(150_000)).seeded(1234);
    let same = draw(&mut gpu, Some(NIGHT), &[same]);
    assert_eq!(first.unlike(&same), 0, "within a sixth of a second");
    let next = draw(&mut gpu, Some(NIGHT), &[edge().timed(Time(170_000))]);
    assert!(next.unlike(&first) > 0, "the sixth after");
    assert!(first.unlike(&plain) > 0, "glitched");
    // The colours are pulled apart across: red from one side, blue from
    // the other, so somewhere beside the edge they differ.
    let fringed = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let [red, _, blue, _] = first.at(*x, *y);
            red.abs_diff(blue) > 100
        })
        .count();
    assert!(fringed > 32, "colour fringes: {fringed} pixels");

    // Rows only move sideways: a picture that is the same all the way
    // across is the same after, away from the sides.
    let bands = of(stacked(RED, BLUE));
    let image = draw(&mut gpu, Some(NIGHT), &[bands]);
    for y in 0..SIDE {
        let expected = if y < 32 { RED } else { BLUE };
        image.is(32, y, expected, "a row of one colour");
    }
}

#[test]
fn the_passes_of_a_blur_go_between_three_targets_and_end_on_the_layer() {
    let blur =
        |radius: f32| Pic::full(RED).with(fx(EffectKind::Blur, &[(ParamKey::Radius, radius)]));
    // 0.05 of 64 pixels is a deviation of 1.6: across, down, and joined.
    let narrow = blur(0.05);
    let chain = pass::chain(&narrow.layer, &narrow.place, CANVAS, &[]);
    let route: Vec<_> = (chain.iter())
        .map(|link| (link.first, link.second, link.into))
        .collect();
    assert_eq!(
        route,
        vec![
            (0, Second::Blank, 1),
            (1, Second::Blank, 2),
            (2, Second::Slot(0), 1),
        ],
        "the layer is kept until the last pass mixes over it"
    );
    // The same radius of a canvas 1080 high is 27 pixels: shrunk by eight.
    let chain = pass::chain(&narrow.layer, &narrow.place, [1920, 1080], &[]);
    assert_eq!(chain.len(), 4, "shrunk first");
    assert!(chain.len() <= pass::chain_len(&narrow.layer));
    assert_eq!(pass::slots(&narrow.layer), 3);

    // A stack carries on from wherever the last effect left the layer.
    let stack = blur(0.05)
        .with(fx(EffectKind::Grade, &[]))
        .with(fx(EffectKind::Glow, &[]))
        .masked(mask(MaskShape::Ellipse, [0.5, 0.5], [1.0, 1.0]));
    let chain = pass::chain(&stack.layer, &stack.place, CANVAS, &[]);
    let mut written = [true, false, false];
    for link in &chain {
        assert_ne!(link.into, link.first, "a pass never reads what it writes");
        assert_ne!(Second::Slot(link.into), link.second);
        assert!(written[link.first], "a pass reads what was drawn");
        if let Second::Slot(slot) = link.second {
            assert!(written[slot], "and mixes over what was drawn");
        }
        written[link.into] = true;
    }
    // The blur's three, the grade, the glow's four and the masks.
    assert_eq!(chain.len(), 9);
    assert!(chain.len() <= pass::chain_len(&stack.layer));
    assert_eq!(pass::slots(&stack.layer), 3);

    // Without an effect that blurs, two targets are enough.
    let graded = Pic::full(RED).with(fx(EffectKind::Grade, &[]));
    assert_eq!(pass::slots(&graded.layer), 2);
    assert_eq!(pass::slots(&Pic::full(RED).layer), 1);
    let chain = pass::chain(&graded.layer, &graded.place, CANVAS, &[]);
    assert_eq!((chain[0].first, chain[0].into), (0, 1));
}

#[test]
fn a_paused_canvas_with_effects_is_composed_once() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    // Everything that is drawn from noise or from time, and a blur.
    let pics = [Pic::of(swatches())
        .with(fx(EffectKind::Grade, &[(ParamKey::Exposure, 0.5)]))
        .with(fx(EffectKind::Blur, &[(ParamKey::Radius, 0.05)]))
        .with(fx(EffectKind::Grain, &[]))
        .with(fx(EffectKind::Glitch, &[]))
        .with(fx(EffectKind::Shake, &[]))
        .timed(Time(400_000))];
    let first = composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    assert!(first[0].is_some(), "a canvas with effects is composited");
    let read = |gpu: Option<&Gpu>| gpu.expect("a device").read(0);
    let shown = read(composer.gpu.as_ref());
    for _ in 0..5 {
        composer.begin_frame();
        assert_eq!(composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]), first);
    }
    let counters = composer.counters();
    assert_eq!(
        (counters.composed, counters.skipped),
        (1, 5),
        "the same plan is not drawn again, noise and all"
    );
    assert_eq!(read(composer.gpu.as_ref()), shown);
    // Two to blend between, three for the chain and the one shown.
    let canvas = (SIDE * SIDE * 4) as usize;
    let held = composer.gpu.as_ref().map_or(0, Gpu::bytes);
    let fixed = composer.gpu.as_ref().map_or(0, Gpu::fixed_bytes);
    assert_eq!(held, fixed + 7 * canvas, "six canvases and the picture");
    assert!(held <= ComposeBudget::PRODUCTION.bytes);

    // The next canvas frame has another seed, and is drawn.
    let mut pics = pics;
    pics[0].layer.seed += 1;
    composer.begin_frame();
    composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    assert_eq!(composer.counters().composed, 2, "another frame's grain");
    assert_ne!(read(composer.gpu.as_ref()), shown);
}

/// The effects of a look as the plan would hold them.
fn look_of(preset: LookPreset, strength: f32) -> Vec<ResolvedEffect> {
    (look::preset_effects(preset, strength).iter())
        .map(|effect| ResolvedEffect {
            points: effect.points.clone(),
            ..fx(
                effect.kind,
                &(effect.params.iter())
                    .map(|param| (param.key, param.value))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[test]
fn the_looks_are_drawn_and_none_of_them_ruins_a_picture() {
    let Some(mut gpu) = gpu() else { return };
    let looks = [
        LookPreset::Cinematic,
        LookPreset::TealOrange,
        LookPreset::Vintage,
        LookPreset::Faded,
        LookPreset::Noir,
        LookPreset::Warm,
        LookPreset::Cool,
    ];
    // A grey ramp across: what a look does to tone is read along it.
    let ramp = || painted(|x, _| grey((x * 4 + 2) as u8));
    let plain = draw(&mut gpu, Some(NIGHT), &[Pic::of(ramp())]);
    for preset in looks {
        let mut none = Pic::of(ramp());
        none.layer.effects = look_of(preset, 0.0);
        let image = draw(&mut gpu, Some(NIGHT), &[none]);
        image.same_as(&plain, &format!("{preset:?} at no strength"));

        let mut full = Pic::of(ramp());
        full.layer.effects = look_of(preset, 1.0);
        let passes = pass::chain(&full.layer, &full.place, CANVAS, &[]).len();
        assert_eq!(
            passes,
            full.layer.effects.len(),
            "every effect of {preset:?} is drawn"
        );
        let image = draw(&mut gpu, Some(NIGHT), &[full]);
        assert!(image.unlike(&plain) > 1000, "{preset:?} shows");
        // Averaged over the rows, so that grain does not decide it.
        let mean = |x: u32| -> [f64; 3] {
            let mut sum = [0.0; 3];
            for y in 24..40 {
                let [r, g, b, _] = image.at(x, y);
                sum = [
                    sum[0] + f64::from(r),
                    sum[1] + f64::from(g),
                    sum[2] + f64::from(b),
                ];
            }
            sum.map(|channel| channel / 16.0)
        };
        let brightness = |x: u32| (0..3).map(|c| LUMA[c] * mean(x)[c]).sum::<f64>();
        // Dark stays dark, light stays light and the middle is between
        // them: no look crushes or washes out the picture.
        let (dark, middle, light) = (brightness(4), brightness(32), brightness(59));
        assert!(dark < 70.0, "{preset:?} leaves shadows: {dark}");
        assert!(light > 170.0, "{preset:?} leaves highlights: {light}");
        assert!(
            (80.0..180.0).contains(&middle) && dark < middle && middle < light,
            "{preset:?} leaves a middle: {dark}, {middle}, {light}"
        );
        let [red, green, blue] = mean(32);
        match preset {
            LookPreset::Noir => assert!(
                (red - green).abs() < 1.5 && (green - blue).abs() < 1.5,
                "noir has no colour: {red}, {green}, {blue}"
            ),
            LookPreset::Warm | LookPreset::Vintage => {
                assert!(red > blue + 4.0, "{preset:?} is warm: {red}, {blue}");
            }
            // A little warm, and no more than that.
            LookPreset::Cinematic => assert!(red > blue && red < blue + 12.0),
            LookPreset::Cool => assert!(blue > red + 4.0, "cool: {red}, {blue}"),
            LookPreset::TealOrange => {
                // Orange in the lights and teal in the shadows.
                let (low, high) = (mean(12), mean(52));
                assert!(high[0] > high[2] + 4.0, "orange highlights: {high:?}");
                assert!(low[2] > low[0] + 4.0, "teal shadows: {low:?}");
            }
            LookPreset::Faded => {
                let black = brightness(0);
                assert!(black > 15.0, "faded lifts black: {black}");
            }
        }
    }
}
