use editor_core::{Asset, Clip, Command, Editor, FrameRate, MediaFolder, MediaKind, Project, Time};
use editor_store::{
    SCHEMA_VERSION, StoreError, checkpoint, duplicate_project, list_checkpoints, load_project,
    load_recovery, recovery_path, remove_recovery, save_project, save_recovery,
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;
use uuid::Uuid;

fn project_fixture(root: &Path) -> Project {
    let mut project = Project::new("Offline footage");
    project.settings.frame_rate = FrameRate {
        numerator: 30_000,
        denominator: 1_001,
    };
    let folder = MediaFolder {
        id: Uuid::new_v4(),
        name: "Sources".into(),
        parent_id: None,
    };
    let mut asset = Asset::new(
        root.join("missing-source.mov"),
        MediaKind::Video,
        Time::from_seconds(20.0),
    );
    asset.folder_id = Some(folder.id);
    let mut clip = Clip::from_asset(&asset, Time::from_seconds(1.0));
    clip.duration = Time::from_seconds(5.0);
    clip.source_in = Time::from_seconds(3.0);
    Arc::make_mut(&mut project.folders).insert(folder.id, folder);
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    project
}

fn rename(editor: &mut Editor, name: &str) {
    editor
        .dispatch(Command::RenameProject { name: name.into() })
        .unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn project_round_trip_preserves_offline_media_and_fractional_frame_settings() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let project = project_fixture(directory.path());

    save_project(&path, &project).unwrap();

    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn successful_replacement_contains_only_the_latest_complete_project() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = project_fixture(directory.path());
    save_project(&path, &project).unwrap();
    project.name = "Replacement".into();

    save_project(&path, &project).unwrap();

    assert_eq!(load_project(&path).unwrap(), project);
    // A persistent write-lock sidecar keeps all cooperating replacements on
    // one inode; it is not a leaked temporary project.
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn invalid_incoming_project_leaves_the_saved_project_unchanged() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = Project::default();
    save_project(&path, &project).unwrap();
    let original = fs::read(&path).unwrap();
    project.settings.width = 0;

    assert!(save_project(&path, &project).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn truncated_existing_project_is_preserved_byte_for_byte() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let original = b"{\"schema_version\":1,\"project\":";
    fs::write(&path, original).unwrap();

    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn semantically_damaged_existing_project_is_preserved_byte_for_byte() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    save_project(&path, &Project::default()).unwrap();
    let mut document = read_json(&path);
    document["project"]["settings"]["width"] = json!(0);
    write_json(&path, &document);
    let original = fs::read(&path).unwrap();

    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn header_without_a_valid_document_cannot_be_overwritten() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    fs::write(&path, b"{\"schema_version\":1}").unwrap();
    let original = fs::read(&path).unwrap();

    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn future_project_with_a_new_shape_cannot_be_overwritten() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let original = format!(
        "{{\"schema_version\":{},\"future_document\":{{\"valuable\":true}}}}",
        SCHEMA_VERSION + 1
    );
    fs::write(&path, &original).unwrap();

    assert!(matches!(
        save_project(&path, &Project::default()),
        Err(StoreError::FutureSchema(_))
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn loading_a_future_project_rejects_its_schema() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    save_project(&path, &Project::default()).unwrap();
    let mut document = read_json(&path);
    document["schema_version"] = json!(SCHEMA_VERSION + 1);
    write_json(&path, &document);

    assert!(matches!(
        load_project(&path),
        Err(StoreError::FutureSchema(_))
    ));
}

#[test]
fn oversized_project_is_rejected_before_reading_its_body() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    fs::File::create(&path)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();

    assert!(matches!(load_project(&path), Err(StoreError::Invalid(_))));
}

#[test]
fn relative_asset_paths_resolve_from_the_project_directory() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = project_fixture(directory.path());
    let asset_id = *project.assets.keys().next().unwrap();
    Arc::make_mut(&mut project.assets)
        .get_mut(&asset_id)
        .unwrap()
        .path = "media/日本語.mov".into();
    save_project(&path, &project).unwrap();

    let restored = load_project(&path).unwrap();

    assert_eq!(
        restored.assets[&asset_id].path,
        directory
            .path()
            .canonicalize()
            .unwrap()
            .join("media/日本語.mov")
    );
}

#[test]
fn failure_to_create_a_destination_preserves_the_blocking_file() {
    let directory = TempDir::new().unwrap();
    let blocking_file = directory.path().join("file-instead-of-directory");
    fs::write(&blocking_file, b"Keep these bytes").unwrap();
    let path = blocking_file.join("project.aster");

    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(blocking_file).unwrap(), b"Keep these bytes");
}

#[test]
fn checkpoints_keep_named_snapshots_without_changing_the_project_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = project_fixture(directory.path());
    save_project(&path, &project).unwrap();
    let original = fs::read(&path).unwrap();
    let first = checkpoint(&path, &project, "First assembly").unwrap();
    project.name = "Revised assembly".into();
    let second = checkpoint(&path, &project, "Revised assembly").unwrap();

    let checkpoints = list_checkpoints(&path).unwrap();

    assert_eq!(checkpoints.len(), 2);
    assert!(
        checkpoints
            .iter()
            .any(|entry| { entry.path == first && entry.label == "First assembly" })
    );
    assert_eq!(load_project(&second).unwrap(), project);
    assert_eq!(load_project(&first).unwrap().name, "Offline footage");
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn duplicate_has_independent_identity_and_keeps_original_source_references() {
    let directory = TempDir::new().unwrap();
    let original = project_fixture(directory.path());

    let duplicate = duplicate_project(&original, "Independent copy");

    assert_ne!(duplicate.id, original.id);
    assert_eq!(duplicate.name, "Independent copy");
    assert!(Arc::ptr_eq(&duplicate.assets, &original.assets));
    assert_eq!(duplicate.tracks, original.tracks);
    assert!(duplicate.validate().is_ok());
}

#[test]
fn recovery_restores_redo_chain_saved_identity_and_dirty_state() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = Editor::new(project_fixture(directory.path())).unwrap();
    rename(&mut editor, "Saved edit");
    let saved_revision = editor.register_save();
    editor.mark_saved(saved_revision);
    rename(&mut editor, "Second edit");
    rename(&mut editor, "Third edit");
    assert!(editor.undo());
    let revision = editor.revision();
    save_recovery(&path, &editor).unwrap();

    let mut restored = load_recovery(&path).unwrap();

    assert_eq!(restored.revision(), revision);
    assert!(restored.is_dirty());
    assert!(restored.can_redo());
    assert_eq!(restored.project().name, "Second edit");
    assert!(restored.undo());
    assert_eq!(restored.project().name, "Saved edit");
    assert!(!restored.is_dirty());
    assert!(restored.redo());
    assert!(restored.redo());
    assert_eq!(restored.project().name, "Third edit");
}

#[test]
fn recovery_rehydrates_shared_document_roots_tracks_and_asset_maps() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = Editor::new(project_fixture(directory.path())).unwrap();
    rename(&mut editor, "First rename");
    rename(&mut editor, "Second rename");
    save_recovery(&path, &editor).unwrap();

    let restored = load_recovery(&path).unwrap();
    let state = restored.session_state();

    assert!(Arc::ptr_eq(&state.past[0].after, &state.past[1].before));
    assert!(Arc::ptr_eq(&state.current, &state.past[1].after));
    assert!(Arc::ptr_eq(
        &state.past[0].before.assets,
        &state.current.assets
    ));
    assert!(Arc::ptr_eq(
        &state.past[0].before.tracks[0],
        &state.current.tracks[0]
    ));
}

#[test]
fn broken_recovery_reference_is_rejected_without_changing_the_archive() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    save_recovery(&path, &Editor::default()).unwrap();
    let archive_path = recovery_path(&path);
    let mut archive = read_json(&archive_path);
    archive["current"] = json!(usize::MAX);
    write_json(&archive_path, &archive);
    let original = fs::read(&archive_path).unwrap();

    assert!(load_recovery(&path).is_err());
    assert_eq!(fs::read(archive_path).unwrap(), original);
}

#[test]
fn recovery_with_an_incoherent_history_chain_is_rejected() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = Editor::default();
    rename(&mut editor, "First rename");
    rename(&mut editor, "Second rename");
    save_recovery(&path, &editor).unwrap();
    let archive_path = recovery_path(&path);
    let mut archive = read_json(&archive_path);
    archive["past"][0]["after"] = archive["current"].clone();
    write_json(&archive_path, &archive);

    assert!(load_recovery(&path).is_err());
}

#[test]
fn recovery_history_cannot_undo_into_a_different_project_identity() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = Editor::default();
    rename(&mut editor, "First edit");
    save_recovery(&path, &editor).unwrap();
    let archive_path = recovery_path(&path);
    let mut archive = read_json(&archive_path);
    let before_index = archive["past"][0]["before"].as_u64().unwrap() as usize;
    archive["projects"][before_index]["id"] = json!(Uuid::new_v4());
    write_json(&archive_path, &archive);

    assert!(load_recovery(&path).is_err());
}

#[test]
fn semantically_damaged_recovery_cannot_be_overwritten() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = Editor::default();
    save_recovery(&path, &editor).unwrap();
    let archive_path = recovery_path(&path);
    let mut archive = read_json(&archive_path);
    archive["current"] = json!(usize::MAX);
    write_json(&archive_path, &archive);
    let original = fs::read(&archive_path).unwrap();

    assert!(save_recovery(&path, &editor).is_err());
    assert_eq!(fs::read(archive_path).unwrap(), original);
}

#[test]
fn future_recovery_archive_is_preserved_on_save() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let archive_path = recovery_path(&path);
    write_json(
        &archive_path,
        &json!({"schema_version": SCHEMA_VERSION + 1, "future_history": true}),
    );
    let original = fs::read(&archive_path).unwrap();

    assert!(matches!(
        save_recovery(&path, &Editor::default()),
        Err(StoreError::FutureSchema(_))
    ));
    assert_eq!(fs::read(archive_path).unwrap(), original);
}

#[test]
fn loading_a_future_shape_reports_schema_before_decoding_the_document() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    write_json(
        &path,
        &json!({"schema_version": SCHEMA_VERSION + 1, "future_shape": true}),
    );

    assert!(matches!(
        load_project(&path),
        Err(StoreError::FutureSchema(_))
    ));
}

#[test]
fn unsupported_older_schema_is_preserved_instead_of_being_replaced() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    save_project(&path, &Project::default()).unwrap();
    let mut document = read_json(&path);
    document["schema_version"] = json!(0);
    write_json(&path, &document);
    let original = fs::read(&path).unwrap();

    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn recovery_resolves_relative_media_once_and_preserves_history_sharing() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = project_fixture(directory.path());
    let asset_id = *project.assets.keys().next().unwrap();
    Arc::make_mut(&mut project.assets)
        .get_mut(&asset_id)
        .unwrap()
        .path = "media/relative.mov".into();
    let mut editor = Editor::new(project).unwrap();
    rename(&mut editor, "Renamed with relative source");
    save_recovery(&path, &editor).unwrap();

    let restored = load_recovery(&path).unwrap();
    let state = restored.session_state();

    assert_eq!(
        restored.project().assets[&asset_id].path,
        directory
            .path()
            .canonicalize()
            .unwrap()
            .join("media/relative.mov")
    );
    assert!(Arc::ptr_eq(
        &state.past[0].before.assets,
        &state.current.assets
    ));
}

#[test]
fn recovery_cleanup_is_idempotent_and_leaves_the_project_file_intact() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = Editor::default();
    save_project(&path, editor.project()).unwrap();
    let original = fs::read(&path).unwrap();
    save_recovery(&path, &editor).unwrap();

    remove_recovery(&path).unwrap();
    remove_recovery(&path).unwrap();

    assert!(!recovery_path(&path).exists());
    assert_eq!(fs::read(path).unwrap(), original);
}
