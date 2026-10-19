use editor_core::{Command, Editor, Project};
use editor_store::{SaveWorker, StoreError, load_project, load_recovery};
use std::{
    fs,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(5);

fn rename(editor: &mut Editor, name: &str) {
    editor
        .dispatch(Command::RenameProject { name: name.into() })
        .unwrap();
}

#[test]
fn pending_saves_to_one_destination_coalesce_to_the_latest_revision() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.nvp");
    let (wake_tx, wake_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let first_wake = AtomicBool::new(true);
    let worker = SaveWorker::new(move || {
        wake_tx.send(()).unwrap();
        if first_wake.swap(false, Ordering::SeqCst) {
            // Hold the worker after its first completion so later requests remain
            // pending together. This tests coalescing without scheduling sleeps.
            release_rx.lock().unwrap().recv_timeout(DEADLINE).unwrap();
        }
    });
    let mut editor = Editor::new(Project::new("First snapshot")).unwrap();
    worker.request(path.clone(), &editor, false).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();

    for name in ["Superseded edit", "Also superseded", "Latest edit"] {
        rename(&mut editor, name);
        worker.request(path.clone(), &editor, false).unwrap();
    }
    let expected_revision = editor.revision();
    release_tx.send(()).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    let completion = worker.poll().unwrap();

    completion.result.unwrap();
    assert_eq!(completion.revision, expected_revision);
    assert_eq!(load_project(&path).unwrap().name, "Latest edit");
    assert!(worker.poll().is_none());
}

#[test]
fn old_save_completion_keeps_new_edits_dirty_and_recovery_restores_saved_identity() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.nvp");
    let (wake_tx, wake_rx) = mpsc::channel();
    let worker = SaveWorker::new(move || {
        let _ = wake_tx.send(());
    });
    let mut editor = Editor::new(Project::new("Submitted snapshot")).unwrap();
    let revision = editor.register_save();
    worker.request(path.clone(), &editor, false).unwrap();
    rename(&mut editor, "Edit during save");
    wake_rx.recv_timeout(DEADLINE).unwrap();
    let completion = worker.poll().unwrap();
    completion.result.unwrap();
    assert_eq!(completion.revision, revision);
    editor.mark_saved(completion.revision);
    assert!(editor.is_dirty());
    assert_eq!(load_project(&path).unwrap().name, "Submitted snapshot");

    worker.request(path.clone(), &editor, true).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();
    let mut restored = load_recovery(&path).unwrap();

    assert!(restored.is_dirty());
    assert!(restored.undo());
    assert!(!restored.is_dirty());
}

#[test]
fn failed_background_save_reports_error_and_keeps_the_existing_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.nvp");
    let original = b"Unrecoverable original document";
    fs::write(&path, original).unwrap();
    let (wake_tx, wake_rx) = mpsc::channel();
    let worker = SaveWorker::new(move || {
        let _ = wake_tx.send(());
    });

    worker
        .request(path.clone(), &Editor::default(), false)
        .unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    let completion = worker.poll().unwrap();

    assert_eq!(completion.path, path);
    assert!(completion.result.is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn queue_capacity_rejects_new_destinations_but_accepts_replacement_snapshots() {
    let directory = TempDir::new().unwrap();
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
    let mut editor = Editor::default();
    worker
        .request(directory.path().join("first.nvp"), &editor, false)
        .unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();

    for index in 0..16 {
        worker
            .request(
                directory.path().join(format!("pending-{index}.nvp")),
                &editor,
                false,
            )
            .unwrap();
    }
    let overflow = worker.request(directory.path().join("overflow.nvp"), &editor, false);
    rename(&mut editor, "Latest at capacity");
    let replacement = worker.request(directory.path().join("pending-0.nvp"), &editor, false);
    release_tx.send(()).unwrap();
    worker.flush(DEADLINE).unwrap();

    assert!(matches!(overflow, Err(StoreError::QueueFull)));
    assert!(replacement.is_ok());
    assert_eq!(
        load_project(&directory.path().join("pending-0.nvp"))
            .unwrap()
            .name,
        "Latest at capacity"
    );
}
