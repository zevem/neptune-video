//! Snapshot-only mix analysis, using the export's own mixer and limiter.
use super::{analysis::LoudnessParams, stamp, track};
use crate::mcp::{
    errors::{ToolError, codes},
    jobs::{JobInfo, Progress},
    server::NeptuneVideoServer,
};
use editor_core::{
    Project, Time, TrackKind,
    export::{SampleSpan, sample_count},
    sound::{self, Sounding},
};
use editor_media::{Loudness, export::AudioRender};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

fn selected(project: &Arc<Project>, ids: &[Uuid]) -> Arc<Project> {
    if ids.is_empty() {
        return project.clone();
    }
    let mut selected = (**project).clone();
    // Keep all clips for detached-audio ownership and keep all solo flags so
    // selecting a track cannot make an originally soloed-out voice audible.
    for track in &mut selected.tracks {
        if !ids.contains(&track.id) {
            Arc::make_mut(track).muted = true;
        }
    }
    Arc::new(selected)
}

/// Match the mixer's sample admission without reading picture-only media,
/// muted/owned/soloed-out clips, fonts or sources outside this sample span.
fn check_mix_readable(
    project: &Project,
    span: SampleSpan,
    roots: &crate::mcp::paths::Roots,
) -> Result<(), ToolError> {
    let sounding = Sounding::of(project);
    let mut used = std::collections::BTreeSet::new();
    let sample = |time: Time| -> u128 {
        let micros = (time - span.start).0.max(0) as u128;
        (micros * u128::from(span.rate))
            .div_ceil(1_000_000)
            .min(u128::from(span.frames))
    };
    for track in &project.tracks {
        for clip in &track.clips {
            if !sounding.sounds(project, track, clip) {
                continue;
            }
            let mut segment = 0;
            while let Some((from, to, after)) = sound::audible_run(clip, segment) {
                segment = after;
                if sample(from) < sample(to) {
                    used.extend(clip.asset_id());
                    break;
                }
            }
        }
    }
    let outside: Vec<_> = used
        .into_iter()
        .filter_map(|id| project.assets.get(&id))
        .filter(|asset| !roots.readable(&asset.path))
        .map(|asset| asset.path.display().to_string())
        .collect();
    if outside.is_empty() {
        return Ok(());
    }
    Err(ToolError::new(codes::OUTSIDE_ROOTS, format!(
        "The measured mix uses {} source file(s) outside this server's folders or missing; add their folder as a root",
        outside.len()
    )).with_detail(serde_json::json!({"paths": outside})))
}

fn figures(measured: Loudness) -> serde_json::Value {
    match measured {
        Loudness::Measured {
            integrated,
            true_peak,
        } => serde_json::json!({
            "status": "measured", "integrated_lufs": integrated, "true_peak_dbfs": true_peak,
        }),
        Loudness::Silent => {
            serde_json::json!({"status": "silent", "integrated_lufs": null, "true_peak_dbfs": null})
        }
        Loudness::TooShort => {
            serde_json::json!({"status": "too_short_or_quiet", "integrated_lufs": null, "true_peak_dbfs": null})
        }
    }
}

fn measure(
    project: Arc<Project>,
    span: SampleSpan,
    progress: &Progress,
    index: usize,
    total: usize,
) -> Result<serde_json::Value, ToolError> {
    if progress.cancelled() {
        return Err(ToolError::new("cancelled", "Cancelled"));
    }
    let render = AudioRender::new();
    let cancel = render.canceller();
    let done = AtomicBool::new(false);
    struct Finish<'a>(&'a AtomicBool);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let outcome = std::thread::scope(|scope| {
        let _finish = Finish(&done);
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if progress.cancelled() {
                    cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        render.measure_loudness(project, span, &|fraction| {
            progress.set((index as f32 + fraction) / total as f32, "Measuring mix")
        })
    })
    .map_err(|error| ToolError::new(codes::JOB_FAILED, error.to_string()))?;
    if progress.cancelled() {
        return Err(ToolError::new("cancelled", "Cancelled"));
    }
    let (measurement, notices) = outcome;
    let mut result = figures(measurement);
    result["notices"] = serde_json::json!(notices);
    Ok(result)
}

pub(super) async fn start(
    server: &NeptuneVideoServer,
    params: LoudnessParams,
) -> Result<JobInfo, ToolError> {
    if !params.clip_ids.is_empty() {
        return Err(ToolError::invalid(
            "Mix scope takes track_ids, not clip_ids",
        ));
    }
    let snapshot = server.on(|workspace| workspace.snapshot()).await?;
    let project = snapshot.project;
    for id in &params.track_ids {
        track(&project, *id)?;
    }
    let rate = project.settings.frame_rate;
    let from = params
        .from
        .as_ref()
        .map(|at| at.exact(rate))
        .transpose()?
        .unwrap_or(Time::ZERO);
    let to = params
        .to
        .as_ref()
        .map(|at| at.exact(rate))
        .transpose()?
        .unwrap_or_else(|| project.duration());
    if from >= to || to > project.duration() {
        return Err(ToolError::invalid(
            "The measurement range must be nonempty and inside the timeline",
        ));
    }
    let measured = selected(&project, &params.track_ids);
    let tracks: Vec<Uuid> = measured
        .tracks
        .iter()
        .filter(|t| {
            t.kind != TrackKind::Text
                && (params.track_ids.is_empty() || params.track_ids.contains(&t.id))
        })
        .map(|t| t.id)
        .collect();
    if params.per_track && tracks.len() > 64 {
        return Err(ToolError::invalid(
            "Select at most 64 track_ids for per_track statistics",
        ));
    }
    let sample_rate = project.settings.audio_sample_rate;
    let frames = sample_count(from, to, sample_rate).max(1);
    let span = SampleSpan {
        start: from,
        frames,
        rate: sample_rate,
        channels: 2,
    };
    check_mix_readable(&measured, span, server.roots())?;
    server.shared.jobs.start("loudness", "Measuring mix", move |progress| {
        let total = 1 + if params.per_track { tracks.len() } else { 0 };
        let mut result = measure(measured.clone(), span, progress, 0, total)?;
        result["scope"] = "timeline".into();
        result["project_id"] = serde_json::json!(project.id);
        result["context_id"] = serde_json::json!(snapshot.context_id);
        result["revision"] = snapshot.revision.into();
        result["from"] = serde_json::json!(stamp(from, &project));
        result["to"] = serde_json::json!(stamp(to, &project));
        result["sample_rate"] = sample_rate.into();
        result["channels"] = 2.into();
        result["stage"] = "post_master_limiter".into();
        if params.per_track {
            result["track_measurement"] = "Each selected track alone, retaining mute/solo and detached-audio ownership, through its own master limiter. LUFS and peak values are not additive.".into();
            let mut stats = Vec::new();
            for (index, id) in tracks.iter().enumerate() {
                let mut item = measure(selected(&measured, &[*id]), span, progress, index + 1, total)?;
                item["track_id"] = serde_json::json!(id);
                stats.push(item);
            }
            result["tracks"] = stats.into();
        }
        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::paths::Roots;
    use editor_core::{Asset, Clip, MediaKind};

    fn span() -> SampleSpan {
        SampleSpan {
            start: Time::ZERO,
            frames: 48_000,
            rate: 48_000,
            channels: 2,
        }
    }

    #[test]
    fn mix_checks_only_audible_source_files_inside_the_requested_range() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let roots = Roots::new(vec![allowed.path().to_path_buf()], allowed.path(), None).unwrap();
        let path = outside.path().join("outside.wav");
        std::fs::write(&path, b"owned root-check fixture").unwrap();
        let asset = Asset::new(path, MediaKind::Audio, Time(3_000_000));
        let mut clip = Clip::from_asset(&asset, Time(2_000_000));
        clip.duration = Time(1_000_000);
        let mut project = Project::new("Mix root checks");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        Arc::make_mut(&mut project.tracks[1]).clips.push(clip);
        // Fonts and picture-only media have no role in PCM analysis.
        let mut image = Asset::new(
            outside.path().join("missing.png"),
            MediaKind::Image,
            Time(1_000_000),
        );
        image.has_audio = false;
        Arc::make_mut(&mut project.assets).insert(image.id, image.clone());
        Arc::make_mut(&mut project.tracks[0])
            .clips
            .push(Clip::from_asset(&image, Time::ZERO));
        let font = Asset::new(
            outside.path().join("missing.ttf"),
            MediaKind::Font,
            Time(1_000_000),
        );
        Arc::make_mut(&mut project.assets).insert(font.id, font);
        check_mix_readable(&project, span(), &roots).unwrap();

        let track = Arc::make_mut(&mut project.tracks[1]);
        track.clips[0].start = Time::ZERO;
        track.muted = true;
        check_mix_readable(&project, span(), &roots).unwrap();
        Arc::make_mut(&mut project.tracks[1]).muted = false;
        let error = check_mix_readable(&project, span(), &roots).unwrap_err();
        assert_eq!(error.code, codes::OUTSIDE_ROOTS);
        assert_eq!(
            error.detail["paths"],
            serde_json::json!([asset.path.display().to_string()])
        );
    }

    #[test]
    fn selecting_tracks_retains_detached_audio_ownership_and_existing_solo() {
        let mut project = Project::new("Track selection");
        let mut asset = Asset::new("outside.mp4".into(), MediaKind::Video, Time(1_000_000));
        asset.has_audio = true;
        let mut video = Clip::from_asset(&asset, Time::ZERO);
        video.linked_id = Some(Uuid::new_v4());
        let mut audio = video.clone();
        audio.id = Uuid::new_v4();
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        Arc::make_mut(&mut project.tracks[0]).clips.push(video);
        Arc::make_mut(&mut project.tracks[1]).clips.push(audio);
        let original = Arc::new(project);
        let isolated = selected(&original, &[original.tracks[0].id]);
        let sounding = Sounding::of(&isolated);
        assert!(!sounding.sounds(&isolated, &isolated.tracks[0], &isolated.tracks[0].clips[0]));
        assert!(isolated.tracks[1].muted);
        assert!(!original.tracks[1].muted);

        let mut soloed = (*original).clone();
        Arc::make_mut(&mut soloed.tracks[0]).clips[0].linked_id = None;
        Arc::make_mut(&mut soloed.tracks[1]).solo = true;
        let isolated = selected(&Arc::new(soloed), &[original.tracks[0].id]);
        assert!(isolated.tracks[1].solo);
        assert!(!Sounding::of(&isolated).sounds(
            &isolated,
            &isolated.tracks[0],
            &isolated.tracks[0].clips[0]
        ));
    }
}
