use editor_core::*;
use std::{sync::Arc, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut project = Project::new("Timeline benchmark");
    project.tracks.clear();
    for i in 0..20 {
        let mut track = Track::new(format!("Text {i}"), TrackKind::Text);
        for j in 0..250 {
            track.clips.push(Clip::text(
                "Title",
                Time::from_seconds(j as f64 * 2.0),
                Time::from_seconds(1.0),
            ));
        }
        project.tracks.push(Arc::new(track));
    }
    let clip_id = project.tracks[0].clips[0].id;
    let mut editor = Editor::new(project)?;
    let started = Instant::now();
    for _ in 0..1000 {
        editor.dispatch(Command::MoveClips {
            clip_ids: vec![clip_id],
            delta: Time(1),
            target_track: None,
        })?;
        editor.undo();
    }
    let edit_us = started.elapsed().as_secs_f64() * 1_000_000.0 / 1000.0;
    let started = Instant::now();
    let mut active = 0;
    for i in 0..100_000 {
        active += std::hint::black_box(editor.project())
            .active_clips(Time::from_seconds((i % 5000) as f64 * 0.1))
            .count();
    }
    let query_us = started.elapsed().as_secs_f64() * 1_000_000.0 / 100_000.0;
    println!(
        "{{\"clips\":5000,\"tracks\":20,\"move_and_undo_us\":{edit_us:.3},\"active_query_us\":{query_us:.3},\"queried_layers\":{active}}}"
    );
    Ok(())
}
