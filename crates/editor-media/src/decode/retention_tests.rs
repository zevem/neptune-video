//! Verify bounded five-pipe retention at the actual decoder-manager seam.
//!
//! Exact pixels and owned child identities cover continuity and reclamation.
//! These checks do not measure playback throughput. FFmpeg is required: missing
//! tools or codec support fail instead of passing an unexecuted fixture.

use super::DecoderPool;
use crate::{
    PreviewConsumer,
    process::{Processes, tool_command},
    service::{FrameJob, Lane},
};
use editor_core::{Asset, FrameRate, MediaKind};
use std::{
    fs,
    io::Write,
    process::{Child, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const SIZE: [u32; 2] = [64, 32];
const SOURCE_FRAMES: usize = 4;

/// Also kills/reaps an encoder if a fixture assertion unwinds before completion.
struct OwnedEncoder(Child);

impl Drop for OwnedEncoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Videos {
    pool: DecoderPool,
    assets: Vec<Asset>,
    expected: Vec<Vec<Vec<u8>>>,
    size: [u32; 2],
    _directory: tempfile::TempDir,
}

impl Videos {
    fn new(count: usize) -> Self {
        Self::sized(count, SIZE)
    }

    fn sized(count: usize, size: [u32; 2]) -> Self {
        let directory = tempfile::tempdir().expect("owned retention fixture directory");
        let rate = FrameRate::default();
        let mut assets = Vec::with_capacity(count);
        let mut expected = Vec::with_capacity(count);

        for asset_index in 0..count {
            let frames: Vec<_> = (0..SOURCE_FRAMES)
                .map(|frame_index| pixels(asset_index, frame_index, size))
                .collect();
            let input = directory.path().join(format!("source-{asset_index}.rgba"));
            let mut raw = fs::File::create(&input).expect("owned RGBA source file");
            for frame in &frames {
                raw.write_all(frame).expect("write complete source frame");
            }
            drop(raw);

            // NUT retains the exact 30-fps time base, unlike millisecond-rounded
            // timestamps that could make a fresh frame-1 seek miss its frame.
            let path = directory.path().join(format!("source-{asset_index}.nut"));
            let mut command = tool_command("ffmpeg");
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-f",
                    "rawvideo",
                    "-pixel_format",
                    "rgba",
                    "-video_size",
                ])
                .arg(format!("{}x{}", size[0], size[1]))
                .args(["-framerate", "30", "-i"])
                .arg(&input)
                .args([
                    "-an",
                    "-c:v",
                    "ffv1",
                    "-level",
                    "3",
                    "-pix_fmt",
                    "bgra",
                    "-threads",
                    "1",
                    "-filter_threads",
                    "1",
                    "-f",
                    "nut",
                ])
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
            let mut encoder = OwnedEncoder(
                command
                    .spawn()
                    .expect("start configured FFmpeg for owned lossless fixture"),
            );
            let status = encoder.0.wait().expect("wait for owned fixture encoder");
            assert!(
                status.success(),
                "lossless fixture encoding failed: {status}"
            );
            drop(encoder);

            let mut asset = Asset::new(
                path,
                MediaKind::Video,
                rate.frame_time(SOURCE_FRAMES as i64),
            );
            asset.width = size[0];
            asset.height = size[1];
            asset.frame_rate = Some(rate);
            asset.file_size = fs::metadata(&asset.path)
                .expect("owned encoded source metadata")
                .len();
            assets.push(asset);
            expected.push(frames);
        }

        Self {
            pool: DecoderPool::new(Processes::default()),
            assets,
            expected,
            size,
            _directory: directory,
        }
    }

    fn job(&self, asset_index: usize, frame_index: usize) -> FrameJob {
        let asset = self.assets[asset_index].clone();
        FrameJob {
            generation: 1,
            sequence: (frame_index * self.assets.len() + asset_index + 1) as u64,
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: self.size,
            },
            size: self.size,
            asset,
            time: FrameRate::default().frame_time(frame_index as i64),
        }
    }
}

impl Drop for Videos {
    fn drop(&mut self) {
        // Reap every retained task-owned decoder before TempDir removes media,
        // including during assertion unwinding and on native Windows hosts.
        self.pool.clear();
    }
}

fn pixels(asset_index: usize, frame_index: usize, size: [u32; 2]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size[0] * size[1] * 4) as usize);
    for y in 0..size[1] {
        for x in 0..size[0] {
            rgba.extend_from_slice(&[
                (asset_index as u8)
                    .wrapping_mul(37)
                    .wrapping_add((frame_index as u8).wrapping_mul(19))
                    .wrapping_add(x as u8),
                (frame_index as u8)
                    .wrapping_mul(71)
                    .wrapping_add((y as u8).wrapping_mul(3)),
                (asset_index as u8)
                    .wrapping_mul(23)
                    .wrapping_add((x ^ y) as u8),
                255,
            ]);
        }
    }
    rgba
}

#[test]
fn five_distinct_video_lanes_keep_their_pipes_for_uncached_next_frame() {
    let mut videos = Videos::new(5);

    for frame_index in 0..2 {
        for asset_index in 0..5 {
            let job = videos.job(asset_index, frame_index);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("current owned video frame");
            assert_eq!([frame.width, frame.height], SIZE);
            assert_eq!(
                frame.rgba.as_ref(),
                videos.expected[asset_index][frame_index].as_slice(),
                "full source pixels for asset {asset_index}, frame {frame_index}"
            );
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(
            videos.pool.starts, 5,
            "all five pipes survive the uncached next-frame round"
        );
    }
}

#[test]
fn six_unprotected_video_lanes_reap_evicted_pipes_and_keep_five_at_most() {
    let mut videos = Videos::new(6);
    let mut children = Vec::new();

    for frame_index in 0..2 {
        for asset_index in 0..6 {
            let job = videos.job(asset_index, frame_index);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("current unprotected overflow frame");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
            assert!(videos.pool.decoders.len() <= 5);
            let child = videos
                .pool
                .decoders
                .iter()
                .find(|decoder| decoder.lane == job.lane)
                .expect("requested overflow decoder is retained");
            children.push(Arc::clone(&child.process.child));
        }
        assert_eq!(videos.pool.decoders.len(), 5);
        assert_eq!(
            videos.pool.starts,
            (frame_index + 1) * 6,
            "cyclic overflow restarts eligible lanes without exceeding the bank"
        );
    }
    for child in &children[..6] {
        assert_hint_child_exited(child);
    }
    videos.pool.clear();
    assert!(videos.pool.decoders.is_empty());
    for child in &children {
        assert_hint_child_exited(child);
    }
}

#[test]
fn four_distinct_video_lanes_keep_their_pipes_for_uncached_next_frame() {
    let mut videos = Videos::new(4);

    for frame_index in 0..2 {
        for asset_index in 0..4 {
            let job = videos.job(asset_index, frame_index);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("current owned video frame");
            assert_eq!([frame.width, frame.height], SIZE);
            assert_eq!(
                frame.rgba.as_ref(),
                videos.expected[asset_index][frame_index].as_slice(),
                "full source pixels for asset {asset_index}, frame {frame_index}"
            );
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(
            videos.pool.starts, 4,
            "all four pipes survive the uncached next-frame round"
        );
    }
}

#[test]
fn five_video_lanes_repeating_cached_frames_do_not_start_more_decoders() {
    let mut videos = Videos::new(5);
    let mut retained_frames = Vec::with_capacity(5);

    for asset_index in 0..5 {
        let job = videos.job(asset_index, 1);
        let frame = videos
            .pool
            .frame(&job, || true)
            .expect("owned cache-warmup frame");
        assert_eq!([frame.width, frame.height], SIZE);
        assert_eq!(
            frame.rgba.as_ref(),
            videos.expected[asset_index][1].as_slice(),
            "full source pixels for warmup asset {asset_index}"
        );
        assert!(videos.pool.decoders.len() <= 5);
        retained_frames.push(frame);
    }
    assert_eq!(videos.pool.starts, 5);

    for _ in 0..3 {
        for (asset_index, retained) in retained_frames.iter().enumerate() {
            let job = videos.job(asset_index, 1);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("exact owned cached frame");
            assert_eq!([frame.width, frame.height], SIZE);
            assert_eq!(
                frame.rgba.as_ref(),
                videos.expected[asset_index][1].as_slice(),
                "full source pixels for cached asset {asset_index}"
            );
            assert!(Arc::ptr_eq(&retained.rgba, &frame.rgba));
            assert!(videos.pool.decoders.len() <= 5);
            assert_eq!(
                videos.pool.starts, 5,
                "exact cache hits do not touch pipe retention"
            );
        }
    }
}

#[test]
fn one_explicit_consumer_reuses_a_continuous_source_then_reclaims_its_pipe() {
    let mut videos = Videos::new(1);
    let consumer = PreviewConsumer::new();
    let first_job = FrameJob {
        lane: Lane::Consumer(consumer),
        ..videos.job(0, 0)
    };
    let first_frame = videos
        .pool
        .frame(&first_job, || true)
        .expect("first current explicit-consumer frame");
    assert_eq!([first_frame.width, first_frame.height], SIZE);
    assert_eq!(first_frame.rgba.as_ref(), videos.expected[0][0].as_slice());
    assert_eq!(videos.pool.starts, 1);
    assert!(videos.pool.decoders.len() <= 5);
    let child = Arc::clone(
        &videos
            .pool
            .decoders
            .front()
            .expect("retained explicit-consumer decoder")
            .process
            .child,
    );

    // A fresh job description retains the same source and consumer binding.
    // This is decoder continuity, not a simulated UI clip/cut handoff.
    let next_job = FrameJob {
        lane: Lane::Consumer(consumer),
        ..videos.job(0, 1)
    };
    let next_frame = videos
        .pool
        .frame(&next_job, || true)
        .expect("next current explicit-consumer frame");
    assert_eq!([next_frame.width, next_frame.height], SIZE);
    assert_eq!(next_frame.rgba.as_ref(), videos.expected[0][1].as_slice());
    assert_eq!(videos.pool.starts, 1, "continuous source reuses its pipe");
    assert!(videos.pool.decoders.len() <= 5);
    assert!(Arc::ptr_eq(
        &child,
        &videos
            .pool
            .decoders
            .front()
            .expect("same retained explicit-consumer decoder")
            .process
            .child
    ));

    videos.pool.retain_lanes(&[]);
    assert!(videos.pool.decoders.is_empty());
    let status = child
        .lock()
        .expect("exact owned decoder child")
        .try_wait()
        .expect("observe owned decoder exit after reclamation");
    assert!(status.is_some(), "reclaimed decoder child has been reaped");
}

#[test]
fn large_cold_video_frames_keep_their_pipe_without_speculative_reads() {
    const LARGE_SIZE: [u32; 2] = [768, 384];
    let mut videos = Videos::sized(1, LARGE_SIZE);
    let consumer = PreviewConsumer::new();
    let first_job = FrameJob {
        lane: Lane::Consumer(consumer),
        ..videos.job(0, 0)
    };
    let first = videos
        .pool
        .frame(&first_job, || true)
        .expect("actual large lossless source frame");
    assert_hint_pixels(&first, LARGE_SIZE, &videos.expected[0][0]);
    assert!(first.byte_len() > 1024 * 1024);
    assert!(
        !videos.pool.has_read_ahead(&first_job),
        "large requested frames do not offer destructive speculative reads"
    );
    let child = hint_decoder_child(&videos.pool, consumer);
    videos
        .pool
        .read_ahead(&first_job, Instant::now() + Duration::from_secs(2), || true);
    assert_eq!(videos.pool.cache.len(), 1);
    assert_eq!(videos.pool.decoders.len(), 1);
    assert_eq!(videos.pool.decoders[0].next_index, 1);

    for frame_index in 1..SOURCE_FRAMES {
        let next_job = FrameJob {
            lane: Lane::Consumer(consumer),
            ..videos.job(0, frame_index)
        };
        let next = videos
            .pool
            .frame(&next_job, || true)
            .expect("uncached large source frame");
        assert_hint_pixels(&next, LARGE_SIZE, &videos.expected[0][frame_index]);
        assert!(!videos.pool.has_read_ahead(&next_job));
        assert_eq!(videos.pool.starts, 1);
        assert!(Arc::ptr_eq(
            &child,
            &hint_decoder_child(&videos.pool, consumer)
        ));
        assert!(videos.pool.bytes <= crate::FRAME_CACHE_BYTES);
        assert!(videos.pool.cache.len() <= 256);
    }
    videos.pool.clear();
    assert!(videos.pool.decoders.is_empty());
    assert_hint_child_exited(&child);
}

fn hinted_job(
    videos: &Videos,
    asset_index: usize,
    frame_index: usize,
    consumer: PreviewConsumer,
) -> FrameJob {
    FrameJob {
        lane: Lane::Consumer(consumer),
        ..videos.job(asset_index, frame_index)
    }
}

fn assert_hint_pixels(frame: &crate::Frame, size: [u32; 2], expected: &[u8]) {
    assert_eq!([frame.width, frame.height], size);
    assert_eq!(frame.rgba.as_ref(), expected, "complete owned RGBA frame");
}

fn hint_decoder_child(
    pool: &DecoderPool,
    consumer: PreviewConsumer,
) -> Arc<std::sync::Mutex<Child>> {
    Arc::clone(
        &pool
            .decoders
            .iter()
            .find(|decoder| decoder.lane == Lane::Consumer(consumer))
            .expect("exact retained consumer decoder")
            .process
            .child,
    )
}

fn assert_hint_child_exited(child: &Arc<std::sync::Mutex<Child>>) {
    let status = child
        .lock()
        .expect("exact owned decoder child")
        .try_wait()
        .expect("observe exact owned decoder status");
    assert!(status.is_some(), "removed owned decoder has been reaped");
}

#[test]
fn hinted_five_consumers_keep_all_five_pipes_over_two_uncached_rounds() {
    let mut videos = Videos::new(5);
    let consumers: [PreviewConsumer; 5] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);

    for frame_index in 0..2 {
        for (asset_index, consumer) in consumers.iter().copied().enumerate() {
            let job = hinted_job(&videos, asset_index, frame_index, consumer);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("current hinted frame");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(
            videos.pool.starts, 5,
            "all five consumers survive the uncached next-frame round"
        );
    }
}

#[test]
fn hinted_eight_consumers_start_thirteen_pipes_over_two_uncached_rounds() {
    let mut videos = Videos::new(8);
    let consumers: [PreviewConsumer; 8] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);

    for frame_index in 0..2 {
        for (asset_index, consumer) in consumers.iter().copied().enumerate() {
            let job = hinted_job(&videos, asset_index, frame_index, consumer);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("current hinted frame");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(
            videos.pool.starts,
            [8, 13][frame_index],
            "larger cohorts still reuse their declared protected subset"
        );
    }
}

#[test]
fn replacing_or_clearing_hints_rewarms_a_disjoint_five_consumer_cohort() {
    for replace_hint in [true, false] {
        let mut videos = Videos::new(10);
        let consumers: [PreviewConsumer; 10] = std::array::from_fn(|_| PreviewConsumer::new());
        videos.pool.set_preview_retention([
            Some(consumers[0]),
            Some(consumers[1]),
            Some(consumers[2]),
        ]);
        for (asset_index, consumer) in consumers[..5].iter().copied().enumerate() {
            let job = hinted_job(&videos, asset_index, 0, consumer);
            let frame = videos.pool.frame(&job, || true).expect("old cohort frame");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][0]);
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(videos.pool.starts, 5);
        let old_children: Vec<_> = videos
            .pool
            .decoders
            .iter()
            .map(|decoder| Arc::clone(&decoder.process.child))
            .collect();

        videos.pool.set_preview_retention(if replace_hint {
            [Some(consumers[5]), Some(consumers[6]), Some(consumers[7])]
        } else {
            [None; 3]
        });
        for frame_index in 0..2 {
            for (offset, consumer) in consumers[5..].iter().copied().enumerate() {
                let asset_index = offset + 5;
                let job = hinted_job(&videos, asset_index, frame_index, consumer);
                let frame = videos.pool.frame(&job, || true).expect("new cohort frame");
                assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
                assert!(videos.pool.decoders.len() <= 5);
            }
            assert_eq!(
                videos.pool.starts, 10,
                "five new initial starts, no next-frame restart after hint change"
            );
        }
        for child in &old_children {
            assert_hint_child_exited(child);
        }
    }
}

#[test]
fn hinted_four_consumers_reuse_their_pipes_after_dispatch_order_reverses() {
    let mut videos = Videos::new(4);
    let consumers: [PreviewConsumer; 4] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);
    for (asset_index, consumer) in consumers.iter().copied().enumerate() {
        let job = hinted_job(&videos, asset_index, 0, consumer);
        let frame = videos
            .pool
            .frame(&job, || true)
            .expect("first cohort frame");
        assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][0]);
        assert!(videos.pool.decoders.len() <= 5);
    }
    assert_eq!(videos.pool.starts, 4);
    let original_children: Vec<_> = consumers
        .iter()
        .map(|consumer| hint_decoder_child(&videos.pool, *consumer))
        .collect();

    for asset_index in (0..4).rev() {
        let job = hinted_job(&videos, asset_index, 1, consumers[asset_index]);
        let frame = videos
            .pool
            .frame(&job, || true)
            .expect("reordered next frame");
        assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][1]);
        assert!(videos.pool.decoders.len() <= 5);
        assert!(Arc::ptr_eq(
            &original_children[asset_index],
            &hint_decoder_child(&videos.pool, consumers[asset_index])
        ));
        assert_eq!(videos.pool.starts, 4);
    }
    videos.pool.retain_lanes(&[]);
    assert!(videos.pool.decoders.is_empty());
    for child in &original_children {
        assert_hint_child_exited(child);
    }
}

#[test]
fn unknown_retention_hints_reserve_no_pipe_capacity() {
    let mut videos = Videos::new(4);
    videos.pool.set_preview_retention([
        Some(PreviewConsumer::new()),
        Some(PreviewConsumer::new()),
        Some(PreviewConsumer::new()),
    ]);
    assert!(videos.pool.decoders.is_empty());
    assert_eq!(videos.pool.starts, 0);
    let consumers: [PreviewConsumer; 4] = std::array::from_fn(|_| PreviewConsumer::new());

    for frame_index in 0..2 {
        for (asset_index, consumer) in consumers.iter().copied().enumerate() {
            let job = hinted_job(&videos, asset_index, frame_index, consumer);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("unprotected current frame");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
            assert!(videos.pool.decoders.len() <= 5);
        }
        assert_eq!(videos.pool.starts, 4, "absent hints do not withhold a slot");
    }
}

#[test]
fn a_hint_does_not_bypass_source_replacement_or_backward_seek() {
    let mut videos = Videos::new(2);
    let consumer = PreviewConsumer::new();
    videos
        .pool
        .set_preview_retention([Some(consumer), None, None]);
    let first_job = hinted_job(&videos, 0, 0, consumer);
    let first = videos
        .pool
        .frame(&first_job, || true)
        .expect("original source frame");
    assert_hint_pixels(&first, SIZE, &videos.expected[0][0]);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(videos.pool.starts, 1);
    let original_child = hint_decoder_child(&videos.pool, consumer);

    // Preserve the asset UUID while replacing its complete source description.
    let mut replaced_job = hinted_job(&videos, 0, 1, consumer);
    replaced_job.asset.path = videos.assets[1].path.clone();
    replaced_job.asset.file_size = videos.assets[1].file_size;
    let replaced = videos
        .pool
        .frame(&replaced_job, || true)
        .expect("replaced source frame");
    assert_hint_pixels(&replaced, SIZE, &videos.expected[1][1]);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(
        videos.pool.starts, 2,
        "a pin cannot preserve the wrong source"
    );
    assert_hint_child_exited(&original_child);
    let replaced_child = hint_decoder_child(&videos.pool, consumer);

    let mut continued_job = replaced_job.clone();
    continued_job.sequence += 1;
    continued_job.time = FrameRate::default().frame_time(2);
    let continued = videos
        .pool
        .frame(&continued_job, || true)
        .expect("replacement next frame");
    assert_hint_pixels(&continued, SIZE, &videos.expected[1][2]);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(videos.pool.starts, 2);
    assert!(Arc::ptr_eq(
        &replaced_child,
        &hint_decoder_child(&videos.pool, consumer)
    ));

    let mut backward_job = replaced_job;
    backward_job.sequence += 2;
    backward_job.time = FrameRate::default().frame_time(0);
    let backward = videos
        .pool
        .frame(&backward_job, || true)
        .expect("uncached backward source frame");
    assert_hint_pixels(&backward, SIZE, &videos.expected[1][0]);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(videos.pool.starts, 3, "pinned backward seek still restarts");
    assert_hint_child_exited(&replaced_child);
    let final_child = hint_decoder_child(&videos.pool, consumer);
    videos.pool.retain_lanes(&[]);
    assert!(videos.pool.decoders.is_empty());
    assert_hint_child_exited(&final_child);
}

#[test]
fn a_hint_does_not_bypass_changed_fitted_dimensions() {
    let mut videos = Videos::new(1);
    let consumer = PreviewConsumer::new();
    videos
        .pool
        .set_preview_retention([Some(consumer), None, None]);
    let first_job = hinted_job(&videos, 0, 0, consumer);
    let first = videos
        .pool
        .frame(&first_job, || true)
        .expect("original full-size frame");
    assert_hint_pixels(&first, SIZE, &videos.expected[0][0]);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(videos.pool.starts, 1);
    let original_child = hint_decoder_child(&videos.pool, consumer);

    // Independent native reference avoids guessing fast_bilinear rounding.
    // The fixture's OwnedEncoder guard also reaps this one-shot reference tool.
    let reference_path = videos._directory.path().join("resized-reference.rgba");
    let mut command = tool_command("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "2",
            "-ss",
            "0.033333333",
            "-i",
        ])
        .arg(&videos.assets[0].path)
        .args([
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            "scale=32:16:flags=fast_bilinear,fps=30/1",
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
        ])
        .arg(&reference_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut reference = OwnedEncoder(command.spawn().expect("start owned resized reference"));
    let status = reference
        .0
        .wait()
        .expect("wait for owned resized reference");
    assert!(
        status.success(),
        "native resized reference failed: {status}"
    );
    drop(reference);
    let expected = fs::read(reference_path).expect("complete owned resized RGBA reference");
    assert_eq!(expected.len(), 32 * 16 * 4);

    let mut resized_job = hinted_job(&videos, 0, 1, consumer);
    resized_job.size = [32, 16];
    let resized = videos
        .pool
        .frame(&resized_job, || true)
        .expect("new fitted dimensions");
    assert_hint_pixels(&resized, [32, 16], &expected);
    assert!(videos.pool.decoders.len() <= 5);
    assert_eq!(
        videos.pool.starts, 2,
        "a pin cannot preserve incompatible dimensions"
    );
    assert_hint_child_exited(&original_child);
    let resized_child = hint_decoder_child(&videos.pool, consumer);
    videos.pool.retain_lanes(&[]);
    assert!(videos.pool.decoders.is_empty());
    assert_hint_child_exited(&resized_child);
}

/// A transition on each of three stacked video tracks: for as long as it runs
/// every track shows two clips, the one that is over and the one it leads
/// into, each a consumer with a decoder of its own. That is six, one more
/// than the bank holds. The viewer hints the first three it paints.
///
/// Nothing waits and no consumer is given another's frame: the three that are
/// not protected take turns at the two pipes left, each starting FFmpeg again
/// for its next frame. When the transitions are over the three clips left
/// decode on without a start.
#[test]
fn a_transition_on_three_stacked_videos_overflows_the_bank_and_settles_after_it() {
    let mut videos = Videos::new(6);
    // In paint order: out, in, out, in, out, in.
    let consumers: [PreviewConsumer; 6] = std::array::from_fn(|_| PreviewConsumer::new());
    videos
        .pool
        .set_preview_retention([Some(consumers[0]), Some(consumers[1]), Some(consumers[2])]);

    let mut starts = Vec::new();
    for frame_index in 0..2 {
        for (asset_index, consumer) in consumers.iter().copied().enumerate() {
            let job = hinted_job(&videos, asset_index, frame_index, consumer);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("a frame for each of the six, however many pipes there are");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
            assert!(videos.pool.decoders.len() <= 5);
        }
        starts.push(videos.pool.starts);
    }

    // The transitions end: the outgoing clips are retired, and the three
    // incoming ones are the three the viewer hints.
    let incoming = [consumers[1], consumers[3], consumers[5]];
    videos.pool.retain_lanes(&incoming.map(Lane::Consumer));
    videos.pool.set_preview_retention(incoming.map(Some));
    for frame_index in 2..SOURCE_FRAMES {
        for (asset_index, consumer) in [(1, incoming[0]), (3, incoming[1]), (5, incoming[2])] {
            let job = hinted_job(&videos, asset_index, frame_index, consumer);
            let frame = videos
                .pool
                .frame(&job, || true)
                .expect("the clips left decode on");
            assert_hint_pixels(&frame, SIZE, &videos.expected[asset_index][frame_index]);
        }
        assert!(videos.pool.decoders.len() <= 3);
        starts.push(videos.pool.starts);
    }
    println!("MEASURE: FFmpeg starts after each round of a six-consumer transition: {starts:?}");
    assert_eq!(
        starts[1] - starts[0],
        3,
        "each unprotected consumer starts again for its next frame; the three \
         protected ones keep their pipes"
    );
    assert_eq!(
        starts[3], starts[2],
        "once only three are left, a next frame starts nothing"
    );
}
