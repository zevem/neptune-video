//! `aster-mcp` end to end: the real server binary, driven by the official
//! Rust MCP client over stdio and Streamable HTTP, on media FFmpeg makes in
//! a task-owned folder. The window test runs with ASTER_TEST_DESKTOP=1 on a
//! computer with a display.
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, ReadResourceRequestParams},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

type Client = RunningService<RoleClient, ()>;

/// The FFmpeg the editor would run: ASTER_FFMPEG, the copy fetched into
/// the checkout, or the one on PATH.
fn ffmpeg() -> PathBuf {
    if let Some(program) = std::env::var_os("ASTER_FFMPEG") {
        return program.into();
    }
    let fetched = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tools/media")
        .join(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
        .join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
    if fetched.is_file() {
        fetched
    } else {
        "ffmpeg".into()
    }
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let status = std::process::Command::new(ffmpeg())
        .args(["-v", "error", "-y"])
        .args(args)
        .arg(&out)
        .status()
        .expect("FFmpeg runs");
    assert!(status.success(), "FFmpeg made {name}");
    out
}

/// Two shots, a cut between two shots at 2 s, and music.
struct Media {
    a: PathBuf,
    b: PathBuf,
    cuts: PathBuf,
    music: PathBuf,
}

fn media(dir: &Path) -> Media {
    let video = [
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
    ];
    let mut a = vec![
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30:duration=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=4",
    ];
    a.extend(video);
    let mut b = vec![
        "-f",
        "lavfi",
        "-i",
        "smptebars=size=640x360:rate=30:duration=3",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=880:duration=3",
    ];
    b.extend(video);
    Media {
        a: make(dir, "a.mp4", &a),
        b: make(dir, "b.mp4", &b),
        cuts: make(
            dir,
            "cuts.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30:duration=2",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:size=640x360:rate=30:duration=2",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ],
        ),
        music: make(
            dir,
            "music.m4a",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=220:duration=6",
                "-c:a",
                "aac",
            ],
        ),
    }
}

/// The server's arguments for a task-owned folder: it is the only root,
/// and windows, presets, models and connection files are looked for
/// beside settings inside it, never in the person's own.
fn server_args(dir: &Path) -> Vec<String> {
    vec![
        "--root".into(),
        dir.display().to_string(),
        "--desktop-settings".into(),
        dir.join("settings/settings.json").display().to_string(),
        "--models".into(),
        dir.join("models").display().to_string(),
    ]
}

async fn stdio(dir: &Path) -> (Client, u32) {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_aster-mcp"));
    command
        .args(server_args(dir))
        .current_dir(dir)
        .stderr(Stdio::null());
    let transport = TokioChildProcess::new(command).expect("aster-mcp starts");
    let pid = transport.id().expect("a process id");
    let client = ().serve(transport).await.expect("an MCP session starts");
    (client, pid)
}

/// A tool's structured result; `Err` holds a refusal's structured error.
async fn try_call(client: &Client, name: &str, arguments: Value) -> Result<Value, Value> {
    let Value::Object(arguments) = arguments else {
        panic!("arguments are an object")
    };
    let result = client
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
        .await
        .unwrap_or_else(|error| panic!("{name} answers: {error}"));
    let content = serde_json::to_value(&result.content).unwrap();
    let structured = result.structured_content.unwrap_or(Value::Null);
    if result.is_error == Some(true) {
        Err(structured)
    } else if structured.is_null() {
        Ok(json!({"content": content}))
    } else {
        let mut value = structured;
        value["_content"] = content;
        Ok(value)
    }
}

async fn call(client: &Client, name: &str, arguments: Value) -> Value {
    try_call(client, name, arguments.clone())
        .await
        .unwrap_or_else(|error| panic!("{name} {arguments} was refused: {error}"))
}

async fn refused(client: &Client, name: &str, arguments: Value) -> String {
    match try_call(client, name, arguments.clone()).await {
        Ok(value) => panic!("{name} {arguments} should be refused, got {value}"),
        Err(error) => error["code"].as_str().unwrap_or_default().to_owned(),
    }
}

fn id(value: &Value) -> String {
    value.as_str().expect("an ID").to_owned()
}

fn images(value: &Value) -> Vec<String> {
    value["_content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "image")
        .map(|block| block["mimeType"].as_str().unwrap_or_default().to_owned())
        .collect()
}

async fn wait(client: &Client, job: &Value) -> Value {
    let job_id = job["job_id"].clone();
    for _ in 0..6 {
        let answer = call(
            client,
            "job",
            json!({"op": "wait", "job_id": job_id, "timeout_seconds": 30}),
        )
        .await;
        let state = answer["job"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if state != "running" && state != "queued" {
            return answer["job"].clone();
        }
    }
    panic!("job {job_id} did not end");
}

async fn timeline(client: &Client) -> Value {
    call(client, "project_inspect", json!({"scope": "timeline"})).await
}

async fn revision(client: &Client) -> u64 {
    call(client, "project_status", json!({})).await["revision"]
        .as_u64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_makes_edits_looks_at_and_exports_a_video_that_a_person_can_open() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("film.aster");
    let (client, _) = stdio(&dir).await;

    let made = call(
        &client,
        "project_create",
        json!({"path": project, "name": "Film", "width": 1280, "height": 720, "frame_rate": 30}),
    )
    .await;
    assert_eq!(made["authority"], "local");
    assert_eq!(made["revision"], 0);

    let imported = call(
        &client,
        "media_import",
        json!({"paths": [media.a, media.b, media.cuts, media.music]}),
    )
    .await;
    let asset = |name: &str| {
        id(&imported["media"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap()["asset_id"])
    };
    assert_eq!(imported["edit"]["created"].as_array().unwrap().len(), 4);
    // The same file again adds nothing.
    let again = call(&client, "media_import", json!({"paths": [media.a]})).await;
    assert_eq!(again["media"][0]["new"], false);

    let a = call(&client, "clip_add", json!({"asset_id": asset("a.mp4")})).await;
    let b = call(&client, "clip_add", json!({"asset_id": asset("b.mp4")})).await;
    let (a, b) = (id(&a["created"][0]), id(&b["created"][0]));
    let clips = timeline(&client).await;
    let b_view = clips["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["clip_id"] == b.as_str())
        .unwrap()
        .clone();
    assert_eq!(b_view["start"]["frame"], 120, "b follows a's 4 s at 30 fps");
    assert_eq!(b_view["start"]["timecode"], "00:00:04:00");

    // Timeline edits.
    let split = call(
        &client,
        "clip_edit",
        json!({"op": "split", "clip_ids": [a], "at": {"seconds": 2}}),
    )
    .await;
    let tail = id(&split["created"][0]);
    call(
        &client,
        "clip_edit",
        json!({"op": "trim", "clip_ids": [b], "edge": "end", "to": {"frame": 195}}),
    )
    .await;
    call(&client, "clip_edit", json!({"op": "transform", "clip_ids": [tail], "transform": {"scale": [0.8, 0.8], "opacity": 0.9}})).await;

    // Effects, a transition, keyframes, a title and music.
    call(
        &client,
        "effect_edit",
        json!({"op": "add", "clip_ids": [b], "kind": "blur"}),
    )
    .await;
    call(
        &client,
        "effect_edit",
        json!({"op": "look", "clip_ids": [a], "look": "cinematic"}),
    )
    .await;
    call(
        &client,
        "transition_set",
        json!({"clip_id": b, "edge": "in", "kind": "dissolve", "duration": {"seconds": 0.5}}),
    )
    .await;
    call(&client, "keyframes_set", json!({"clip_id": a, "key": "opacity", "keys": [{"at": {"seconds": 0}, "value": 0.0}, {"at": {"seconds": 1}, "value": 1.0}]})).await;
    let title = call(&client, "text_add", json!({"text": "Made by an agent", "at": {"seconds": 0.5}, "duration": {"seconds": 2}, "enter": {"kind": "fade"}})).await;
    assert_eq!(title["created"].as_array().map(Vec::len), Some(1));
    call(
        &client,
        "audio_edit",
        json!({"op": "music", "asset_id": asset("music.m4a")}),
    )
    .await;
    call(
        &client,
        "audio_edit",
        json!({"op": "volume", "clip_ids": [b], "db": -6.0}),
    )
    .await;

    // A batch is one history entry, all or nothing.
    let before = call(&client, "history", json!({"op": "list"})).await["history"]["past"]
        .as_array()
        .unwrap()
        .len();
    let batch = call(
        &client,
        "edit_apply",
        json!({"label": "Rename and quiet", "commands": [
            {"type": "rename_project", "name": "Film, cut"},
            {"type": "set_gain", "clip_id": tail, "gain": 0.5},
        ]}),
    )
    .await;
    assert_eq!(batch["label"], "Rename and quiet");
    let after = call(&client, "history", json!({"op": "list"})).await["history"]["past"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after, before + 1);
    let at = revision(&client).await;
    let code = refused(
        &client,
        "edit_apply",
        json!({"commands": [
            {"type": "rename_project", "name": "Never"},
            {"type": "set_gain", "clip_id": "00000000-0000-4000-8000-000000000000", "gain": 0.5},
        ]}),
    )
    .await;
    assert!(!code.is_empty());
    assert_eq!(
        revision(&client).await,
        at,
        "a refused batch changes nothing"
    );
    assert_eq!(
        call(&client, "project_status", json!({})).await["name"],
        "Film, cut"
    );

    // A dry run and a stale revision change nothing.
    let dry = call(
        &client,
        "clip_edit",
        json!({"op": "delete", "clip_ids": [b], "dry_run": true}),
    )
    .await;
    assert_eq!(dry["dry_run"], true);
    assert_eq!(revision(&client).await, at);
    let stale = refused(
        &client,
        "clip_edit",
        json!({"op": "delete", "clip_ids": [b], "expected_revision": at - 1}),
    )
    .await;
    assert_eq!(stale, "stale_revision");

    // Look at the edit.
    let frames = call(&client, "frame_render", json!({"times": [{"seconds": 1}, {"timecode": "00:00:05:00"}], "size": 320, "save_to": dir.join("look.png")})).await;
    assert_eq!(images(&frames), ["image/png", "image/png"]);
    let saved = PathBuf::from(frames["saved"][0].as_str().unwrap());
    assert!(std::fs::read(&saved).unwrap().starts_with(b"\x89PNG"));
    let sheet = call(
        &client,
        "contact_sheet",
        json!({"count": 6, "cell_size": 160}),
    )
    .await;
    assert_eq!(images(&sheet).len(), 1);

    // Analysis runs as jobs.
    let scenes = wait(
        &client,
        &call(
            &client,
            "scenes_start",
            json!({"asset_id": asset("cuts.mp4")}),
        )
        .await,
    )
    .await;
    assert_eq!(scenes["state"], "done");
    let cut = scenes["result"]["cuts"][0]["media_time"]["seconds"]
        .as_f64()
        .expect("the cut is found");
    assert!((cut - 2.0).abs() < 0.1, "the cut is at 2 s, found {cut}");
    let loudness = wait(
        &client,
        &call(&client, "loudness_start", json!({"clip_ids": [b]})).await,
    )
    .await;
    assert_eq!(loudness["state"], "done");
    let normalized = call(
        &client,
        "audio_edit",
        json!({"op": "normalize", "clip_ids": [b], "job_id": loudness["job_id"]}),
    )
    .await;
    assert_eq!(normalized["changed"], true);

    // Preview, then export.
    let preview = call(
        &client,
        "render_export_start",
        json!({"destination": dir.join("preview.mp4"), "preview": true}),
    )
    .await;
    let preview = wait(&client, &preview).await;
    assert_eq!(preview["state"], "done", "{preview}");
    assert!(std::fs::metadata(dir.join("preview.mp4")).unwrap().len() > 1000);
    let code = refused(
        &client,
        "render_export_start",
        json!({"destination": dir.join("preview.mp4"), "preview": true}),
    )
    .await;
    assert_eq!(
        code, "destination_exists",
        "an export replaces nothing unasked"
    );
    let export = call(&client, "render_export_start", json!({"destination": dir.join("film.mp4"), "preset": "small-file", "from": {"seconds": 0}, "to": {"seconds": 3}})).await;
    let export = wait(&client, &export).await;
    assert_eq!(export["state"], "done", "{export}");

    // Undo takes back the agent's own last edit; checkpoints restore.
    let clips_now = timeline(&client).await["clips"].as_array().unwrap().len();
    call(
        &client,
        "checkpoint",
        json!({"op": "create", "label": "before cleanup"}),
    )
    .await;
    call(
        &client,
        "clip_edit",
        json!({"op": "delete", "clip_ids": [tail]}),
    )
    .await;
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        clips_now - 1
    );
    call(&client, "history", json!({"op": "undo"})).await;
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        clips_now
    );
    call(
        &client,
        "clip_edit",
        json!({"op": "delete", "clip_ids": [tail]}),
    )
    .await;
    call(
        &client,
        "checkpoint",
        json!({"op": "restore", "label": "before cleanup"}),
    )
    .await;
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        clips_now
    );

    // The resource is the same project.
    let resource = client
        .read_resource(ReadResourceRequestParams::new("aster://project"))
        .await
        .expect("the project resource reads");
    assert!(
        serde_json::to_string(&resource)
            .unwrap()
            .contains("Film, cut")
    );

    call(&client, "project_close", json!({})).await;
    client.cancel().await.unwrap();

    // A person opens the file the agent left with the editor's own loader.
    let opened = editor_store::load_project(&project).expect("the GUI's loader opens it");
    assert_eq!(opened.name, "Film, cut");
    assert_eq!(
        opened
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum::<usize>(),
        clips_now
    );

    // And another agent continues it.
    let (client, _) = stdio(&dir).await;
    let reopened = call(&client, "project_open", json!({"path": project})).await;
    assert_eq!(reopened["clips"].as_u64(), Some(clips_now as u64));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_tool_is_described_typed_and_the_list_fits_a_client_budget() {
    let dir = tempfile::tempdir().unwrap();
    let (client, _) = stdio(dir.path()).await;
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.len() >= 35, "{} tools", tools.len());
    for tool in &tools {
        assert!(
            tool.description.as_deref().is_some_and(|d| d.len() > 20),
            "{} is described",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&json!("object")),
            "{} takes an object",
            tool.name
        );
    }
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    for wanted in [
        "project_create",
        "media_import",
        "clip_edit",
        "effect_edit",
        "keyframes_set",
        "text_add",
        "audio_edit",
        "transcribe_start",
        "contact_sheet",
        "templates",
        "render_export_start",
        "job",
        "history",
        "checkpoint",
        "edit_apply",
    ] {
        assert!(names.contains(&wanted), "{wanted} is offered");
    }
    let listed = serde_json::to_vec(&tools).unwrap().len();
    assert!(listed < 40 * 1024, "tools/list is {listed} bytes");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn files_outside_the_roots_are_refused_and_nothing_is_replaced_unasked() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.mp4");
    std::fs::write(&secret, b"not media").unwrap();
    let (client, _) = stdio(&dir).await;

    assert_eq!(
        refused(
            &client,
            "project_create",
            json!({"path": outside.path().join("x.aster")})
        )
        .await,
        "outside_roots"
    );
    call(
        &client,
        "project_create",
        json!({"path": dir.join("p.aster")}),
    )
    .await;
    assert_eq!(
        refused(&client, "media_import", json!({"paths": [secret]})).await,
        "outside_roots"
    );
    let climbing = format!(
        "{}/../{}/secret.mp4",
        dir.display(),
        outside.path().display()
    );
    assert_eq!(
        refused(&client, "media_import", json!({"paths": [climbing]})).await,
        "invalid",
        "`..` is never followed"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, dir.join("link.mp4")).unwrap();
        assert_eq!(
            refused(
                &client,
                "media_import",
                json!({"paths": [dir.join("link.mp4")]})
            )
            .await,
            "outside_roots"
        );
    }
    assert_eq!(
        refused(
            &client,
            "project_create",
            json!({"path": dir.join("p.aster")})
        )
        .await,
        "destination_exists"
    );
    assert_eq!(
        refused(
            &client,
            "project_save",
            json!({"destination": outside.path().join("copy.aster")})
        )
        .await,
        "outside_roots"
    );
    assert_eq!(revision(&client).await, 0);
    client.cancel().await.unwrap();
}

/// `kill -9` stands for a crash; Windows has no `kill`.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn work_survives_a_server_that_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("p.aster");
    let (client, pid) = stdio(&dir).await;
    call(&client, "project_create", json!({"path": project})).await;
    let imported = call(&client, "media_import", json!({"paths": [media.a]})).await;
    call(
        &client,
        "clip_add",
        json!({"asset_id": imported["media"][0]["asset_id"]}),
    )
    .await;
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    drop(client);

    let (client, _) = stdio(&dir).await;
    let opened = call(&client, "project_open", json!({"path": project})).await;
    assert_eq!(opened["clips"], 1, "every edit was saved when it was made");
    assert_eq!(
        refused(
            &client,
            "project_open",
            json!({"path": project, "recover": true})
        )
        .await,
        "invalid"
    );
    assert_eq!(
        call(&client, "project_status", json!({})).await["clips"],
        1,
        "a refused open keeps the project"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unsaved_work_a_stopped_editor_left_is_opened_only_when_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("p.aster");
    let (client, _) = stdio(&dir).await;
    call(
        &client,
        "project_create",
        json!({"path": project, "name": "Saved"}),
    )
    .await;
    call(&client, "project_close", json!({})).await;
    // A window renamed the project and stopped before saving.
    let (saved, session) = editor_store::load_project_with_session(&project).unwrap();
    let mut editor = editor_core::Editor::new(saved).unwrap();
    editor
        .dispatch(editor_core::Command::RenameProject {
            name: "Unsaved".into(),
        })
        .unwrap();
    editor_store::save_recovery_with_document(&session, &editor).unwrap();
    drop(session);

    assert_eq!(
        refused(&client, "project_open", json!({"path": project})).await,
        "recovery_available"
    );
    assert!(
        editor_store::recovery_path(&project).exists(),
        "the copy is left as it was"
    );
    let recovered = call(
        &client,
        "project_open",
        json!({"path": project, "recover": true}),
    )
    .await;
    assert_eq!(recovered["name"], "Unsaved");
    // Recovered work is in no file yet: nothing lets it go silently.
    assert_eq!(
        refused(&client, "project_close", json!({})).await,
        "invalid"
    );
    assert_eq!(
        refused(
            &client,
            "project_create",
            json!({"path": dir.join("other.aster")})
        )
        .await,
        "invalid"
    );
    assert!(!dir.join("other.aster").exists());
    // A server stopped while holding it leaves it in the recovery copy.
    client.cancel().await.unwrap();
    let (client, _) = stdio(&dir).await;
    assert_eq!(
        refused(&client, "project_open", json!({"path": project})).await,
        "recovery_available",
        "the stopped server kept the work for the next editor"
    );
    let recovered = call(
        &client,
        "project_open",
        json!({"path": project, "recover": true}),
    )
    .await;
    assert_eq!(recovered["name"], "Unsaved");
    call(&client, "project_save", json!({})).await;
    assert_eq!(
        editor_store::load_project(&project).unwrap().name,
        "Unsaved"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_takes_back_and_redoes_its_own_edits_one_after_another() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let (client, _) = stdio(&dir).await;
    call(
        &client,
        "project_create",
        json!({"path": dir.join("p.aster"), "name": "A"}),
    )
    .await;
    for name in ["B", "C", "D"] {
        call(
            &client,
            "edit_apply",
            json!({"commands": [{"type": "rename_project", "name": name}]}),
        )
        .await;
    }
    async fn name(client: &Client) -> String {
        call(client, "project_status", json!({})).await["name"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    // Every step is the agent's own, though each moves the revision on.
    for expected in ["C", "B", "A"] {
        call(&client, "history", json!({"op": "undo"})).await;
        assert_eq!(name(&client).await, expected);
    }
    for expected in ["B", "C"] {
        call(&client, "history", json!({"op": "redo"})).await;
        assert_eq!(name(&client).await, expected);
    }
    call(&client, "history", json!({"op": "undo"})).await;
    assert_eq!(name(&client).await, "B");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn links_and_lists_inside_the_roots_cannot_reach_outside_them() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let media = media(&dir);
    let project = dir.join("p.aster");
    let (client, _) = stdio(&dir).await;
    call(&client, "project_create", json!({"path": project})).await;
    let imported = call(&client, "media_import", json!({"paths": [media.a]})).await;
    let asset = id(&imported["media"][0]["asset_id"]);
    call(&client, "clip_add", json!({"asset_id": asset})).await;

    // A list of other files, named as media, is not media.
    let list = dir.join("list.mp4");
    std::fs::write(&list, "ffconcat version 1.0\nfile a.mp4\n").unwrap();
    assert_eq!(
        refused(&client, "media_import", json!({"paths": [list]})).await,
        "not_found"
    );
    let mut forged = editor_store::load_project(&project).unwrap().assets
        [&asset.parse::<uuid::Uuid>().unwrap()]
        .clone();
    forged.id = uuid::Uuid::new_v4();
    forged.path = list.clone();
    assert_eq!(
        refused(
            &client,
            "edit_apply",
            json!({"commands": [{"type": "add_assets", "assets": [forged]}]})
        )
        .await,
        "invalid",
        "what a command says media is is not taken on trust"
    );
    let inspected = call(
        &client,
        "media_inspect",
        json!({"path": list, "details": true}),
    )
    .await;
    assert!(
        inspected.get("details").is_none_or(Value::is_null),
        "a list's streams are never read: {inspected}"
    );

    #[cfg(unix)]
    {
        // A link that leads nowhere yet would be followed by a write.
        let target = outside.path().join("new.png");
        std::os::unix::fs::symlink(&target, dir.join("shot.png")).unwrap();
        assert_eq!(
            refused(
                &client,
                "frame_render",
                json!({"times": [{"frame": 0}], "save_to": dir.join("shot.png"), "overwrite": true})
            )
            .await,
            "outside_roots"
        );
        // Numbered files are each checked, before anything is drawn.
        std::os::unix::fs::symlink(&target, dir.join("seq-02.png")).unwrap();
        assert_eq!(
            refused(
                &client,
                "frame_render",
                json!({"times": [{"frame": 0}, {"frame": 1}], "save_to": dir.join("seq.png"), "overwrite": true})
            )
            .await,
            "outside_roots"
        );
        assert!(!target.exists(), "nothing was written outside the roots");
        assert!(!dir.join("seq-01.png").exists());
        // A folder with a link to media outside: the link is not read.
        let folder = dir.join("folder");
        std::fs::create_dir(&folder).unwrap();
        let secret = outside.path().join("secret.mp4");
        std::fs::copy(&media.b, &secret).unwrap();
        std::os::unix::fs::symlink(&secret, folder.join("secret.mp4")).unwrap();
        std::fs::copy(&media.b, folder.join("b.mp4")).unwrap();
        let walked = call(&client, "media_import", json!({"paths": [folder]})).await;
        assert_eq!(walked["media"].as_array().unwrap().len(), 1);
        let refusal = walked["errors"].to_string();
        assert!(refusal.contains("outside"), "{refusal}");
        assert!(
            !refusal.contains(&outside.path().display().to_string()),
            "where the link leads is not told: {refusal}"
        );
    }
    client.cancel().await.unwrap();
}

/// One HTTP/1.1 POST, answered with its status code.
fn post(address: &str, headers: &[(&str, &str)]) -> u16 {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !headers.iter().any(|(name, _)| *name == "Host") {
        request.push_str(&format!("Host: {address}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = [0u8; 12];
    stream.read_exact(&mut answer).unwrap();
    String::from_utf8_lossy(&answer[9..12]).parse().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn http_answers_only_its_token_on_the_loopback_and_works_for_a_client() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let token = "a-test-token-that-is-long-enough-0123456789";
    let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_aster-mcp"))
        .args(server_args(&dir))
        .args(["--http", "127.0.0.1:0"])
        .env("ASTER_MCP_TOKEN", token)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let connections = dir.join("settings/mcp");
    let file = connections.join(format!("{}.json", server.id()));
    let mut found = None;
    for _ in 0..100 {
        if let Some(info) = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        {
            found = Some(info);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let info = found.expect("the connection file is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let url = info["url"].as_str().unwrap().to_owned();
    let address = url
        .trim_start_matches("http://")
        .trim_end_matches("/mcp")
        .to_owned();

    let bearer = format!("Bearer {token}");
    assert_eq!(post(&address, &[]), 401);
    assert_eq!(post(&address, &[("Authorization", "Bearer wrong")]), 401);
    assert_eq!(
        post(
            &address,
            &[("Authorization", &bearer), ("Host", "evil.example")]
        ),
        403
    );
    assert_eq!(
        post(
            &address,
            &[
                ("Authorization", &bearer),
                ("Origin", "https://evil.example")
            ]
        ),
        403
    );
    assert_eq!(post(&address, &[("Authorization", &bearer)]), 200);

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    );
    let client = ().serve(transport).await.expect("an HTTP session starts");
    call(
        &client,
        "project_create",
        json!({"path": dir.join("h.aster"), "name": "Over HTTP"}),
    )
    .await;
    let status = call(&client, "project_status", json!({})).await;
    assert_eq!(status["name"], "Over HTTP");
    client.cancel().await.unwrap();

    // Stopped as a service manager stops it; Windows has no SIGTERM.
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &server.id().to_string()])
            .status();
        let _ = server.wait();
        assert!(!file.exists(), "the connection file goes with the server");
    }
    #[cfg(not(unix))]
    {
        let _ = server.kill();
        let _ = server.wait();
    }
    assert!(editor_store::load_project(&dir.join("h.aster")).is_ok());
}

/// The same project in an Aster window and over MCP: the agent's edits
/// appear in the window, and the window's own edits in what the agent reads.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_edits_the_project_open_in_a_window() {
    if std::env::var_os("ASTER_TEST_DESKTOP").is_none() {
        eprintln!("skipped: set ASTER_TEST_DESKTOP=1 on a computer with a display");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("shared.aster");
    // The agent makes the project; a person opens it in a window.
    let (client, _) = stdio(&dir).await;
    call(&client, "project_create", json!({"path": project})).await;
    let imported = call(
        &client,
        "media_import",
        json!({"paths": [media.a, media.b]}),
    )
    .await;
    call(
        &client,
        "clip_add",
        json!({"asset_id": imported["media"][0]["asset_id"]}),
    )
    .await;
    client.cancel().await.unwrap();

    let settings = dir.join("settings/settings.json");
    let mut window = std::process::Command::new(env!("CARGO_BIN_EXE_aster"))
        .arg("--settings")
        .arg(&settings)
        .arg(&project)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (client, _) = stdio(&dir).await;
    let mut listed = Value::Null;
    for _ in 0..90 {
        listed = call(&client, "desktop", json!({"op": "list"})).await;
        if listed["windows"].as_array().is_some_and(|w| !w.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        listed["windows"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );

    let opened = call(&client, "project_open", json!({"path": project})).await;
    assert_eq!(opened["authority"], "desktop");
    let added = call(
        &client,
        "clip_add",
        json!({"asset_id": imported["media"][1]["asset_id"]}),
    )
    .await;
    assert_eq!(added["authority"], "desktop");
    let shown = timeline(&client).await;
    assert_eq!(
        shown["clips"].as_array().unwrap().len(),
        2,
        "the window has the agent's clip"
    );
    let frame = call(
        &client,
        "frame_render",
        json!({"times": [{"seconds": 5}], "size": 320}),
    )
    .await;
    assert_eq!(images(&frame), ["image/png"]);
    call(&client, "history", json!({"op": "undo"})).await;
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        1
    );
    call(&client, "history", json!({"op": "redo"})).await;
    let saved = call(&client, "project_save", json!({})).await;
    assert_eq!(saved["dirty"], false, "{saved}");
    client.cancel().await.unwrap();

    let opened = editor_store::load_project(&project).unwrap();
    assert_eq!(
        opened
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum::<usize>(),
        2
    );
    let _ = window.kill();
    let _ = window.wait();
}
