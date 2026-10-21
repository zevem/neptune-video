//! Issue 13: what the viewer's layers add up to.
//!
//! Two seams. The first composes layers with [`Textures::compose`] in a real
//! egui context, at boxes the viewer could ask at. The second paints a
//! timeline with [`show`] on the [`Stage`] of the tests this file is nested
//! under. In both, the frame that arrives for a layer fills the box the layer
//! asks at, which is the most it can be sent, and what is kept is read from
//! the context's texture manager: the textures named `layer-…` and the bytes
//! of their texels. What a [`Textures`] counts itself as keeping is only ever
//! compared with that.
//!
//! A failure says what kind it is. `ISSUE13:` is the behavior the issue asks
//! for. `CONTROL:` holds before the correction and must hold after it.
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the viewer.
//!
//! Nothing here decodes, and no media service is given: a layer that waits
//! for room is told from one that asks by [`Textures::asked`], which is
//! nothing for a layer that asks at no box.
use super::*;
use crate::textures::{LayerUsage, ViewerBudget, Wanted, layer_textures};
use editor_media::PreviewConsumer;

const MIB: usize = 1024 * 1024;
const VIOLET: [u8; 4] = [150, 70, 210, 255];
const TEAL: [u8; 4] = [30, 170, 170, 255];
/// One picture for each clip hung on a [`Wall`].
const COLOURS: [[u8; 4]; 6] = [RED, BLUE, AMBER, GREEN, VIOLET, TEAL];
/// Canvas pixels between the rows clips are hung in. With [`SCALE`], layers
/// in different rows never overlap.
const ROW: f32 = 360.0;
/// The step a clip at [`SCALE`] needs in the stage's window, and the step the
/// viewer budget's ladder has under it.
const HUNG: u32 = 320;
const UNDER_HUNG: u32 = 256;
/// The box a clip at [`SCALE`] asks at in the stage's window: that step, in
/// the shape of the scenes' asset, which is 1920 by 1080.
const HUNG_BOX: [u32; 2] = [HUNG, 180];
/// The picture of the scenes' asset, whole.
const WHOLE: [u32; 2] = [1920, 1080];

fn square(side: u32) -> [u32; 2] {
    [side, side]
}

/// The RGBA bytes of a box, or nothing where they cannot be counted.
fn box_bytes([width, height]: [u32; 2]) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

fn texels([width, height]: [usize; 2]) -> usize {
    width * height * 4
}

fn mib(bytes: usize) -> String {
    format!("{:.4} MiB", bytes as f64 / MIB as f64)
}

/// A frame that fills `size`: the most a layer asking at that box is sent.
fn filling([width, height]: [u32; 2], colour: [u8; 4]) -> Frame {
    Frame {
        width,
        height,
        rgba: colour.repeat(width as usize * height as usize).into(),
    }
}

/// The layer textures `ctx` holds, which must be those `textures` keeps.
fn held(ctx: &egui::Context, textures: &Textures) -> LayerUsage {
    let held = layer_textures(ctx);
    assert_eq!(
        textures.layer_usage(),
        held,
        "CONTROL: the layers' own handles are the left; the layer textures the \
         context holds are the right"
    );
    held
}

/// Holds the layer textures of `ctx` to the viewer budget of `textures`.
fn within_budget(ctx: &egui::Context, textures: &Textures, when: &str) {
    let budget = textures.viewer_budget().bytes;
    let held = held(ctx, textures);
    assert!(
        held.bytes <= budget,
        "ISSUE13: {when}, the layer textures hold {} bytes ({}) in {} pictures where \
         the viewer budget is {budget} bytes ({})",
        held.bytes,
        mib(held.bytes),
        held.pictures,
        mib(budget)
    );
}

/// A frame for the layer kept for `viewing`, filling the box it asks at.
/// Returns whether it was taken; a layer that asks at no box is sent none.
fn arrive_filling(
    ctx: &egui::Context,
    textures: &mut Textures,
    viewing: Viewing,
    asset: &Asset,
    colour: [u8; 4],
) -> bool {
    let Some((consumer, size)) = textures.asked(viewing) else {
        return false;
    };
    let frame = filling(size, colour);
    let time = textures.admitted(consumer);
    textures.accept(
        ctx,
        Some(asset),
        Some(consumer),
        asset.id,
        time,
        size,
        &frame,
    )
}

/// Whether the layer kept for `viewing` does not yet show a frame that fills
/// the box it asks at: it asks at none, has no picture, or has one of another
/// size.
fn waits(textures: &Textures, viewing: Viewing) -> bool {
    let Some((_, [width, height])) = textures.asked(viewing) else {
        return true;
    };
    let wanted = [width as usize, height as usize];
    textures.picture(viewing).map(|picture| picture.size()) != Some(wanted)
}

// ---- Layers composed without a viewer ---------------------------------------

/// Layers of an asset each, composed in a real context.
struct Bench {
    ctx: egui::Context,
    textures: Textures,
    assets: Vec<Asset>,
    clips: Vec<ClipId>,
}

impl Bench {
    fn new(budget: ViewerBudget, layers: usize) -> Self {
        Self {
            ctx: egui::Context::default(),
            textures: Textures::with_viewer_budget(budget),
            assets: (0..layers)
                .map(|index| video(&format!("/media/layer-{index}.mp4")))
                .collect(),
            clips: (0..layers).map(|_| ClipId::new_v4()).collect(),
        }
    }

    fn viewing(&self, layer: usize) -> Viewing {
        Viewing::Clip(self.clips[layer])
    }

    fn budget(&self) -> usize {
        self.textures.viewer_budget().bytes
    }

    /// One frame that draws the layers `drawn` names, each wanted at a square
    /// box of the side given, in that order. Returns how many went unplaced.
    fn frame(&mut self, playing: bool, drawn: &[(usize, u32)]) -> usize {
        self.textures.begin_frame();
        let wanted: Vec<Wanted<'_>> = drawn
            .iter()
            .map(|&(layer, side)| Wanted {
                viewing: Viewing::Clip(self.clips[layer]),
                asset: &self.assets[layer],
                size: square(side),
                time: Time::ZERO,
                carries_on: Vec::new(),
            })
            .collect();
        self.textures.compose(None, playing, &wanted).unplaced
    }

    /// The box the layer asks at.
    fn granted(&self, layer: usize) -> Option<[u32; 2]> {
        self.textures
            .asked(self.viewing(layer))
            .map(|(_, size)| size)
    }

    fn consumer(&self, layer: usize) -> Option<PreviewConsumer> {
        self.textures
            .asked(self.viewing(layer))
            .map(|(consumer, _)| consumer)
    }

    /// The size of the picture the layer has.
    fn picture(&self, layer: usize) -> Option<[usize; 2]> {
        self.textures
            .picture(self.viewing(layer))
            .map(|picture| picture.size())
    }

    fn granted_all(&self) -> Vec<Option<[u32; 2]>> {
        (0..self.clips.len())
            .map(|layer| self.granted(layer))
            .collect()
    }

    fn pictures(&self) -> Vec<Option<[usize; 2]>> {
        (0..self.clips.len())
            .map(|layer| self.picture(layer))
            .collect()
    }

    fn held(&self) -> LayerUsage {
        held(&self.ctx, &self.textures)
    }

    /// What the layers hold room for: for each, the larger of the picture it
    /// has and the box it asks at. Nothing where that cannot be counted.
    fn reserved(&self) -> Option<usize> {
        (0..self.clips.len()).try_fold(0usize, |sum, layer| {
            let kept = self.picture(layer).map_or(0, texels);
            let asked = match self.granted(layer) {
                Some(size) => box_bytes(size)?,
                None => 0,
            };
            sum.checked_add(kept.max(asked))
        })
    }

    /// A frame for the layer, filling the box it asks at, with nothing held
    /// to the budget. Returns whether it was taken.
    fn fill(&mut self, layer: usize) -> bool {
        let viewing = self.viewing(layer);
        arrive_filling(
            &self.ctx,
            &mut self.textures,
            viewing,
            &self.assets[layer],
            RED,
        )
    }

    /// The frames `layers` ask for arrive in that order, each filling its
    /// box. The layer textures are held to the budget after each, and so is
    /// the room that was held before the first.
    fn arrivals(&mut self, layers: &[usize], when: &str) {
        let before = self.reserved();
        for layer in layers {
            if self.fill(*layer) {
                let when = format!("{when}, after a frame arrived for layer {layer}");
                within_budget(&self.ctx, &self.textures, &when);
            }
        }
        let budget = self.budget();
        assert!(
            before.is_some_and(|reserved| reserved <= budget),
            "ISSUE13: {when}, before a frame arrived, the layers held room for \
             {before:?} bytes in pictures and boxes asked at, where the viewer budget \
             is {budget} bytes"
        );
    }

    /// Frames and arrivals until every layer drawn shows a frame that fills
    /// the box it asks at. Returns how many went unplaced in the last frame.
    fn settle(&mut self, playing: bool, drawn: &[(usize, u32)], when: &str) -> usize {
        self.settle_as(playing, drawn, Some(when))
    }

    /// As [`Self::settle`], with nothing held to the budget.
    fn settle_unheld(&mut self, playing: bool, drawn: &[(usize, u32)]) -> usize {
        self.settle_as(playing, drawn, None)
    }

    fn settle_as(&mut self, playing: bool, drawn: &[(usize, u32)], when: Option<&str>) -> usize {
        for _ in 0..16 {
            let unplaced = self.frame(playing, drawn);
            let waiting: Vec<usize> = drawn
                .iter()
                .map(|(layer, _)| *layer)
                .filter(|layer| waits(&self.textures, self.viewing(*layer)))
                .collect();
            if waiting.is_empty() {
                return unplaced;
            }
            match when {
                Some(when) => self.arrivals(&waiting, when),
                None => {
                    for layer in waiting {
                        self.fill(layer);
                    }
                }
            }
        }
        panic!(
            "ISSUE13: after sixteen frames with every frame asked for arriving, drawn \
             layers still wait. They ask at {:?} and have pictures of {:?}",
            self.granted_all(),
            self.pictures()
        );
    }
}

/// `count` layers, the first ones of a bench, each wanted at `side`.
fn each_at(count: usize, side: u32) -> Vec<(usize, u32)> {
    (0..count).map(|layer| (layer, side)).collect()
}

#[test]
fn five_drawn_layers_under_a_budget_for_three_each_keep_a_picture_within_it() {
    let side = 64;
    let one = texels([64, 64]);
    let budget = ViewerBudget {
        bytes: 3 * one,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 5);
    let drawn = each_at(5, side);
    let unplaced = bench.settle(false, &drawn, "five layers drawn under a budget for three");
    assert_eq!(
        unplaced, 0,
        "ISSUE13: layers are reported unplaced where each could have a picture"
    );
    assert!(
        bench.pictures().iter().all(Option::is_some),
        "ISSUE13: a drawn layer has no picture. They have {:?}",
        bench.pictures()
    );
    assert_eq!(
        bench.held().pictures,
        5,
        "ISSUE13: the context does not hold a layer texture for each drawn layer"
    );
    bench.arrivals(&[], "five layers settled under a budget for three");
}

#[test]
fn five_layers_at_the_largest_size_stay_within_the_production_budget() {
    let production = Textures::default().viewer_budget();
    assert_eq!(
        (production.bytes, production.spare_layers),
        (64 * MIB, 8),
        "CONTROL: the viewer budget in production is not 64 MiB and eight spare layers"
    );
    assert_eq!(
        (production.bytes, production.spare_layers),
        (
            ViewerBudget::PRODUCTION.bytes,
            ViewerBudget::PRODUCTION.spare_layers
        ),
        "CONTROL: the viewer is not given its production budget by default"
    );
    let mut bench = Bench::new(production, 5);
    // Five boxes of 1920 a side are 70.3125 MiB.
    let drawn = each_at(5, 1920);
    bench.settle(false, &drawn, "five layers drawn at 1920");
    assert!(
        bench.pictures().iter().all(Option::is_some),
        "ISSUE13: a drawn layer has no picture. They have {:?}",
        bench.pictures()
    );
    bench.arrivals(&[], "five layers settled at the production budget");
}

#[test]
fn a_fifth_layer_fits_beside_four_full_ones_and_a_sixth_waits_for_room() {
    const FULL: usize = 1920 * 1920 * 4;
    const LESS: usize = 1280 * 1280 * 4;
    let mut bench = Bench::new(ViewerBudget::PRODUCTION, 6);
    let four = each_at(4, 1920);
    let six = each_at(6, 1920);
    bench.settle(false, &four, "four layers drawn at 1920");
    assert_eq!(
        bench.held().bytes,
        4 * FULL,
        "CONTROL: four layers at 1920 are not kept whole, at 56.25 MiB"
    );

    // Two more come onto the screen, paused. A fifth at 1280 makes 62.5 MiB;
    // a sixth would make 68.75 MiB.
    bench.frame(false, &six);
    assert_eq!(
        bench.pictures()[..4],
        [Some([1920, 1920]); 4],
        "CONTROL: a layer did not hold its own picture when more layers were drawn"
    );
    bench.arrivals(&[4], "a fifth layer drawn beside four at 1920");
    assert_eq!(
        (bench.picture(4), bench.held().bytes),
        (Some([1280, 1280]), 4 * FULL + LESS),
        "ISSUE13: the fifth layer and what is held are the left; a picture at 1280 \
         beside the four, 62.5 MiB, is the right"
    );
    assert_eq!(
        bench.granted(5),
        None,
        "ISSUE13: a sixth layer is asked for while the four hold their pictures: \
         68.75 MiB would be held"
    );
    assert_eq!(
        bench.granted_all()[..4],
        [Some(square(1280)); 4],
        "ISSUE13: the four are not asked at the size that lets six fit"
    );
    assert_eq!(
        bench.pictures()[..4],
        [Some([1920, 1920]); 4],
        "ISSUE13: a layer gave up its picture before the smaller one arrived"
    );

    // One of the four's smaller frames lands. The next frame has the room.
    bench.arrivals(&[0], "the first smaller frame");
    assert_eq!(bench.held().bytes, 3 * FULL + 2 * LESS);
    bench.frame(false, &six);
    assert_eq!(
        bench.granted(5),
        Some(square(1280)),
        "ISSUE13: the sixth layer still waits in the frame after room was freed"
    );
    bench.arrivals(
        &[5, 1, 2, 3],
        "the sixth layer and the other smaller frames",
    );
    assert_eq!(
        (bench.pictures(), bench.held().bytes),
        (vec![Some([1280, 1280]); 6], 6 * LESS),
        "ISSUE13: six layers did not settle at 1280 each, 37.5 MiB"
    );
}

#[test]
fn layers_no_longer_drawn_are_kept_only_as_spares_after_one_frame() {
    let side = 32;
    let cases = [
        (
            ViewerBudget {
                bytes: ViewerBudget::PRODUCTION.bytes,
                spare_layers: 2,
            },
            5,
        ),
        (ViewerBudget::PRODUCTION, 12),
    ];
    for (budget, count) in cases {
        // One layer more than is drawn at first.
        let mut bench = Bench::new(budget, count + 1);
        let drawn = each_at(count, side);
        bench.settle(false, &drawn, "layers drawn within the budget");
        assert_eq!(
            bench.held().pictures,
            count,
            "CONTROL: not every drawn layer has a picture"
        );
        let asked: Vec<_> = (0..count).map(|layer| bench.consumer(layer)).collect();

        // One frame that draws none of them. No frame arrives and no other
        // frame is painted: paused, there is nothing to count frames by.
        bench.frame(false, &[]);
        let held = bench.held();
        assert!(
            held.pictures <= budget.spare_layers,
            "ISSUE13: one frame after {count} layers went undrawn, {} of their \
             pictures ({} bytes) are kept where {} spare layers are allowed",
            held.pictures,
            held.bytes,
            budget.spare_layers
        );
        assert!(
            bench.textures.layers_kept() <= budget.spare_layers,
            "ISSUE13: {} undrawn layers are kept where {} are allowed",
            bench.textures.layers_kept(),
            budget.spare_layers
        );
        bench.frame(false, &[(count, side)]);
        assert!(
            bench.textures.layers_kept() <= 1 + budget.spare_layers,
            "ISSUE13: {} layers are kept where one is drawn and {} spare are allowed",
            bench.textures.layers_kept(),
            budget.spare_layers
        );

        // What was still decoding for a layer given up has nowhere to arrive.
        let mut given_up = 0;
        for (layer, old) in asked.iter().enumerate() {
            let Some(old) = *old else {
                panic!("FIXTURE: a settled layer asked under no consumer");
            };
            if bench.consumer(layer).is_some() {
                continue;
            }
            given_up += 1;
            let before = bench.held();
            let asset = &bench.assets[layer];
            let late = filling(square(side), BLUE);
            let time = bench.textures.admitted(old);
            let taken = bench.textures.accept(
                &bench.ctx,
                Some(asset),
                Some(old),
                asset.id,
                time,
                square(side),
                &late,
            );
            let wanted = bench
                .textures
                .failed(Some(old), asset.id, Some(square(side)));
            assert!(
                !taken && !wanted && bench.held() == before,
                "ISSUE13: a frame or a failure for a layer that was given up was taken"
            );
        }
        assert_eq!(
            given_up,
            count - budget.spare_layers,
            "ISSUE13: the layers given up are not those past the spare ones"
        );

        // Drawn again, one given up is a new layer with nothing to show.
        let Some(back) = (0..count).find(|layer| bench.consumer(*layer).is_none()) else {
            panic!("FIXTURE: no layer was given up");
        };
        bench.frame(false, &[(back, side)]);
        assert!(
            bench.consumer(back).is_some()
                && bench.consumer(back) != asked[back]
                && bench.picture(back).is_none(),
            "ISSUE13: a layer drawn again after it was given up asks under the \
             consumer it had, or shows a picture"
        );
    }
}

#[test]
fn a_frame_larger_than_the_box_it_was_asked_at_is_not_kept() {
    let one = texels([32, 32]);
    let budget = ViewerBudget {
        bytes: one,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 1);
    bench.settle(false, &[(0, 32)], "one layer drawn");
    let Some((consumer, size)) = bench.textures.asked(bench.viewing(0)) else {
        panic!("FIXTURE: the drawn layer asks at nothing");
    };
    fixture(
        bench.picture(0) == Some([32, 32]) && size == square(32),
        "the layer shows a picture that fills its box",
    );

    // Addressed as it was asked for, and four times the texels of the box.
    let asset = &bench.assets[0];
    let large = filling(square(64), BLUE);
    let time = bench.textures.admitted(consumer);
    bench.textures.accept(
        &bench.ctx,
        Some(asset),
        Some(consumer),
        asset.id,
        time,
        size,
        &large,
    );
    let held = bench.held();
    assert!(
        held.bytes <= one,
        "ISSUE13: a frame of 64×64 arrived for a layer that asks at 32×32 and was \
         kept: the layer textures hold {} bytes where the viewer budget is {one}",
        held.bytes
    );
    assert_eq!(
        bench.picture(0),
        Some([32, 32]),
        "ISSUE13: the layer lost the picture it had, which fitted its box"
    );
}

#[test]
fn a_shrink_that_fails_gives_up_that_picture_alone_and_lets_the_waiting_layer_in() {
    let one = texels([64, 64]);
    let budget = ViewerBudget {
        bytes: 2 * one,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 3);
    let three = each_at(3, 64);
    bench.settle(false, &each_at(2, 64), "two layers drawn");
    assert_eq!(
        (bench.pictures()[..2].to_vec(), bench.held().bytes),
        (vec![Some([64, 64]); 2], 2 * one),
        "CONTROL: two layers that fill the budget are not kept whole"
    );

    // A third comes onto the screen. The two must shrink before it has room.
    bench.frame(false, &three);
    bench.arrivals(&[2], "a third layer drawn beside two that fill the budget");
    assert_eq!(
        bench.granted(2),
        None,
        "ISSUE13: the third layer is asked for while the two fill the budget"
    );
    assert_eq!(
        bench.pictures()[..2],
        [Some([64, 64]); 2],
        "ISSUE13: a layer gave up its picture before the smaller one arrived"
    );

    // The first layer's smaller frame cannot be decoded.
    let Some((consumer, size)) = bench.textures.asked(bench.viewing(0)) else {
        panic!("ISSUE13: a layer that must shrink asks at nothing");
    };
    assert!(
        box_bytes(size).is_some_and(|bytes| bytes < one),
        "ISSUE13: a layer that must shrink asks at {size:?}, no smaller than before"
    );
    let asset = bench.assets[0].id;
    assert!(
        bench.textures.failed(Some(consumer), asset, Some(size)),
        "CONTROL: a failure of what a layer asks for now is not taken as its own"
    );
    assert_eq!(
        bench.pictures()[..2],
        [None, Some([64, 64])],
        "ISSUE13: after the first layer's smaller frame failed, the two have the \
         left. The first holds room the third waits for and its picture cannot be \
         replaced within the budget; the second's frame did not fail"
    );
    assert!(
        bench.consumer(0) == Some(consumer),
        "ISSUE13: the layer whose frame failed is no longer the clip's own"
    );

    // The next frame lets the third in, and the first asks again.
    bench.frame(false, &three);
    assert!(
        bench.granted(2).is_some(),
        "ISSUE13: the third layer still waits after the failed picture gave up \
         its room"
    );
    let unplaced = bench.settle(false, &three, "after a failed shrink");
    assert_eq!(unplaced, 0);
    assert!(
        bench.pictures().iter().all(Option::is_some),
        "ISSUE13: a drawn layer has no picture. They have {:?}",
        bench.pictures()
    );
}

#[test]
fn a_layer_wanted_larger_than_the_budget_has_room_for_keeps_the_picture_it_has() {
    let large = texels([64, 64]);
    let small = texels([32, 32]);
    let budget = ViewerBudget {
        bytes: large + small,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 2);
    bench.settle(false, &[(0, 64)], "one layer drawn");
    bench.settle(
        false,
        &[(0, 64), (1, 32)],
        "a smaller layer drawn beside it",
    );
    assert_eq!(
        (bench.pictures(), bench.held().bytes),
        (vec![Some([64, 64]), Some([32, 32])], large + small),
        "CONTROL: two layers that fill the budget are not kept as they were asked"
    );

    // The window grows: the second is now drawn as large as the first.
    let grown = [(0, 64), (1, 64)];
    bench.frame(false, &grown);
    assert_eq!(
        bench.pictures(),
        [Some([64, 64]), Some([32, 32])],
        "CONTROL: a layer did not hold its picture when it was wanted larger"
    );
    bench.arrivals(&[1], "a layer wanted larger than the budget has room for");
    bench.arrivals(&[0], "the other layer's frame");
    let unplaced = bench.settle(false, &grown, "two layers wanted at more than fits");
    assert_eq!(unplaced, 0);
    assert!(
        bench.pictures().iter().all(Option::is_some),
        "ISSUE13: a drawn layer has no picture. They have {:?}",
        bench.pictures()
    );

    // The window shrinks: each asks at the smaller box in that frame, and
    // holds what it has until the frame at that box arrives.
    let before = bench.pictures();
    bench.frame(false, &[(0, 16), (1, 16)]);
    assert_eq!(
        (bench.granted_all(), bench.pictures()),
        (vec![Some(square(16)); 2], before),
        "CONTROL: layers wanted smaller do not ask at the smaller box at once, or \
         did not hold their pictures"
    );
}

#[test]
fn no_layer_is_asked_for_more_while_playing() {
    let small = texels([32, 32]);
    let mut bench = Bench::new(ViewerBudget::PRODUCTION, 2);
    bench.settle(false, &[(0, 32)], "one layer drawn");
    assert_eq!(bench.held().bytes, small);

    // Playing, it is drawn larger. There is room, and it is not asked for
    // more: what it is sent still fills the box it asked at before.
    bench.frame(true, &[(0, 64)]);
    bench.arrivals(&[0], "a layer drawn larger while playing");
    assert_eq!(
        (bench.picture(0), bench.held().bytes),
        (Some([32, 32]), small),
        "ISSUE13: while playing, a layer drawn larger was asked for more. Its \
         picture and the bytes held are the left; what it had is the right"
    );

    // One that comes onto the screen while playing is asked for at once.
    bench.frame(true, &[(0, 64), (1, 64)]);
    assert_eq!(
        bench.granted_all(),
        [Some(square(32)), Some(square(64))],
        "ISSUE13: while playing, the left is asked at; a layer that had a box \
         keeps it and a new one is asked at what it is drawn at"
    );

    // The first paused frame asks for what is drawn.
    bench.frame(false, &[(0, 64), (1, 64)]);
    assert_eq!(
        bench.granted(0),
        Some(square(64)),
        "ISSUE13: paused, a layer is not asked at the size it is drawn at"
    );
    bench.settle(false, &[(0, 64), (1, 64)], "paused after playing");
    assert_eq!(bench.pictures(), [Some([64, 64]); 2]);
}

#[test]
fn layers_past_one_texel_each_are_reported_and_take_no_room() {
    // Room for two texels.
    let budget = ViewerBudget {
        bytes: 8,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 3);
    let drawn = each_at(3, 32);
    let unplaced = bench.frame(false, &drawn);
    bench.arrivals(
        &[0, 1, 2],
        "three layers drawn under a budget of two texels",
    );
    assert_eq!(
        unplaced, 1,
        "ISSUE13: three layers are drawn where two texels can be kept, and the \
         left were reported unplaced"
    );
    assert_eq!(
        (bench.granted_all(), bench.pictures(), bench.held().bytes),
        (
            vec![Some(square(1)), Some(square(1)), None],
            vec![Some([1, 1]), Some([1, 1]), None],
            8
        ),
        "ISSUE13: the first two painted do not each keep one texel, or the third \
         takes room"
    );
    // It is said again for as long as it is so, and nothing changes.
    assert_eq!(
        (bench.frame(false, &drawn), bench.held().bytes),
        (1, 8),
        "ISSUE13: the next frame does not report the same layer unplaced"
    );
}

#[test]
fn a_box_too_large_to_count_is_not_asked_for() {
    let mut bench = Bench::new(ViewerBudget::PRODUCTION, 1);
    let budget = bench.budget();
    bench.frame(false, &[(0, u32::MAX)]);
    let granted = bench.granted(0);
    assert!(
        granted
            .and_then(box_bytes)
            .is_some_and(|bytes| bytes <= budget),
        "ISSUE13: a layer wanted at {0}×{0} asks at {granted:?}, a box whose bytes \
         cannot be counted or kept in the viewer budget of {budget}",
        u32::MAX
    );
    assert!(
        bench.reserved().is_some_and(|reserved| reserved <= budget),
        "ISSUE13: the room held for it is {:?} bytes",
        bench.reserved()
    );
}

#[test]
fn a_settled_set_of_layers_does_not_change_with_nothing_arriving() {
    let one = texels([64, 64]);
    let budget = ViewerBudget {
        bytes: 3 * one,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 5);
    let drawn = each_at(5, 64);
    // More than the budget was drawn for; whether it is held to is not this
    // test's question.
    let unplaced = bench.settle_unheld(false, &drawn);
    assert_eq!(
        unplaced, 0,
        "CONTROL: a layer that could have a picture is reported unplaced"
    );
    let ids = |bench: &Bench| -> Vec<_> {
        (0..5)
            .map(|layer| {
                bench
                    .textures
                    .picture(bench.viewing(layer))
                    .map(|picture| picture.id())
            })
            .collect()
    };
    let settled = (
        bench.granted_all(),
        bench.pictures(),
        ids(&bench),
        bench.held(),
        bench.textures.layers_kept(),
    );
    assert!(
        settled.1.iter().all(Option::is_some),
        "CONTROL: a settled layer has no picture"
    );
    for _ in 0..8 {
        let unplaced = bench.frame(false, &drawn);
        assert_eq!(
            (
                unplaced,
                (
                    bench.granted_all(),
                    bench.pictures(),
                    ids(&bench),
                    bench.held(),
                    bench.textures.layers_kept()
                )
            ),
            (0, settled.clone()),
            "CONTROL: a settled set of layers changed with nothing arriving"
        );
    }
}

#[test]
fn another_project_keeps_the_viewer_budget_and_none_of_the_layers() {
    let one = texels([32, 32]);
    let budget = ViewerBudget {
        bytes: 5 * one,
        spare_layers: 3,
    };
    let mut bench = Bench::new(budget, 3);
    let drawn = each_at(3, 32);
    bench.settle_unheld(false, &drawn);
    assert_eq!(
        bench.held(),
        LayerUsage {
            pictures: 3,
            bytes: 3 * one,
        },
        "CONTROL: three settled layers are not three pictures"
    );
    let asked: Vec<_> = (0..3).map(|layer| bench.consumer(layer)).collect();

    // One asset now decodes from another file.
    bench.textures.forget_asset(bench.assets[1].id);
    assert_eq!(
        (bench.held(), bench.pictures()),
        (
            LayerUsage {
                pictures: 2,
                bytes: 2 * one,
            },
            vec![Some([32, 32]), None, Some([32, 32])]
        ),
        "CONTROL: forgetting an asset did not free its layer and no other"
    );

    // Another project.
    bench.textures.clear();
    let kept = bench.textures.viewer_budget();
    assert_eq!(
        (kept.bytes, kept.spare_layers),
        (budget.bytes, budget.spare_layers),
        "CONTROL: the viewer budget did not outlast another project"
    );
    assert_eq!(
        (bench.held(), bench.textures.layers_kept()),
        (
            LayerUsage {
                pictures: 0,
                bytes: 0,
            },
            0
        ),
        "CONTROL: a layer or its texture is kept from before the project was replaced"
    );
    bench.frame(false, &drawn);
    for (layer, old) in asked.iter().enumerate() {
        let Some(old) = *old else {
            panic!("FIXTURE: a settled layer asked under no consumer");
        };
        let asset = &bench.assets[layer];
        let late = filling(square(32), BLUE);
        let time = bench.textures.admitted(old);
        let taken = bench.textures.accept(
            &bench.ctx,
            Some(asset),
            Some(old),
            asset.id,
            time,
            square(32),
            &late,
        );
        assert!(
            !taken && bench.held().bytes == 0,
            "CONTROL: a frame asked for before the project was replaced was taken"
        );
    }
}

#[test]
fn a_frame_at_a_box_no_longer_asked_at_is_not_taken() {
    let mut bench = Bench::new(ViewerBudget::PRODUCTION, 1);
    bench.settle_unheld(false, &[(0, 32)]);
    let Some(consumer) = bench.consumer(0) else {
        panic!("FIXTURE: the drawn layer asks under no consumer");
    };
    // Drawn smaller: it asks at the smaller box in that frame.
    bench.frame(false, &[(0, 16)]);
    assert_eq!(
        (bench.granted(0), bench.consumer(0), bench.picture(0)),
        (Some(square(16)), Some(consumer), Some([32, 32])),
        "CONTROL: a layer drawn smaller does not ask at once at the smaller box \
         under its own consumer, with the picture it had"
    );
    // What was still decoding at the box asked before arrives now.
    let asset = &bench.assets[0];
    let late = filling(square(32), BLUE);
    let before = bench.textures.picture(bench.viewing(0)).map(|p| p.id());
    let time = bench.textures.admitted(consumer);
    let taken = bench.textures.accept(
        &bench.ctx,
        Some(asset),
        Some(consumer),
        asset.id,
        time,
        square(32),
        &late,
    );
    assert!(
        !taken,
        "CONTROL: a frame at a box the layer no longer asks at was taken"
    );
    assert!(
        bench.fill(0),
        "CONTROL: the frame at the box asked now was not taken"
    );
    assert_eq!(
        (
            bench.picture(0),
            bench.textures.picture(bench.viewing(0)).map(|p| p.id())
        ),
        (Some([16, 16]), before),
        "CONTROL: the smaller frame did not arrive in the layer's own texture"
    );
}

#[test]
fn a_cut_carries_on_with_no_spare_layer_and_the_source_monitor_takes_no_clips_layer() {
    let one = texels([32, 32]);
    // No layer may be kept that is not drawn.
    let budget = ViewerBudget {
        bytes: 2 * one,
        spare_layers: 0,
    };
    let mut bench = Bench::new(budget, 3);
    // One asset throughout: the layers are told apart by what they are for.
    let asset = bench.assets[0].clone();
    let [outgoing, incoming, other] = [0, 1, 2].map(|layer| bench.clips[layer]);
    let want = |clip: ClipId, carries_on: &[ClipId]| Wanted {
        viewing: Viewing::Clip(clip),
        asset: &asset,
        size: square(32),
        time: Time::ZERO,
        carries_on: carries_on.to_vec(),
    };

    bench.textures.begin_frame();
    bench
        .textures
        .compose(None, true, &[want(outgoing, &[]), want(other, &[])]);
    for clip in [outgoing, other] {
        let taken = arrive_filling(
            &bench.ctx,
            &mut bench.textures,
            Viewing::Clip(clip),
            &asset,
            RED,
        );
        fixture(taken, "a drawn layer did not take its frame");
    }
    let asked = bench.textures.asked(Viewing::Clip(outgoing));
    let picture = |bench: &Bench, viewing: Viewing| {
        bench.textures.picture(viewing).map(|picture| picture.id())
    };
    let shown_before = picture(&bench, Viewing::Clip(outgoing));
    fixture(shown_before.is_some(), "the outgoing clip shows a picture");

    // A clip that is drawn gives nothing to one that would carry on from it.
    bench.textures.begin_frame();
    bench.textures.compose(
        None,
        true,
        &[want(outgoing, &[]), want(incoming, &[outgoing])],
    );
    assert!(
        picture(&bench, Viewing::Clip(incoming)).is_none()
            && picture(&bench, Viewing::Clip(outgoing)) == shown_before,
        "CONTROL: a layer drawn this frame was taken by another clip"
    );
    bench.textures.forget_asset(asset.id);
    bench.textures.begin_frame();
    bench
        .textures
        .compose(None, true, &[want(outgoing, &[]), want(other, &[])]);
    for clip in [outgoing, other] {
        let taken = arrive_filling(
            &bench.ctx,
            &mut bench.textures,
            Viewing::Clip(clip),
            &asset,
            RED,
        );
        fixture(taken, "a drawn layer did not take its frame");
    }
    let asked_again = bench.textures.asked(Viewing::Clip(outgoing));
    fixture(
        asked_again.is_some() && asked_again != asked,
        "the outgoing clip has a layer of its own again",
    );
    let outgoings = picture(&bench, Viewing::Clip(outgoing));
    let others = picture(&bench, Viewing::Clip(other));

    // Played across the cut: the outgoing clip is no longer drawn, and the
    // incoming one takes its layer before anything undrawn is given up.
    bench.textures.begin_frame();
    bench
        .textures
        .compose(None, true, &[want(incoming, &[outgoing]), want(other, &[])]);
    assert_eq!(
        (
            bench.textures.asked(Viewing::Clip(incoming)),
            picture(&bench, Viewing::Clip(incoming)),
            picture(&bench, Viewing::Clip(other)),
            bench.textures.has_layer(Viewing::Clip(outgoing)),
        ),
        (asked_again, outgoings, others, false),
        "CONTROL: across a cut with no spare layer allowed, the incoming clip did \
         not take the outgoing one's consumer and picture, or the other clip changed"
    );
    within_budget(&bench.ctx, &bench.textures, "across a cut");
    assert!(
        bench.textures.layers_kept() <= 2,
        "CONTROL: {} layers are kept where two are drawn",
        bench.textures.layers_kept()
    );

    // The source monitor carries on from nothing, whatever it is told.
    bench.textures.begin_frame();
    bench.textures.compose(
        None,
        false,
        &[Wanted {
            viewing: Viewing::Source,
            ..want(incoming, &[incoming, other])
        }],
    );
    let source = bench.textures.asked(Viewing::Source);
    assert!(
        source.is_some()
            && source.map(|(consumer, _)| consumer) != asked_again.map(|(consumer, _)| consumer)
            && picture(&bench, Viewing::Source).is_none(),
        "CONTROL: the source monitor took a timeline clip's layer"
    );
}

// ---- The timeline painted under a budget ------------------------------------

/// Where on the canvas the clip hung at `index` goes: three columns by three
/// rows, apart enough that layers at [`SCALE`] never overlap.
fn slot(index: usize) -> [f32; 2] {
    [
        ((index % 3) as f32 - 1.0) * COLUMN,
        ((index / 3) as f32 - 1.0) * ROW,
    ]
}

/// Clips of one asset, each on a track and in a slot of its own, with a
/// colour each. Built with the commands the editor uses.
struct Wall {
    scene: Scene,
    clips: Vec<ClipId>,
}

impl Wall {
    fn new() -> Self {
        Self {
            scene: Scene::new(),
            clips: Vec::new(),
        }
    }

    /// Hangs one more clip, on screen from `start` for `duration` seconds.
    fn hang(&mut self, start: f64, duration: f64) -> ClipId {
        let index = self.clips.len();
        fixture(index < COLOURS.len(), "a colour for every clip");
        let track = Track::new(format!("Wall {index}"), TrackKind::Video);
        let track_id = track.id;
        self.scene.edit(Command::AddTrack { track });
        let mut clip = Clip::from_asset(&self.scene.asset, Time::from_seconds(start));
        // Every clip is at a source time of its own.
        clip.source_in = Time::from_seconds(index as f64 * 0.5);
        clip.duration = Time::from_seconds(duration);
        clip.transform = Transform {
            position: slot(index),
            scale: [SCALE, SCALE],
            ..Transform::default()
        };
        let id = clip.id;
        self.scene.edit(Command::AddClip { track_id, clip });
        self.clips.push(id);
        id
    }

    /// The clips with the colour of each.
    fn coloured(&self) -> Vec<(ClipId, [u8; 4])> {
        self.clips.iter().copied().zip(COLOURS).collect()
    }
}

/// What `clip` is painted with: the quad whose centre is in the clip's slot.
fn seen_in_slot(stage: &Stage, painted: &Painted, project: &Project, clip: ClipId) -> Seen {
    let Some((_, clip)) = project.clip(clip) else {
        panic!("FIXTURE: the clip is not in the project");
    };
    let [x, y] = clip.transform.position;
    let cell = (column_of(x, COLUMN * 0.5), column_of(y, ROW * 0.5));
    let centre = window().center();
    let mut found = painted.quads.iter().filter(|quad| {
        let across = column_of(quad.centre.x - centre.x, 100.0);
        // The canvas is centred a little above the window's centre.
        let down = column_of(quad.centre.y - centre.y, 90.0);
        (across, down) == cell
    });
    let texture = found.next().map(|quad| quad.texture);
    fixture(
        found.next().is_none(),
        "two layers were painted in one slot",
    );
    Seen {
        texture,
        picture: texture.and_then(|texture| stage.pictures.get(&texture).copied()),
    }
}

/// A stage whose viewer is given `budget`.
fn stage_with(budget: ViewerBudget) -> Stage {
    let mut stage = Stage::new();
    stage.textures = Textures::with_viewer_budget(budget);
    stage
}

/// Paints `clips`, all of `asset` and all on screen at `playhead`, until each
/// shows a frame that fills the box it asks at: a paint, a frame for each
/// clip that waits, and so on. Every paint must show each clip its own picture
/// or none. With `when`, the layer textures are held to the budget after
/// every arrival. Returns what each clip is painted with at the end.
fn decode(
    stage: &mut Stage,
    project: &Project,
    playhead: Time,
    asset: &Asset,
    clips: &[(ClipId, [u8; 4])],
    when: Option<&str>,
) -> HashMap<ClipId, Seen> {
    for _ in 0..16 {
        let painted = stage.paint(project, playhead, false);
        let mut seen = HashMap::new();
        for (clip, colour) in clips {
            let now = seen_in_slot(stage, &painted, project, *clip);
            assert!(
                now.picture.is_none() || now.picture == Some(shown(*colour)),
                "CONTROL: a clip is painted with {now:?}, which is not its own \
                 picture {:?}",
                shown(*colour)
            );
            seen.insert(*clip, now);
        }
        let waiting: Vec<_> = clips
            .iter()
            .filter(|(clip, _)| waits(&stage.textures, Viewing::Clip(*clip)))
            .collect();
        if waiting.is_empty() {
            for (clip, colour) in clips {
                assert_eq!(
                    seen[clip].picture,
                    Some(shown(*colour)),
                    "ISSUE13: a clip on screen whose frame arrived is painted \
                     without its picture"
                );
            }
            assert_eq!(
                painted.quads.len(),
                clips.len(),
                "CONTROL: not exactly one layer is painted for each clip"
            );
            return seen;
        }
        for (clip, colour) in waiting {
            let viewing = Viewing::Clip(*clip);
            let taken = arrive_filling(&stage.ctx, &mut stage.textures, viewing, asset, *colour);
            if let (true, Some(when)) = (taken, when) {
                within_budget(&stage.ctx, &stage.textures, when);
            }
        }
    }
    panic!(
        "ISSUE13: after sixteen paints with every frame asked for arriving, clips on \
         screen still wait for a picture"
    );
}

/// The bytes of the box a clip at [`SCALE`] asks at in the stage's window,
/// read from a viewer that is given the production budget and one clip.
fn hung_box() -> usize {
    let mut wall = Wall::new();
    let clip = wall.hang(0.0, 4.0);
    let project = wall.scene.project();
    let mut stage = Stage::new();
    stage.paint(&project, frame(59), false);
    let asked = stage.textures.asked(Viewing::Clip(clip));
    let Some(bytes) = asked.and_then(|(_, size)| box_bytes(size)) else {
        panic!("FIXTURE: a painted clip asks at no box");
    };
    fixture(
        asked.map(|(_, size)| size) == Some(HUNG_BOX),
        "a clip at the scale of these tests asks at 320 by 180",
    );
    bytes
}

#[test]
fn every_active_clip_is_painted_with_its_own_picture_within_a_budget_for_three() {
    let one = hung_box();
    let mut wall = Wall::new();
    for _ in 0..5 {
        wall.hang(0.0, 4.0);
    }
    let project = wall.scene.project();
    fixture(
        active(&project, frame(59)) == wall.clips,
        "five clips are painted, in the order they were hung",
    );
    let mut stage = stage_with(ViewerBudget {
        bytes: 3 * one,
        spare_layers: 8,
    });
    let clips = wall.coloured();
    let seen = decode(
        &mut stage,
        &project,
        frame(59),
        &wall.scene.asset,
        &clips,
        Some("five clips on screen under a budget for three"),
    );
    let textures: std::collections::HashSet<_> = seen.values().map(|seen| seen.texture).collect();
    assert_eq!(
        textures.len(),
        5,
        "CONTROL: two clips are painted with one texture"
    );
    within_budget(&stage.ctx, &stage.textures, "five clips settled");
    // Nothing changes while nothing arrives.
    let painted = stage.paint(&project, frame(59), false);
    for (clip, _) in &clips {
        assert_eq!(
            seen_in_slot(&stage, &painted, &project, *clip),
            seen[clip],
            "CONTROL: a settled clip changed with nothing decoded"
        );
    }
}

#[test]
fn four_clips_hold_their_pictures_while_two_more_come_within_the_budget() {
    let one = hung_box();
    // The box of a clip held to the step under the one it asks at: its
    // width is, and its height is within it already.
    let Some(less) = box_bytes([UNDER_HUNG, HUNG_BOX[1]]) else {
        panic!("FIXTURE: the smaller box cannot be counted");
    };
    let mut wall = Wall::new();
    for _ in 0..4 {
        wall.hang(0.0, 4.0);
    }
    let fifth = wall.hang(2.0, 2.0);
    let sixth = wall.hang(2.0, 2.0);
    let project = wall.scene.project();
    let clips = wall.coloured();
    let (four, six) = (frame(59), frame(90));
    fixture(
        active(&project, four) == wall.clips[..4] && active(&project, six) == wall.clips,
        "four clips are painted at frame 59 and six at frame 90",
    );
    // Four whole boxes and one at the step under them: as 64 MiB holds four
    // at 1920 and one at 1280, and not a second.
    let budget = 4 * one + less;
    let mut stage = stage_with(ViewerBudget {
        bytes: budget,
        spare_layers: 8,
    });
    let asset = wall.scene.asset.clone();
    let before = decode(
        &mut stage,
        &project,
        four,
        &asset,
        &clips[..4],
        Some("four clips on screen"),
    );
    assert_eq!(
        held(&stage.ctx, &stage.textures).bytes,
        4 * one,
        "CONTROL: four clips that fit the budget are not kept at the box they \
         are drawn at"
    );

    // A paused seek to where two more are on screen.
    let painted = stage.paint(&project, six, false);
    for (clip, _) in &clips[..4] {
        assert_eq!(
            seen_in_slot(&stage, &painted, &project, *clip),
            before[clip],
            "CONTROL: a clip did not hold its own picture when more came on screen"
        );
    }
    for clip in [fifth, sixth] {
        assert_eq!(
            seen_in_slot(&stage, &painted, &project, clip),
            Seen::NOTHING,
            "CONTROL: a clip new on screen is painted before a frame was decoded \
             for it"
        );
    }
    for (clip, colour) in &clips[4..] {
        let viewing = Viewing::Clip(*clip);
        if arrive_filling(&stage.ctx, &mut stage.textures, viewing, &asset, *colour) {
            within_budget(
                &stage.ctx,
                &stage.textures,
                "a frame arrived for a clip new on screen beside four that hold \
                 their pictures",
            );
        }
    }
    assert!(
        stage.textures.asked(Viewing::Clip(sixth)).is_none(),
        "ISSUE13: the sixth clip is asked for before any of the four has shrunk"
    );
    let painted = stage.paint(&project, six, false);
    for (clip, _) in &clips[..4] {
        assert_eq!(
            seen_in_slot(&stage, &painted, &project, *clip),
            before[clip],
            "ISSUE13: a clip gave up its picture before its smaller one arrived"
        );
    }
    assert_eq!(
        (
            seen_in_slot(&stage, &painted, &project, fifth).picture,
            seen_in_slot(&stage, &painted, &project, sixth),
        ),
        (Some(shown(COLOURS[4])), Seen::NOTHING),
        "ISSUE13: the fifth clip does not show its frame, or the sixth, which \
         waits for room, is painted with something"
    );

    // The smaller frames arrive, and then the sixth clip's.
    let after = decode(
        &mut stage,
        &project,
        six,
        &asset,
        &clips,
        Some("six clips on screen"),
    );
    for (clip, _) in &clips[..4] {
        assert_eq!(
            after[clip].texture, before[clip].texture,
            "ISSUE13: a clip's smaller frame did not arrive in the texture it had"
        );
    }
    within_budget(&stage.ctx, &stage.textures, "six clips settled");
}

#[test]
fn a_cut_hands_its_picture_on_when_no_layer_may_be_kept_undrawn() {
    let one = hung_box();
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 0.0,
        source_in: 0.0,
        duration: 4.0,
    });
    let b = scene.clip(Place {
        track: scene.upper,
        column: 1,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    let at = Time::from_seconds(2.0);
    scene.edit(Command::SplitClip { clip_id: a, at });
    let project = scene.project();
    let incoming = starting(&project, a, at);
    // Room for the two on screen and for nothing that is not drawn.
    let mut stage = stage_with(ViewerBudget {
        bytes: 2 * one,
        spare_layers: 0,
    });
    let before = decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &[(a, RED), (b, BLUE)],
        Some("two clips on screen"),
    );

    let playback = played(&project, frame(59), 50);
    fixture(
        playback.playing && active(&project, playback.position) == [incoming, b],
        "playback is past the cut, inside the incoming clip and B",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        (
            stage.seen(&painted, &project, incoming),
            stage.seen(&painted, &project, b)
        ),
        (before[&a], before[&b]),
        "CONTROL: across a cut with no spare layer allowed, the incoming clip and B \
         are painted with the left; the outgoing clip's frame and B's own are the \
         right"
    );
    within_budget(&stage.ctx, &stage.textures, "across a cut");
    assert!(
        stage.textures.layers_kept() <= 2,
        "CONTROL: {} layers are kept where two are drawn",
        stage.textures.layers_kept()
    );
}

#[test]
fn the_source_monitor_and_the_timeline_share_a_budget_and_no_picture() {
    let one = hung_box();
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let clips = [(a, RED), (b, BLUE)];
    // Room for the two clips, and so for nothing more while both are kept.
    let mut stage = stage_with(ViewerBudget {
        bytes: 2 * one,
        spare_layers: 8,
    });
    let before = decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &clips,
        Some("two clips on screen"),
    );
    let asked = [a, b].map(|clip| stage.textures.asked(Viewing::Clip(clip)));

    // The same asset in the source monitor.
    let painted = stage.paint_source(&scene.asset);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: the source monitor painted a picture before one was decoded for it"
    );
    if arrive_filling(
        &stage.ctx,
        &mut stage.textures,
        Viewing::Source,
        &scene.asset,
        GREEN,
    ) {
        within_budget(
            &stage.ctx,
            &stage.textures,
            "a frame arrived for the source monitor, opened over two clips that \
             fill the budget",
        );
    }
    let painted = stage.paint_source(&scene.asset);
    let [source] = painted.quads.as_slice() else {
        panic!("ISSUE13: the source monitor did not paint exactly one picture");
    };
    assert!(
        Some(source.texture) != before[&a].texture && Some(source.texture) != before[&b].texture,
        "CONTROL: the source monitor is painted with a timeline clip's texture"
    );
    assert_eq!(
        stage.pictures.get(&source.texture),
        Some(&shown(GREEN)),
        "CONTROL: the source monitor does not show the frame decoded for it"
    );

    // Back on the timeline, each clip shows its own picture or none.
    let painted = stage.paint(&project, frame(59), false);
    for clip in [a, b] {
        let seen = stage.seen(&painted, &project, clip);
        assert!(
            seen == before[&clip] || seen == Seen::NOTHING,
            "CONTROL: after the source monitor, a timeline clip is painted with \
             {seen:?}; its own was {:?}",
            before[&clip]
        );
    }
    within_budget(&stage.ctx, &stage.textures, "back on the timeline");
    // What was still decoding for a layer given up meanwhile arrives now.
    for (clip, old) in [a, b].into_iter().zip(asked) {
        let Some((consumer, size)) = old else {
            panic!("FIXTURE: a settled clip kept no layer");
        };
        let now = stage.textures.asked(Viewing::Clip(clip));
        if now.map(|(consumer, _)| consumer) == Some(consumer) {
            continue;
        }
        let late = filling(size, GREEN);
        let time = stage.textures.admitted(consumer);
        let taken = stage.textures.accept(
            &stage.ctx,
            Some(&scene.asset),
            Some(consumer),
            scene.asset.id,
            time,
            size,
            &late,
        );
        let wanted = stage
            .textures
            .failed(Some(consumer), scene.asset.id, Some(size));
        assert!(
            !taken && !wanted,
            "ISSUE13: a frame or a failure for a clip's layer that was given up \
             was taken"
        );
    }
    decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &clips,
        Some("two clips on screen again"),
    );
}

#[test]
fn a_clip_that_comes_back_is_painted_with_its_own_picture_or_none() {
    let one = hung_box();
    let Pair { mut scene, a, b } = overlapping();
    let project = scene.project();
    let clips = [(a, RED), (b, BLUE)];
    let mut stage = stage_with(ViewerBudget {
        bytes: 2 * one,
        spare_layers: 0,
    });
    let before = decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &clips,
        Some("two clips on screen"),
    );
    let own_or_none = |stage: &mut Stage, project: &Project, what: &str| {
        let painted = stage.paint(project, frame(59), false);
        let seen = stage.seen(&painted, project, a);
        assert!(
            seen == before[&a] || seen == Seen::NOTHING,
            "CONTROL: {what}, A is painted with {seen:?}; its own was {:?} and B's \
             {:?}",
            before[&a],
            before[&b]
        );
        assert_eq!(
            stage.seen(&painted, project, b),
            before[&b],
            "CONTROL: {what}, B lost its own picture"
        );
        within_budget(&stage.ctx, &stage.textures, what);
    };

    // A's track is hidden and shown again.
    scene.hide(scene.lower);
    let hidden = scene.project();
    fixture(active(&hidden, frame(59)) == [b], "only B is painted");
    let painted = stage.paint(&hidden, frame(59), false);
    assert_eq!(
        stage.seen(&painted, &hidden, b),
        before[&b],
        "CONTROL: B changed when the track under it was hidden"
    );
    fixture(scene.editor.undo(), "the hide is undone");
    let shown_again = scene.project();
    own_or_none(&mut stage, &shown_again, "back from a hidden track");

    // A is deleted and comes back by undo.
    scene.edit(Command::DeleteClips {
        clip_ids: vec![a],
        ripple: false,
    });
    let deleted = scene.project();
    fixture(active(&deleted, frame(59)) == [b], "only B is painted");
    stage.paint(&deleted, frame(59), false);
    fixture(scene.editor.undo(), "the delete is undone");
    let restored = scene.project();
    own_or_none(&mut stage, &restored, "back by undo");

    decode(
        &mut stage,
        &restored,
        frame(59),
        &scene.asset,
        &clips,
        Some("two clips on screen again"),
    );
}

#[test]
fn a_replaced_project_holds_no_layer_texels_and_keeps_the_viewer_budget() {
    let one = hung_box();
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let budget = ViewerBudget {
        bytes: 2 * one,
        spare_layers: 1,
    };
    let mut stage = stage_with(budget);
    decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &[(a, RED), (b, BLUE)],
        Some("two clips on screen"),
    );
    fixture(
        held(&stage.ctx, &stage.textures).pictures == 2,
        "two layer textures are held",
    );

    // What `App::replace_editor` does to the textures.
    stage.textures.clear();
    let empty = LayerUsage {
        pictures: 0,
        bytes: 0,
    };
    assert_eq!(
        held(&stage.ctx, &stage.textures),
        empty,
        "CONTROL: layer texels are held from before the project was replaced"
    );
    let kept = stage.textures.viewer_budget();
    assert_eq!(
        (kept.bytes, kept.spare_layers),
        (budget.bytes, budget.spare_layers),
        "CONTROL: the viewer budget did not outlast another project"
    );
    let painted = stage.paint(&project, frame(59), false);
    assert!(
        painted.quads.is_empty() && held(&stage.ctx, &stage.textures) == empty,
        "CONTROL: a layer is painted with a picture kept from before the project \
         was replaced"
    );
}

#[test]
fn a_settled_viewer_under_a_tight_budget_asks_for_no_repaint_of_its_own() {
    let one = hung_box();
    let mut wall = Wall::new();
    for _ in 0..5 {
        wall.hang(0.0, 4.0);
    }
    let project = wall.scene.project();
    let clips = wall.coloured();
    // What the same scene asks of the paint loop with room for everything is
    // egui's and the transport's doing, and none of the budget's.
    let mut repaints = Vec::new();
    for budget in [
        ViewerBudget::PRODUCTION,
        ViewerBudget {
            bytes: 3 * one,
            spare_layers: 0,
        },
    ] {
        let mut stage = stage_with(budget);
        // Whether the layers are held to the budget is not this test's
        // question.
        let settled = decode(
            &mut stage,
            &project,
            frame(59),
            &wall.scene.asset,
            &clips,
            None,
        );
        let usage = held(&stage.ctx, &stage.textures);
        let mut asked = Vec::new();
        for _ in 0..6 {
            let painted = stage.paint(&project, frame(59), false);
            asked.push(stage.ctx.has_requested_repaint());
            for (clip, _) in &clips {
                assert_eq!(
                    seen_in_slot(&stage, &painted, &project, *clip),
                    settled[clip],
                    "CONTROL: a settled clip changed with nothing decoded"
                );
            }
            assert_eq!(
                held(&stage.ctx, &stage.textures),
                usage,
                "CONTROL: the layer textures of a settled scene changed"
            );
        }
        repaints.push(asked);
    }
    assert_eq!(
        repaints[1], repaints[0],
        "CONTROL: paused and settled, the viewer under a tight budget asks for \
         repaints as the left, frame by frame; with room for everything it asks as \
         the right"
    );
}

// ---- Added with the correction ----------------------------------------------

#[test]
fn a_failure_keeps_a_picture_that_fits_and_a_frame_too_large_gives_up_only_one_due_to_shrink() {
    let one = texels([64, 64]);
    let budget = ViewerBudget {
        bytes: 2 * one,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 3);
    let three = each_at(3, 64);
    bench.settle(false, &each_at(2, 64), "two layers drawn");
    let asked = |bench: &Bench, layer: usize| {
        let Some(asked) = bench.textures.asked(bench.viewing(layer)) else {
            panic!("FIXTURE: a drawn layer that has a picture asks at nothing");
        };
        asked
    };

    // A frame of the first cannot be decoded, at the box its picture fills.
    let (consumer, size) = asked(&bench, 0);
    let asset = bench.assets[0].id;
    assert!(
        bench.textures.failed(Some(consumer), asset, Some(size)),
        "CONTROL: a failure of what a layer asks for now is not taken as its own"
    );
    assert_eq!(
        bench.pictures()[..2],
        [Some([64, 64]); 2],
        "CONTROL: a failure took a picture that fits the box it is asked at"
    );

    // A third is drawn, so the two are asked at less. What the second is
    // sent is no smaller than what it has.
    bench.frame(false, &three);
    let (consumer, size) = asked(&bench, 1);
    fixture(
        size == square(32),
        "a layer that must shrink asks at 32 a side",
    );
    let asset = &bench.assets[1];
    let large = filling(square(64), BLUE);
    let time = bench.textures.admitted(consumer);
    bench.textures.accept(
        &bench.ctx,
        Some(asset),
        Some(consumer),
        asset.id,
        time,
        size,
        &large,
    );
    assert_eq!(
        bench.pictures(),
        [Some([64, 64]), None, None],
        "ISSUE13: after a frame too large for its box arrived for the second layer, \
         the three have the left. The second holds room the third waits for and its \
         picture was not replaced; the first's smaller frame is still on its way"
    );
    assert!(
        bench.consumer(1) == Some(consumer),
        "ISSUE13: the layer sent a frame too large is no longer the clip's own"
    );
    within_budget(
        &bench.ctx,
        &bench.textures,
        "after a frame too large for its box",
    );
    let unplaced = bench.settle(false, &three, "after a frame too large for its box");
    assert_eq!(unplaced, 0);
    assert!(
        bench.pictures().iter().all(Option::is_some),
        "ISSUE13: a drawn layer has no picture. They have {:?}",
        bench.pictures()
    );
}

#[test]
fn pictures_that_go_unplaced_are_news_once_until_that_changes() {
    // Room for two texels.
    let budget = ViewerBudget {
        bytes: 8,
        spare_layers: 8,
    };
    let mut bench = Bench::new(budget, 4);
    // How many went unplaced in a frame that draws so many layers, and how
    // many of them it is news of.
    let draw = |bench: &mut Bench, drawn: usize| {
        let unplaced = bench.frame(false, &each_at(drawn, 32));
        let news = bench.textures.newly_unplaced();
        (unplaced, news.map(|composed| composed.unplaced))
    };
    let told: Vec<_> = [3, 3, 4, 4, 2, 2, 3, 3]
        .into_iter()
        .map(|drawn| draw(&mut bench, drawn))
        .collect();
    assert_eq!(
        told,
        [
            (1, Some(1)),
            (1, None),
            (2, Some(2)),
            (2, None),
            (0, None),
            (0, None),
            (1, Some(1)),
            (1, None),
        ],
        "ISSUE13: frames that draw three, three, four, four, two, two, three and \
         three layers where two texels can be kept left the left unplaced and were \
         news of the right of each pair; it is news when it comes about and when it \
         comes to another number, and not while it lasts"
    );
    // Another project hears of its own.
    bench.textures.clear();
    assert_eq!(
        draw(&mut bench, 3),
        (1, Some(1)),
        "ISSUE13: what went unplaced in a replaced project is not news"
    );
}

#[test]
fn a_sound_in_the_source_monitor_holds_the_timeline_layers_to_the_spare_ones() {
    let one = hung_box();
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = stage_with(ViewerBudget {
        bytes: 2 * one,
        spare_layers: 1,
    });
    decode(
        &mut stage,
        &project,
        frame(59),
        &scene.asset,
        &[(a, RED), (b, BLUE)],
        Some("two clips on screen"),
    );

    // The monitor draws no picture for a sound. That is a frame all the same.
    let sound = Asset::new(
        PathBuf::from("/media/voice.wav"),
        MediaKind::Audio,
        Time::from_seconds(6.0),
    );
    stage.paint_source(&sound);
    let kept = held(&stage.ctx, &stage.textures);
    assert!(
        kept.pictures <= 1 && stage.textures.layers_kept() <= 1,
        "ISSUE13: one frame of the source monitor on a sound after two clips went \
         undrawn, {} of their pictures ({} bytes) and {} layers are kept where one \
         spare layer is allowed",
        kept.pictures,
        kept.bytes,
        stage.textures.layers_kept()
    );
    assert!(
        stage.textures.has_layer(Viewing::Clip(b)) && !stage.textures.has_layer(Viewing::Clip(a)),
        "ISSUE13: the layer kept is not that of the clip painted last"
    );
}

/// One frame of the timeline viewer in a window of `size`, paused: what
/// [`Stage::paint`] does in the window of the other tests.
fn paint_in(stage: &mut Stage, size: Vec2, project: &Project, playhead: Time) -> Painted {
    let query = editor_core::ProjectQuery::new(std::sync::Arc::new(project.clone()));
    stage.textures.begin_frame();
    let ctx = stage.ctx.clone();
    let mut actions: Vec<Action> = Vec::new();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        let rect = ui.max_rect();
        show(
            ui,
            rect,
            &mut stage.viewer,
            &mut stage.selection,
            &mut stage.draft,
            &mut stage.textures,
            &ViewerInput {
                p: &stage.palette,
                project: query.project(),
                query: Some(&query),
                playhead,
                playing: false,
                speed: 1.0,
                looping: false,
                media: None,
                record: Default::default(),
                library: true,
                quality: Default::default(),
                cards: &[],
            },
            &mut actions,
        );
    });
    stage.painted(output)
}

/// The box a clip at [`SCALE`] asks at in a window of `size`, read from a
/// viewer that is given the production budget and one clip.
fn box_in(size: Vec2) -> [u32; 2] {
    let mut wall = Wall::new();
    let clip = wall.hang(0.0, 4.0);
    let project = wall.scene.project();
    let mut stage = Stage::new();
    paint_in(&mut stage, size, &project, frame(59));
    let Some((_, asked)) = stage.textures.asked(Viewing::Clip(clip)) else {
        panic!("FIXTURE: a painted clip asks at no box");
    };
    asked
}

/// The texture each clip is painted with in a window of `size`, or nothing
/// for one that has no picture. A clip's picture must be painted once, inside
/// the window, and show the clip's own colour; nothing else is painted with a
/// picture.
fn own_in(
    stage: &Stage,
    painted: &Painted,
    size: Vec2,
    clips: &[(ClipId, [u8; 4])],
) -> Vec<Option<TextureId>> {
    let window = Rect::from_min_size(Pos2::ZERO, size);
    let own: Vec<Option<TextureId>> = clips
        .iter()
        .map(|(clip, colour)| {
            let texture = stage
                .textures
                .picture(Viewing::Clip(*clip))
                .map(|picture| picture.id())?;
            let quads: Vec<&Quad> = painted
                .quads
                .iter()
                .filter(|quad| quad.texture == texture)
                .collect();
            let [quad] = quads.as_slice() else {
                panic!(
                    "CONTROL: a clip's picture is painted {} times in a window of {size:?}",
                    quads.len()
                );
            };
            assert!(
                window.contains(quad.centre),
                "CONTROL: a clip is painted around {:?}, outside a window of {size:?}",
                quad.centre
            );
            assert_eq!(
                stage.pictures.get(&texture),
                Some(&shown(*colour)),
                "CONTROL: a clip is painted with a picture that is not its own"
            );
            Some(texture)
        })
        .collect();
    assert_eq!(
        painted.quads.len(),
        own.iter().flatten().count(),
        "CONTROL: something is painted with a picture that is no clip's own layer"
    );
    own
}

#[test]
fn a_resized_window_asks_at_the_box_it_has_room_for_and_clips_hold_their_pictures() {
    let small = window().size();
    let (middle, large) = (vec2(2000.0, 1300.0), vec2(4000.0, 2600.0));
    let boxes = [small, middle, large].map(box_in);
    let [Some(least), Some(room), Some(most)] = boxes.map(box_bytes) else {
        panic!("FIXTURE: a box asked at cannot be counted");
    };
    fixture(
        boxes[0] == HUNG_BOX && least < room && room < most,
        "a clip asks at a larger box in each larger window",
    );
    // What two boxes are held to when they do not fit: one side for both,
    // the largest the budget has room for two of. Two of 480 a side come to
    // the two boxes the middle window asks at, which is the budget here.
    let held_to = square(480);
    fixture(
        boxes[1] == [640, 360] && box_bytes(held_to) == Some(room),
        "the middle window asks at 640 by 360, the bytes of a box 480 a side",
    );
    let mut wall = Wall::new();
    for _ in 0..2 {
        wall.hang(0.0, 4.0);
    }
    let project = wall.scene.project();
    let clips = wall.coloured();
    let asset = wall.scene.asset.clone();
    // Room for both at what the middle window asks, and for no more.
    let mut stage = stage_with(ViewerBudget {
        bytes: 2 * room,
        spare_layers: 8,
    });
    let asked = |stage: &Stage| -> Vec<Option<[u32; 2]>> {
        clips
            .iter()
            .map(|(clip, _)| {
                let asked = stage.textures.asked(Viewing::Clip(*clip));
                asked.map(|(_, size)| size)
            })
            .collect()
    };
    let sizes = |stage: &Stage| -> Vec<Option<[usize; 2]>> {
        clips
            .iter()
            .map(|(clip, _)| {
                let picture = stage.textures.picture(Viewing::Clip(*clip));
                picture.map(|picture| picture.size())
            })
            .collect()
    };
    let filled = |size: [u32; 2]| vec![Some(size.map(|side| side as usize)); 2];
    // The frames asked for arrive, and the next paint shows them.
    let arrive = |stage: &mut Stage, size: Vec2, when: &str| {
        for (clip, colour) in &clips {
            let viewing = Viewing::Clip(*clip);
            let taken = arrive_filling(&stage.ctx, &mut stage.textures, viewing, &asset, *colour);
            fixture(taken, "a clip that asks at a box did not take its frame");
            within_budget(&stage.ctx, &stage.textures, when);
        }
        let painted = paint_in(stage, size, &project, frame(59));
        own_in(stage, &painted, size, &clips)
    };

    paint_in(&mut stage, small, &project, frame(59));
    let own = arrive(&mut stage, small, "two clips in the small window");
    assert!(
        own.iter().all(Option::is_some) && sizes(&stage) == filled(boxes[0]),
        "CONTROL: two clips did not settle at the box the small window asks"
    );

    // The window grows, with room for what it now asks.
    let painted = paint_in(&mut stage, middle, &project, frame(59));
    assert_eq!(
        (
            asked(&stage),
            sizes(&stage),
            own_in(&stage, &painted, middle, &clips)
        ),
        (vec![Some(boxes[1]); 2], filled(boxes[0]), own.clone()),
        "CONTROL: in a window grown within the budget, the clips do not ask at the \
         larger box at once, each painted with the picture it had"
    );
    assert_eq!(
        (
            arrive(&mut stage, middle, "two clips in the grown window"),
            sizes(&stage)
        ),
        (own.clone(), filled(boxes[1])),
        "CONTROL: the larger frames did not arrive in the textures the clips had"
    );

    // It grows past what the budget has room for.
    let painted = paint_in(&mut stage, large, &project, frame(59));
    assert_eq!(
        (
            asked(&stage),
            sizes(&stage),
            own_in(&stage, &painted, large, &clips)
        ),
        (vec![Some(held_to); 2], filled(boxes[1]), own.clone()),
        "ISSUE13: in a window grown past the budget, the clips ask at the left and \
         have pictures of the middle; the box there is room for, and the pictures \
         they had, are the right"
    );
    within_budget(&stage.ctx, &stage.textures, "in the largest window");

    // And back to the small one: asked at once, and held until it arrives.
    let painted = paint_in(&mut stage, small, &project, frame(59));
    assert_eq!(
        (
            asked(&stage),
            sizes(&stage),
            own_in(&stage, &painted, small, &clips)
        ),
        (vec![Some(boxes[0]); 2], filled(boxes[1]), own.clone()),
        "CONTROL: in a window made smaller, the clips do not ask at the smaller box \
         at once, each painted with the picture it had"
    );
    assert_eq!(
        (
            arrive(&mut stage, small, "two clips in the small window again"),
            sizes(&stage)
        ),
        (own, filled(boxes[0])),
        "CONTROL: the smaller frames did not arrive in the textures the clips had"
    );
    within_budget(&stage.ctx, &stage.textures, "back in the small window");
}

// ---- Whole pictures ---------------------------------------------------------

#[test]
fn five_whole_pictures_are_asked_for_and_kept_within_the_production_budget() {
    let Some(one) = box_bytes(WHOLE) else {
        panic!("FIXTURE: a whole picture cannot be counted");
    };
    fixture(
        5 * one == 41_472_000 && 5 * one <= 64 * MIB,
        "five whole pictures are 41,472,000 bytes, within 64 MiB",
    );
    // Five clips of the asset over one another, each as large as the canvas.
    let mut scene = Scene::new();
    let mut clips: Vec<(ClipId, [u8; 4])> = Vec::new();
    for (index, colour) in COLOURS.into_iter().take(5).enumerate() {
        let track = Track::new(format!("Whole {index}"), TrackKind::Video);
        let track_id = track.id;
        scene.edit(Command::AddTrack { track });
        let mut clip = Clip::from_asset(&scene.asset, Time::ZERO);
        // Every clip is at a source time of its own.
        clip.source_in = Time::from_seconds(index as f64 * 0.5);
        clip.duration = Time::from_seconds(4.0);
        clip.transform = Transform {
            scale: [1.0, 1.0],
            ..Transform::default()
        };
        let id = clip.id;
        scene.edit(Command::AddClip { track_id, clip });
        clips.push((id, colour));
    }
    let project = scene.project();
    let drawn: Vec<ClipId> = clips.iter().map(|(clip, _)| *clip).collect();
    fixture(
        active(&project, frame(59)) == drawn,
        "five clips are painted, in the order they were added",
    );
    // A window that shows the canvas at more than 1280 across: the whole
    // picture is what covers it.
    let size = vec2(2400.0, 1450.0);
    let mut stage = Stage::new();
    fixture(
        stage.textures.viewer_budget().bytes == 64 * MIB,
        "the viewer is given the production budget",
    );
    let asked = |stage: &Stage| -> Vec<Option<[u32; 2]>> {
        drawn
            .iter()
            .map(|clip| {
                let asked = stage.textures.asked(Viewing::Clip(*clip));
                asked.map(|(_, size)| size)
            })
            .collect()
    };

    paint_in(&mut stage, size, &project, frame(59));
    assert_eq!(
        asked(&stage),
        vec![Some(WHOLE); 5],
        "WHOLE: five clips of a 1920 by 1080 asset, drawn together as large as a \
         canvas shown at more than 1280 across, ask at the left; the whole picture \
         each, which the production budget has room for five of, is the right"
    );
    assert_eq!(
        stage.textures.layers_kept(),
        5,
        "CONTROL: five clips drawn together do not each have a layer of their own"
    );
    for (clip, colour) in &clips {
        let viewing = Viewing::Clip(*clip);
        let taken = arrive_filling(
            &stage.ctx,
            &mut stage.textures,
            viewing,
            &scene.asset,
            *colour,
        );
        assert!(
            taken,
            "WHOLE: a whole frame was not taken by the layer that asked for it"
        );
        within_budget(&stage.ctx, &stage.textures, "a whole frame arrived");
    }
    let kept = held(&stage.ctx, &stage.textures);
    assert_eq!(
        (kept.pictures, kept.bytes),
        (5, 5 * one),
        "WHOLE: the layer textures and their bytes are the left; five whole \
         pictures are the right"
    );

    // Nothing is held to less, given up or asked for again in the next frame.
    paint_in(&mut stage, size, &project, frame(59));
    assert_eq!(
        asked(&stage),
        vec![Some(WHOLE); 5],
        "WHOLE: with five whole pictures kept, the clips no longer ask at the \
         whole picture each"
    );
    for clip in &drawn {
        let picture = stage.textures.picture(Viewing::Clip(*clip));
        assert_eq!(
            picture.map(|picture| picture.size()),
            Some([1920, 1080]),
            "WHOLE: a clip whose whole frame arrived does not keep it whole"
        );
    }
    assert!(
        stage.textures.newly_unplaced().is_none(),
        "CONTROL: a picture went unplaced in a budget with room for all five"
    );
    assert_eq!(
        held(&stage.ctx, &stage.textures),
        kept,
        "CONTROL: the layer textures of a settled scene changed"
    );
}
