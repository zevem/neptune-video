use editor_core::{Command, Editor, Project};
use editor_store::{
    DocumentSession, RecoveryLease, SaveWorker, StoreError, load_project,
    load_project_with_session, load_recovery, load_recovery_with_document, recovery_path,
    save_project, save_project_with_session, save_recovery, save_recovery_with_document,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(10);

fn opened(path: &Path) -> (Editor, Arc<DocumentSession>) {
    save_project(path, &Project::new("Ordinary baseline")).unwrap();
    let (project, document) = load_project_with_session(path).unwrap();
    (Editor::new(project).unwrap(), document)
}

fn rename(editor: &mut Editor, name: &str) {
    editor
        .dispatch(Command::RenameProject { name: name.into() })
        .unwrap();
}

fn archive_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(recovery_path(path)).unwrap()).unwrap()
}

fn write_archive(path: &Path, value: &Value) {
    fs::write(recovery_path(path), serde_json::to_vec(value).unwrap()).unwrap();
}

// The test executable is an actual independent ordinary-document writer. Only
// the parent supplies an owned temporary destination, and it waits for exit.
#[test]
fn recovery_independent_writer_child() {
    let Some(path) = std::env::var_os("ASTER_RECOVERY_BINDING_CHILD_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    let (mut project, document) = load_project_with_session(&path).unwrap();
    project.name = "Saved by the independent process after the crash".into();
    save_project_with_session(&document, &project).unwrap();
}

#[test]
fn restored_crash_history_cannot_overwrite_an_independent_processes_later_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (mut editor, document) = opened(&path);
    rename(&mut editor, "Recovered unsaved edit");
    rename(&mut editor, "Recovered redo branch");
    assert!(editor.undo());
    let old_context = editor.instance_id();
    save_recovery_with_document(&document, &editor).unwrap();
    drop(document);
    drop(editor);

    let output = ProcessCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "recovery_independent_writer_child",
            "--nocapture",
        ])
        .env("ASTER_RECOVERY_BINDING_CHILD_PATH", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let winner_bytes = fs::read(&path).unwrap();
    let mut restored = load_recovery_with_document(&path).unwrap();

    assert!(restored.document_changed);
    assert!(restored.editor.is_dirty());
    assert_ne!(restored.editor.instance_id(), old_context);
    assert_eq!(restored.editor.project().name, "Recovered unsaved edit");
    let document = restored.document.unwrap();
    for _ in 0..2 {
        assert!(matches!(
            save_project_with_session(&document, restored.editor.project()),
            Err(StoreError::DocumentConflict(_))
        ));
    }
    assert_eq!(fs::read(&path).unwrap(), winner_bytes);
    assert!(restored.editor.redo());
    assert_eq!(restored.editor.project().name, "Recovered redo branch");
    assert!(restored.editor.undo());
    assert!(restored.editor.undo());
    assert_eq!(restored.editor.project().name, "Ordinary baseline");
    // Undoing to an old saved root cannot advertise independently changed disk
    // bytes as this recovered editor's saved state.
    assert!(restored.editor.is_dirty());
}

#[test]
fn same_baseline_recovery_can_continue_protected_saves_and_history() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (mut editor, document) = opened(&path);
    rename(&mut editor, "Recovered edit");
    save_recovery_with_document(&document, &editor).unwrap();
    let mut restored = load_recovery_with_document(&path).unwrap();
    assert!(!restored.document_changed);
    let document = restored.document.unwrap();
    assert_eq!(document.path(), path.canonicalize().unwrap());
    assert!(restored.editor.undo());
    assert!(!restored.editor.is_dirty());
    assert!(restored.editor.redo());
    let revision = restored.editor.register_save();
    save_project_with_session(&document, restored.editor.project()).unwrap();
    restored.editor.mark_saved(revision);
    assert!(!restored.editor.is_dirty());
    rename(&mut restored.editor, "Next recovered edit");
    save_project_with_session(&document, restored.editor.project()).unwrap();
    assert_eq!(load_project(&path).unwrap(), *restored.editor.project());
}

#[test]
fn a_missing_original_baseline_is_preserved_until_a_competing_creation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("not-yet-saved.aster");
    let document = DocumentSession::create(&path).unwrap();
    let editor = Editor::default();
    save_recovery_with_document(&document, &editor).unwrap();
    let restored = load_recovery_with_document(&path).unwrap();
    assert!(!restored.document_changed);
    let restored_document = restored.document.unwrap();
    let winner = Project::new("Independent creator");
    save_project(&path, &winner).unwrap();
    assert!(matches!(
        save_project_with_session(&restored_document, restored.editor.project()),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(load_project(&path).unwrap(), winner);
}

#[test]
fn legacy_recovery_restores_unsaved_without_inferring_a_current_disk_baseline() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("legacy.aster");
    let (editor, _) = opened(&path);
    assert!(!editor.is_dirty());
    save_recovery(&path, &editor).unwrap();
    assert!(archive_json(&path).get("document_binding").is_none());

    let recovered = load_recovery_with_document(&path).unwrap();
    assert!(recovered.document.is_none());
    assert!(!recovered.document_changed);
    assert!(recovered.editor.is_dirty());
    assert_eq!(recovered.editor.project(), editor.project());
    // The history-only compatibility API intentionally remains unchanged.
    assert!(!load_recovery(&path).unwrap().is_dirty());
}

#[test]
fn private_recovery_is_unbound_and_rejects_an_ordinary_binding() {
    let dir = TempDir::new().unwrap();
    let ordinary = dir.path().join("film.aster");
    let private = dir.path().join("window-instance.aster");
    let (mut editor, document) = opened(&ordinary);
    rename(&mut editor, "Second window's unsaved work");
    let lease = Arc::new(RecoveryLease::acquire(&private).unwrap());
    let worker = SaveWorker::new(|| {});
    assert!(matches!(
        worker.request_recovery(private.clone(), &editor, lease.clone(), Some(document)),
        Err(StoreError::Invalid(_))
    ));
    assert!(!recovery_path(&private).exists());
    worker
        .request_recovery(private.clone(), &editor, lease, None)
        .unwrap();
    worker.flush(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();
    let restored = load_recovery_with_document(&private).unwrap();
    assert!(restored.document.is_none());
    assert!(restored.editor.is_dirty());
    assert_eq!(restored.editor.project(), editor.project());
    assert_eq!(load_project(&ordinary).unwrap().name, "Ordinary baseline");
}

#[test]
fn copying_named_recovery_elsewhere_does_not_grant_its_ordinary_save_authority() {
    let dir = TempDir::new().unwrap();
    let ordinary = dir.path().join("film.aster");
    let copy = dir.path().join("copied-recovery.aster");
    let (editor, document) = opened(&ordinary);
    save_recovery_with_document(&document, &editor).unwrap();
    fs::copy(recovery_path(&ordinary), recovery_path(&copy)).unwrap();
    let restored = load_recovery_with_document(&copy).unwrap();
    assert!(restored.document.is_none());
    assert!(restored.editor.is_dirty());
    assert!(!copy.exists());
}

#[test]
fn queued_recovery_keeps_its_submitted_baseline_when_an_earlier_job_advances_the_session() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (mut editor, document) = opened(&path);
    let path = document.path().to_path_buf();
    let lease = Arc::new(RecoveryLease::acquire(&path).unwrap());
    let (wake_tx, wake_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let first_wake = AtomicBool::new(true);
    let worker = SaveWorker::new(move || {
        let _ = wake_tx.send(());
        if first_wake.swap(false, Ordering::SeqCst) {
            release_rx.lock().unwrap().recv_timeout(DEADLINE).unwrap();
        }
    });
    // Deterministically pause the worker at its first completion, with no
    // sleeps or storage/process manipulation outside this owned directory.
    worker
        .request(dir.path().join("gate.aster"), &Editor::default(), false)
        .unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();

    rename(&mut editor, "Recovery snapshot at submission");
    let submitted_context = editor.instance_id();
    let submitted_revision = editor.revision();
    worker
        .request_recovery(path.clone(), &editor, lease.clone(), Some(document.clone()))
        .unwrap();
    rename(&mut editor, "Ordinary save queued later");
    editor.register_save();
    worker.request_document(document.clone(), &editor).unwrap();
    // Same-destination ordinary saves sort before recovery; the session will
    // advance before the archive is written, but not change its captured bind.
    release_tx.send(()).unwrap();
    worker.flush(DEADLINE).unwrap();
    let first = worker.poll().unwrap();
    let second = worker.poll().unwrap();
    first.result.unwrap();
    second.result.unwrap();
    assert!(!first.recovery);
    assert!(second.recovery);
    assert_eq!(second.context_id, submitted_context);
    assert_eq!(second.revision, submitted_revision);
    let restored = load_recovery_with_document(&path).unwrap();
    assert!(restored.document_changed);
    assert_eq!(
        restored.editor.project().name,
        "Recovery snapshot at submission"
    );
    assert!(matches!(
        save_project_with_session(&restored.document.unwrap(), restored.editor.project()),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(
        load_project(&path).unwrap().name,
        "Ordinary save queued later"
    );

    // An acknowledged later recovery submission does capture the new baseline.
    editor.mark_saved(first.revision);
    worker
        .request_recovery(path.clone(), &editor, lease, Some(document))
        .unwrap();
    worker.flush(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();
    let restored = load_recovery_with_document(&path).unwrap();
    assert!(!restored.document_changed);
    assert!(!restored.editor.is_dirty());
    save_project_with_session(&restored.document.unwrap(), restored.editor.project()).unwrap();
}

#[test]
fn deleted_original_remains_a_conflict_after_restoring_recovery() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (editor, document) = opened(&path);
    save_recovery_with_document(&document, &editor).unwrap();
    fs::remove_file(&path).unwrap();
    let restored = load_recovery_with_document(&path).unwrap();
    assert!(restored.document_changed);
    assert!(matches!(
        save_project_with_session(&restored.document.unwrap(), restored.editor.project()),
        Err(StoreError::DocumentConflict(_))
    ));
    assert!(!path.exists());
}

#[test]
fn damaged_or_future_ordinary_files_do_not_acquire_save_authority_from_archive_hashes() {
    use sha2::{Digest, Sha256};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (editor, document) = opened(&path);
    save_recovery_with_document(&document, &editor).unwrap();
    let original_archive = archive_json(&path);
    for bytes in [
        b"Not a valid project".as_slice(),
        b"{\"schema_version\":4294967295,\"project\":{}}".as_slice(),
        b"{\"schema_version\":1,\"project\":{}}".as_slice(),
    ] {
        fs::write(&path, bytes).unwrap();
        let mut archive = original_archive.clone();
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        archive["document_binding"]["baseline"] = json!({"Present": {
            "bytes": bytes.len(), "sha256": digest,
        }});
        write_archive(&path, &archive);
        let restored = load_recovery_with_document(&path).unwrap();
        assert!(restored.document.is_none());
        assert!(restored.editor.is_dirty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn future_or_malformed_binding_versions_preserve_the_existing_archive() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let (editor, document) = opened(&path);
    save_recovery_with_document(&document, &editor).unwrap();
    let original = archive_json(&path);
    let mut cases = Vec::new();
    let mut future = original.clone();
    future["document_binding"]["schema_version"] = json!(2);
    cases.push(future);
    let mut malformed = original.clone();
    malformed["document_binding"]["baseline"]["Present"]["sha256"] = json!([1, 2, 3]);
    cases.push(malformed);
    let mut oversized = original;
    oversized["document_binding"]["baseline"]["Present"]["bytes"] = json!(256 * 1024 * 1024 + 1u64);
    cases.push(oversized);
    for case in cases {
        write_archive(&path, &case);
        let before = fs::read(recovery_path(&path)).unwrap();
        assert!(load_recovery_with_document(&path).is_err());
        assert!(save_recovery_with_document(&document, &editor).is_err());
        assert_eq!(fs::read(recovery_path(&path)).unwrap(), before);
    }
}
