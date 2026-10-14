use editor_agent::*;
use editor_core::export::{
    Capabilities, Container, ExportError, ExportRange, ExportSettings, RenderJobInfo, RenderState,
    builtin_presets,
};
use editor_core::{Command, Editor, Time};
use std::{sync::mpsc, thread, time::Duration};

fn request(operation: AgentOperation, revision: Option<u64>) -> AgentRequest {
    AgentRequest {
        request_id: "test".into(),
        expected_revision: revision,
        expected_project_id: None,
        expected_context_id: None,
        operation,
    }
}
fn apply(editor: &mut Editor, mut request: AgentRequest) -> AgentResponse {
    request.expected_project_id = Some(editor.project().id);
    request.expected_context_id = Some(editor.instance_id());
    handle(editor, request)
}
#[test]
fn inspection_is_read_only_and_returns_stable_ids() {
    let mut editor = Editor::default();
    let id = editor.project().id;
    let response = apply(&mut editor, request(AgentOperation::Inspect, None));
    match response.result {
        AgentResult::Project { project, dirty, .. } => {
            assert_eq!(id, project.id);
            assert!(dirty);
        }
        _ => panic!("Expected project"),
    };
    assert_eq!(editor.revision(), 0);
}
#[test]
fn capabilities_name_every_command_in_the_schema() {
    let mut editor = Editor::default();
    let AgentResult::Capabilities {
        commands,
        command_schema,
        ..
    } = handle(&mut editor, request(AgentOperation::Capabilities, None)).result
    else {
        panic!("Expected capabilities");
    };
    let mut tags: Vec<_> = command_schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| variant["properties"]["type"]["const"].as_str().unwrap())
        .collect();
    let mut named: Vec<_> = commands.iter().map(String::as_str).collect();
    tags.sort_unstable();
    named.sort_unstable();
    assert_eq!(named, tags);
    for name in [
        "set_effects",
        "set_masks",
        "set_blend_mode",
        "set_keyframes",
        "set_retime",
        "set_transition",
        "apply_preset",
        "set_tracker",
        "remove_tracker",
        "set_follow",
    ] {
        assert!(named.contains(&name), "{name}");
    }
}
#[test]
fn mutations_require_revision_and_reject_stale_plans() {
    let mut editor = Editor::default();
    let operation = AgentOperation::Apply {
        command: Box::new(Command::RenameProject {
            name: "Film".into(),
        }),
    };
    assert!(
        matches!(apply(&mut editor,request(operation.clone(),None)).result,AgentResult::Error{code,..} if code=="revision_required")
    );
    assert!(
        matches!(apply(&mut editor,request(operation.clone(),Some(3))).result,AgentResult::Error{code,..} if code=="stale_revision")
    );
    assert!(matches!(
        apply(&mut editor, request(operation, Some(0))).result,
        AgentResult::Applied { changed: true, .. }
    ));
    assert_eq!(editor.project().name, "Film");
}
#[test]
fn agent_edits_undo_through_the_same_history() {
    let mut editor = Editor::default();
    let before = editor.snapshot();
    apply(
        &mut editor,
        request(
            AgentOperation::Apply {
                command: Box::new(Command::RenameProject {
                    name: "Film".into(),
                }),
            },
            Some(0),
        ),
    );
    let response = apply(&mut editor, request(AgentOperation::Undo, Some(1)));
    assert!(matches!(
        response.result,
        AgentResult::History { changed: true }
    ));
    assert_eq!(&*before, editor.project());
    let response = apply(&mut editor, request(AgentOperation::Redo, Some(2)));
    assert!(matches!(
        response.result,
        AgentResult::History { changed: true }
    ));
}
#[test]
fn invalid_agent_transaction_has_no_partial_effects() {
    let mut editor = Editor::default();
    let response = apply(
        &mut editor,
        request(
            AgentOperation::Apply {
                command: Box::new(Command::Batch {
                    label: "AI edit".into(),
                    commands: vec![
                        Command::RenameProject {
                            name: "Film".into(),
                        },
                        Command::RenameProject { name: "".into() },
                    ],
                }),
            },
            Some(0),
        ),
    );
    assert!(matches!(response.result, AgentResult::Error { .. }));
    assert_eq!(editor.project().name, "Untitled");
    assert_eq!(editor.revision(), 0);
}
#[test]
fn newline_protocol_round_trips() {
    let value = r#"{"request_id":"42","operation":"apply","expected_revision":0,"command":{"type":"rename_project","name":"Sequence"}}"#;
    let request: AgentRequest = serde_json::from_str(value).unwrap();
    let mut editor = Editor::default();
    let response = apply(&mut editor, request);
    assert_eq!(response.request_id, "42");
    assert_eq!(response.revision, 1);
    assert!(
        serde_json::from_str::<AgentResponse>(&serde_json::to_string(&response).unwrap()).is_ok()
    );
}
#[test]
fn live_transport_routes_to_the_owning_editor() {
    let (wake, wakes) = mpsc::channel();
    let host = AgentHost::new(move || {
        let _ = wake.send(());
    })
    .unwrap();
    let descriptor = host.descriptor().clone();
    let mut editor = Editor::default();
    let project_id = editor.project().id;
    let context_id = editor.instance_id();
    let client = thread::spawn(move || {
        connect(
            &descriptor,
            AgentRequest {
                expected_project_id: Some(project_id),
                expected_context_id: Some(context_id),
                ..request(
                    AgentOperation::Apply {
                        command: Box::new(Command::RenameProject {
                            name: "Live film".into(),
                        }),
                    },
                    Some(0),
                )
            },
        )
    });
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    host.poll().unwrap().execute(&mut editor);
    let response = client.join().unwrap().unwrap();
    assert!(matches!(
        response.result,
        AgentResult::Applied { changed: true, .. }
    ));
    assert_eq!(editor.project().name, "Live film");
}
#[test]
fn wrong_session_token_is_rejected_before_the_editor_sees_it() {
    let host = AgentHost::new(|| {}).unwrap();
    let mut descriptor = host.descriptor().clone();
    descriptor.token = "incorrect".into();
    let response = connect(&descriptor, request(AgentOperation::Inspect, None)).unwrap();
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="unauthorized"));
    assert!(host.poll().is_none());
}
#[test]
fn nonlocal_descriptors_are_rejected() {
    let descriptor = SessionDescriptor {
        protocol_version: 1,
        address: "192.0.2.1:9999".parse().unwrap(),
        token: "unused".into(),
        instance_id: uuid::Uuid::new_v4(),
    };
    assert!(connect(&descriptor, request(AgentOperation::Inspect, None)).is_err());
}
#[test]
fn expired_queued_request_cannot_later_mutate_the_editor() {
    let (wake, wakes) = mpsc::channel();
    let host = AgentHost::new(move || {
        let _ = wake.send(());
    })
    .unwrap();
    let descriptor = host.descriptor().clone();
    let mut editor = Editor::default();
    let project_id = editor.project().id;
    let context_id = editor.instance_id();
    let client = thread::spawn(move || {
        connect(
            &descriptor,
            AgentRequest {
                expected_project_id: Some(project_id),
                expected_context_id: Some(context_id),
                ..request(
                    AgentOperation::Apply {
                        command: Box::new(Command::RenameProject {
                            name: "Too late".into(),
                        }),
                    },
                    Some(0),
                )
            },
        )
    });
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    let call = host.poll().unwrap();
    let response = client.join().unwrap().unwrap();
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="timeout"));
    call.execute(&mut editor);
    assert_eq!(editor.project().name, "Untitled");
    assert_eq!(editor.revision(), 0);
}
#[test]
fn private_descriptor_is_removed_only_by_its_own_instance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut host = AgentHost::new(|| {}).unwrap();
    host.publish(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(path.exists());
    drop(host);
    assert!(!path.exists());
    let mut host = AgentHost::new(|| {}).unwrap();
    host.publish(&path).unwrap();
    let mut replacement = host.descriptor().clone();
    replacement.instance_id = uuid::Uuid::new_v4();
    std::fs::write(&path, serde_json::to_vec(&replacement).unwrap()).unwrap();
    drop(host);
    assert!(path.exists());
}

#[test]
fn a_plan_for_a_replaced_project_is_rejected_even_at_the_same_revision() {
    let original = Editor::default();
    let mut replacement = Editor::default();
    let mut request = request(
        AgentOperation::Apply {
            command: Box::new(Command::RenameProject {
                name: "Wrong project".into(),
            }),
        },
        Some(0),
    );
    request.expected_project_id = Some(original.project().id);
    let response = handle(&mut replacement, request);
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="stale_project"));
    assert_eq!(replacement.project().name, "Untitled");
    assert_eq!(replacement.revision(), 0);
}
#[test]
fn mutation_without_project_identity_is_rejected() {
    let mut editor = Editor::default();
    let response = handle(&mut editor, request(AgentOperation::Undo, Some(0)));
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="project_required"));
}

#[test]
fn a_plan_for_the_same_document_before_reload_cannot_target_its_reset_revision() {
    let original = Editor::default();
    let mut request = request(
        AgentOperation::Apply {
            command: Box::new(Command::RenameProject {
                name: "Stale plan".into(),
            }),
        },
        Some(0),
    );
    request.expected_project_id = Some(original.project().id);
    request.expected_context_id = Some(original.instance_id());
    let mut reloaded = Editor::new(original.project().clone()).unwrap();
    let response = handle(&mut reloaded, request);
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="stale_context"));
    assert_eq!(reloaded.project().name, "Untitled");
    assert_eq!(reloaded.revision(), 0);
}

fn render_operations() -> Vec<AgentOperation> {
    let job = uuid::Uuid::new_v4();
    vec![
        AgentOperation::ExportCapabilities,
        AgentOperation::QueueExport {
            settings: ExportSettings::default(),
            destination: std::env::temp_dir().join("aster-never-written.mp4"),
            overwrite: false,
        },
        AgentOperation::RenderJobs,
        AgentOperation::CancelRender { job },
        AgentOperation::PauseRender { job },
        AgentOperation::ResumeRender { job },
        AgentOperation::RetryRender { job },
        AgentOperation::RemoveRender { job },
    ]
}
fn targeted(editor: &Editor, operation: AgentOperation) -> AgentRequest {
    AgentRequest {
        request_id: "render".into(),
        expected_revision: Some(editor.revision()),
        expected_project_id: Some(editor.project().id),
        expected_context_id: Some(editor.instance_id()),
        operation,
    }
}
fn code(result: Result<(), AgentResult>) -> String {
    match result {
        Err(AgentResult::Error { code, .. }) => code,
        other => panic!("Expected an error, got {other:?}"),
    }
}
fn job_info() -> RenderJobInfo {
    RenderJobInfo {
        id: uuid::Uuid::new_v4(),
        name: "Film.mp4".into(),
        destination: "/films/Film.mp4".into(),
        settings: ExportSettings::default(),
        project_id: uuid::Uuid::new_v4(),
        revision: 7,
        state: RenderState::Rendering,
        progress: 0.25,
        frames_done: 30,
        frames_total: 120,
        speed_fps: 60.0,
        elapsed_seconds: 0.5,
        eta_seconds: Some(1.5),
        encoder: Some("libx264".into()),
        output_bytes: None,
        context_id: uuid::Uuid::new_v4(),
        message: None,
        attempts: 1,
    }
}

#[test]
fn capabilities_name_every_operation_in_the_schema() {
    let mut editor = Editor::default();
    let AgentResult::Capabilities { operations, .. } =
        handle(&mut editor, request(AgentOperation::Capabilities, None)).result
    else {
        panic!("Expected capabilities");
    };
    let schema = serde_json::to_value(schemars::schema_for!(AgentOperation)).unwrap();
    let mut tags: Vec<_> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| {
            variant["properties"]["operation"]["const"]
                .as_str()
                .unwrap()
        })
        .collect();
    let mut named: Vec<_> = operations.iter().map(String::as_str).collect();
    assert_eq!(named, OPERATIONS);
    tags.sort_unstable();
    named.sort_unstable();
    assert_eq!(named, tags);
}

#[test]
fn capabilities_from_an_older_session_parse_without_operations() {
    let value = r#"{"request_id":"1","revision":0,"result":{"type":"capabilities","protocol_version":1,"commands":[],"command_schema":null}}"#;
    let response: AgentResponse = serde_json::from_str(value).unwrap();
    assert!(matches!(
        response.result,
        AgentResult::Capabilities { operations, .. } if operations.is_empty()
    ));
}

#[test]
fn render_requests_round_trip() {
    let job = uuid::Uuid::new_v4();
    let settings = ExportSettings {
        range: ExportRange::Span {
            start: Time(1_000_000),
            end: Time(3_500_000),
        },
        ..ExportSettings::default()
    };
    let mut operations = render_operations();
    operations.push(AgentOperation::QueueExport {
        settings: settings.clone(),
        destination: "/films/Cut.mp4".into(),
        overwrite: true,
    });
    for operation in operations {
        let request = AgentRequest {
            request_id: "round-trip".into(),
            expected_revision: Some(4),
            expected_project_id: Some(job),
            expected_context_id: Some(job),
            operation,
        };
        let line = serde_json::to_string(&request).unwrap();
        let parsed: AgentRequest = serde_json::from_str(&line).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), line);
        assert_eq!(parsed.expected_revision, Some(4));
    }
    let line = format!(r#"{{"request_id":"j","operation":"cancel_render","job":"{job}"}}"#);
    let parsed: AgentRequest = serde_json::from_str(&line).unwrap();
    assert!(matches!(parsed.operation, AgentOperation::CancelRender { job: id } if id == job));
    let line = r#"{"request_id":"q","operation":"queue_export","expected_revision":0,"settings":{"container":"wav","video":null,"audio":{"codec":"pcm24","kbps":0,"sample_rate":48000,"channels":2}},"destination":"/films/Sound.wav"}"#;
    let AgentOperation::QueueExport {
        settings,
        destination,
        overwrite,
    } = serde_json::from_str::<AgentRequest>(line)
        .unwrap()
        .operation
    else {
        panic!("Expected queue_export");
    };
    assert_eq!(settings.container, Container::Wav);
    assert!(settings.video.is_none());
    assert_eq!(settings.range, ExportRange::Timeline);
    assert_eq!(destination, std::path::Path::new("/films/Sound.wav"));
    assert!(!overwrite);
}

#[test]
fn render_results_round_trip() {
    let job = job_info();
    let results = [
        AgentResult::ExportCapabilities {
            capabilities: Box::new(Capabilities {
                ffmpeg: "ffmpeg version 8.0.1".into(),
                containers: vec![Container::Mp4, Container::Wav],
                ..Capabilities::default()
            }),
            presets: builtin_presets(),
            settings_schema: export_settings_schema(),
        },
        AgentResult::RenderJobs {
            jobs: vec![job.clone()],
        },
        AgentResult::RenderJob {
            job: Box::new(job.clone()),
        },
    ];
    for result in results {
        let response = AgentResponse {
            request_id: "result".into(),
            revision: 3,
            result,
        };
        let line = serde_json::to_string(&response).unwrap();
        let parsed: AgentResponse = serde_json::from_str(&line).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), line);
        match parsed.result {
            AgentResult::ExportCapabilities {
                capabilities,
                presets,
                settings_schema,
            } => {
                assert_eq!(capabilities.containers, [Container::Mp4, Container::Wav]);
                assert_eq!(presets, builtin_presets());
                assert!(settings_schema["properties"]["container"].is_object());
            }
            AgentResult::RenderJobs { jobs } => assert_eq!(jobs, std::slice::from_ref(&job)),
            AgentResult::RenderJob { job: parsed } => assert_eq!(*parsed, job),
            other => panic!("Unexpected {other:?}"),
        }
    }
}

#[test]
fn the_settings_schema_describes_export_settings() {
    let schema = export_settings_schema();
    let properties = schema["properties"].as_object().unwrap();
    for field in ["container", "video", "audio", "range", "subtitles"] {
        assert!(properties.contains_key(field), "{field}");
    }
}

#[test]
fn handle_answers_render_operations_unsupported_without_editing() {
    let mut editor = Editor::default();
    let before = editor.snapshot();
    for operation in render_operations() {
        assert!(operation.is_render() && !operation.is_mutation());
        let request = targeted(&editor, operation);
        let response = handle(&mut editor, request);
        assert!(
            matches!(&response.result, AgentResult::Error { code, .. } if code == codes::UNSUPPORTED),
            "{response:?}"
        );
        assert_eq!(response.revision, 0);
    }
    assert_eq!(&*before, editor.project());
    assert_eq!(editor.revision(), 0);
}

#[test]
fn export_errors_map_to_their_codes() {
    for (error, expected) in [
        (ExportError::Invalid("x".into()), codes::INVALID_EXPORT),
        (
            ExportError::Unsupported("x".into()),
            codes::UNSUPPORTED_EXPORT,
        ),
        (ExportError::Empty("x".into()), codes::EMPTY_EXPORT),
    ] {
        assert!(matches!(
            AgentResult::export_error(&error),
            AgentResult::Error { code, message } if code == expected && message == "x"
        ));
    }
}

#[test]
fn queue_export_needs_the_inspected_project_and_a_free_absolute_destination() {
    let dir = tempfile::tempdir().unwrap();
    let editor = Editor::default();
    let queue = |destination: std::path::PathBuf, overwrite| {
        targeted(
            &editor,
            AgentOperation::QueueExport {
                settings: ExportSettings::default(),
                destination,
                overwrite,
            },
        )
    };
    let free = dir.path().join("Film.mp4");
    assert!(check_queue_export(&editor, &queue(free.clone(), false)).is_ok());

    type Spoil = fn(&mut AgentRequest);
    let guards: [(Spoil, &str); 6] = [
        (|r| r.expected_revision = None, codes::REVISION_REQUIRED),
        (|r| r.expected_revision = Some(9), codes::STALE_REVISION),
        (|r| r.expected_project_id = None, codes::PROJECT_REQUIRED),
        (
            |r| r.expected_project_id = Some(uuid::Uuid::new_v4()),
            codes::STALE_PROJECT,
        ),
        (|r| r.expected_context_id = None, codes::CONTEXT_REQUIRED),
        (
            |r| r.expected_context_id = Some(uuid::Uuid::new_v4()),
            codes::STALE_CONTEXT,
        ),
    ];
    for (spoil, expected) in guards {
        let mut request = queue(free.clone(), false);
        spoil(&mut request);
        assert_eq!(code(check_queue_export(&editor, &request)), expected);
        assert_eq!(code(check_target(&editor, &request)), expected);
    }

    let relative = std::path::PathBuf::from("Film.mp4");
    assert_eq!(
        code(check_queue_export(&editor, &queue(relative, true))),
        codes::INVALID_EXPORT
    );
    let missing = dir.path().join("missing").join("Film.mp4");
    assert_eq!(
        code(check_queue_export(&editor, &queue(missing, true))),
        codes::INVALID_EXPORT
    );
    assert_eq!(
        code(check_queue_export(&editor, &queue(dir.path().into(), true))),
        codes::INVALID_EXPORT
    );
    std::fs::write(&free, b"earlier export").unwrap();
    assert_eq!(
        code(check_queue_export(&editor, &queue(free.clone(), false))),
        codes::DESTINATION_EXISTS
    );
    assert!(check_queue_export(&editor, &queue(free.clone(), true)).is_ok());
    assert_eq!(std::fs::read(&free).unwrap(), b"earlier export");

    let not_queue = targeted(&editor, AgentOperation::RenderJobs);
    assert_eq!(
        code(check_queue_export(&editor, &not_queue)),
        codes::INVALID_EXPORT
    );
}

#[test]
fn a_claimed_call_is_answered_once_by_its_owner() {
    let (wake, wakes) = mpsc::channel();
    let host = AgentHost::new(move || {
        let _ = wake.send(());
    })
    .unwrap();
    let descriptor = host.descriptor().clone();
    let client =
        thread::spawn(move || connect(&descriptor, request(AgentOperation::RenderJobs, None)));
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    let call = host.poll().unwrap();
    assert!(call.is_live());
    let claimed = call.claim().unwrap();
    assert!(matches!(
        claimed.request.operation,
        AgentOperation::RenderJobs
    ));
    let job = job_info();
    let request_id = claimed.request.request_id.clone();
    claimed.reply(AgentResponse {
        request_id,
        revision: 0,
        result: AgentResult::RenderJobs {
            jobs: vec![job.clone()],
        },
    });
    let response = client.join().unwrap().unwrap();
    assert!(matches!(response.result, AgentResult::RenderJobs { jobs } if jobs == [job]));
}

#[test]
fn an_expired_call_cannot_be_claimed() {
    let (wake, wakes) = mpsc::channel();
    let host = AgentHost::new(move || {
        let _ = wake.send(());
    })
    .unwrap();
    let descriptor = host.descriptor().clone();
    let editor = Editor::default();
    let operation = AgentOperation::QueueExport {
        settings: ExportSettings::default(),
        destination: "/films/Film.mp4".into(),
        overwrite: false,
    };
    let sent = targeted(&editor, operation);
    let client = thread::spawn(move || connect(&descriptor, sent));
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    let call = host.poll().unwrap();
    let response = client.join().unwrap().unwrap();
    assert!(matches!(response.result,AgentResult::Error{code,..} if code=="timeout"));
    assert!(!call.is_live());
    assert!(call.claim().is_none());
}

#[test]
fn every_documented_request_parses() {
    let doc = include_str!("../../../docs/agent-protocol.md");
    let lines: Vec<_> = doc
        .lines()
        .filter(|line| line.starts_with(r#"{"request_id":"#))
        .collect();
    assert!(lines.len() >= 7);
    for line in lines {
        let request: AgentRequest =
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        if let AgentOperation::QueueExport { settings, .. } = request.operation {
            assert_eq!(settings.container, Container::Mp4);
        }
    }
}
