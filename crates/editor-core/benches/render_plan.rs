//! Metadata-only playback measurement. No decoding, font rasterization or GPU
//! work is included. Every measured plan has five full-quality media layers;
//! only its expired history grows. Keep timings outside correctness tests.
use editor_core::{Asset, Clip, MediaKind, Project, ProjectQuery, Time, Track, TrackKind, resolve};
use std::{hint::black_box, sync::Arc, time::Instant};

const SAMPLES: usize = 1_000;
const CURRENT_LAYERS: usize = 5;

fn fixture(tracks: usize, clips_per_track: usize) -> Project {
    let mut project = Project::new("Expired-prefix render-plan benchmark");
    project.tracks.clear();
    let asset = Asset::new(
        "synthetic-video.mp4".into(),
        MediaKind::Video,
        Time::from_seconds(2.0),
    );
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let current = Time::from_seconds((clips_per_track * 2 + 2) as f64);
    for index in 0..tracks {
        let mut track = Track::new(format!("Video {index}"), TrackKind::Video);
        let history = clips_per_track.saturating_sub(usize::from(index < CURRENT_LAYERS));
        track.clips.reserve(clips_per_track);
        for clip_index in 0..history {
            let mut clip = Clip::from_asset(&asset, Time::from_seconds((clip_index * 2) as f64));
            clip.duration = Time::from_seconds(1.0);
            track.clips.push(clip);
        }
        if index < CURRENT_LAYERS {
            track.clips.push(Clip::from_asset(&asset, current));
        }
        project.tracks.push(Arc::new(track));
    }
    project
}

fn distribution(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let percentile = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
    serde_json::json!({
        "samples": samples.len(),
        "p50_us": percentile(50),
        "p95_us": percentile(95),
        "p99_us": percentile(99),
        "max_us": samples[samples.len() - 1],
        "sorted_us": samples,
    })
}

fn measure(
    tracks: usize,
    clips_per_track: usize,
) -> Result<serde_json::Value, editor_core::EditError> {
    let project = Arc::new(fixture(tracks, clips_per_track));
    project.validate()?;
    let current = Time::from_seconds((clips_per_track * 2 + 2) as f64);
    let rate = project.settings.frame_rate;
    let first_frame = rate.frame_at(current);
    let times: Vec<Time> = (0..SAMPLES)
        .map(|sample| rate.frame_time(first_frame + (sample % 30) as i64))
        .collect();
    let mut compact = (*project).clone();
    for track in &mut compact.tracks {
        Arc::make_mut(track)
            .clips
            .retain(|clip| clip.end() > current);
    }
    compact.validate()?;
    // Compare complete plans (source positions, transforms, effects and order),
    // outside timing. Deleted history must not change any visible picture.
    for at in times.iter().take(30) {
        let full = resolve(&project, *at, &[]);
        assert_eq!(full, resolve(&compact, *at, &[]));
        assert_eq!(full.layers.len(), CURRENT_LAYERS);
    }
    let query = ProjectQuery::new(Arc::clone(&project));
    let mut resolve_us = Vec::with_capacity(SAMPLES);
    let mut compact_us = Vec::with_capacity(SAMPLES);
    let mut query_us = Vec::with_capacity(SAMPLES);
    let mut indexed_resolve_us = Vec::with_capacity(SAMPLES);
    for at in times.iter().take(30) {
        assert_eq!(query.resolve(*at, &[]), resolve(&project, *at, &[]));
    }
    for at in &times {
        let started = Instant::now();
        black_box(resolve(black_box(&project), black_box(*at), &[]));
        resolve_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let started = Instant::now();
        black_box(resolve(black_box(&compact), black_box(*at), &[]));
        compact_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let started = Instant::now();
        assert_eq!(
            black_box(query.active_clips(black_box(*at)).count()),
            CURRENT_LAYERS
        );
        query_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let started = Instant::now();
        black_box(query.resolve(black_box(*at), &[]));
        indexed_resolve_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    Ok(serde_json::json!({
        "tracks": tracks,
        "clips": project.tracks.iter().map(|track| track.clips.len()).sum::<usize>(),
        "active_media_layers": CURRENT_LAYERS,
        "first_time_us": current.0,
        "full_plan_equal_to_compact": true,
        "direct_resolve": distribution(resolve_us),
        "compact_direct_resolve": distribution(compact_us),
        "existing_active_query": distribution(query_us),
        "indexed_resolve": distribution(indexed_resolve_us),
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for (tracks, clips_per_track) in [(20, 250), (1_000, 100)] {
        println!("{}", measure(tracks, clips_per_track)?);
    }
    Ok(())
}
