//! What a layer takes of the frames asked for before its playhead was moved.
//!
//! Composed and published as in the tests this is nested in, and handed over
//! in the order the application does it. A seek, a step or a jump is applied
//! at the end of a frame; the next frame first takes what the worker has
//! published and only then composes, which is where the frame sought is asked
//! for. So a frame asked for before the seek, published and not yet taken, is
//! handed over after the playhead moved and before anything was asked at
//! where it moved to: [`Textures::jumped`], then [`Textures::accept`], then
//! [`Textures::compose`]. The tests of what a playing layer asks for compose
//! before they take, which is the other order.
//!
//! `SEEK:` is the behavior these were written for: from the moment the
//! playhead is moved, a layer takes nothing asked for before that, whether or
//! not the service has taken the request that is to take its place, and keeps
//! the picture it has until the frame sought arrives. `CONTROL:` holds without
//! that too, and is what a correction must not cost: a frame at the very time
//! the playhead was moved to is still shown, asked for again where it has to
//! be, and a layer that already shows it asks for nothing.
//!
//! The stills here are one colour at every time, so a frame that was wrongly
//! taken looks like the one it took the place of. What is read instead is
//! whether the layer kept it, and what the context was handed to upload.
use super::*;

/// The lanes the worker keeps asked for before it has no room for another.
const LANES: usize = 32;

/// A frame published for `consumer` at the time it was asked at, filling its
/// box with the colour of its still, and whether its layer kept it.
fn published(
    consumer: PreviewConsumer,
    asset: AssetId,
    time: Time,
    colour: [u8; 4],
    taken: bool,
) -> Outcome {
    Outcome {
        consumer,
        asset,
        size: [BOX, BOX],
        result: Published::Picture {
            time,
            texels: [BOX, BOX],
            colour: Some(colour),
            taken,
        },
    }
}

/// Those of `outcomes` that were published for `consumer`, in order.
fn of(outcomes: Vec<Outcome>, consumer: PreviewConsumer) -> Vec<Outcome> {
    outcomes
        .into_iter()
        .filter(|outcome| outcome.consumer == consumer)
        .collect()
}

/// How many times the context was handed something to upload for `texture`
/// since this was last read.
fn uploads(rig: &Rig, texture: egui::TextureId) -> usize {
    let mut delta = rig.ctx.tex_manager().write().take_delta();
    let uploads = delta.set.get(&texture).map_or(0, |deltas| deltas.len());
    delta.clear();
    uploads
}

/// The texture of the picture the layer kept for `viewing` has, with what
/// was handed over to be uploaded to it so far read and forgotten.
fn settled(rig: &Rig, viewing: Viewing) -> egui::TextureId {
    let Some(texture) = rig.textures.picture(viewing).map(|picture| picture.id()) else {
        panic!("FIXTURE: a layer that was served has no picture");
    };
    uploads(rig, texture);
    texture
}

/// Whether the layer kept for `viewing` still shows `texture`, at the size of
/// its box, with nothing handed over to be uploaded to it since it settled.
fn untouched(rig: &Rig, viewing: Viewing, texture: egui::TextureId) -> bool {
    let shows = rig
        .textures
        .picture(viewing)
        .map(|picture| (picture.id(), picture.size()));
    shows == Some((texture, side(BOX))) && uploads(rig, texture) == 0
}

/// Asks for the test's own still under as many consumers no layer has as the
/// worker keeps asked for. It is held where it last published, so none is
/// taken up and it has no room for anything a layer then asks for.
fn crowd(rig: &Rig) -> Vec<PreviewConsumer> {
    (0..LANES)
        .map(|_| {
            let consumer = PreviewConsumer::new();
            let asked =
                rig.media
                    .service
                    .request_preview(consumer, &rig.marker, Time::ZERO, [1, 1]);
            fixture(asked.is_ok(), "the worker did not take one of the crowd");
            consumer
        })
        .collect()
}

/// Lets the worker on to one of the crowd's pictures, which leaves it room
/// for one request more.
fn room(rig: &mut Rig, crowd: &[PreviewConsumer]) {
    let made = rig
        .result()
        .is_some_and(|outcome| crowd.contains(&outcome.consumer));
    fixture(made, "the worker did not go on to one of the crowd");
}

/// One frame of playback in which the clip `to` is drawn alone and carries
/// on from `from`: the playhead has crossed the cut between them.
fn cut(rig: &mut Rig, from: ClipId, to: ClipId, asset: AssetId, time: Time) {
    rig.textures.begin_frame();
    let wanted = [Wanted {
        viewing: Viewing::Clip(to),
        asset: &rig.library[&asset],
        size: [BOX, BOX],
        time,
        carries_on: vec![from],
    }];
    rig.textures
        .compose(Some(&rig.media.service), true, &wanted);
}

#[test]
fn a_frame_published_before_a_paused_seek_is_not_taken_in_the_frame_after_it() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(3))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused layer was not served once, the frame it asked for"
    );
    let texture = settled(&rig, clip);

    // The playhead is dragged on, and the frame at where it was dragged to
    // is published and not yet taken when it is moved somewhere else.
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    rig.textures.jumped();

    // The next frame takes what was published before it composes.
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(5), BLUE, false),
        "SEEK: a frame asked for before a seek was taken by its layer after the seek, \
         before the frame sought was asked for"
    );
    assert!(
        untouched(&rig, clip, texture),
        "SEEK: the picture a layer had when its playhead was moved was replaced, or \
         uploaded to, before the frame sought arrived"
    );

    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(40), BLUE, true)],
        "SEEK: a layer whose earlier frame was turned away was not served once, the \
         frame it was moved to"
    );
    assert_eq!(rig.picture(clip), Some(side(BOX)));
}

#[test]
fn frames_the_clock_asked_for_before_a_seek_are_not_taken_in_the_frames_after_it() {
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

    // The clock asks for its next frame, and for the one after it while that
    // is published and not yet taken: two are the layer's to take, as long
    // as the clock carries on.
    rig.frame(true, &[at(clip, shot, BOX, frame(1))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(2))]);
    fixture(
        !rig.textures.refused(),
        "the service did not take the clock's next request with the one before decoded",
    );

    // The application moves the playhead instead.
    rig.textures.jumped();
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(1), RED, false),
        "SEEK: the frame the clock asked for before its next was taken by its layer \
         after a seek"
    );
    fixture(
        rig.publication(),
        "the worker published nothing for the frame the clock asked for last",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(2), RED, false),
        "SEEK: the frame the clock last asked for before a seek was taken by its layer \
         after the seek, before the frame sought was asked for"
    );
    assert!(
        untouched(&rig, clip, texture),
        "SEEK: the picture a playing layer had when its playhead was moved was \
         replaced, or uploaded to, before the frame sought arrived"
    );

    // The clock carries on from where the playhead was moved to.
    rig.frame(true, &[at(clip, shot, BOX, frame(40))]);
    rig.frame(true, &[at(clip, shot, BOX, frame(41))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(40), RED, true)],
        "SEEK: a playing layer whose earlier frames were turned away was not served \
         once, the frame it was moved to"
    );

    // And from there a frame published while its next is asked for is the
    // layer's again, once.
    rig.frame(true, &[at(clip, shot, BOX, frame(42))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(43))]);
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(42), RED, true),
        "CONTROL: after a seek, a frame the clock asked for, published while its next \
         was asked for, was not taken by its layer"
    );
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(43), RED, true)],
        "CONTROL: a playing layer did not carry on with the clock after a seek"
    );
}

#[test]
fn a_frame_published_before_a_seek_is_not_taken_while_the_frame_sought_is_refused() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(3))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused layer was not served once, the frame it asked for"
    );
    let texture = settled(&rig, clip);
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The playhead is moved while the worker has no room: the frame sought
    // is asked for and not taken, and the service still holds, as the last
    // request it took for this layer, the one from before the seek.
    let crowd = crowd(&rig);
    rig.textures.jumped();
    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    fixture(
        rig.textures.refused(),
        "the service took a request it was to have no room for",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(5), BLUE, false),
        "SEEK: a frame asked for before a seek was taken by its layer after the seek, \
         because the service had no room for the frame sought"
    );
    assert!(
        untouched(&rig, clip, texture),
        "SEEK: the picture a layer had when its playhead was moved was replaced, or \
         uploaded to, while the frame sought was refused"
    );

    // With room, the frame sought is asked for again and served, once.
    room(&mut rig, &crowd);
    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    assert!(
        !rig.textures.refused(),
        "CONTROL: a layer is still said to be refused after its request was taken"
    );
    room(&mut rig, &crowd);
    assert_eq!(
        of(rig.asked_for(), consumer),
        [published(consumer, shot, frame(40), BLUE, true)],
        "SEEK: a layer whose frame sought was refused was not served it once, when \
         the service had room"
    );
    assert_eq!(rig.picture(clip), Some(side(BOX)));
}

#[test]
fn a_seek_while_playing_that_is_refused_keeps_the_picture_and_is_served_when_there_is_room() {
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
    rig.frame(true, &[at(clip, shot, BOX, frame(1))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The playhead is moved, what was published is taken, and only then is
    // the frame sought asked for, of a worker that has no room for it.
    rig.textures.jumped();
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(1), RED, false),
        "SEEK: a frame asked for before a seek while playing was taken by its layer \
         after the seek, before the frame sought was asked for"
    );
    let crowd = crowd(&rig);
    for index in 40..42 {
        rig.frame(true, &[at(clip, shot, BOX, frame(index))]);
        fixture(
            rig.textures.refused(),
            "the service took a request it was to have no room for",
        );
    }
    assert!(
        untouched(&rig, clip, texture),
        "SEEK: the picture a playing layer had when its playhead was moved was \
         replaced, or uploaded to, while the frame sought was refused"
    );

    // With room, it is served the frame the clock is then at, in the place
    // of nothing, and carries on with the clock from there.
    room(&mut rig, &crowd);
    rig.frame(true, &[at(clip, shot, BOX, frame(42))]);
    assert!(
        !rig.textures.refused(),
        "CONTROL: a layer is still said to be refused after its request was taken"
    );
    room(&mut rig, &crowd);
    assert_eq!(
        of(rig.asked_for(), consumer),
        [published(consumer, shot, frame(42), RED, true)],
        "SEEK: a playing layer whose frame sought was refused was not served once, \
         the frame its clock was at when the service had room"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(43))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(43), RED, true)],
        "CONTROL: a playing layer did not carry on with the clock after a seek that \
         was refused"
    );
}

#[test]
fn a_frame_of_the_clip_before_a_cut_is_not_taken_by_the_clip_that_carries_on_while_refused() {
    let mut rig = Rig::new();
    let (from, to) = (ClipId::new_v4(), ClipId::new_v4());
    let shot = rig.painted(BOX, AMBER);
    rig.frame(true, &[at(Viewing::Clip(from), shot, BOX, frame(0))]);
    let consumer = rig.consumer(Viewing::Clip(from));
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(0), AMBER, true)],
        "CONTROL: a playing layer was not served once, the frame it first asked for"
    );
    let texture = settled(&rig, Viewing::Clip(from));
    rig.frame(true, &[at(Viewing::Clip(from), shot, BOX, frame(1))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The playhead crosses the cut while the worker has no room: the next
    // clip takes the layer over and its own frame is not taken.
    let crowd = crowd(&rig);
    cut(&mut rig, from, to, shot, frame(40));
    fixture(
        rig.textures.asked(Viewing::Clip(to)) == Some((consumer, [BOX, BOX])),
        "the layer of the clip before the cut did not change hands",
    );
    fixture(
        rig.textures.refused(),
        "the service took a request it was to have no room for",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(1), AMBER, false),
        "SEEK: a frame asked for by the clip before a cut was taken by the clip that \
         carries on from it, because the service had no room for that clip's own"
    );
    assert!(
        untouched(&rig, Viewing::Clip(to), texture),
        "SEEK: the picture a layer had when it changed hands was replaced, or \
         uploaded to, while the frame of the clip that took it over was refused"
    );

    room(&mut rig, &crowd);
    rig.frame(true, &[at(Viewing::Clip(to), shot, BOX, frame(41))]);
    room(&mut rig, &crowd);
    assert_eq!(
        of(rig.asked_for(), consumer),
        [published(consumer, shot, frame(41), AMBER, true)],
        "SEEK: a clip that took over the layer of a cut while refused was not served \
         once, its own frame, when the service had room"
    );
}

/// The playhead of a paused layer is moved to where it already is, with the
/// frame it asked for there published and not yet taken: a seek to the same
/// time, or an edit that leaves the clip under the playhead where it was.
/// Whether that publication is the layer's or one asked for again is, the
/// frame is shown and nothing more is asked for; `crowded`, the worker has no
/// room when it is asked again.
fn sought_where_it_was(crowded: bool) {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    let consumer = rig.consumer(clip);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    rig.textures.jumped();
    let before = rig.take();
    let Published::Picture { taken, .. } = before.result else {
        panic!("FIXTURE: the worker failed to decode a still that is there");
    };
    fixture(
        before == published(consumer, shot, frame(5), BLUE, taken),
        "the worker did not publish the frame the layer asked for",
    );

    let crowd = if crowded { crowd(&rig) } else { Vec::new() };
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    if crowded {
        assert_eq!(
            rig.textures.refused(),
            !taken,
            "CONTROL: a layer sought to the time of a frame it did not take is said to \
             be refused, or one that took it is not, on the left; whether it did not \
             take it, on the right"
        );
        room(&mut rig, &crowd);
        rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
        room(&mut rig, &crowd);
    }
    let again = of(rig.asked_for(), consumer);
    if taken {
        assert!(
            again.is_empty(),
            "CONTROL: a layer that took the frame at the time it was sought to asked \
             for it again: {again:?}"
        );
    } else {
        assert_eq!(
            again,
            [published(consumer, shot, frame(5), BLUE, true)],
            "CONTROL: a layer sought to the time of a frame it turned away was not \
             served that frame once more"
        );
    }
    assert_eq!(
        rig.picture(clip),
        Some(side(BOX)),
        "CONTROL: a layer sought to the time it had already asked for never showed \
         the frame at it"
    );

    // And there it settles.
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    let more = of(rig.asked_for(), consumer);
    assert!(
        more.is_empty(),
        "CONTROL: a layer that shows the frame it was sought to asked for it again: \
         {more:?}"
    );
}

#[test]
fn a_seek_to_the_time_a_published_frame_is_of_shows_that_frame() {
    sought_where_it_was(false);
}

#[test]
fn a_seek_to_the_time_a_published_frame_is_of_shows_that_frame_when_refused_at_first() {
    sought_where_it_was(true);
}

#[test]
fn a_seek_to_the_time_still_waited_for_is_served_that_frame_once() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    // Asked for and not yet decoded when the playhead is moved to where it
    // already is.
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    let consumer = rig.consumer(clip);
    rig.textures.jumped();
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(5), BLUE, true)],
        "CONTROL: a layer sought to the time it was still waiting for was not served \
         once, the frame at it, and given it"
    );
    assert_eq!(rig.picture(clip), Some(side(BOX)));
}

#[test]
fn a_seek_to_the_time_already_shown_asks_for_nothing_and_keeps_the_picture() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(5), BLUE, true)],
        "CONTROL: a paused layer was not served once, the frame it asked for"
    );
    let texture = settled(&rig, clip);

    // Every edit says the playhead may show something else. A layer that
    // shows the frame at where it is has nothing to decode again.
    for _ in 0..3 {
        rig.textures.jumped();
        rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    }
    let again = rig.asked_for();
    assert!(
        again.is_empty(),
        "CONTROL: a layer that shows the frame at the time it was sought to asked \
         for it again: {again:?}"
    );
    assert!(
        untouched(&rig, clip, texture),
        "CONTROL: the picture of a layer sought to the time it shows was replaced or \
         uploaded to"
    );
}

// ---- What the end of a frame takes before its seek --------------------------

/// Nested here for what is read above, and gone through in the order the
/// application ends a frame in.
#[path = "end_of_frame_tests.rs"]
mod end_of_frame_tests;

// ---- What the source monitor takes after it was opened again ----------------

/// Nested here for what is read above, and opened through what the
/// application opens the source monitor with.
#[path = "source_reset_tests.rs"]
mod source_reset_tests;
