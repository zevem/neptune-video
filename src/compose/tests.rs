//! The compositor's decisions, without a device: how a plan is split, what a
//! frame costs, when a canvas is drawn again and when everything is given up.
use super::{
    CANVAS_STEPS, ComposeBudget, Job, Ledger, MAX_CANVASES, MAX_PASSES, Run, Verdict,
    gpu_tests::{BLUE, CANVAS, NIGHT, Pic, RED, job, mask, solid},
    pass::{self, Uniform},
    runs, target_size,
};
use crate::textures::LAYER_IDLE_FRAMES;
use editor_core::{BlendMode, LayerSource, MaskOp, MaskShape, RenderPlan};
use eframe::egui::vec2;

fn plan(layers: Vec<editor_core::Layer>) -> RenderPlan {
    RenderPlan {
        canvas: [1920, 1080],
        background: NIGHT,
        layers,
    }
}

fn plain() -> editor_core::Layer {
    Pic::full(RED).layer
}

fn blended() -> editor_core::Layer {
    Pic::full(RED).blend(BlendMode::Screen).layer
}

fn text() -> editor_core::Layer {
    let mut layer = plain();
    layer.source = LayerSource::Text {
        text: "Title".to_owned(),
        font_size: 48.0,
        color: [255; 4],
        style: Default::default(),
        enter: None,
        exit: None,
        words: None,
    };
    layer
}

fn graphic() -> editor_core::Layer {
    let mut layer = plain();
    layer.source = LayerSource::Graphic {
        size: [400.0, 120.0],
        elements: Vec::new(),
    };
    layer
}

#[test]
fn a_plain_plan_is_all_quads() {
    assert_eq!(runs(&plan(Vec::new())), Vec::new());
    assert_eq!(
        runs(&plan(vec![plain(), plain(), plain()])),
        vec![Run::Plain(0..3)]
    );
    assert_eq!(
        runs(&plan(vec![plain(), text(), plain()])),
        vec![Run::Plain(0..1), Run::Title(1), Run::Plain(2..3)]
    );
}

#[test]
fn a_run_is_composited_up_to_its_highest_layer_that_is_not_plain() {
    // What is under a blended layer is what it blends with; what is over it
    // is a quad as before.
    assert_eq!(
        runs(&plan(vec![plain(), blended(), plain(), plain()])),
        vec![Run::Composite(0..2), Run::Plain(2..4)]
    );
    assert_eq!(
        runs(&plan(vec![blended(), plain(), blended()])),
        vec![Run::Composite(0..3)]
    );
    let masked = Pic::full(RED)
        .masked(mask(MaskShape::Ellipse, [0.5, 0.5], [0.5, 0.5]))
        .layer;
    assert_eq!(runs(&plan(vec![masked])), vec![Run::Composite(0..1)]);
}

#[test]
fn titles_and_graphics_part_the_layers_into_runs_of_their_own() {
    assert_eq!(
        runs(&plan(vec![
            plain(),
            blended(),
            text(),
            plain(),
            graphic(),
            blended()
        ])),
        vec![
            Run::Composite(0..2),
            Run::Title(2),
            Run::Plain(3..4),
            Run::Title(4),
            Run::Composite(5..6),
        ]
    );
}

#[test]
fn a_canvas_is_sized_by_the_pixels_it_covers_on_screen() {
    let hd = [1920, 1080];
    // A texel to a pixel, so what is composited is resampled once, as a
    // quad is, and not again when the canvas is drawn.
    assert_eq!(target_size(hd, vec2(600.0, 337.5), 1.0), [600, 338]);
    assert_eq!(target_size(hd, vec2(610.0, 343.0), 1.0), [610, 343]);
    assert_eq!(target_size(hd, vec2(600.0, 337.5), 2.0), [1200, 675]);
    // Past the largest step too: a wide window is not drawn from less.
    assert_eq!(target_size(hd, vec2(1500.0, 843.75), 2.0), [3000, 1688]);
    assert_eq!(
        target_size([1080, 1920], vec2(200.0, 355.0), 1.0),
        [200, 355]
    );
    assert_eq!(target_size([4000, 10], vec2(100.0, 0.25), 1.0), [100, 1]);
    assert_eq!(target_size(hd, vec2(0.0, 0.0), 1.0), [1, 1]);
}

#[test]
fn a_uniform_is_laid_out_as_the_shaders_declare_it() {
    assert_eq!(Uniform::BYTES, 496, "three vectors and twenty-eight more");
    let uniform = Uniform::new([64, 32])
        .op(pass::OP_MASK)
        .variant(3)
        .count(2)
        .mix(0.25)
        .opacity(0.5)
        .set(0, [1.0, 2.0, 3.0, 4.0])
        .scalars(1, [5.0, 6.0, 7.0, 8.0, 9.0]);
    let bytes = uniform.bytes();
    let word = |index: usize| {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[index * 4..index * 4 + 4]);
        word
    };
    let float = |index: usize| f32::from_ne_bytes(word(index));
    assert_eq!(u32::from_ne_bytes(word(0)), pass::OP_MASK);
    assert_eq!(u32::from_ne_bytes(word(1)), 3);
    assert_eq!(u32::from_ne_bytes(word(2)), 2);
    assert_eq!(
        [float(4), float(5), float(6), float(7)],
        [64.0, 32.0, 1.0 / 64.0, 1.0 / 32.0]
    );
    assert_eq!([float(10), float(11)], [0.25, 0.5]);
    assert_eq!([float(12), float(15)], [1.0, 4.0], "p[0]");
    assert_eq!(
        [float(16), float(19), float(20)],
        [5.0, 8.0, 9.0],
        "p[1] on"
    );
}

#[test]
fn a_seed_is_the_same_two_fractions_wherever_it_is_packed() {
    let bytes = Uniform::new([8, 8]).seed(0x0123_4567_89ab_cdef).bytes();
    let float = |index: usize| {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[index * 4..index * 4 + 4]);
        f32::from_ne_bytes(word)
    };
    assert_eq!(float(8), 0x00ab_cdef as f32 / 16_777_216.0);
    assert_eq!(float(9), 0x0045_6789 as f32 / 16_777_216.0);
    assert!((0.0..1.0).contains(&float(8)) && (0.0..1.0).contains(&float(9)));
}

#[test]
fn a_layers_chain_is_its_masks_in_one_pass_and_nothing_for_what_is_not_drawn_yet() {
    let pic = Pic::full(RED);
    assert!(pass::chain(&pic.layer, &pic.place, CANVAS, &[]).is_empty());
    assert_eq!(pass::chain_len(&pic.layer), 0);

    let mut second = mask(MaskShape::Ellipse, [0.25, 0.75], [0.5, 0.125]);
    second.op = MaskOp::Intersect;
    second.invert = true;
    second.rotation = 90.0;
    second.feather = 0.1;
    let pic = pic
        .masked(mask(MaskShape::Rectangle, [0.5, 0.5], [1.0, 1.0]))
        .masked(second);
    let chain = pass::chain(&pic.layer, &pic.place, CANVAS, &[]);
    assert_eq!(chain.len(), 1, "every mask in one pass");
    assert_eq!(pass::chain_len(&pic.layer), 1);
    let expected = Uniform::new(CANVAS)
        .op(pass::OP_MASK)
        .count(2)
        .seed(pic.layer.seed)
        .place(&pic.place)
        .set(2, [0.5, 0.5, 1.0, 1.0])
        .set(3, [1.0, 0.0, 0.0, 0.0])
        .set(4, [0.0, 0.0, 0.0, 0.0])
        .set(5, [0.25, 0.75, 0.5, 0.125])
        .set(
            6,
            [90f32.to_radians().cos(), 90f32.to_radians().sin(), 0.1, 0.0],
        )
        .set(7, [1.0, 2.0, 1.0, 0.0]);
    assert_eq!(chain[0].uniform, expected);

    // An effect no pass draws yet costs none, and the layer shows without it.
    let mut pic = Pic::full(RED);
    pic.layer.effects.push(editor_core::ResolvedEffect {
        id: uuid::Uuid::new_v4(),
        kind: editor_core::EffectKind::Stabilize,
        mix: 1.0,
        params: Vec::new(),
        points: Vec::new(),
        asset_id: None,
    });
    assert_eq!(pass::chain_len(&pic.layer), 0);
    assert!(pass::chain(&pic.layer, &pic.place, CANVAS, &[]).is_empty());
}

#[test]
fn every_blend_mode_has_a_variant_of_its_own() {
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Add,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
    ];
    let variants: Vec<u32> = modes.into_iter().map(pass::blend_mode).collect();
    assert_eq!(variants, (0..9).collect::<Vec<u32>>());
}

const WANT: [u32; 2] = [640, 360];

fn canvas_bytes([width, height]: [u32; 2]) -> usize {
    (width * height * 4) as usize
}

#[test]
fn a_paused_canvas_is_drawn_once() {
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let pics = [Pic::full(RED), Pic::full(BLUE).blend(BlendMode::Multiply)];
    let jobs = [job(Some(NIGHT), &pics)];
    let turn = ledger.turn(WANT, &jobs);
    assert_eq!(turn.verdicts, vec![Verdict::Render]);
    assert_eq!((turn.size, turn.targets), (WANT, 3));
    // The same plan, pictures and size, frame after frame.
    for _ in 0..3 * LAYER_IDLE_FRAMES {
        assert!(!ledger.begin_frame(), "a canvas in use is kept");
        assert_eq!(ledger.turn(WANT, &jobs).verdicts, vec![Verdict::Skip]);
    }
    let counters = ledger.counters();
    assert_eq!(
        (counters.composed, counters.skipped),
        (1, 3 * LAYER_IDLE_FRAMES)
    );
}

#[test]
fn a_canvas_is_drawn_again_for_whatever_changes_what_it_shows() {
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let mut pics = [Pic::full(RED).blend(BlendMode::Multiply)];
    let render = |ledger: &mut Ledger, want: [u32; 2], job: Job<'_>| {
        ledger.turn(want, &[job]).verdicts[0] == Verdict::Render
    };
    assert!(render(&mut ledger, WANT, job(Some(NIGHT), &pics)));
    assert!(!render(&mut ledger, WANT, job(Some(NIGHT), &pics)));

    // The picture its layer shows.
    pics[0].serial += 1_000_000;
    assert!(render(&mut ledger, WANT, job(Some(NIGHT), &pics)));
    // The plan: any value of a layer.
    pics[0].layer.transform.opacity = 0.5;
    assert!(render(&mut ledger, WANT, job(Some(NIGHT), &pics)));
    pics[0].layer.seed += 1;
    assert!(render(&mut ledger, WANT, job(Some(NIGHT), &pics)));
    // Where the layer is.
    pics[0].place.center[0] += 1.0;
    assert!(render(&mut ledger, WANT, job(Some(NIGHT), &pics)));
    // What is under it, and the size it is drawn at.
    assert!(render(&mut ledger, WANT, job(Some(RED), &pics)));
    assert!(render(&mut ledger, WANT, job(None, &pics)));
    assert!(render(&mut ledger, [320, 180], job(None, &pics)));
    assert!(!render(&mut ledger, [320, 180], job(None, &pics)));
    assert_eq!(ledger.counters().composed, 8);
}

#[test]
fn the_books_count_canvases_and_pictures_and_let_go_of_pictures_not_shown() {
    let fixed = 1000;
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, fixed);
    assert_eq!(
        ledger.bytes(),
        0,
        "nothing is held before anything is drawn"
    );
    let pics = [Pic::full(RED), Pic::full(BLUE).blend(BlendMode::Add)];
    let picture = 16 * 16 * 4;
    ledger.turn(WANT, &[job(Some(NIGHT), &pics)]);
    // Two canvases to blend between, the layer, and the one shown.
    assert_eq!(ledger.bytes(), fixed + 2 * picture + 4 * canvas_bytes(WANT));
    assert!(ledger.live(pics[0].serial) && ledger.live(pics[1].serial));

    // A masked layer needs one more canvas to go to and fro on.
    let masked = [Pic::full(RED).masked(mask(MaskShape::Ellipse, [0.5; 2], [0.5; 2]))];
    let turn = ledger.turn(WANT, &[job(Some(NIGHT), &masked)]);
    assert_eq!(turn.targets, 4);
    assert_eq!(ledger.bytes(), fixed + picture + 5 * canvas_bytes(WANT));
    assert!(!ledger.live(pics[0].serial), "a picture no canvas shows");
    assert!(ledger.live(masked[0].serial));

    // One picture drawn twice is held once.
    let mut twice = [Pic::full(RED), Pic::full(RED).blend(BlendMode::Add)];
    twice[1].serial = twice[0].serial;
    ledger.turn(WANT, &[job(None, &twice)]);
    assert_eq!(ledger.bytes(), fixed + picture + 4 * canvas_bytes(WANT));
}

#[test]
fn the_canvas_steps_down_until_everything_fits_the_budget() {
    let pics = [Pic::of(solid(RED, [1920, 1080])).blend(BlendMode::Screen)];
    let picture = canvas_bytes([1920, 1080]);
    let want = [1920, 1080];
    // Four canvases at full size and the picture come to five of them.
    let mut roomy = Ledger::new(ComposeBudget::PRODUCTION, 0);
    assert_eq!(roomy.turn(want, &[job(Some(NIGHT), &pics)]).size, want);
    assert!(roomy.bytes() <= ComposeBudget::PRODUCTION.bytes);
    assert_eq!(roomy.counters().stepped_down, 0);

    for (budget, size) in [
        (picture + 4 * canvas_bytes([1920, 1080]), [1920, 1080]),
        (picture + 4 * canvas_bytes([1920, 1080]) - 1, [1280, 720]),
        (picture + 4 * canvas_bytes([1280, 720]) - 1, [960, 540]),
        (picture + 4 * canvas_bytes([320, 180]) - 1, [160, 90]),
    ] {
        let mut ledger = Ledger::new(ComposeBudget { bytes: budget }, 0);
        let turn = ledger.turn(want, &[job(Some(NIGHT), &pics)]);
        assert_eq!(turn.size, size, "with a budget of {budget}");
        assert!(ledger.bytes() <= budget);
        assert_eq!(turn.verdicts, vec![Verdict::Render]);
        assert_eq!(
            ledger.counters().stepped_down,
            u64::from(size != want),
            "said once"
        );
        // The smaller canvas is then kept: it is not made again each frame.
        assert_eq!(
            ledger.turn(want, &[job(Some(NIGHT), &pics)]).verdicts,
            vec![Verdict::Skip]
        );
        assert_eq!(ledger.counters().stepped_down, u64::from(size != want));
    }

    // No step fits: every job is left to the mesh path, and nothing is held.
    let smallest = canvas_bytes([CANVAS_STEPS[0], 90]);
    let mut ledger = Ledger::new(
        ComposeBudget {
            bytes: picture + 4 * smallest - 1,
        },
        0,
    );
    let turn = ledger.turn(want, &[job(Some(NIGHT), &pics)]);
    assert_eq!(turn.verdicts, vec![Verdict::Refuse]);
    assert_eq!((turn.targets, ledger.bytes()), (0, 0));
    assert_eq!(ledger.counters().refused, 1);
}

#[test]
fn canvases_past_the_most_a_frame_may_have_and_jobs_of_too_many_passes_are_refused() {
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let pics = [Pic::full(RED).blend(BlendMode::Screen)];
    let jobs: Vec<Job<'_>> = (0..MAX_CANVASES + 2).map(|_| job(None, &pics)).collect();
    let turn = ledger.turn(WANT, &jobs);
    let mut expected = vec![Verdict::Render; MAX_CANVASES];
    expected.extend([Verdict::Refuse; 2]);
    assert_eq!(turn.verdicts, expected);
    assert_eq!(
        ledger.bytes(),
        16 * 16 * 4 + (3 + MAX_CANVASES) * canvas_bytes(WANT),
        "the canvases between passes are shared by the canvases of a frame"
    );

    // Two passes a layer and one to encode.
    let many: Vec<Pic> = (0..MAX_PASSES / 2)
        .map(|_| Pic::full(RED).blend(BlendMode::Screen))
        .collect();
    assert_eq!(job(None, &many).passes(), MAX_PASSES + 1);
    let turn = ledger.turn(WANT, &[job(None, &many[1..]), job(None, &many)]);
    assert_eq!(turn.verdicts, vec![Verdict::Render, Verdict::Refuse]);

    // A job with nothing to draw has no canvas.
    let turn = ledger.turn(WANT, &[job(Some(NIGHT), &[])]);
    assert_eq!(turn.verdicts, vec![Verdict::Refuse]);
    assert_eq!(ledger.bytes(), 0);
}

#[test]
fn everything_is_given_up_after_as_many_idle_frames_as_a_viewer_layer_is_kept() {
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let pics = [Pic::full(RED).blend(BlendMode::Screen)];
    ledger.turn(WANT, &[job(Some(NIGHT), &pics)]);
    assert_eq!(
        ledger.drawn().collect::<Vec<_>>(),
        vec![(pics[0].layer.clip, pics[0].serial)]
    );
    for frame in 1..=LAYER_IDLE_FRAMES {
        assert!(!ledger.begin_frame(), "kept in idle frame {frame}");
        assert!(ledger.bytes() > 0);
        assert_eq!(ledger.drawn().count(), 0, "nothing was drawn this frame");
    }
    assert!(ledger.begin_frame(), "given up in the frame after");
    assert_eq!(ledger.bytes(), 0);
    assert!(!ledger.live(pics[0].serial));
    assert_eq!(ledger.counters().released, 1);
    assert!(!ledger.begin_frame(), "and only once");

    // Drawn again from nothing, and given up with the project.
    assert_eq!(
        ledger.turn(WANT, &[job(Some(NIGHT), &pics)]).verdicts,
        vec![Verdict::Render]
    );
    ledger.clear();
    assert_eq!(ledger.bytes(), 0);
    assert_eq!(ledger.counters().released, 2);
    ledger.clear();
    assert_eq!(ledger.counters().released, 2, "nothing to give up twice");
}
