//! Opt-in real provider acceptance using synthetic in-memory editing metadata.
//! No media, credentials, project files or provider tools are accessed here.
use editor_agent::{AiConfig, AiContext, AiEvent, AiProvider, AiRuntime, proposal_schema};
use editor_core::{Clip, Editor, Project, Time, TrackKind};
use serde_json::json;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = match std::env::args().nth(1).as_deref() {
        Some("schema") => {
            println!("{}", serde_json::to_string_pretty(&proposal_schema())?);
            return Ok(());
        }
        Some("claude") => AiProvider::Claude,
        Some("codex") => AiProvider::Codex,
        _ => {
            return Err(
                "usage: cargo run -p editor-agent --example ai-smoke -- claude|codex|schema".into(),
            );
        }
    };
    let split = match std::env::args().nth(2).as_deref() {
        None => false,
        Some("split") => true,
        _ => return Err("optional acceptance mode: split".into()),
    };
    let mut project = Project::new("Synthetic AI acceptance");
    let title = Clip::text("Synthetic title", Time(2_000_000), Time(3_000_000));
    let selected = title.id;
    Arc::make_mut(
        project
            .tracks
            .iter_mut()
            .find(|track| track.kind == TrackKind::Text)
            .unwrap(),
    )
    .clips
    .push(title);
    let mut editor = Editor::new(project)?;
    let before = editor.snapshot();
    let runtime = AiRuntime::new(|| {})?;
    let config = AiConfig {
        provider,
        executable: std::env::var_os("NEPTUNE_VIDEO_AI_EXECUTABLE").map(Into::into),
        model: std::env::var("NEPTUNE_VIDEO_AI_MODEL").ok(),
        timeout_seconds: 180,
        ..AiConfig::default()
    };
    let started = Instant::now();
    let prompt = if split {
        "Use exactly one SplitClip command to split the selected title at timeline time 3.5 seconds (3500000 microseconds). Preserve its title, transform and all other project settings. Do not add, move, rename or group anything. Propose this edit now."
    } else {
        "Rename this project to 'Native AI verified', move the selected title exactly one second later without changing its duration, and set a 1080 by 1920 portrait canvas. Preserve frame rate, background and audio settings. Propose these edits now."
    };
    let id = runtime.request(
        config,
        AiContext::capture(&editor, &[selected], Time(2_000_000)),
        prompt.into(),
    )?;
    let deadline = started + Duration::from_secs(190);
    loop {
        if let Some(event) = runtime.poll() {
            match event {
                AiEvent::Started { .. } => {}
                AiEvent::Failed { message, .. } => return Err(message.into()),
                AiEvent::Cancelled { .. } => return Err("provider was cancelled".into()),
                AiEvent::Proposal {
                    id: delivered,
                    proposal,
                } => {
                    assert_eq!(delivered, id);
                    assert!(Arc::ptr_eq(&editor.snapshot(), &before));
                    if std::env::var_os("NEPTUNE_VIDEO_AI_TRACE_PROPOSAL").is_some() {
                        // This opt-in harness creates only synthetic metadata.
                        // Product runtime diagnostics never expose proposals.
                        eprintln!(
                            "{}",
                            serde_json::to_string(&json!({
                                "kind":"synthetic_acceptance_proposal",
                                "summary":proposal.summary,"commands":proposal.commands
                            }))?
                        );
                    }
                    proposal.apply(&mut editor)?;
                    // Includes generated clip/relationship IDs, not just pixels
                    // or timing: review and commit must be the identical state.
                    assert_eq!(editor.project(), proposal.after.as_ref());
                    if split {
                        assert_eq!(editor.project().name, before.name);
                        assert_eq!(editor.project().settings, before.settings);
                        let track = editor
                            .project()
                            .tracks
                            .iter()
                            .find(|track| track.kind == TrackKind::Text)
                            .unwrap();
                        assert_eq!(track.clips.len(), 2);
                        assert_eq!(track.clips[0].id, selected);
                        assert_ne!(track.clips[1].id, selected);
                        assert_eq!(
                            (track.clips[0].start, track.clips[0].duration),
                            (Time(2_000_000), Time(1_500_000))
                        );
                        assert_eq!(
                            (track.clips[1].start, track.clips[1].duration),
                            (Time(3_500_000), Time(1_500_000))
                        );
                        for clip in &track.clips {
                            assert_eq!(clip.name, "Synthetic title");
                            assert_eq!(clip.transform, before.clip(selected).unwrap().1.transform);
                        }
                    } else {
                        assert_eq!(editor.project().name, "Native AI verified");
                        assert_eq!(
                            (
                                editor.project().settings.width,
                                editor.project().settings.height
                            ),
                            (1080, 1920)
                        );
                        let (_, title) = editor.project().clip(selected).unwrap();
                        assert_eq!(
                            (title.start, title.duration),
                            (Time(3_000_000), Time(3_000_000))
                        );
                    }
                    assert_eq!(
                        editor.project().settings.frame_rate,
                        before.settings.frame_rate
                    );
                    assert_eq!(
                        editor.project().settings.background,
                        before.settings.background
                    );
                    assert_eq!(
                        editor.project().settings.audio_sample_rate,
                        before.settings.audio_sample_rate
                    );
                    assert_eq!(editor.history().count(), 1);
                    assert!(editor.undo());
                    assert_eq!(editor.project(), before.as_ref());
                    assert!(editor.redo());
                    assert_eq!(editor.project(), proposal.after.as_ref());
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "provider":provider, "mode":if split { "split" } else { "three_edits" },
                            "model":proposal.model, "elapsed_seconds":started.elapsed().as_secs_f64(),
                            "summary":proposal.summary, "commands":proposal.commands,
                            "checks":{"provider_completed":true,"review_was_read_only":true,"requested_changes":true,
                                "committed_reviewed_state_including_ids":true,
                                "preserved_settings":true,"single_undo":true,"exact_undo":true,"exact_redo":true}
                        }))?
                    );
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline {
            runtime.cancel();
            return Err("acceptance timed out".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}
