//! Which file a viewer layer reads its frames from, and the preview's
//! quality: rule 19 and rule 21 of the export and rendering spec.
//!
//! A layer reads a video's proxy only when proxies are read, the proxy is
//! ready, of the file as the asset says it is now, readable, and at least as
//! large as the box; a change of what it reads asks afresh under another
//! consumer, so nothing of the other file is taken. `Auto` quality halves
//! the boxes only once frames were measured late during a playback.
use super::*;
use crate::settings::PreviewQuality;
use editor_media::SourceIdentity;
use std::path::PathBuf;

fn uhd(path: &str) -> Asset {
    let mut asset = Asset::new(
        PathBuf::from(path),
        MediaKind::Video,
        Time::from_seconds(6.0),
    );
    asset.width = 3840;
    asset.height = 2160;
    asset.frame_rate = Some(editor_core::FrameRate::default());
    asset.file_size = 900_000_000;
    asset
}

fn proxy_of(asset: &Asset) -> Proxy {
    Proxy {
        path: PathBuf::from(format!("/cache/proxies/{}.mp4", asset.id)),
        width: 1280,
        height: 720,
        source: SourceIdentity::of(asset),
    }
}

fn want(asset: &Asset, size: [u32; 2], time: Time) -> Wanted<'_> {
    Wanted {
        viewing: Viewing::Source,
        asset,
        size,
        time,
        carries_on: Vec::new(),
    }
}

/// Textures that read proxies and know `asset`'s as ready.
fn reading(asset: &Asset) -> (Textures, Proxy) {
    let mut textures = Textures::default();
    textures.read_proxies(true);
    let proxy = proxy_of(asset);
    textures.set_proxy(asset.id, ProxyStatus::Ready(proxy.clone()));
    (textures, proxy)
}

fn frame([width, height]: [u32; 2]) -> Frame {
    Frame {
        width,
        height,
        rgba: vec![255; (width * height * 4) as usize].into(),
    }
}

fn consumer(textures: &Textures) -> PreviewConsumer {
    textures.asked(Viewing::Source).expect("a box was given").0
}

#[test]
fn a_ready_proxy_as_large_as_the_box_is_read() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [1280, 720], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    assert_eq!(textures.proxies_open(), std::slice::from_ref(&proxy.path));
    // A smaller box is served as well.
    textures.compose(None, false, &[want(&asset, [640, 360], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
}

#[test]
fn a_box_larger_than_the_proxy_reads_the_original() {
    let asset = uhd("/media/a.mov");
    let (mut textures, _) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [1920, 1080], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
    assert!(textures.proxies_open().is_empty());
}

#[test]
fn proxies_turned_off_are_not_read_even_when_ready() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    textures.read_proxies(false);
    textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
}

#[test]
fn a_proxy_of_what_the_file_was_is_not_read() {
    let asset = uhd("/media/a.mov");
    let (mut textures, _) = reading(&asset);
    // The same asset, now of a rewritten file: another length.
    let mut rewritten = asset.clone();
    rewritten.file_size += 1;
    textures.compose(None, false, &[want(&rewritten, [960, 540], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
    // Nor one of another place.
    let mut moved = asset.clone();
    moved.path = PathBuf::from("/media/b.mov");
    textures.compose(None, false, &[want(&moved, [960, 540], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
}

#[test]
fn only_a_ready_proxy_of_a_video_is_read() {
    let asset = uhd("/media/a.mov");
    let mut textures = Textures::default();
    textures.read_proxies(true);
    for status in [
        ProxyStatus::Queued,
        ProxyStatus::Generating(0.5),
        ProxyStatus::Failed("no".to_owned()),
    ] {
        textures.set_proxy(asset.id, status);
        textures.compose(None, false, &[want(&asset, [640, 360], Time::ZERO)]);
        assert_eq!(textures.reads(Viewing::Source), None);
    }
    let mut still = uhd("/media/still.png");
    still.kind = MediaKind::Image;
    textures.set_proxy(still.id, ProxyStatus::Ready(proxy_of(&still)));
    textures.compose(None, false, &[want(&still, [640, 360], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
}

#[test]
fn turning_to_the_proxy_asks_afresh_and_takes_nothing_of_the_original() {
    let ctx = egui::Context::default();
    let asset = uhd("/media/a.mov");
    let mut textures = Textures::default();
    textures.read_proxies(true);
    let at = Time::from_seconds(1.0);
    // The original, asked for and shown.
    textures.compose(None, false, &[want(&asset, [960, 540], at)]);
    let original = consumer(&textures);
    assert_eq!(textures.reads(Viewing::Source), None);
    textures.layers.get_mut(&original).unwrap().admitted = Some(at);
    textures.layers.get_mut(&original).unwrap().requested = Some(at);
    assert!(textures.accept(
        &ctx,
        Some(&asset),
        Some(original),
        asset.id,
        at,
        [960, 540],
        &frame([960, 540]),
    ));
    let picture = textures.picture(Viewing::Source).map(TextureHandle::id);
    assert!(picture.is_some());

    // Its proxy becomes ready: the layer reads it from the next frame on.
    let proxy = proxy_of(&asset);
    textures.set_proxy(asset.id, ProxyStatus::Ready(proxy.clone()));
    textures.compose(None, false, &[want(&asset, [960, 540], at)]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    let fresh = consumer(&textures);
    assert_ne!(
        fresh, original,
        "the other file is read under a consumer of its own"
    );
    // Asked again, as a box that changed is, and not as what it waited for.
    assert_eq!(textures.requested(Viewing::Source), None);
    assert_eq!(textures.layers[&fresh].admitted, None);
    assert_eq!(textures.layers[&fresh].failed, None);
    // The picture it had stays until the first frame of the proxy comes.
    assert_eq!(
        textures.picture(Viewing::Source).map(TextureHandle::id),
        picture
    );
    // The service is told to let the original's decoder go.
    assert!(!textures.layers.contains_key(&original));

    // A frame of the original, published late, is not taken, at the very
    // time and box the layer now asks for.
    assert!(!textures.accept(
        &ctx,
        Some(&asset),
        Some(original),
        asset.id,
        at,
        [960, 540],
        &frame([960, 540]),
    ));
    // Nor is one of the proxy before the service took a request for it.
    assert!(!textures.accept(
        &ctx,
        Some(&asset),
        Some(fresh),
        asset.id,
        at,
        [960, 540],
        &frame([960, 540]),
    ));
    // Once it has, the proxy's frame is.
    textures.layers.get_mut(&fresh).unwrap().admitted = Some(at);
    assert!(textures.accept(
        &ctx,
        Some(&asset),
        Some(fresh),
        asset.id,
        at,
        [960, 540],
        &frame([960, 540]),
    ));
}

#[test]
fn a_proxy_that_cannot_be_decoded_is_given_up_for_the_original() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
    let read = consumer(&textures);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    // Nothing to tell the person: the original is read in its place.
    assert!(!textures.failed(Some(read), asset.id, Some([960, 540])));
    assert!(textures.proxy_unreadable(asset.id));
    assert_eq!(textures.reads(Viewing::Source), None);
    let fallen_back = consumer(&textures);
    assert_ne!(fallen_back, read);
    // It is not read again in this session, though it is still ready.
    textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
    assert_eq!(consumer(&textures), fallen_back);
    // A failure of the original is the person's to hear of, as before.
    assert!(textures.failed(Some(fallen_back), asset.id, Some([960, 540])));
    // Another project keeps that knowledge: the asset is the same file.
    textures.clear();
    assert!(textures.proxy_unreadable(asset.id));
}

#[test]
fn an_unreadable_proxy_is_tried_again_once_it_is_removed_or_made_anew() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    let read = |textures: &mut Textures| {
        textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
        textures.reads(Viewing::Source).map(PathBuf::from)
    };
    assert_eq!(read(&mut textures), Some(proxy.path.clone()));
    assert!(!textures.failed(Some(consumer(&textures)), asset.id, Some([960, 540])));
    assert!(textures.proxy_unreadable(asset.id));

    // The service saying again that the same file is ready changes nothing.
    textures.set_proxy(asset.id, ProxyStatus::Ready(proxy.clone()));
    assert!(textures.proxy_unreadable(asset.id));
    assert_eq!(read(&mut textures), None);

    // Removed, and made again at the very same path: it is read.
    textures.set_proxy(asset.id, ProxyStatus::None);
    assert!(!textures.proxy_unreadable(asset.id));
    textures.set_proxy(asset.id, ProxyStatus::Queued);
    textures.set_proxy(asset.id, ProxyStatus::Generating(0.5));
    textures.set_proxy(asset.id, ProxyStatus::Ready(proxy.clone()));
    assert!(!textures.proxy_unreadable(asset.id));
    assert_eq!(read(&mut textures), Some(proxy.path.clone()));

    // Unreadable again, then replaced by another file at once: that is read.
    assert!(!textures.failed(Some(consumer(&textures)), asset.id, Some([960, 540])));
    assert_eq!(read(&mut textures), None);
    let other = Proxy {
        path: PathBuf::from("/cache/proxies/other.mp4"),
        ..proxy.clone()
    };
    textures.set_proxy(asset.id, ProxyStatus::Ready(other.clone()));
    assert!(!textures.proxy_unreadable(asset.id));
    assert_eq!(read(&mut textures), Some(other.path.clone()));

    // Unreadable once more, and asked for by hand while still ready: tried.
    assert!(!textures.failed(Some(consumer(&textures)), asset.id, Some([960, 540])));
    assert_eq!(read(&mut textures), None);
    textures.forget_unreadable(asset.id);
    assert_eq!(read(&mut textures), Some(other.path.clone()));
}

#[test]
fn a_box_held_to_the_proxy_by_the_viewer_budget_reads_the_proxy() {
    let asset = uhd("/media/a.mov");
    // Room for a box of 1280 by 1080 and not for one 1920 wide: the box
    // wanted at 1920 by 1080 is held to 1280 by 1080, which the proxy fills.
    let mut textures = Textures::with_viewer_budget(ViewerBudget {
        bytes: box_bytes([1280, 1080]),
        spare_layers: 8,
    });
    textures.read_proxies(true);
    let proxy = proxy_of(&asset);
    textures.set_proxy(asset.id, ProxyStatus::Ready(proxy.clone()));
    textures.compose(None, false, &[want(&asset, [1920, 1080], Time::ZERO)]);
    assert_eq!(
        textures.asked(Viewing::Source).map(|(_, size)| size),
        Some([1280, 1080])
    );
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
}

#[test]
fn a_box_grown_while_playing_keeps_the_proxy_until_it_is_granted() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    let rate = editor_core::FrameRate::default();
    textures.compose(None, true, &[want(&asset, [1280, 720], Time::ZERO)]);
    let read = consumer(&textures);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    // The window grows while playing: the layer is still asked at the box
    // it was granted, which the proxy fills, so it neither turns to the
    // original nor starts over.
    textures.compose(
        None,
        true,
        &[want(&asset, [1920, 1080], rate.frame_time(1))],
    );
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    assert_eq!(textures.asked(Viewing::Source), Some((read, [1280, 720])));
    // Paused, the larger box is granted, and the original is read for it.
    textures.compose(
        None,
        false,
        &[want(&asset, [1920, 1080], rate.frame_time(1))],
    );
    assert_eq!(textures.reads(Viewing::Source), None);
    assert_eq!(
        textures.asked(Viewing::Source).map(|(_, size)| size),
        Some([1920, 1080])
    );
}

#[test]
fn a_box_grown_past_the_proxy_turns_back_to_the_original() {
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [1280, 720], Time::ZERO)]);
    let read = consumer(&textures);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    textures.compose(None, false, &[want(&asset, [1920, 1080], Time::ZERO)]);
    assert_eq!(textures.reads(Viewing::Source), None);
    assert_ne!(consumer(&textures), read);
}

#[test]
fn the_proxy_goes_only_into_the_picture_request() {
    // A request the service takes names the file it is decoded from: the
    // proxy's for a layer reading one, and the asset's own identity,
    // length and rate either way.
    let asset = uhd("/media/a.mov");
    let (mut textures, proxy) = reading(&asset);
    textures.compose(None, false, &[want(&asset, [960, 540], Time::ZERO)]);
    let layer = &textures.layers[&consumer(&textures)];
    let read = layer.proxy.as_ref().expect("the proxy is read");
    assert_eq!(read.id, asset.id);
    assert_eq!(read.duration, asset.duration);
    assert_eq!(read.frame_rate, asset.frame_rate);
    assert_eq!(read.path, proxy.path);
    assert_eq!([read.width, read.height], [1280, 720]);
    // What the layer is of, and placed by, is the asset itself.
    assert_eq!(layer.asset, asset.id);
}

#[test]
fn quality_divides_the_boxes_the_viewer_asks_at() {
    let mut textures = Textures::default();
    assert_eq!(textures.detail(), 1, "Auto is full while nothing was late");
    for (quality, divisor) in [
        (PreviewQuality::Full, 1),
        (PreviewQuality::Half, 2),
        (PreviewQuality::Quarter, 4),
        (PreviewQuality::Auto, 1),
    ] {
        textures.set_quality(quality);
        assert_eq!(textures.detail(), divisor, "{quality:?}");
    }
}

/// Plays `asset` through `frames` source frames with the frames decoded
/// for it never arriving: every advance of the clock is late.
fn play_late(textures: &mut Textures, media: &MediaService, asset: &Asset, frames: i64) {
    let rate = editor_core::FrameRate::default();
    for index in 0..frames {
        let at = rate.frame_time(index);
        textures.begin_frame();
        textures.compose(Some(media), true, &[want(asset, [960, 540], at)]);
    }
}

#[test]
fn auto_quality_halves_only_after_late_frames_and_until_playback_stops() {
    let Ok(media) = MediaService::new(|| {}) else {
        println!("SKIP: the media workers did not start");
        return;
    };
    let mut asset = uhd("/nowhere/never-decoded.mov");
    asset.path = std::env::temp_dir().join(format!("neptune-video-absent-{}.mov", asset.id));
    let mut textures = Textures::default();
    textures.set_quality(PreviewQuality::Auto);
    // Paused, and then playing: starting playback changes nothing.
    textures.compose(Some(&media), false, &[want(&asset, [960, 540], Time::ZERO)]);
    textures.compose(Some(&media), true, &[want(&asset, [960, 540], Time::ZERO)]);
    assert_eq!(textures.detail(), 1);
    // A few late frames, as a decoder starting has, are not enough.
    play_late(&mut textures, &media, &asset, i64::from(LATE_STEP) - 1);
    assert!(!textures.playing_late());
    assert_eq!(textures.detail(), 1);
    // Enough of them are: half, for the rest of the playback.
    play_late(&mut textures, &media, &asset, i64::from(LATE_STEP) + 2);
    assert!(textures.playing_late());
    assert_eq!(textures.detail(), 2);
    textures.compose(
        Some(&media),
        true,
        &[want(&asset, [960, 540], Time::from_seconds(5.0))],
    );
    assert_eq!(textures.detail(), 2);
    // Stopped: full again.
    textures.compose(
        Some(&media),
        false,
        &[want(&asset, [960, 540], Time::from_seconds(5.0))],
    );
    assert_eq!(textures.detail(), 1);
    // Only Auto steps.
    textures.set_quality(PreviewQuality::Full);
    play_late(&mut textures, &media, &asset, 2 * i64::from(LATE_WINDOW));
    assert_eq!(textures.detail(), 1);
}

#[test]
fn frames_that_arrive_in_time_never_step_auto_quality_down() {
    let ctx = egui::Context::default();
    let asset = uhd("/media/a.mov");
    let mut textures = Textures::default();
    let rate = editor_core::FrameRate::default();
    for index in 0..(4 * i64::from(LATE_WINDOW)) {
        let at = rate.frame_time(index);
        textures.begin_frame();
        textures.compose(None, true, &[want(&asset, [960, 540], at)]);
        // Each frame is asked for and arrives before the clock moves on.
        let read = consumer(&textures);
        let layer = textures.layers.get_mut(&read).unwrap();
        layer.pending = true;
        layer.admitted = Some(at);
        textures.accept(
            &ctx,
            Some(&asset),
            Some(read),
            asset.id,
            at,
            [960, 540],
            &frame([96, 54]),
        );
    }
    assert!(!textures.playing_late());
    assert_eq!(textures.detail(), 1);
}

#[test]
fn proxy_statuses_are_kept_until_forgotten() {
    let asset = uhd("/media/a.mov");
    let mut textures = Textures::default();
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    textures.set_proxy(asset.id, ProxyStatus::Generating(0.25));
    assert_eq!(
        textures.proxy_status(asset.id),
        &ProxyStatus::Generating(0.25)
    );
    textures.set_proxy(asset.id, ProxyStatus::None);
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
    textures.set_proxy(asset.id, ProxyStatus::Queued);
    textures.forget_proxies();
    assert_eq!(textures.proxy_status(asset.id), &ProxyStatus::None);
}
