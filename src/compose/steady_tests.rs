//! A stabilized layer's pixels, read back from a real device: a correction
//! that is known moves the picture by the pixels it says, and the crop
//! enlarges it about its centre.
//!
//! Pictures are 64 pixels square on a canvas of the same, one texel a pixel.
//! As in `gpu_tests`, a test without an adapter prints `SKIP:` and passes.
use super::{
    ComposeBudget, Ledger, Verdict,
    gpu::Gpu,
    gpu_tests::{BLACK, CANVAS, Image, NIGHT, Pic, WHITE, gpu, job},
};
use editor_media::{Fix, Frame, Sampling};

const SIDE: u32 = 64;

/// Black, with a white square of eight texels whose left top is at
/// `(24, 28)`.
fn marked() -> Frame {
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let white = (24..32).contains(&x) && (28..36).contains(&y);
            rgba.extend_from_slice(if white { &WHITE } else { &BLACK });
        }
    }
    Frame {
        width: SIDE,
        height: SIDE,
        rgba: rgba.into(),
    }
}

fn steadied(gpu: &mut Gpu, steady: Option<Sampling>) -> Image {
    let pics = [Pic::of(marked())];
    let mut job = job(Some(NIGHT), &pics);
    job.draws[0].steady = steady;
    gpu.hold(CANVAS, 2 + job.slots(), 1);
    gpu.render(0, &job);
    Image(gpu.read(0))
}

/// The square's corners are where they are said to be: white inside each
/// and black outside, `soft` pixels in from the edge and a pixel more out
/// from it, for an edge an enlargement has spread over that many.
#[track_caller]
fn square_at(image: &Image, [left, top]: [u32; 2], side: u32, soft: u32, what: &str) {
    let (left, top) = (left + soft, top + soft);
    let (right, bottom) = (left + side - 1 - 2 * soft, top + side - 1 - 2 * soft);
    for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
        image.is(x, y, WHITE, what);
    }
    let out = 2 * soft + 1;
    for (x, y) in [
        (left - out, top),
        (right + out, top),
        (left, top - out),
        (left, bottom + out),
    ] {
        image.is(x, y, BLACK, what);
    }
}

#[test]
fn a_known_correction_moves_the_picture_by_the_pixels_it_says() {
    let Some(mut gpu) = gpu() else { return };
    // Uncorrected, and with the correction that is none.
    let plain = steadied(&mut gpu, None);
    square_at(&plain, [24, 28], 8, 0, "the picture as it is");
    let same = steadied(&mut gpu, Some(Sampling::IDENTITY));
    assert_eq!(plain.0, same.0, "no correction changes no pixel");

    // Six pixels right and four up, as fractions of the frame.
    let moved = Fix {
        shift: [6.0 / 64.0, -4.0 / 64.0],
        ..Fix::default()
    };
    let image = steadied(&mut gpu, Some(moved.sampling(1.0, 1.0)));
    square_at(&image, [30, 24], 8, 0, "moved six right and four up");
    // What the move brought in from past the picture's edge is the edge.
    image.is(0, 32, BLACK, "the left border");

    // Half a pixel: the square's edges are half covered.
    let half = Fix {
        shift: [0.5 / 64.0, 0.0],
        ..Fix::default()
    };
    let image = steadied(&mut gpu, Some(half.sampling(1.0, 1.0)));
    let (left, inside, right) = (
        image.at(24, 32)[0],
        image.at(28, 32)[0],
        image.at(32, 32)[0],
    );
    assert_eq!(inside, 255);
    // Half of white, in linear light.
    assert!((180..=195).contains(&left), "the left edge is {left}");
    assert!((180..=195).contains(&right), "the right edge is {right}");
}

#[test]
fn a_crop_enlarges_the_picture_about_its_centre_and_a_turn_turns_it() {
    let Some(mut gpu) = gpu() else { return };
    // Twice: the square's corner at (24, 28) is eight left and four up of
    // the centre, and goes to sixteen left and eight up of it.
    let image = steadied(&mut gpu, Some(Fix::default().sampling(2.0, 1.0)));
    square_at(&image, [16, 24], 16, 1, "enlarged twice");

    // Moved and cropped: the move is in the frame's own pixels, so on the
    // enlarged picture it is twice as far.
    let moved = Fix {
        shift: [3.0 / 64.0, 0.0],
        ..Fix::default()
    };
    let image = steadied(&mut gpu, Some(moved.sampling(2.0, 1.0)));
    square_at(&image, [22, 24], 16, 1, "moved three and enlarged twice");

    // A quarter turn clockwise about the centre: the square, eight left of
    // the centre and four up to four down, goes to eight above it.
    let turned = Fix {
        turn: std::f32::consts::FRAC_PI_2,
        ..Fix::default()
    };
    let image = steadied(&mut gpu, Some(turned.sampling(1.0, 1.0)));
    square_at(&image, [28, 24], 8, 0, "turned a quarter clockwise");
}

#[test]
fn a_canvas_is_drawn_again_when_its_correction_changes() {
    let pics = [Pic::of(marked())];
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let turn = |ledger: &mut Ledger, steady: Option<Sampling>| {
        let mut job = job(Some(NIGHT), &pics);
        job.draws[0].steady = steady;
        ledger.begin_frame();
        ledger.turn(CANVAS, &[job]).verdicts
    };
    let moved = Fix {
        shift: [0.01, 0.0],
        ..Fix::default()
    };
    assert_eq!(turn(&mut ledger, None), vec![Verdict::Render]);
    assert_eq!(turn(&mut ledger, None), vec![Verdict::Skip]);
    // The path came: the same layer, picture and place, read otherwise.
    let read = Some(moved.sampling(1.1, 1.0));
    assert_eq!(turn(&mut ledger, read), vec![Verdict::Render]);
    assert_eq!(turn(&mut ledger, read), vec![Verdict::Skip]);
    assert_eq!(turn(&mut ledger, None), vec![Verdict::Render]);
}
