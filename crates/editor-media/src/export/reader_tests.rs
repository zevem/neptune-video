//! The export's source reader against continuous decodes and the preview's
//! own decoder, with real FFmpeg; stills and GIFs without it.
use super::{
    MAX_READ_BYTES, MAX_READ_SIDE, SourceReader,
    reader::{read_command, read_dimensions},
    tests::ffmpeg_available,
};
use crate::{
    MediaError,
    decode::DecoderPool,
    import::asset_from_probe,
    process::{Processes, capture, tool_command},
    service::{FrameJob, Lane},
};
use editor_core::{Asset, FrameRate, MediaKind, Time};
use image::{Delay, Frame as ImageFrame, Rgba, RgbaImage, codecs::gif::GifEncoder};
use std::{
    fs::File,
    path::Path,
    process::Stdio,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const RATE: FrameRate = FrameRate {
    numerator: 30,
    denominator: 1,
};

macro_rules! require_ffmpeg {
    ($fixture:expr) => {
        if !ffmpeg_available() {
            eprintln!("FFmpeg is unavailable; {} skipped", $fixture);
            return;
        }
    };
}

/// A clip of `frames` all-different pictures. `codec` args pick a long-GOP
/// H.264 (seeks land between keyframes) or lossless RGB.
fn clip(
    dir: &Path,
    name: &str,
    size: [u32; 2],
    frames: u32,
    codec: &[&str],
    declared: f64,
) -> Asset {
    let path = dir.join(name);
    let made = tool_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("testsrc2=size={}x{}:rate=30", size[0], size[1]))
        .args(["-frames:v", &frames.to_string()])
        .args(codec)
        .arg(&path)
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(made, "fixture {name} could not be made");
    let probe = format!(
        r#"{{"format":{{"duration":"{declared}"}},"streams":[{{"codec_type":"video","width":{},"height":{},"avg_frame_rate":"30/1"}}]}}"#,
        size[0], size[1]
    );
    asset_from_probe(&path, probe.as_bytes()).unwrap()
}

fn long_gop(dir: &Path, size: [u32; 2], frames: u32) -> Asset {
    clip(
        dir,
        "long-gop.mp4",
        size,
        frames,
        &[
            "-c:v", "libx264", "-g", "48", "-bf", "2", "-pix_fmt", "yuv420p",
        ],
        f64::from(frames) / 30.0,
    )
}

/// Every frame decoded from zero with the reader's filters: the oracle.
fn continuous(asset: &Asset, size: [u32; 2], frames: usize) -> Vec<Vec<u8>> {
    let bytes = size[0] as usize * size[1] as usize * 4;
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&asset.path)
        .args(["-map", "0:V:0", "-an", "-vf"])
        .arg(format!(
            "scale={}:{}:flags=bicubic+accurate_rnd+full_chroma_int,fps=30/1",
            size[0], size[1]
        ))
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    let raw = capture(
        "ffmpeg",
        &mut command,
        &Processes::default(),
        bytes * (frames + 4),
    )
    .unwrap();
    assert_eq!(raw.len(), bytes * frames, "the fixture has {frames} frames");
    raw.chunks_exact(bytes).map(<[u8]>::to_vec).collect()
}

fn at(index: i64) -> Time {
    RATE.frame_time(index)
}

#[test]
fn frames_read_in_order_and_after_jumps_are_the_frames_decoded_from_zero() {
    require_ffmpeg!("export reader exactness");
    let dir = tempfile::tempdir().unwrap();
    let asset = long_gop(dir.path(), [160, 90], 120);
    for size in [[160, 90], [80, 45]] {
        let oracle = continuous(&asset, size, 120);
        let mut reader = SourceReader::new();
        let lane = Uuid::new_v4();
        let check = |reader: &mut SourceReader, index: i64| {
            let frame = reader
                .picture(lane, &asset, at(index), [size[0], size[0]])
                .unwrap();
            assert_eq!([frame.width, frame.height], size);
            assert!(
                frame.rgba.as_ref() == oracle[index as usize].as_slice(),
                "frame {index} at {size:?}"
            );
        };
        // Sequential, from a seek between keyframes.
        for index in 5..20 {
            check(&mut reader, index);
        }
        assert_eq!(reader.starts, 1, "a run in order is one pipe");
        // Forward within the gap reads on; a repeat reads nothing.
        check(&mut reader, 26);
        check(&mut reader, 26);
        assert_eq!(reader.starts, 1);
        // Backward and far jumps start again, exactly.
        check(&mut reader, 10);
        assert_eq!(reader.starts, 2);
        check(&mut reader, 100);
        assert_eq!(reader.starts, 3);
        check(&mut reader, 119);
        check(&mut reader, 0);
        assert_eq!(reader.pipes(), 1);
    }
}

#[test]
fn the_reader_shows_the_preview_decoders_frames() {
    require_ffmpeg!("export reader preview parity");
    let dir = tempfile::tempdir().unwrap();
    // Lossless RGB at its own size: the scalers copy, so the two decoders
    // must agree byte for byte on which frame each time shows.
    let size = [96, 64];
    let asset = clip(
        dir.path(),
        "rgb.mkv",
        size,
        90,
        &["-pix_fmt", "bgr0", "-c:v", "ffv1"],
        3.0,
    );
    let mut preview = DecoderPool::new(Processes::default());
    let mut reader = SourceReader::new();
    let lane = Uuid::new_v4();
    let preview_lane = Lane::Legacy {
        asset_id: asset.id,
        size,
    };
    for (sequence, index) in [0_i64, 1, 2, 3, 30, 31, 89, 45, 12, 13]
        .into_iter()
        .enumerate()
    {
        // Times off the grid show the frame they are in.
        let time = at(index) + Time(7);
        let job = FrameJob {
            generation: 1,
            sequence: sequence as u64,
            asset: asset.clone(),
            time,
            lane: preview_lane,
            size,
        };
        let shown = preview.frame(&job, || true).unwrap();
        let read = reader.picture(lane, &asset, time, size).unwrap();
        assert_eq!([read.width, read.height], [shown.width, shown.height]);
        assert!(read.rgba == shown.rgba, "frame {index}");
    }
    preview.clear();
}

#[test]
fn past_the_streams_real_end_the_last_frame_holds() {
    require_ffmpeg!("export reader end of stream");
    let dir = tempfile::tempdir().unwrap();
    // The container claims five seconds; the stream has one.
    let asset = clip(
        dir.path(),
        "short.mkv",
        [64, 32],
        30,
        &["-c:v", "ffv1"],
        5.0,
    );
    let oracle = continuous(&asset, [64, 32], 30);
    let last = oracle[29].as_slice();
    let lane = Uuid::new_v4();
    // Read in order past the end.
    let mut reader = SourceReader::new();
    for index in 25..40 {
        let frame = reader.picture(lane, &asset, at(index), [64, 32]).unwrap();
        let expected = oracle.get(index as usize).map_or(last, Vec::as_slice);
        assert!(frame.rgba.as_ref() == expected, "frame {index}");
    }
    // Sought past the end, cold.
    let mut reader = SourceReader::new();
    let frame = reader
        .picture(lane, &asset, Time::from_seconds(3.0), [64, 32])
        .unwrap();
    assert!(frame.rgba.as_ref() == last);
    let frame = reader
        .picture(lane, &asset, Time::from_seconds(4.9), [64, 32])
        .unwrap();
    assert!(frame.rgba.as_ref() == last);
    // Back inside, it reads again.
    let frame = reader.picture(lane, &asset, at(3), [64, 32]).unwrap();
    assert!(frame.rgba.as_ref() == oracle[3].as_slice());
}

#[test]
fn lanes_keep_their_own_pipes_and_retain_drops_the_rest() {
    require_ffmpeg!("export reader lanes");
    let dir = tempfile::tempdir().unwrap();
    let asset = long_gop(dir.path(), [64, 32], 60);
    let mut reader = SourceReader::new();
    let (picture, plate) = (Uuid::new_v4(), Uuid::new_v4());
    for index in 0..10 {
        reader
            .picture(picture, &asset, at(index), [64, 32])
            .unwrap();
        reader
            .picture(plate, &asset, at(index + 30), [64, 32])
            .unwrap();
    }
    assert_eq!(reader.starts, 2, "two lanes, two pipes, no restarts");
    assert_eq!(reader.pipes(), 2);
    reader.retain(&|lane| lane == picture);
    assert_eq!(reader.pipes(), 1);
    reader.picture(picture, &asset, at(10), [64, 32]).unwrap();
    assert_eq!(reader.starts, 2);
}

#[test]
fn a_canceller_on_another_thread_stops_the_reader() {
    require_ffmpeg!("export reader cancellation");
    let dir = tempfile::tempdir().unwrap();
    let asset = long_gop(dir.path(), [64, 32], 60);
    let mut reader = SourceReader::new();
    let lane = Uuid::new_v4();
    reader.picture(lane, &asset, at(0), [64, 32]).unwrap();
    let cancel = reader.canceller();
    thread::spawn(cancel).join().unwrap();
    assert!(matches!(
        reader.picture(lane, &asset, at(1), [64, 32]),
        Err(MediaError::Cancelled)
    ));
    let processes = reader.processes().clone();
    drop(reader);
    let until = Instant::now() + Duration::from_secs(3);
    while processes.retained_monitors() != 0 {
        assert!(Instant::now() < until, "a pipe outlived its reader");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_missing_source_is_an_error_not_a_held_frame() {
    require_ffmpeg!("export reader missing source");
    let dir = tempfile::tempdir().unwrap();
    let mut asset = long_gop(dir.path(), [64, 32], 30);
    asset.path = dir.path().join("gone.mp4");
    let error = SourceReader::new()
        .picture(Uuid::new_v4(), &asset, at(3), [64, 32])
        .unwrap_err();
    assert!(matches!(error, MediaError::Decode(_)), "{error:?}");
}

fn still(dir: &Path) -> Asset {
    let path = dir.join("still.png");
    let image = RgbaImage::from_fn(40, 20, |x, y| {
        Rgba([(x * 6) as u8, (y * 12) as u8, 90, 255])
    });
    image.save(&path).unwrap();
    let mut asset = Asset::new(path, MediaKind::Image, Time::from_seconds(5.0));
    asset.width = 40;
    asset.height = 20;
    asset
}

#[test]
fn stills_are_decoded_once_fitted_and_never_upscaled() {
    let dir = tempfile::tempdir().unwrap();
    let asset = still(dir.path());
    let mut reader = SourceReader::new();
    let lane = Uuid::new_v4();
    let full = reader
        .picture(lane, &asset, Time::ZERO, [100, 100])
        .unwrap();
    assert_eq!([full.width, full.height], [40, 20]);
    assert_eq!(&full.rgba[..4], &[0, 0, 90, 255]);
    let again = reader
        .picture(lane, &asset, Time::from_seconds(3.0), [100, 100])
        .unwrap();
    assert!(
        Arc::ptr_eq(&full.rgba, &again.rgba),
        "a still is decoded once"
    );
    let half = reader.picture(lane, &asset, Time::ZERO, [20, 20]).unwrap();
    assert_eq!([half.width, half.height], [20, 10]);
    reader.retain(&|_| false);
    assert_eq!(reader.pipes(), 0);
}

#[test]
fn gifs_show_the_frame_a_time_is_in_and_hold_the_last() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("three.gif");
    {
        let mut encoder = GifEncoder::new(File::create(&path).unwrap());
        for (shade, ms) in [(10_u8, 100_u32), (120, 200), (240, 100)] {
            let image = RgbaImage::from_pixel(8, 8, Rgba([shade, shade, shade, 255]));
            encoder
                .encode_frame(ImageFrame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                ))
                .unwrap();
        }
    }
    let mut asset = Asset::new(path, MediaKind::Gif, Time::from_seconds(0.4));
    asset.width = 8;
    asset.height = 8;
    let mut reader = SourceReader::new();
    let lane = Uuid::new_v4();
    let mut shade = |seconds: f64| {
        reader
            .picture(lane, &asset, Time::from_seconds(seconds), [8, 8])
            .unwrap()
            .rgba[0]
    };
    let first = shade(0.05);
    let second = shade(0.15);
    let third = shade(0.35);
    assert!(first < second && second < third, "{first} {second} {third}");
    assert_eq!(shade(0.29), second);
    // Back to the start, and past the end (clamped to the last frame).
    assert_eq!(shade(0.0), first);
    assert_eq!(shade(9.0), third);
}

#[test]
fn the_reader_has_its_own_size_bounds() {
    assert_eq!(
        read_dimensions([3840, 2160], [1920, 1920]).unwrap(),
        [1920, 1080]
    );
    // Never upscaled.
    assert_eq!(
        read_dimensions([640, 360], [1920, 1080]).unwrap(),
        [640, 360]
    );
    // Above the preview's 4096 limit, up to 8192 a side and 256 MiB.
    assert_eq!(
        read_dimensions([7680, 4320], [7680, 4320]).unwrap(),
        [7680, 4320]
    );
    assert_eq!(
        read_dimensions([8192, 8192], [8192, 8192]).unwrap(),
        [8192, 8192]
    );
    assert_eq!(8192 * 8192 * 4, MAX_READ_BYTES);
    assert!(read_dimensions([16384, 8192], [MAX_READ_SIDE + 1, 100]).is_err());
    assert!(read_dimensions([100, 100], [0, 100]).is_err());
    assert!(read_dimensions([0, 100], [100, 100]).is_err());
    // A tiny source side stays at least one pixel.
    assert_eq!(read_dimensions([8000, 2], [100, 100]).unwrap(), [100, 1]);
}

#[test]
fn a_pipe_decodes_with_a_bounded_number_of_threads() {
    // FFmpeg's automatic decoder threads (up to 16 frame threads, each
    // holding pictures) took a paced 4K pipe from about 780 to 1,100 MiB;
    // up to sixteen pipes may be open at once.
    let asset = Asset::new(
        "/media/source.mp4".into(),
        MediaKind::Video,
        Time::from_seconds(5.0),
    );
    let command = read_command(&asset, RATE, 30, [3840, 2160]);
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let input = args.iter().position(|arg| arg == "-i").unwrap();
    // Before the input, so the decoder takes it; after, only the raw
    // output's single thread.
    let threads = args[..input]
        .windows(2)
        .find(|pair| pair[0] == "-threads")
        .map(|pair| pair[1].as_str());
    assert_eq!(threads, Some("4"));
    let after: Vec<&str> = args[input..].iter().map(String::as_str).collect();
    assert!(after.windows(2).any(|pair| pair == ["-threads:v", "1"]));
    assert!(
        after
            .windows(2)
            .any(|pair| pair == ["-filter_threads", "2"])
    );
    assert!(!after.contains(&"-threads"));
}
