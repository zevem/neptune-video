//! `neptune-video-mcp` end to end: the real server binary, driven by the official
//! Rust MCP client over stdio and Streamable HTTP, on media FFmpeg makes in
//! a task-owned folder. The window test runs with NEPTUNE_VIDEO_TEST_DESKTOP=1 on a
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

/// The FFmpeg the editor would run: NEPTUNE_VIDEO_FFMPEG, the copy fetched into
/// the checkout, or the one on PATH.
fn ffmpeg() -> PathBuf {
    if let Some(program) = std::env::var_os("NEPTUNE_VIDEO_FFMPEG") {
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
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_neptune-video-mcp"));
    command
        .args(server_args(dir))
        .current_dir(dir)
        .stderr(Stdio::null());
    let transport = TokioChildProcess::new(command).expect("neptune-video-mcp starts");
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
async fn an_mcp_server_closes_its_render_workers_before_process_exit() {
    for variant in ["frame", "export", "both"] {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let source = make(
            &dir,
            "shutdown.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30:duration=0.2",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ],
        );
        let stderr = dir.join("server.stderr");
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_neptune-video-mcp"))
            .args(server_args(&dir))
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        // Retain the real child: TokioChildProcess's graceful-shutdown wrapper
        // discards its exit status, which hid native render-worker drop crashes.
        let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
        let client = ().serve(transport).await.unwrap();
        call(
            &client,
            "project_create",
            json!({"path": dir.join("shutdown.nvp"), "width": 160, "height": 90}),
        )
        .await;
        let imported = call(&client, "media_import", json!({"paths": [source]})).await;
        call(
            &client,
            "clip_add",
            json!({"asset_id": imported["media"][0]["asset_id"]}),
        )
        .await;
        if variant != "export" {
            call(
                &client,
                "frame_render",
                json!({"times": [{"frame": 0}], "size": 160}),
            )
            .await;
        }
        if variant != "frame" {
            let queued = call(
                &client,
                "render_export_start",
                json!({"destination": dir.join("rendered.mp4"), "preset": "small-file"}),
            )
            .await;
            let done = wait(&client, &queued).await;
            assert_eq!(done["state"], "done", "{variant}: {done}");
        }
        client.cancel().await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(20), child.wait())
            .await
            .expect("the owned server exits without a timeout")
            .unwrap();
        assert!(
            status.success(),
            "{variant} server exit {status}; {}",
            std::fs::read_to_string(&stderr).unwrap()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mix_loudness_measures_the_export_mix_without_editing_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let tone = make(
        &dir,
        "tone.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=2",
            "-c:a",
            "pcm_s16le",
        ],
    );
    let (client, _) = stdio(&dir).await;
    call(
        &client,
        "project_create",
        json!({"path": dir.join("mix.nvp")}),
    )
    .await;
    let imported = call(&client, "media_import", json!({"paths": [tone]})).await;
    let asset = id(&imported["media"][0]["asset_id"]);
    let first = call(&client, "clip_add", json!({"asset_id": &asset[..8]})).await;
    let clip = id(&first["created"][0]);
    let tracks = timeline(&client).await;
    let first_track = id(&tracks["clips"][0]["track_id"]);
    let added = call(
        &client,
        "track_edit",
        json!({"op": "add", "kind": "audio", "name": "Second"}),
    )
    .await;
    let second_track = id(&added["created"][0]);
    call(
        &client,
        "clip_add",
        json!({"asset_id": asset, "track_id": second_track, "at": {"seconds": 0}}),
    )
    .await;
    let before = revision(&client).await;
    let uses = call(
        &client,
        "media_usage",
        json!({"asset_id": &asset[..8], "limit": 1}),
    )
    .await;
    assert_eq!(uses["asset_id"], asset);
    assert_eq!(uses["total"], 2);
    assert_eq!(uses["uses"].as_array().unwrap().len(), 1);
    assert_eq!(uses["next_offset"], 1);
    let probe = call(&client, "media_inspect", json!({"asset_id": &asset[..8]})).await;
    assert_eq!(probe["asset"]["asset_id"], asset);
    for dry_run in [false, true] {
        assert_eq!(refused(&client, "edit_apply", json!({"commands": [{"type": "rename_project", "name": "Stale"}], "expected_revision": before - 1, "dry_run": dry_run})).await, "stale_revision");
    }
    assert_eq!(
        refused(
            &client,
            "media_import",
            json!({"paths": [tone], "expected_revision": before - 1})
        )
        .await,
        "stale_revision"
    );
    let mix = wait(
        &client,
        &call(
            &client,
            "loudness_start",
            json!({"scope": "timeline", "per_track": true}),
        )
        .await,
    )
    .await;
    assert_eq!(mix["state"], "done", "{mix}");
    let result = &mix["result"];
    assert_eq!(result["revision"], before);
    let measured = |v: &Value| v["integrated_lufs"].as_f64().expect("measured LUFS");
    let track = result["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["track_id"] == first_track)
        .unwrap();
    assert!(
        (measured(result) - measured(track) - 6.0).abs() < 0.4,
        "two identical voices add 6 dB: {result}"
    );
    assert!(result["true_peak_dbfs"].as_f64().unwrap().is_finite());
    assert_eq!(revision(&client).await, before, "measurement never edits");
    call(
        &client,
        "audio_edit",
        json!({"op": "track", "track_id": second_track, "muted": true}),
    )
    .await;
    let muted = wait(
        &client,
        &call(&client, "loudness_start", json!({"scope": "mix"})).await,
    )
    .await;
    assert!(
        (measured(&muted["result"]) - measured(track)).abs() < 0.2,
        "muted voices are excluded: {muted}"
    );
    assert_eq!(
        refused(
            &client,
            "audio_edit",
            json!({"op": "normalize", "clip_ids": [clip], "job_id": mix["job_id"]})
        )
        .await,
        "invalid"
    );
    call(
        &client,
        "audio_edit",
        json!({"op": "track", "track_id": first_track, "muted": true}),
    )
    .await;
    let silent = wait(
        &client,
        &call(
            &client,
            "loudness_start",
            json!({"scope": "timeline", "from": {"seconds": 0.5}, "to": {"seconds": 1.5}}),
        )
        .await,
    )
    .await;
    assert_eq!(silent["result"]["status"], "silent", "{silent}");
    assert!(silent["result"]["integrated_lufs"].is_null());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_inspects_and_patches_live_clip_state_with_stable_ids() {
    let dir = tempfile::tempdir().unwrap();
    let (client, _) = stdio(dir.path()).await;
    call(
        &client,
        "project_create",
        json!({"name": "Unsaved live state"}),
    )
    .await;
    let added = call(
        &client,
        "text_add",
        json!({"text": "Presenter", "at": {"seconds": 0}, "duration": {"seconds": 4}}),
    )
    .await;
    let clip_id = id(&added["created"][0]);
    call(&client, "keyframes_set", json!({"clip_id": clip_id, "key": "opacity", "keys": [{"at": {"seconds": 0}, "value": 0}, {"at": {"seconds": 4}, "value": 1}]})).await;
    let at = revision(&client).await;
    let patched = call(
        &client,
        "patch_clip",
        json!({"clip_id": clip_id, "expected_revision": at,
            "start": {"seconds": 2}, "duration": {"seconds": 2.5},
            "changes": {"content": {"style": {"wrap": 600}}, "transform": {"opacity": 0.8}}
        }),
    )
    .await;
    assert_eq!(
        patched["created"],
        json!([]),
        "an in-place edit adds no IDs"
    );
    let inspect = call(
        &client,
        "project_inspect",
        json!({"scope": "clips", "clip_ids": [clip_id]}),
    )
    .await;
    let clip: editor_core::Clip =
        serde_json::from_value(inspect["clips"][0]["clip"].clone()).unwrap();
    assert_eq!(clip.id.to_string(), clip_id);
    assert_eq!(clip.start.0, 2_000_000);
    assert_eq!(clip.duration.0, 2_500_000);
    assert_eq!(clip.animation[0].keys.last().unwrap().at.0, 2_500_000);
    assert!((clip.animation[0].keys.last().unwrap().value - 0.625).abs() < 0.001);
    let editor_core::ClipContent::Text { style, .. } = clip.content else {
        panic!("text")
    };
    assert_eq!(style.wrap, 600.0);
    let changes = call(
        &client,
        "history",
        json!({"op": "changes_since", "since_revision": at}),
    )
    .await;
    assert_eq!(changes["changes"]["complete"], true);
    let objects = changes["changes"]["entries"][0]["objects"]
        .as_array()
        .unwrap();
    assert!(
        objects
            .iter()
            .any(|object| object["id"] == clip_id && object["fields"]["duration"] == 2_500_000)
    );
    let shifted = call(
        &client,
        "offset_clips",
        json!({"clip_ids": [clip_id], "delta": {"seconds": -0.5}}),
    )
    .await;
    let shifted_revision = shifted["revision"].as_u64().unwrap();
    let state = call(
        &client,
        "project_inspect",
        json!({"scope": "timeline", "clip_ids": [clip_id], "fields": ["start"]}),
    )
    .await;
    assert_eq!(state["clips"][0]["start"]["us"], 1_500_000);
    assert_eq!(
        refused(
            &client,
            "patch_clip",
            json!({"clip_id": clip_id, "changes": {"transform": {"opactiy": 0.5}}})
        )
        .await,
        "invalid_edit"
    );
    assert_eq!(revision(&client).await, shifted_revision);
    call(
        &client,
        "history",
        json!({"op": "undo", "expected_revision": shifted_revision}),
    )
    .await;
    let restored = call(
        &client,
        "project_inspect",
        json!({"scope": "clips", "clip_ids": [clip_id]}),
    )
    .await;
    assert_eq!(restored["clips"][0]["clip"]["start"], 2_000_000);
    assert!(
        call(&client, "project_status", json!({})).await["path"].is_null(),
        "live inspection never requires a saved project file"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_makes_edits_looks_at_and_exports_a_video_that_a_person_can_open() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("film.nvp");
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
        .read_resource(ReadResourceRequestParams::new("neptune-video://project"))
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
        "patch_clip",
        "offset_clips",
        "media_usage",
        "frames_export",
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
    eprintln!("tools/list: {listed} bytes for {} tools", tools.len());
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
            json!({"path": outside.path().join("x.nvp")})
        )
        .await,
        "outside_roots"
    );
    call(
        &client,
        "project_create",
        json!({"path": dir.join("p.nvp")}),
    )
    .await;
    assert_eq!(
        refused(&client, "media_import", json!({"paths": [secret]})).await,
        "outside_roots"
    );
    // Out of the root and into the folder beside it, which holds the secret.
    // Joined, not written out: in a canonical Windows path only `\` parts it.
    let climbing = dir
        .join("..")
        .join(outside.path().file_name().unwrap())
        .join("secret.mp4");
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
            json!({"path": dir.join("p.nvp")})
        )
        .await,
        "destination_exists"
    );
    assert_eq!(
        refused(
            &client,
            "project_save",
            json!({"destination": outside.path().join("copy.nvp")})
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
    let project = dir.join("p.nvp");
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
    let project = dir.join("p.nvp");
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
            json!({"path": dir.join("other.nvp")})
        )
        .await,
        "invalid"
    );
    assert!(!dir.join("other.nvp").exists());
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
        json!({"path": dir.join("p.nvp"), "name": "A"}),
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
    let media = media(&dir);
    let project = dir.join("p.nvp");
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
        let outside = tempfile::tempdir().unwrap();
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
    let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_neptune-video-mcp"))
        .args(server_args(&dir))
        .args(["--http", "127.0.0.1:0"])
        .env("NEPTUNE_VIDEO_MCP_TOKEN", token)
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
        json!({"path": dir.join("h.nvp"), "name": "Over HTTP"}),
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
    assert!(editor_store::load_project(&dir.join("h.nvp")).is_ok());
}

/// The same project in a Neptune Video window and over MCP: the agent's edits
/// appear in the window, and the window's own edits in what the agent reads.
/// A window saves on its own time: waits until `project_status` says its
/// save to `path` is over, and gives that status.
async fn window_saved(client: &Client, path: &Path) -> Value {
    let mut status = Value::Null;
    for _ in 0..120 {
        status = call(client, "project_status", json!({})).await;
        if status["saving"] == false && status["last_save"]["path"] == json!(path) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(status["last_save"]["ok"], true, "{status}");
    status
}

/// A window a test started, closed however the test ends.
struct Window(std::process::Child);

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_edits_the_project_open_in_a_window() {
    if std::env::var_os("NEPTUNE_VIDEO_TEST_DESKTOP").is_none() {
        eprintln!("skipped: set NEPTUNE_VIDEO_TEST_DESKTOP=1 on a computer with a display");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("shared.nvp");
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
    let _window = Window(
        std::process::Command::new(env!("CARGO_BIN_EXE_neptune-video"))
            .arg("--settings")
            .arg(&settings)
            .arg(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
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
    call(&client, "project_save", json!({})).await;
    let saved = window_saved(&client, &project).await;
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
}

/// The server Neptune Video's own Agent tab starts: pinned to one window's session
/// and naming its requests with the tab's tag.
async fn pinned(dir: &Path, session: &Path) -> Client {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_neptune-video-mcp"));
    command
        .args(server_args(dir))
        .arg("--window")
        .arg(session)
        .args(["--tag", "neptune-video-tab"])
        .current_dir(dir)
        .stderr(Stdio::null());
    let transport = TokioChildProcess::new(command).expect("neptune-video-mcp starts");
    ().serve(transport).await.expect("an MCP session starts")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_started_for_a_closed_window_still_makes_projects_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let client = pinned(&dir, &dir.join("settings/sessions/gone.json")).await;
    let status = call(&client, "project_status", json!({})).await;
    assert_eq!(status["authority"], "none", "{status}");
    // It follows its window, which does not answer.
    assert_eq!(
        refused(&client, "project_inspect", json!({})).await,
        "desktop_closed"
    );
    assert_eq!(
        refused(&client, "desktop", json!({"op": "attach"})).await,
        "desktop_closed"
    );
    // Like any server, it makes a project of its own and stays with it.
    let other = dir.join("other.nvp");
    let made = call(&client, "project_create", json!({"path": other})).await;
    assert_eq!(made["authority"], "local", "{made}");
    call(&client, "track_edit", json!({"op": "add", "kind": "video"})).await;
    assert_eq!(
        call(&client, "project_status", json!({})).await["authority"],
        "local"
    );
    call(&client, "project_close", json!({})).await;
    // Once it let go by choice it does not go looking for the window.
    let status = call(&client, "project_status", json!({})).await;
    assert_eq!(status["authority"], "none", "{status}");
    let opened = call(&client, "project_open", json!({"path": other})).await;
    assert_eq!(opened["authority"], "local", "{opened}");
    client.cancel().await.unwrap();
    assert!(other.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folders_in_a_roots_file_are_read_again_when_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let (work, more) = (dir.join("work"), dir.join("more"));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&more).unwrap();
    let footage = more.join("b.mp4");
    std::fs::copy(&media.b, &footage).unwrap();
    let file = dir.join("agent.roots");
    let write = |roots: &[&Path]| {
        let part = dir.join("agent.roots.part");
        std::fs::write(&part, serde_json::to_vec(&json!({"roots": roots})).unwrap()).unwrap();
        std::fs::rename(&part, &file).unwrap();
    };
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_neptune-video-mcp"));
    command
        .arg("--roots-file")
        .arg(&file)
        .arg("--desktop-settings")
        .arg(dir.join("settings/settings.json"))
        .arg("--models")
        .arg(dir.join("models"))
        .current_dir(&dir)
        .stderr(Stdio::null());
    let client =
        ().serve(TokioChildProcess::new(command).expect("neptune-video-mcp starts"))
            .await
            .expect("an MCP session starts");
    // No file yet: no folder, not even the one it was started in.
    let project = work.join("roots.nvp");
    assert_eq!(
        refused(&client, "project_create", json!({"path": project})).await,
        "outside_roots"
    );
    write(&[&work]);
    call(&client, "project_create", json!({"path": project})).await;
    assert_eq!(
        refused(&client, "media_import", json!({"paths": [footage]})).await,
        "outside_roots"
    );
    // A folder named later is usable at once, in the same session; one
    // that is too wide is left out.
    write(&[&work, &more, Path::new("/")]);
    let imported = call(&client, "media_import", json!({"paths": [footage]})).await;
    assert_eq!(imported["media"].as_array().unwrap().len(), 1, "{imported}");
    let status = call(&client, "project_status", json!({})).await;
    assert_eq!(status["roots"], json!([work, more]), "{status}");
    // A folder taken out is refused again.
    write(&[&work]);
    assert_eq!(
        refused(&client, "media_inspect", json!({"path": footage})).await,
        "outside_roots"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_tab_server_starts_in_its_window_and_may_leave_and_come_back() {
    if std::env::var_os("NEPTUNE_VIDEO_TEST_DESKTOP").is_none() {
        eprintln!("skipped: set NEPTUNE_VIDEO_TEST_DESKTOP=1 on a computer with a display");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let media = media(&dir);
    let project = dir.join("tab.nvp");
    let (client, _) = stdio(&dir).await;
    call(&client, "project_create", json!({"path": project})).await;
    let imported = call(&client, "media_import", json!({"paths": [media.a]})).await;
    client.cancel().await.unwrap();

    let _window = Window(
        std::process::Command::new(env!("CARGO_BIN_EXE_neptune-video"))
            .arg("--settings")
            .arg(dir.join("settings/settings.json"))
            .arg(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let sessions = dir.join("settings/sessions");
    let mut session = None;
    for _ in 0..90 {
        session = std::fs::read_dir(&sessions).ok().and_then(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        });
        if session.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let session = session.expect("the window publishes its session");

    let client = pinned(&dir, &session).await;
    let added = call(
        &client,
        "clip_add",
        json!({"asset_id": imported["media"][0]["asset_id"]}),
    )
    .await;
    assert_eq!(added["authority"], "desktop", "{added}");
    call(&client, "history", json!({"op": "undo"})).await;
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        0
    );
    call(&client, "history", json!({"op": "redo"})).await;
    let copy = dir.join("copy.nvp");
    call(&client, "project_save", json!({"destination": copy})).await;
    window_saved(&client, &copy).await;
    let saved = editor_store::load_project(&copy).unwrap();
    assert_eq!(saved.tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 1);

    // It may leave the window for a project of its own, as any agent may…
    let other = dir.join("other.nvp");
    let made = call(&client, "project_create", json!({"path": other})).await;
    assert_eq!(made["authority"], "local", "{made}");
    let theirs = call(&client, "media_import", json!({"paths": [media.a]})).await;
    call(
        &client,
        "clip_add",
        json!({"asset_id": theirs["media"][0]["asset_id"]}),
    )
    .await;
    // …which leaves the window's project as it was…
    let window_project = editor_store::load_project(&project).unwrap();
    assert_eq!(
        window_project
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        0,
        "nothing was saved to the window's file"
    );
    // …and come back to its window by naming none.
    let back = call(&client, "desktop", json!({"op": "attach"})).await;
    assert_eq!(back["status"]["authority"], "desktop", "{back}");
    assert_eq!(
        timeline(&client).await["clips"].as_array().unwrap().len(),
        1,
        "the window still has the agent's clip"
    );
    client.cancel().await.unwrap();
    let other = editor_store::load_project(&other).unwrap();
    assert_eq!(other.tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn exact_source_times_isolated_layers_and_streaming_dumps_preserve_the_project() {
    use editor_core::{Clip, Command, Effect, EffectKind, ParamKey, ParamValue, Time};

    fn picture(value: &Value) -> &str {
        value["_content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|block| block["type"] == "image")
            .unwrap()["data"]
            .as_str()
            .unwrap()
    }

    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let source = make(
        &dir,
        "sixty.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=60:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
    );
    let (client, _) = stdio(&dir).await;
    let path = dir.join("layers.nvp");
    call(
        &client,
        "project_create",
        json!({"path": path, "width": 160, "height": 90, "frame_rate": 30}),
    )
    .await;
    let imported = call(&client, "media_import", json!({"paths": [source]})).await;
    let asset_id = id(&imported["media"][0]["asset_id"]);
    let stored = editor_store::load_project(&path).unwrap();
    let asset = &stored.assets[&asset_id.parse().unwrap()];
    let mut placed = Clip::from_asset(asset, Time::ZERO);
    placed.source_in = Time(16_667);
    placed.duration = Time::from_seconds(1.0);
    let clip_id = placed.id;
    let track_id = stored.tracks[0].id;
    call(
        &client,
        "edit_apply",
        json!({"commands": [Command::AddClip { track_id, clip: placed }]}),
    )
    .await;

    let exact = call(
        &client,
        "frame_render",
        json!({
            "asset_id": &asset_id[..8], "times": [{"us": 16667}], "size": 160,
            "save_to": dir.join("new/nested/source.png"),
        }),
    )
    .await;
    let source_frame = call(
        &client,
        "frame_render",
        json!({"asset_id": asset_id, "times": [{"frame": 1}], "size": 160}),
    )
    .await;
    assert_eq!(exact["frames"][0]["at"]["us"], 16667);
    assert_eq!(exact["frames"][0]["at"]["frame"], 1);
    assert!(
        picture(&exact) == picture(&source_frame),
        "a source frame uses the 60 fps source grid"
    );
    let off_grid_clip = call(
        &client,
        "frame_render",
        json!({"solo_clip": clip_id, "times": [{"us": 0}], "size": 160}),
    )
    .await;
    assert!(
        picture(&exact) == picture(&off_grid_clip),
        "source render matches an off-project-grid clip in-point"
    );
    assert!(dir.join("new/nested/source.png").is_file());

    call(&client, "clip_edit", json!({"op": "transform", "clip_ids": [clip_id], "transform": {"scale": [0.7, 0.7], "position": [10.0, 0.0]}})).await;
    let mut grade = Effect::new(EffectKind::Grade);
    grade.params = vec![ParamValue {
        key: ParamKey::Exposure,
        value: -1.0,
    }];
    call(
        &client,
        "edit_apply",
        json!({"commands": [Command::SetEffects { clip_id, effects: vec![grade] }]}),
    )
    .await;
    let graded = call(
        &client,
        "frame_render",
        json!({"times": [{"us": 0}], "size": 160}),
    )
    .await;
    assert!(
        picture(&exact) != picture(&graded),
        "transform and grade affect the picture"
    );
    call(
        &client,
        "track_edit",
        json!({"op": "set", "track_id": track_id, "hidden": true}),
    )
    .await;
    let before = revision(&client).await;
    let hidden = call(
        &client,
        "frame_render",
        json!({"times": [{"us": 0}], "size": 160}),
    )
    .await;
    assert!(picture(&hidden) != picture(&graded));
    let isolated = call(
        &client,
        "frame_render",
        json!({"solo_clip": clip_id, "times": [{"us": 0}], "size": 160}),
    )
    .await;
    let only_track = call(
        &client,
        "frame_render",
        json!({"only_tracks": [track_id], "times": [{"us": 0}], "size": 160}),
    )
    .await;
    assert!(
        picture(&isolated) == picture(&graded),
        "isolation keeps the transform and grade"
    );
    assert!(picture(&only_track) == picture(&graded));
    let excluded = call(
        &client,
        "frame_render",
        json!({"exclude_tracks": [track_id], "times": [{"us": 0}], "size": 160}),
    )
    .await;
    assert!(picture(&excluded) == picture(&hidden));

    let sheet = call(
        &client,
        "contact_sheet",
        json!({
            "solo_clip": clip_id, "times": [{"us": 16667}, {"us": 100000}], "cell_size": 64,
            "save_to": dir.join("sheet/new/contact.jpg"),
        }),
    )
    .await;
    assert_eq!(sheet["cells"].as_array().unwrap().len(), 2);
    assert_eq!(sheet["cells"][0]["at"]["us"], 16667);
    assert_eq!(sheet["cells"][1]["at"]["us"], 100000);
    assert!(dir.join("sheet/new/contact.jpg").is_file());

    let dump = wait(
        &client,
        &call(
            &client,
            "frames_export",
            json!({
                "asset_id": asset_id, "from": {"us": 0}, "to": {"seconds": 1},
                "size": 64, "destination": dir.join("dump/new"),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(dump["state"], "done", "{dump}");
    assert_eq!(
        dump["result"]["count"], 60,
        "one job writes more than the inline 48-frame limit"
    );
    assert_eq!(std::fs::read_dir(dir.join("dump/new")).unwrap().count(), 60);
    assert!(dir.join("dump/new/frame-000060.png").is_file());
    let solo_dump = wait(
        &client,
        &call(
            &client,
            "frames_export",
            json!({
                "solo_clip": clip_id, "from": {"us": 0}, "to": {"us": 33333},
                "size": 160, "destination": dir.join("isolated"),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(solo_dump["state"], "done", "{solo_dump}");
    let encoded =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, picture(&graded))
            .unwrap();
    assert!(
        std::fs::read(dir.join("isolated/frame-000001.png")).unwrap() == encoded,
        "an isolated frame dump must match the same clip's transformed and graded still"
    );
    assert_eq!(
        revision(&client).await,
        before,
        "picture work changes no editing revision"
    );
    let stored = editor_store::load_project(&path).unwrap();
    assert!(
        stored.track(track_id).unwrap().hidden,
        "picture work preserves the person's visibility"
    );
    client.cancel().await.unwrap();
}
