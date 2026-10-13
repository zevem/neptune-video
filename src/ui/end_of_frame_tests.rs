//! What the end of a frame hands its layers before its actions are applied.
//!
//! A frame composes, which is where its pictures are asked for, and ends by
//! applying what its widgets asked for: a seek, while the playhead is dragged,
//! in every frame. A picture the worker already has is published at once,
//! while the frame that asked for it is still being drawn. Left for the next
//! frame to take, it is by then a frame asked for before a seek and is turned
//! away, and the one after it likewise: a playhead that is dragged steadily
//! shows nothing until it rests.
//!
//! So the frames published are taken once at the end of a frame, before its
//! actions. These tests go through that end with [`end_frame`], the order the
//! application ends a frame in, and take with [`take_published_frames`], which
//! is what the application takes with there, against the real service of the
//! tests this is nested in. What an action then does is the test's own: a seek
//! that moves the playhead is [`Textures::jumped`], as in the application.
//!
//! `END:` is the behavior these were written for. `CONTROL:` holds without it
//! too, and is what it must not cost: a frame published after the end of the
//! frame that sought is still turned away, and nothing is taken or asked for
//! twice.
use super::*;
use crate::app::{FrameEnd, end_frame, take_published_frames};
use std::collections::VecDeque;

/// The end of a frame of the rig.
struct Ending<'a> {
    rig: &'a mut Rig,
    /// What the worker published that is not a frame.
    held: VecDeque<MediaEvent>,
}

impl FrameEnd for Ending<'_> {
    fn take_published(&mut self, ctx: &egui::Context) {
        let library = &self.rig.library;
        take_published_frames(
            ctx,
            &self.rig.media.service,
            &mut self.rig.textures,
            &mut self.held,
            |asset| library.get(&asset),
        );
    }

    fn act(&mut self, _: &egui::Context, action: Action) {
        match action {
            // The playhead of every test here is moved by its seek.
            Action::Seek(_) => self.rig.textures.jumped(),
            other => panic!("FIXTURE: the frame ended with {other:?}, which no test asks for"),
        }
    }

    fn settle(&mut self, _: &egui::Context) {}
}

/// Ends the frame last composed with what its widgets asked for.
fn end(rig: &mut Rig, actions: Vec<Action>) {
    let ctx = rig.ctx.clone();
    let mut ending = Ending {
        rig,
        held: VecDeque::new(),
    };
    end_frame(&mut ending, &ctx, actions);
    fixture(
        ending.held.is_empty(),
        "the worker published something that is not a frame",
    );
}

/// Whether the layer kept for `viewing` still has `texture`, at the size of
/// its box.
fn same(rig: &Rig, viewing: Viewing, texture: egui::TextureId) -> bool {
    let shows = rig
        .textures
        .picture(viewing)
        .map(|picture| (picture.id(), picture.size()));
    shows == Some((texture, side(BOX)))
}

/// A paused layer that shows frame 3 and has asked for frame 5, the frame it
/// was dragged to: its viewing, its consumer, its asset and its texture.
fn dragged(rig: &mut Rig) -> (Viewing, PreviewConsumer, AssetId, egui::TextureId) {
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(3))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused layer was not served once, the frame it asked for"
    );
    let texture = settled(rig, clip);
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    (clip, consumer, shot, texture)
}

#[test]
fn a_frame_published_while_its_frame_is_drawn_is_taken_before_that_frames_seek() {
    let mut rig = Rig::new();
    let (clip, consumer, shot, texture) = dragged(&mut rig);
    // The frame dragged to is published before the frame that asked for it
    // has ended, and the drag goes on: that frame ends with another seek.
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    end(&mut rig, vec![Action::Seek(frame(40))]);
    assert_eq!(
        uploads(&rig, texture),
        1,
        "END: a frame published while the frame that asked for it was drawn was \
         handed to its layer the left number of times before that frame's seek"
    );
    assert!(
        same(&rig, clip, texture),
        "END: a layer that took a frame at the end of a frame did not keep it in \
         the texture it had"
    );

    // The next frame has nothing left to take, and asks for the frame sought.
    assert!(
        rig.media.service.poll().is_none(),
        "CONTROL: what the end of a frame took was left to be taken again"
    );
    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(40), BLUE, true)],
        "CONTROL: a layer that took a frame at the end of the frame that sought was \
         not served once, the frame it was moved to"
    );
    assert!(same(&rig, clip, texture));
}

#[test]
fn a_frame_published_after_the_end_of_the_frame_that_sought_is_not_taken() {
    let mut rig = Rig::new();
    let (clip, consumer, shot, texture) = dragged(&mut rig);
    // The frame ends with a seek while the worker is still on the frame
    // dragged to: there is nothing for its end to take.
    end(&mut rig, vec![Action::Seek(frame(40))]);
    assert!(
        untouched(&rig, clip, texture),
        "CONTROL: the end of a frame in which nothing was published replaced the \
         picture of a layer, or uploaded to it"
    );

    // It is published then, and taken by the next frame before it composes.
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(5), BLUE, false),
        "CONTROL: a frame asked for before a seek, and published after the end of \
         the frame that sought, was taken by its layer"
    );
    assert!(
        untouched(&rig, clip, texture),
        "CONTROL: the picture a layer had when its playhead was moved was replaced, \
         or uploaded to, before the frame sought arrived"
    );
    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(40), BLUE, true)],
        "CONTROL: a layer whose earlier frame was turned away was not served once, \
         the frame it was moved to"
    );
}

#[test]
fn a_frame_taken_at_the_end_of_a_frame_is_not_asked_for_or_taken_again() {
    let mut rig = Rig::new();
    let (clip, consumer, shot, texture) = dragged(&mut rig);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    // The playhead rests: the frame ends with nothing asked for.
    end(&mut rig, Vec::new());
    assert_eq!(
        uploads(&rig, texture),
        1,
        "END: a frame published while the frame that asked for it was drawn was \
         handed to its layer the left number of times at the end of that frame"
    );
    assert!(
        rig.media.service.poll().is_none(),
        "CONTROL: what the end of a frame took was left to be taken again"
    );
    fixture(rig.consumer(clip) == consumer, "the layer changed hands");
    for _ in 0..3 {
        rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    }
    let again = rig.asked_for();
    assert!(
        again.is_empty(),
        "CONTROL: a layer that shows the frame taken at the end of a frame asked \
         for it again: {again:?}"
    );
    assert!(
        untouched(&rig, clip, texture),
        "CONTROL: the picture of a layer that rests on a frame taken at the end of \
         a frame was replaced or uploaded to"
    );
}

#[test]
fn frames_taken_before_and_after_a_frame_is_composed_leave_its_layer_one_upload() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, RED);
    rig.frame(true, &[at(clip, shot, BOX, frame(0))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(0), RED, true)],
        "CONTROL: a playing layer was not served once, the frame it first asked for"
    );
    let texture = settled(&rig, clip);

    // The clock's next frame is published, and taken by the frame after the
    // one that asked for it, before it composes.
    rig.frame(true, &[at(clip, shot, BOX, frame(1))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(1), RED, true),
        "CONTROL: a frame the clock asked for was not taken by its layer"
    );
    // That frame asks for the one after it, which the worker has at once.
    rig.frame(true, &[at(clip, shot, BOX, frame(2))]);
    fixture(
        !rig.textures.refused(),
        "the service did not take the clock's next request with the one before decoded",
    );
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    end(&mut rig, Vec::new());
    assert!(
        rig.media.service.poll().is_none(),
        "END: a frame published while the frame that asked for it was drawn was \
         left for the next frame to take"
    );
    assert_eq!(
        uploads(&rig, texture),
        1,
        "END: two frames taken for one layer with nothing painted between them, one \
         before the frame was composed and one at its end, left the left number of \
         uploads waiting"
    );
    assert!(same(&rig, clip, texture));

    // The clock carries on, and its next frame is one upload again.
    rig.frame(true, &[at(clip, shot, BOX, frame(3))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), RED, true)],
        "CONTROL: a playing layer did not carry on with the clock after a frame was \
         taken at the end of a frame"
    );
    assert_eq!(
        uploads(&rig, texture),
        1,
        "CONTROL: the frame taken by the frame after the end of one left the left \
         number of uploads waiting"
    );
}
