use editor_core::{Command, Editor, Project};
use editor_store::{
    DocumentSession, RecoveryLease, SCHEMA_VERSION, SaveWorker, StoreError, checkpoint,
    load_project, load_project_with_session, save_project, save_project_with_session,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(10);

fn initial(path: &Path, name: &str) {
    // No lock sidecar exists just because a document can be opened.
    let file = File::create(path).unwrap();
    serde_json::to_writer(
        file,
        &serde_json::json!({"schema_version": SCHEMA_VERSION, "project": Project::new(name)}),
    )
    .unwrap();
}

fn write_lock(path: &Path) -> File {
    let mut name = path.as_os_str().to_os_string();
    name.push(".write-lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(PathBuf::from(name))
        .unwrap();
    file.try_lock().unwrap();
    file
}

#[test]
fn a_stale_window_cannot_replace_another_windows_saved_edits() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (mut first, first_session) = load_project_with_session(&path).unwrap();
    let (mut second, second_session) = load_project_with_session(&path).unwrap();
    first.name = "First window's saved edit".into();
    second.name = "Second window's unsaved edit".into();
    save_project_with_session(&first_session, &first).unwrap();
    let saved = fs::read(&path).unwrap();

    assert!(matches!(
        save_project_with_session(&second_session, &second),
        Err(StoreError::DocumentConflict(ref conflicted)) if conflicted.as_path() == first_session.path()
    ));
    assert_eq!(fs::read(&path).unwrap(), saved);
    assert_eq!(second.name, "Second window's unsaved edit");

    first.name = "First window's next saved edit".into();
    save_project_with_session(&first_session, &first).unwrap();
    assert_eq!(load_project(&path).unwrap(), first);
}

#[test]
fn independent_creators_cannot_replace_the_first_saved_new_document() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new.aster");
    let first = DocumentSession::create(&path).unwrap();
    let second = DocumentSession::create(&path).unwrap();
    let winner = Project::new("First creator");
    save_project_with_session(&first, &winner).unwrap();

    assert!(matches!(
        save_project_with_session(&second, &Project::new("Second creator")),
        Err(StoreError::DocumentConflict(_))
    ));
    assert!(matches!(
        DocumentSession::create(&path),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(load_project(&path).unwrap(), winner);
}

#[test]
fn save_as_and_duplicate_cannot_overwrite_a_destination_changed_after_it_was_chosen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("chosen.aster");
    initial(&path, "Chosen destination");
    let chosen = DocumentSession::prepare(&path).unwrap();
    let new_contents = Project::new("Another writer's intervening edit");
    save_project(&path, &new_contents).unwrap();

    assert!(matches!(
        save_project_with_session(&chosen, &Project::new("My duplicate")),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(load_project(&path).unwrap(), new_contents);

    let new_path = dir.path().join("missing-when-chosen.aster");
    let chosen = DocumentSession::prepare(&new_path).unwrap();
    save_project(&new_path, &new_contents).unwrap();
    assert!(matches!(
        save_project_with_session(&chosen, &Project::new("My Save As")),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(load_project(&new_path).unwrap(), new_contents);
}

#[test]
fn an_in_place_equal_length_rewrite_is_detected_by_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "AAAA");
    let (mut project, session) = load_project_with_session(&path).unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let changed = before.replace("AAAA", "BBBB");
    assert_eq!(changed.len(), before.len());
    fs::write(&path, &changed).unwrap();
    project.name = "My edit".into();

    assert!(matches!(
        save_project_with_session(&session, &project),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), changed);
}

#[test]
fn deletion_is_a_conflict_and_does_not_recreate_the_removed_project() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (project, session) = load_project_with_session(&path).unwrap();
    fs::remove_file(&path).unwrap();

    assert!(matches!(
        save_project_with_session(&session, &project),
        Err(StoreError::DocumentConflict(_))
    ));
    assert!(!path.exists());
    // The persistent advisory lock is the only artifact; no temporary file.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn opening_reads_no_lock_and_creates_no_missing_parent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let _ = load_project_with_session(&path).unwrap();
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);

    let missing = dir.path().join("missing/film.aster");
    assert!(load_project_with_session(&missing).is_err());
    assert!(!dir.path().join("missing").exists());
}

#[test]
fn preparing_a_new_destination_creates_and_canonicalizes_missing_parents() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new/nested/film.aster");
    let session = DocumentSession::create(&path).unwrap();
    let project = Project::new("New nested project");
    save_project_with_session(&session, &project).unwrap();

    assert_eq!(session.path(), path.canonicalize().unwrap());
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn validation_failure_does_not_advance_the_baseline_or_replace_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (mut project, session) = load_project_with_session(&path).unwrap();
    let original = fs::read(&path).unwrap();
    project.settings.width = 0;
    assert!(save_project_with_session(&session, &project).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    project.settings.width = 1920;
    project.name = "A valid retry".into();
    save_project_with_session(&session, &project).unwrap();
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn write_failure_before_replacement_preserves_the_baseline_for_a_valid_retry() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("destination");
    let path = parent.join("film.aster");
    let session = DocumentSession::create(&path).unwrap();
    fs::remove_dir(&parent).unwrap();
    fs::write(&parent, b"A file now occupies the destination's directory").unwrap();
    let project = Project::new("My new project");
    assert!(save_project_with_session(&session, &project).is_err());
    assert_eq!(
        fs::read(&parent).unwrap(),
        b"A file now occupies the destination's directory"
    );
    fs::remove_file(&parent).unwrap();
    save_project_with_session(&session, &project).unwrap();
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn an_independently_created_invalid_file_is_a_conflict_and_is_preserved() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new.aster");
    let session = DocumentSession::create(&path).unwrap();
    fs::write(&path, b"An intervening creator's incomplete work").unwrap();
    assert!(matches!(
        save_project_with_session(&session, &Project::new("My creation")),
        Err(StoreError::DocumentConflict(_))
    ));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"An intervening creator's incomplete work"
    );
}

#[test]
fn protected_save_submission_does_no_disk_check_on_the_calling_thread() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (project, session) = load_project_with_session(&path).unwrap();
    let held = write_lock(session.path());
    let (wake_tx, wake_rx) = mpsc::channel();
    let worker = SaveWorker::new(move || {
        let _ = wake_tx.send(());
    });
    let mut editor = Editor::new(project).unwrap();
    editor.register_save();
    // Ownership is only passed to the queue. Busy is a worker completion, not a
    // caller-side filesystem operation or lock wait.
    worker.request_document(session, &editor).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    assert!(matches!(
        worker.poll().unwrap().result,
        Err(StoreError::DocumentBusy(_))
    ));
    drop(held);
}

#[test]
fn damaged_and_future_destinations_cannot_be_prepared_for_replacement() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let future = b"{\"schema_version\":4294967295,\"project\":{}}";
    fs::write(&path, future).unwrap();
    assert!(matches!(
        DocumentSession::prepare(&path),
        Err(StoreError::FutureSchema(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), future);
    let damaged = b"{\"schema_version\":1,\"project\":";
    fs::write(&path, damaged).unwrap();
    assert!(DocumentSession::prepare(&path).is_err());
    assert_eq!(fs::read(path).unwrap(), damaged);
}

#[test]
fn a_busy_writer_is_refused_without_advancing_the_baseline_and_can_be_retried() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (mut project, session) = load_project_with_session(&path).unwrap();
    project.name = "My edit".into();
    let original = fs::read(&path).unwrap();
    let held = write_lock(session.path());

    assert!(matches!(
        save_project_with_session(&session, &project),
        Err(StoreError::DocumentBusy(_))
    ));
    assert!(matches!(
        save_project(&path, &project),
        Err(StoreError::DocumentBusy(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(held);
    save_project_with_session(&session, &project).unwrap();
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn recovery_ownership_and_checkpoint_creation_do_not_block_manual_document_saves() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (mut project, session) = load_project_with_session(&path).unwrap();
    let _recovery = RecoveryLease::acquire(&path).unwrap();
    let before = checkpoint(&path, &project, "Before editing").unwrap();
    project.name = "Edited".into();
    save_project_with_session(&session, &project).unwrap();
    let restored = load_project(&before).unwrap();
    save_project_with_session(&session, &restored).unwrap();
    assert_eq!(load_project(&path).unwrap(), restored);
    assert!(RecoveryLease::is_active(&path).unwrap());
}

#[cfg(unix)]
#[test]
fn symlink_aliases_use_one_canonical_destination_and_detect_independent_staleness() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let alias = dir.path().join("alias.aster");
    initial(&path, "Opened");
    symlink(&path, &alias).unwrap();
    let (mut first, session) = load_project_with_session(&path).unwrap();
    let (second, aliased) = load_project_with_session(&alias).unwrap();
    assert_eq!(session.path(), aliased.path());
    first.name = "Updated actual file".into();
    save_project_with_session(&session, &first).unwrap();
    assert!(matches!(
        save_project_with_session(&aliased, &second),
        Err(StoreError::DocumentConflict(_))
    ));
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(load_project(&alias).unwrap(), first);

    let parent_alias = dir.path().join("directory-alias");
    symlink(dir.path(), &parent_alias).unwrap();
    let new = DocumentSession::create(&parent_alias.join("new.aster")).unwrap();
    assert_eq!(
        new.path(),
        dir.path().canonicalize().unwrap().join("new.aster")
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_not_silently_replaced() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dangling.aster");
    symlink(dir.path().join("absent.aster"), &path).unwrap();
    assert!(DocumentSession::prepare(&path).is_err());
    assert!(DocumentSession::create(&path).is_err());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn coalesced_background_saves_follow_the_baseline_advanced_by_the_previous_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    let session = DocumentSession::create(&path).unwrap();
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
    let mut editor = Editor::new(Project::new("First save")).unwrap();
    editor.register_save();
    worker.request_document(session.clone(), &editor).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    worker.poll().unwrap().result.unwrap();
    for name in ["Second save", "Superseded save", "Latest save"] {
        editor
            .dispatch(Command::RenameProject { name: name.into() })
            .unwrap();
        editor.register_save();
        worker.request_document(session.clone(), &editor).unwrap();
    }
    let revision = editor.revision();
    // UI replacement/drop does not release ownership of the accepted request.
    drop(session);
    release_tx.send(()).unwrap();
    wake_rx.recv_timeout(DEADLINE).unwrap();
    let done = worker.poll().unwrap();
    done.result.unwrap();
    assert_eq!(done.revision, revision);
    assert_eq!(done.path, path.canonicalize().unwrap());
    assert_eq!(load_project(&path).unwrap().name, "Latest save");
    assert!(worker.poll().is_none());
}

#[test]
fn background_conflict_keeps_local_edits_dirty_and_does_not_rebaseline_a_retry() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let (project, stale) = load_project_with_session(&path).unwrap();
    let mut editor = Editor::new(project).unwrap();
    editor
        .dispatch(Command::RenameProject {
            name: "My unsaved edit".into(),
        })
        .unwrap();
    let winner = Project::new("Another window's saved edit");
    save_project(&path, &winner).unwrap();
    let (wake_tx, wake_rx) = mpsc::channel();
    let worker = SaveWorker::new(move || {
        let _ = wake_tx.send(());
    });
    for _ in 0..2 {
        editor.register_save();
        worker.request_document(stale.clone(), &editor).unwrap();
        wake_rx.recv_timeout(DEADLINE).unwrap();
        assert!(matches!(
            worker.poll().unwrap().result,
            Err(StoreError::DocumentConflict(_))
        ));
        assert!(editor.is_dirty());
    }
    assert_eq!(load_project(path.as_path()).unwrap(), winner);
    assert_eq!(editor.project().name, "My unsaved edit");
}

// The test executable doubles as a second-process writer. No personal paths or
// inherited session state are used; the parent supplies an owned temp path.
#[test]
fn document_writer_child() {
    let Some(path) = std::env::var_os("ASTER_STORE_CONFLICT_CHILD_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    let (mut project, session) = load_project_with_session(&path).unwrap();
    project.name = std::env::var("ASTER_STORE_CONFLICT_CHILD_NAME").unwrap();
    println!("writer-ready");
    std::io::stdout().flush().unwrap();
    let mut trigger = String::new();
    std::io::stdin().read_line(&mut trigger).unwrap();
    assert_eq!(trigger, "save\n");
    let outcome = match save_project_with_session(&session, &project) {
        Ok(()) => "saved",
        Err(StoreError::DocumentConflict(_)) => "conflict",
        Err(StoreError::DocumentBusy(_)) => "busy",
        Err(error) => panic!("Unexpected child save failure: {error}"),
    };
    println!("writer-result={outcome}");
    std::io::stdout().flush().unwrap();
}

struct ChildWriter {
    child: Child,
    lines: mpsc::Receiver<String>,
    output: Option<std::thread::JoinHandle<()>>,
}

impl ChildWriter {
    fn start(path: &Path, name: &str) -> Self {
        let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
            .args(["--exact", "document_writer_child", "--nocapture"])
            .env("ASTER_STORE_CONFLICT_CHILD_PATH", path)
            .env("ASTER_STORE_CONFLICT_CHILD_NAME", name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        let output = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let writer = Self {
            child,
            lines,
            output: Some(output),
        };
        loop {
            if writer
                .lines
                .recv_timeout(DEADLINE)
                .unwrap()
                .ends_with("writer-ready")
            {
                break;
            }
        }
        writer
    }

    fn save(&mut self) {
        self.child
            .stdin
            .take()
            .unwrap()
            .write_all(b"save\n")
            .unwrap();
    }

    fn result(&self) -> String {
        loop {
            let line = self.lines.recv_timeout(DEADLINE).unwrap();
            if let Some(result) = line.strip_prefix("writer-result=") {
                return result.into();
            }
        }
    }
}

impl Drop for ChildWriter {
    fn drop(&mut self) {
        // A failed assertion must not leave a task-owned writer waiting on stdin.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}

#[test]
fn independent_processes_share_the_check_and_replace_lock() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("film.aster");
    initial(&path, "Opened");
    let mut first = ChildWriter::start(&path, "First process");
    let mut second = ChildWriter::start(&path, "Second process");
    // Both baselines are established before either writer is allowed to save.
    first.save();
    second.save();
    let outcomes = [first.result(), second.result()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| result.as_str() == "saved")
            .count(),
        1
    );
    assert!(
        outcomes
            .iter()
            .any(|result| matches!(result.as_str(), "busy" | "conflict"))
    );
    assert!(matches!(
        load_project(&path).unwrap().name.as_str(),
        "First process" | "Second process"
    ));
}
