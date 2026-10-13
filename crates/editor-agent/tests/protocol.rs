use editor_agent::*;
use editor_core::{Command, Editor};
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
