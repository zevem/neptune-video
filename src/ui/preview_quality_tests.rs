//! The preview's quality and its proxies, as the viewer asks for pictures:
//! the boxes and canvases a quality holds to, and which file a layer reads
//! for the box the screen needs of the original.
use super::{SourceInput, decode_box, decode_box_at, detailed_size, show_source};
use crate::{
    compose,
    settings::PreviewQuality,
    textures::{Textures, Viewing},
    theme::{self, Accent, Appearance, Palette},
    ui::Action,
};
use editor_core::{Asset, FrameRate, MediaKind, Time};
use editor_media::{Proxy, ProxyStatus, SourceIdentity};
use eframe::egui::{self, Pos2, Rect, Vec2, vec2};
use std::path::PathBuf;

fn pictured(width: u32, height: u32) -> Asset {
    let mut asset = Asset::new(
        PathBuf::from("/media/wide.mov"),
        MediaKind::Video,
        Time::from_seconds(8.0),
    );
    asset.width = width;
    asset.height = height;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = 500_000_000;
    asset
}

#[test]
fn half_and_quarter_ask_at_a_half_and_a_quarter_of_the_full_step() {
    for (needed, full, half, quarter) in [
        (1920.0, 1920, 960, 480),
        (9000.0, 1920, 960, 480),
        (1200.0, 1280, 640, 320),
        (900.0, 960, 480, 320),
        (600.0, 640, 320, 320),
        (100.0, 320, 320, 320),
    ] {
        assert_eq!(detailed_size(needed, 1), [full, full], "{needed}");
        assert_eq!(detailed_size(needed, 2), [half, half], "{needed}");
        assert_eq!(detailed_size(needed, 4), [quarter, quarter], "{needed}");
    }
}

#[test]
fn a_quality_box_keeps_the_shape_of_the_picture() {
    let uhd = pictured(3840, 2160);
    assert_eq!(decode_box_at(1920.0, &uhd, 1), [1920, 1080]);
    assert_eq!(decode_box_at(1920.0, &uhd, 2), [960, 540]);
    assert_eq!(decode_box_at(1920.0, &uhd, 4), [480, 270]);
    let tall = pictured(1080, 1920);
    assert_eq!(decode_box_at(1920.0, &tall, 2), [540, 960]);
    // Full is what the viewer asked before there was a quality.
    for needed in [1.0, 600.0, 1281.0, 9000.0] {
        assert_eq!(decode_box_at(needed, &uhd, 1), decode_box(needed, &uhd));
    }
}

#[test]
fn a_quality_canvas_steps_down_and_never_below_the_smallest_step() {
    assert_eq!(compose::detailed([1920, 1080], 1), [1920, 1080]);
    assert_eq!(compose::detailed([2400, 1350], 1), [2400, 1350]);
    assert_eq!(compose::detailed([1920, 1080], 2), [960, 540]);
    assert_eq!(compose::detailed([1920, 1080], 4), [480, 270]);
    // A half of 2400 is 1200: the step within it is 960.
    assert_eq!(compose::detailed([2400, 1350], 2), [960, 540]);
    assert_eq!(compose::detailed([1080, 1920], 2), [540, 960]);
    // Never under the smallest step, and never larger than asked.
    assert_eq!(compose::detailed([300, 169], 4), [160, 90]);
    assert_eq!(compose::detailed([100, 56], 4), [100, 56]);
    assert_eq!(compose::detailed([0, 0], 2), [0, 0]);
}

/// The source monitor painted at `size` with `textures`, as the window
/// paints it, until its layer settles.
fn paint(textures: &mut Textures, asset: &Asset, size: Vec2, playing: bool) {
    let ctx = egui::Context::default();
    let palette = Palette::new(Appearance::Dark, Accent::default());
    theme::install_fonts(&ctx);
    for _ in 0..2 {
        textures.begin_frame();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut actions: Vec<Action> = Vec::new();
        let output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            show_source(
                ui,
                rect,
                textures,
                &SourceInput {
                    p: &palette,
                    asset,
                    rate: FrameRate::default(),
                    background: [0, 0, 0, 255],
                    playhead: Time::ZERO,
                    playing,
                    speed: 1.0,
                    looping: false,
                    media: None,
                },
                &mut actions,
            );
        });
        output.drop_without_applying_deltas();
    }
}

fn asked(textures: &Textures) -> [u32; 2] {
    textures
        .asked(Viewing::Source)
        .expect("the monitor's layer has a box")
        .1
}

#[test]
fn the_viewer_asks_at_the_quality_chosen_and_not_more_for_starting_to_play() {
    let uhd = pictured(3840, 2160);
    // The monitor's picture is about 964 points across: the 1280 step.
    let window = vec2(1000.0, 700.0);
    let mut textures = Textures::default();
    for (quality, box_) in [
        (PreviewQuality::Full, [1280, 720]),
        (PreviewQuality::Half, [640, 360]),
        (PreviewQuality::Quarter, [320, 180]),
        (PreviewQuality::Auto, [1280, 720]),
    ] {
        textures.set_quality(quality);
        paint(&mut textures, &uhd, window, false);
        assert_eq!(asked(&textures), box_, "{quality:?}");
    }
    // Auto, playing with nothing measured late, asks as it did paused.
    paint(&mut textures, &uhd, window, true);
    assert_eq!(asked(&textures), [1280, 720]);
}

#[test]
fn the_viewer_reads_a_proxy_only_for_a_box_it_can_fill() {
    let uhd = pictured(3840, 2160);
    let proxy = Proxy {
        path: PathBuf::from("/cache/proxies/wide.mp4"),
        width: 1280,
        height: 720,
        source: SourceIdentity::of(&uhd),
    };
    let mut textures = Textures::default();
    textures.read_proxies(true);
    textures.set_proxy(uhd.id, ProxyStatus::Ready(proxy.clone()));
    textures.set_quality(PreviewQuality::Full);
    // A box of 1280 by 720, worked out from the original: the proxy fills it.
    paint(&mut textures, &uhd, vec2(1000.0, 700.0), false);
    assert_eq!(asked(&textures), [1280, 720]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
    // A larger window needs the 1920 step: the original is read.
    paint(&mut textures, &uhd, vec2(2000.0, 1300.0), false);
    assert_eq!(asked(&textures), [1920, 1080]);
    assert_eq!(textures.reads(Viewing::Source), None);
    // At half, the proxy serves that window too.
    textures.set_quality(PreviewQuality::Half);
    paint(&mut textures, &uhd, vec2(2000.0, 1300.0), false);
    assert_eq!(asked(&textures), [960, 540]);
    assert_eq!(textures.reads(Viewing::Source), Some(proxy.path.as_path()));
}
