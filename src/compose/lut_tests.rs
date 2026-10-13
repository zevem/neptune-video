//! Lookup tables in the compositor: the pass a table is drawn with, what the
//! books count for it, and its pixels read back from a real device.
//!
//! Every table here is the text of a `.cube` file read by the parser the
//! import uses. What a pixel should be is worked out by the parser's own
//! lookup, on the bytes as they are: a table is of display colour, not of
//! linear light.
//!
//! As in `gpu_tests`, a test without an adapter prints `SKIP:` and passes.
use super::{
    ComposeBudget, Ledger, Verdict,
    gpu::Gpu,
    gpu_tests::{
        AMBER, BLUE, CANVAS, GREEN, Image, NIGHT, Pic, RED, composer, draw, encoded, gpu, job,
        linear, quartered,
    },
    lut::{self, MAX_LUTS},
    pass::{self, Second},
};
use editor_core::{EffectKind, ResolvedEffect};
use editor_media::{Cube, Frame};
use std::sync::Arc;
use uuid::Uuid;

const SIDE: u32 = 64;

/// The text of a `.cube` file of `size` that maps each colour by `map`.
fn cube_text(size: u32, map: impl Fn([f32; 3]) -> [f32; 3]) -> String {
    let mut text = format!("TITLE \"test\"\r\n# a comment\r\nLUT_3D_SIZE {size}\r\n\r\n");
    let last = (size - 1) as f32;
    for blue in 0..size {
        for green in 0..size {
            for red in 0..size {
                let [r, g, b] = map([red as f32 / last, green as f32 / last, blue as f32 / last]);
                text.push_str(&format!("{r:.7} {g:.7} {b:.7}\r\n"));
            }
        }
    }
    text
}

fn cube(size: u32, map: impl Fn([f32; 3]) -> [f32; 3]) -> Arc<Cube> {
    Arc::new(Cube::parse(&cube_text(size, map)).expect("the text is a table"))
}

fn lookup(mix: f32) -> ResolvedEffect {
    ResolvedEffect {
        id: Uuid::new_v4(),
        kind: EffectKind::Lut,
        mix,
        params: Vec::new(),
        points: Vec::new(),
        asset_id: Some(Uuid::new_v4()),
    }
}

impl Pic {
    /// The picture looked up in `table`, which is uploaded as `serial`.
    fn looked_up(mut self, table: &Arc<Cube>, serial: u64, mix: f32) -> Self {
        let effect = lookup(mix);
        self.luts.push((effect.id, serial, Arc::clone(table)));
        self.layer.effects.push(effect);
        self
    }
}

/// A picture of the canvas's size, a texel a pixel, with every channel
/// running through its values at a pace of its own.
fn ramps() -> Frame {
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (x, y) = (x as u8, y as u8);
            rgba.extend_from_slice(&[x * 4 + 3, y * 4 + 1, (x * 2 + y * 2) ^ 0x55, 255]);
        }
    }
    Frame {
        width: SIDE,
        height: SIDE,
        rgba: rgba.into(),
    }
}

/// What `table` makes of a colour's bytes.
fn found(table: &Cube, [r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let out = table.sample([r, g, b].map(|byte| f32::from(byte) / 255.0));
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(out[0]), byte(out[1]), byte(out[2]), a]
}

/// The most any channel of any pixel differs by between two canvases.
fn furthest(a: &Image, b: &Image) -> u8 {
    (a.0.iter().zip(&b.0))
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_lookup_is_one_pass_given_its_table_and_none_without() {
    let table = cube(17, |c| c);
    let pic = Pic::full(RED).looked_up(&table, 5, 0.75);
    let effect = pic.layer.effects[0].id;
    // Until the table has come the layer is left as it is, at no cost.
    assert_eq!(pass::chain_len(&pic.layer), 1);
    assert!(pass::chain(&pic.layer, &pic.place, CANVAS, &[]).is_empty());
    let other = [(
        Uuid::new_v4(),
        lut::Lut {
            serial: 5,
            table: &table,
        }
        .told(),
    )];
    assert!(pass::chain_with(&pic.layer, &pic.place, CANVAS, &[], &other).is_empty());

    let mut wide = (*table).clone();
    wide.domain_min = [0.0, 0.25, -1.0];
    wide.domain_max = [2.0, 0.75, 1.0];
    let told = [(
        effect,
        lut::Lut {
            serial: 5,
            table: &wide,
        }
        .told(),
    )];
    let chain = pass::chain_with(&pic.layer, &pic.place, CANVAS, &[], &told);
    assert_eq!(chain.len(), 1);
    assert_eq!((chain[0].first, chain[0].into), (0, 1));
    assert_eq!(chain[0].second, Second::Lut(5));
    let expected = pass::Uniform::new(CANVAS)
        .seed(pic.layer.seed)
        .mix(0.75)
        .op(pass::OP_LUT)
        .variant(5)
        .count(17)
        .set(4, [0.0, 0.25, -1.0, 0.0])
        .set(5, [0.5, 2.0, 0.5, 0.0]);
    assert_eq!(chain[0].uniform, expected);
    // A lookup goes to and fro on two targets, like any pass on colour.
    assert_eq!(pass::chain_slots(&pic.layer), 2);
    assert_eq!(job(None, std::slice::from_ref(&pic)).passes(), 4);
}

#[test]
fn the_books_count_a_table_once_and_let_go_of_one_no_canvas_is_drawn_with() {
    let fixed = 1000;
    let want = [64, 64];
    let canvas = 64 * 64 * 4;
    let picture = 16 * 16 * 4;
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, fixed);
    let small = cube(2, |c| c);
    let large = cube(33, |c| c);
    // The colours as they are kept and the texture, at eight bytes a texel.
    assert_eq!(lut::bytes(&small), 8 * 12 + 4 * 2 * 8);
    assert_eq!(lut::bytes(&large), 33 * 33 * 33 * 12 + 198 * 198 * 8);

    let pics = [
        Pic::full(RED).looked_up(&small, 1, 1.0),
        Pic::full(BLUE).looked_up(&small, 1, 1.0),
    ];
    let turn = ledger.turn(want, &[job(Some(NIGHT), &pics)]);
    assert_eq!(turn.verdicts, vec![Verdict::Render]);
    // Two canvases to blend between, two a layer goes to and fro on, and
    // the one shown; one table drawn with twice is held once.
    assert_eq!(
        ledger.bytes(),
        fixed + 2 * picture + lut::bytes(&small) + 5 * canvas
    );
    assert!(ledger.live_lut(1) && !ledger.live_lut(2));
    // Paused, nothing is drawn again.
    assert_eq!(
        ledger.turn(want, &[job(Some(NIGHT), &pics)]).verdicts,
        vec![Verdict::Skip]
    );

    // The table read anew, or another: the canvas is drawn again and the
    // old one is let go.
    let mut again = [Pic::full(RED)];
    again[0].serial = pics[0].serial;
    again[0].layer = pics[0].layer.clone();
    again[0].luts = vec![(pics[0].layer.effects[0].id, 2, Arc::clone(&large))];
    let turn = ledger.turn(want, &[job(Some(NIGHT), &again)]);
    assert_eq!(turn.verdicts, vec![Verdict::Render]);
    assert_eq!(
        ledger.bytes(),
        fixed + picture + lut::bytes(&large) + 5 * canvas
    );
    assert!(!ledger.live_lut(1) && ledger.live_lut(2));

    // A table that has not come yet costs nothing and is not waited for.
    let mut bare = Pic::full(RED).looked_up(&small, 3, 1.0);
    bare.luts.clear();
    ledger.turn(want, &[job(Some(NIGHT), &[bare])]);
    assert_eq!(ledger.bytes(), fixed + picture + 5 * canvas);

    // A budget the tables leave no canvas in refuses the frame.
    let mut tight = Ledger::new(
        ComposeBudget {
            bytes: lut::bytes(&large) + 1000,
        },
        0,
    );
    let turn = tight.turn(want, &[job(Some(NIGHT), &again)]);
    assert_eq!(turn.verdicts, vec![Verdict::Refuse]);
    assert_eq!(tight.bytes(), 0);
}

#[test]
fn a_frame_is_drawn_with_four_tables_and_refused_with_five() {
    let table = cube(2, |c| c);
    let mut ledger = Ledger::new(ComposeBudget::PRODUCTION, 0);
    let pics: Vec<Pic> = (0..=MAX_LUTS as u64)
        .map(|serial| Pic::full(RED).looked_up(&table, 100 + serial, 1.0))
        .collect();
    let turn = ledger.turn([64, 64], &[job(Some(NIGHT), &pics[..MAX_LUTS])]);
    assert_eq!(turn.verdicts, vec![Verdict::Render]);
    assert_eq!(
        ledger.bytes(),
        4 * 16 * 16 * 4 + 4 * lut::bytes(&table) + 5 * 64 * 64 * 4
    );
    let turn = ledger.turn([64, 64], &[job(Some(NIGHT), &pics)]);
    assert_eq!(turn.verdicts, vec![Verdict::Refuse]);
    assert_eq!(
        ledger.bytes(),
        0,
        "nothing is held for a frame that is refused"
    );
    assert!(!ledger.live_lut(100));
}

#[test]
fn an_identity_table_leaves_every_colour_within_one_code() {
    let Some(mut gpu) = gpu() else { return };
    let plain = draw(&mut gpu, Some(NIGHT), &[Pic::of(ramps())]);
    // The picture as it is drawn is the picture: the ramps are what the
    // lookups below are held to.
    plain.is(10, 20, [43, 81, 60 ^ 0x55, 255], "the ramps as they are");
    for (serial, size) in [(1, 2), (2, 3), (3, 17), (4, 33), (5, 65)] {
        let identity = cube(size, |c| c);
        let pic = Pic::of(ramps()).looked_up(&identity, serial, 1.0);
        let image = draw(&mut gpu, Some(NIGHT), &[pic]);
        let off = furthest(&plain, &image);
        assert!(
            off <= 1,
            "an identity of {size} moves a colour by {off} codes"
        );
    }
    assert_eq!(gpu.luts(), 5);
    gpu.keep_luts(|serial| serial == 5);
    assert_eq!(gpu.luts(), 1);
}

#[test]
fn a_channel_swap_table_swaps_the_channels() {
    let Some(mut gpu) = gpu() else { return };
    let swap = cube(2, |[r, g, b]| [b, g, r]);
    let colours = [RED, BLUE, GREEN, AMBER];
    let pic = Pic::of(quartered(colours)).looked_up(&swap, 1, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    for ((x, y), [r, g, b, a]) in [(16, 16), (48, 16), (16, 48), (48, 48)]
        .into_iter()
        .zip(colours)
    {
        let actual = image.at(x, y);
        let near = (actual.iter().zip(&[b, g, r, a])).all(|(a, e)| a.abs_diff(*e) <= 1);
        assert!(
            near,
            "({x}, {y}) is {actual:?}, expected {:?}",
            [b, g, r, a]
        );
    }
    // And every colour of the ramps, not only four.
    let plain = draw(&mut gpu, Some(NIGHT), &[Pic::of(ramps())]);
    let swapped = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::of(ramps()).looked_up(&swap, 1, 1.0)],
    );
    for (before, after) in plain.0.chunks_exact(4).zip(swapped.0.chunks_exact(4)) {
        let expected = [before[2], before[1], before[0], before[3]];
        assert!(
            (after.iter().zip(&expected)).all(|(a, e)| a.abs_diff(*e) <= 1),
            "{before:?} became {after:?}"
        );
    }
}

#[test]
fn a_table_is_looked_up_in_display_colour_between_its_points() {
    let Some(mut gpu) = gpu() else { return };
    // A curve that is far from a line, on few points: what is between them
    // is the mix of the points, in the encoding the file is of. Looked up
    // in linear light, a middle grey would come out several codes away.
    let table = cube(5, |[r, g, b]| [r * r, g.sqrt(), 1.0 - b]);
    let pic = Pic::of(ramps()).looked_up(&table, 1, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    let source = ramps();
    let mut worst = 0;
    for (before, after) in source.rgba.chunks_exact(4).zip(image.0.chunks_exact(4)) {
        let expected = found(&table, [before[0], before[1], before[2], before[3]]);
        let off = (after.iter().zip(&expected)).map(|(a, e)| a.abs_diff(*e));
        worst = worst.max(off.max().unwrap_or(0));
    }
    assert!(
        worst <= 2,
        "a looked-up colour is {worst} codes from the table's"
    );
    // The two readings differ by far more than that slack.
    let grey = 128u8;
    let encoded_reading = found(&table, [grey; 4])[2];
    let linear_reading = {
        let light = linear(grey) as f32;
        encoded(f64::from(table.sample([light; 3])[2]))
    };
    assert!(encoded_reading.abs_diff(linear_reading) > 20);
    let pic = Pic::full([grey, grey, grey, 255]).looked_up(&table, 1, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    assert!(image.at(32, 32)[2].abs_diff(encoded_reading) <= 2);
}

#[test]
fn a_tables_domain_scales_what_is_looked_up_and_its_edge_holds_what_is_past_it() {
    let Some(mut gpu) = gpu() else { return };
    // The table covers inputs up to two: a colour is found half way to
    // where it would be.
    let mut wide = (*cube(9, |c| c)).clone();
    wide.domain_max = [2.0; 3];
    let wide = Arc::new(wide);
    let pic = Pic::full([200, 100, 50, 255]).looked_up(&wide, 1, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(32, 32, [100, 50, 25, 255], "half way into a domain of two");
    // And one that covers the middle half alone holds its ends past it.
    let mut narrow = (*cube(9, |c| c)).clone();
    narrow.domain_min = [0.25; 3];
    narrow.domain_max = [0.75; 3];
    let narrow = Arc::new(narrow);
    let pic = Pic::full([255, 128, 0, 255]).looked_up(&narrow, 2, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    let expected = found(&narrow, [255, 128, 0, 255]);
    assert!(expected[0] == 255 && expected[1].abs_diff(128) <= 1 && expected[2] == 0);
    image.is(32, 32, expected, "past, in and before a narrow domain");
}

#[test]
fn a_lookup_is_mixed_in_by_its_mix_and_keeps_what_is_clear_clear() {
    let Some(mut gpu) = gpu() else { return };
    let swap = cube(2, |[r, g, b]| [b, g, r]);
    let none = Pic::full(RED).looked_up(&swap, 1, 0.0);
    draw(&mut gpu, Some(NIGHT), &[none]).is(32, 32, RED, "nothing mixed in");
    // Half of each, in linear light like every effect's mix.
    let half = Pic::full(RED).looked_up(&swap, 1, 0.5);
    let [r, g, b, _] = RED;
    let between = |a: u8, b: u8| encoded(0.5 * linear(a) + 0.5 * linear(b));
    let expected = [between(r, b), g, between(b, r), 255];
    draw(&mut gpu, Some(NIGHT), &[half]).is(32, 32, expected, "half mixed in");

    // A layer that covers a part of the canvas: the rest is the canvas's
    // colour, not that colour looked up.
    let invert = cube(2, |[r, g, b]| [1.0 - r, 1.0 - g, 1.0 - b]);
    let small = Pic::full(RED)
        .at([16.0, 16.0], [8.0, 8.0])
        .looked_up(&invert, 2, 1.0);
    let image = draw(&mut gpu, Some(NIGHT), &[small]);
    image.is(16, 16, [35, 215, 215, 255], "red, inverted");
    image.is(48, 48, NIGHT, "the canvas beside the layer");
}

#[test]
fn a_lookup_without_its_table_leaves_the_layer_as_it_is() {
    let Some(mut gpu) = gpu() else { return };
    let mut pic = Pic::of(quartered([RED, BLUE, GREEN, AMBER])).looked_up(&cube(2, |c| c), 1, 1.0);
    pic.luts.clear();
    let image = draw(&mut gpu, Some(NIGHT), &[pic]);
    image.is(16, 16, RED, "no table to look up in");
    image.is(48, 48, AMBER, "no table to look up in");
    assert_eq!(gpu.luts(), 0);
}

#[test]
fn the_composer_holds_a_table_while_it_is_drawn_with_and_draws_a_paused_one_once() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let swap = cube(17, |[r, g, b]| [b, g, r]);
    let pics = [Pic::full(RED).looked_up(&swap, 1, 1.0)];
    let held = |composer: &super::Composer| composer.gpu.as_ref().map_or(0, Gpu::bytes);
    let first = composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    assert!(first[0].is_some());
    let image = Image(composer.gpu.as_ref().expect("a device").read(0));
    let [r, g, b, a] = RED;
    image.is(32, 32, [b, g, r, a], "red with its channels swapped");
    let with_table = held(&composer);
    for _ in 0..3 {
        composer.begin_frame();
        assert_eq!(composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]), first);
    }
    let counters = composer.counters();
    assert_eq!((counters.composed, counters.skipped), (1, 3));
    // One picture and one table went up, once.
    assert_eq!(composer.gpu.as_ref().map(Gpu::uploads), Some(2));

    // The effect switched off: the table is no longer held.
    let mut bare = [Pic::full(RED)];
    bare[0].serial = pics[0].serial;
    bare[0].layer.blend = editor_core::BlendMode::Screen;
    composer.frame(CANVAS, &[job(Some(NIGHT), &bare)]);
    assert_eq!(composer.gpu.as_ref().map(Gpu::luts), Some(0));
    // The layer with a lookup held one target more than the blended one.
    assert_eq!(
        with_table - held(&composer),
        lut::bytes(&swap) + 64 * 64 * 4,
        "the table's bytes are counted while it is held"
    );
    composer.release();
    assert_eq!(held(&composer), 0);
}
