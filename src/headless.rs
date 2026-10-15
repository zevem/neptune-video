//! Headless editing of one project file: an `Editor` that saves through a
//! protected `DocumentSession` after every changing mutation and keeps its
//! session history in the recovery archive. `aster-agent --project` and the
//! MCP server's local projects are both this code.
use editor_agent::{AgentOperation, AgentRequest, AgentResponse, AgentResult};
use editor_core::Editor;
use editor_store::{DocumentSession, RecoveryLease, StoreError};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// One headless editing context owns one advancing ordinary-file baseline.
/// Recovery ownership is separate and retained once obtained, so another live
/// editor's session history cannot be overwritten by this command-line client.
pub struct HeadlessDocument {
    session: Option<Arc<DocumentSession>>,
    recovery_base: PathBuf,
    recovery_lease: Option<RecoveryLease>,
}

pub fn open_headless(
    path: &Path,
    create: bool,
    recover: bool,
) -> Result<(Editor, HeadlessDocument), StoreError> {
    if create && recover {
        return Err(StoreError::Invalid(
            "Choose either --create or --recover".into(),
        ));
    }
    if create {
        let session = DocumentSession::create(path)?;
        let mut editor = Editor::default();
        editor_store::save_project_with_session(&session, editor.project())?;
        editor.mark_saved(editor.revision());
        return Ok((
            editor,
            HeadlessDocument {
                recovery_base: session.path().to_path_buf(),
                session: Some(session),
                recovery_lease: None,
            },
        ));
    }
    if recover {
        // Protect the exact archive being adopted before reading it. A raw
        // alias may name its own older recovery copy, while the ordinary-file
        // session independently resolves the actual save destination.
        let recovery_base = std::path::absolute(path)?;
        let lease = RecoveryLease::acquire(&recovery_base)?;
        let recovered = editor_store::load_recovery_with_document(&recovery_base)?;
        if recovered.document_changed {
            eprintln!(
                "aster-agent: The project changed since this recovery copy; protected saves will conflict. Use --save-as NEWPATH to keep this work separately."
            );
        } else if recovered.document.is_none() {
            eprintln!(
                "aster-agent: Recovery has no ordinary-file save authority. Use --save-as NEWPATH before durable edits."
            );
        }
        // A real canonical destination may differ from an older alias's copy.
        // Lease the exact archive we read, but never use that alias lease to
        // write the canonical destination's recovery file.
        let (recovery_base, recovery_lease) = match &recovered.document {
            Some(session) if session.path() != recovery_base => {
                (session.path().to_path_buf(), None)
            }
            _ => (recovery_base, Some(lease)),
        };
        return Ok((
            recovered.editor,
            HeadlessDocument {
                session: recovered.document,
                recovery_base,
                recovery_lease,
            },
        ));
    }
    let (project, session) = match editor_store::load_project_with_session(path) {
        Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(StoreError::Invalid(
                "Project does not exist; use --create to make it".into(),
            ));
        }
        result => result?,
    };
    Ok((
        Editor::new(project)?,
        HeadlessDocument {
            recovery_base: session.path().to_path_buf(),
            session: Some(session),
            recovery_lease: None,
        },
    ))
}

impl HeadlessDocument {
    fn write_recovery(&mut self, editor: &Editor) -> Result<(), StoreError> {
        if self.recovery_lease.is_none() {
            self.recovery_lease = Some(RecoveryLease::acquire(&self.recovery_base)?);
        }
        match &self.session {
            Some(session) => editor_store::save_recovery_with_document(session, editor),
            None => editor_store::save_recovery(&self.recovery_base, editor),
        }
    }
}

/// Explicitly saves restored history to a new, never-overwritten destination.
/// Ordinary-save failure leaves the old session untouched. Journal failure is
/// separate: the new ordinary file is already durable and remains adopted.
pub fn configure_save_as(
    editor: &mut Editor,
    document: &mut HeadlessDocument,
    path: &Path,
) -> Result<Option<StoreError>, StoreError> {
    let session = DocumentSession::create(path)?;
    editor_store::save_project_with_session(&session, editor.project())?;
    editor.mark_saved(editor.revision());
    if document.recovery_base != session.path() {
        document.recovery_lease = None;
    }
    document.recovery_base = session.path().to_path_buf();
    document.session = Some(session);
    Ok(document.write_recovery(editor).err())
}

/// Saves a project that has no file yet to `path`, which must not exist,
/// and returns the document that now owns that file.
pub fn create_document(
    editor: &mut Editor,
    path: &Path,
) -> Result<(HeadlessDocument, Option<StoreError>), StoreError> {
    let session = DocumentSession::create(path)?;
    editor_store::save_project_with_session(&session, editor.project())?;
    editor.mark_saved(editor.revision());
    let mut document = HeadlessDocument {
        recovery_base: session.path().to_path_buf(),
        session: Some(session),
        recovery_lease: None,
    };
    let recovery = document.write_recovery(editor).err();
    Ok((document, recovery))
}

impl HeadlessDocument {
    /// The file this document saves to, once it has the authority to.
    pub fn path(&self) -> Option<&Path> {
        self.session.as_ref().map(|session| session.path())
    }

    /// The version of the file as this document last read or wrote it.
    pub fn file_version(&self) -> Option<String> {
        self.session.as_ref().and_then(|session| session.version())
    }

    /// Takes the recovery lease of the file now rather than at the first
    /// edit, so a second writer (another server, `aster-agent`) is refused
    /// before it edits anything. Refused while another process holds it.
    pub fn claim(&mut self) -> Result<(), StoreError> {
        if self.recovery_lease.is_none() {
            self.recovery_lease = Some(RecoveryLease::acquire(&self.recovery_base)?);
        }
        Ok(())
    }

    /// Lets go of the file but keeps its recovery copy: the editor holds
    /// work that is in no file (recovered work, an edit whose save failed),
    /// which the next editor to open the file is offered.
    pub fn abandon(mut self) {
        self.recovery_lease = None;
    }

    /// Gives the file up to another editor that holds the same version: the
    /// recovery copy is removed while the lease is still held (its history
    /// ends here), then the lease is let go.
    pub fn release(mut self) -> Result<(), StoreError> {
        let removed = if self.recovery_lease.is_some() {
            editor_store::remove_recovery(&self.recovery_base)
        } else {
            Ok(())
        };
        self.recovery_lease = None;
        removed
    }
}

/// Answers a request for a headless editor that may or may not own a file:
/// `save` with a destination gives a document to an editor that had none.
/// Everything else is `execute_headless`.
pub fn execute(
    editor: &mut Editor,
    document: &mut Option<HeadlessDocument>,
    request: AgentRequest,
) -> Result<AgentResponse, StoreError> {
    if let (
        None,
        AgentOperation::Save {
            destination: Some(path),
        },
    ) = (document.as_ref(), &request.operation)
    {
        if let Err(result) = editor_agent::check_identity(editor, &request) {
            return Ok(AgentResponse {
                request_id: request.request_id,
                revision: editor.revision(),
                result,
            });
        }
        return Ok(match create_document(editor, path) {
            Ok((created, recovery)) => {
                *document = Some(created);
                saved(editor, document.as_ref(), request.request_id, recovery)
            }
            Err(error) => storage_error(editor, request.request_id, &error),
        });
    }
    execute_headless(editor, document.as_mut(), request)
}

/// `execute` for a host that reports a recovery copy that could not be
/// written beside the edit's own result: the edit is applied and saved, so
/// its result (label, created IDs) still stands.
pub fn execute_keeping_result(
    editor: &mut Editor,
    document: &mut Option<HeadlessDocument>,
    request: AgentRequest,
) -> Result<(AgentResponse, Option<StoreError>), StoreError> {
    if document.is_none() || request.operation.is_host() {
        return execute(editor, document, request).map(|response| (response, None));
    }
    run_headless(editor, document.as_mut(), request)
}

/// The answer to a save that wrote the file; a recovery copy that could not
/// be written is said, as it is for an edit.
fn saved(
    editor: &Editor,
    document: Option<&HeadlessDocument>,
    request_id: String,
    recovery: Option<StoreError>,
) -> AgentResponse {
    match recovery {
        Some(error) => AgentResponse::error(
            request_id,
            editor.revision(),
            recovery_code(&error),
            format!("Project saved; session recovery failed: {error}"),
        ),
        None => AgentResponse {
            request_id,
            revision: editor.revision(),
            result: describe(editor, document),
        },
    }
}

fn describe(editor: &Editor, document: Option<&HeadlessDocument>) -> AgentResult {
    AgentResult::Document {
        document: Box::new(info(editor, document)),
    }
}

/// The document as a headless host knows it.
pub fn info(editor: &Editor, document: Option<&HeadlessDocument>) -> editor_agent::DocumentInfo {
    let path = document
        .and_then(HeadlessDocument::path)
        .map(Path::to_path_buf);
    let mut info = editor_agent::DocumentInfo::of(editor, path);
    info.file_version = document.and_then(HeadlessDocument::file_version);
    info
}

fn recovery_code(error: &StoreError) -> &'static str {
    if matches!(error, StoreError::RecoveryInUse(_)) {
        "recovery_busy"
    } else {
        "recovery_failed"
    }
}

fn storage_error(editor: &Editor, request_id: String, error: &StoreError) -> AgentResponse {
    let code = match error {
        StoreError::DocumentConflict(_) => "storage_conflict",
        StoreError::DocumentBusy(_) => "storage_busy",
        _ => "storage",
    };
    AgentResponse::error(request_id, editor.revision(), code, error.to_string())
}

/// `document` and `save` for a headless editor. A headless document is saved
/// after every change, so `save` writes only work that is not on disk yet:
/// restored work, or an edit whose save failed.
fn answer_host(
    editor: &mut Editor,
    document: Option<&mut HeadlessDocument>,
    request: AgentRequest,
) -> AgentResponse {
    let request_id = request.request_id.clone();
    match &request.operation {
        AgentOperation::Save { destination } => {
            if let Err(result) = editor_agent::check_identity(editor, &request) {
                return AgentResponse {
                    request_id,
                    revision: editor.revision(),
                    result,
                };
            }
            let Some(document) = document else {
                return AgentResponse::error(
                    request_id,
                    editor.revision(),
                    editor_agent::codes::NO_DESTINATION,
                    "This project has no file; send a destination for a new one",
                );
            };
            if let Some(path) = destination {
                return match configure_save_as(editor, document, path) {
                    Ok(recovery) => saved(editor, Some(document), request_id, recovery),
                    Err(error) => storage_error(editor, request_id, &error),
                };
            }
            let Some(session) = document.session.clone() else {
                return AgentResponse::error(
                    request_id,
                    editor.revision(),
                    "storage_unsaved",
                    "Recovered work has no ordinary-file save authority; send a new destination",
                );
            };
            if !editor.is_dirty() {
                return saved(editor, Some(document), request_id, None);
            }
            match editor_store::save_project_with_session(&session, editor.project()) {
                Ok(()) => {
                    editor.mark_saved(editor.revision());
                    let recovery = document.write_recovery(editor).err();
                    saved(editor, Some(document), request_id, recovery)
                }
                Err(error) => storage_error(editor, request_id, &error),
            }
        }
        _ => AgentResponse {
            request_id,
            revision: editor.revision(),
            result: describe(editor, document.as_deref()),
        },
    }
}

pub fn execute_headless(
    editor: &mut Editor,
    document: Option<&mut HeadlessDocument>,
    request: AgentRequest,
) -> Result<AgentResponse, StoreError> {
    if request.operation.is_host() {
        return Ok(answer_host(editor, document, request));
    }
    let (response, recovery) = run_headless(editor, document, request)?;
    Ok(match recovery {
        // The ordinary project is already durable. Keep that state; the
        // error explicitly distinguishes the unkept undo history.
        Some(error) => AgentResponse::error(
            response.request_id,
            editor.revision(),
            recovery_code(&error),
            format!("Project saved; session recovery failed: {error}"),
        ),
        None => response,
    })
}

/// An edit, undo or redo (or a read) with its save; a recovery copy that
/// could not be written is returned beside the edit's answer.
fn run_headless(
    editor: &mut Editor,
    document: Option<&mut HeadlessDocument>,
    request: AgentRequest,
) -> Result<(AgentResponse, Option<StoreError>), StoreError> {
    if request.operation.is_host() {
        return Ok((answer_host(editor, document, request), None));
    }
    let mutating = matches!(
        request.operation,
        AgentOperation::Apply { .. } | AgentOperation::Undo | AgentOperation::Redo
    );
    // Inspection/capabilities do not clone the history or touch the filesystem.
    let previous = (mutating && document.is_some()).then(|| editor.session_state());
    let mut response = editor_agent::handle(editor, request);
    let changed = matches!(
        response.result,
        AgentResult::Applied { changed: true, .. } | AgentResult::History { changed: true }
    );
    let Some(document) = document.filter(|_| changed) else {
        return Ok((response, None));
    };
    let Some(session) = &document.session else {
        if let Some(previous) = previous {
            *editor = Editor::from_session(previous)?;
        }
        return Ok((
            AgentResponse::error(
                response.request_id,
                editor.revision(),
                "storage_unsaved",
                "Recovered work has no ordinary-file save authority; restart with --save-as NEWPATH. Editing state restored; inspect the new context before retrying",
            ),
            None,
        ));
    };
    let mut recovery = None;
    match editor_store::save_project_with_session(session, editor.project()) {
        Ok(()) => {
            editor.mark_saved(editor.revision());
            recovery = document.write_recovery(editor).err();
        }
        Err(error) => {
            if let Some(previous) = previous {
                // Rebuild the previous document/history/revision, deliberately
                // renewing context identity so plans made against the failed
                // transient state cannot be retargeted. Clients re-inspect.
                *editor = Editor::from_session(previous)?;
            }
            let code = match &error {
                StoreError::DocumentConflict(_) => "storage_conflict",
                StoreError::DocumentBusy(_) => "storage_busy",
                _ => "storage",
            };
            response = AgentResponse::error(
                response.request_id,
                editor.revision(),
                code,
                format!("{error}. Editing state restored; inspect the new context before retrying"),
            );
        }
    }
    Ok((response, recovery))
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Command, Project};
    use std::fs::{self, OpenOptions};

    struct OwnedDirectory(PathBuf);

    impl OwnedDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("aster-headless-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn project(&self) -> PathBuf {
            self.0.join("film.aster")
        }
    }

    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mutation(editor: &Editor, operation: AgentOperation) -> AgentRequest {
        AgentRequest {
            request_id: "headless-test".into(),
            expected_revision: Some(editor.revision()),
            expected_project_id: Some(editor.project().id),
            expected_context_id: Some(editor.instance_id()),
            operation,
        }
    }

    fn rename(editor: &Editor, name: &str) -> AgentRequest {
        mutation(
            editor,
            AgentOperation::Apply {
                command: Box::new(Command::RenameProject { name: name.into() }),
            },
        )
    }

    fn assert_code(response: AgentResponse, expected: &str) {
        assert!(
            matches!(response.result, AgentResult::Error { ref code, .. } if code == expected),
            "Unexpected response: {response:?}"
        );
    }

    #[test]
    fn create_refuses_existing_work_and_conflicting_create_recover_modes() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (_, _document) = open_headless(&path, true, false).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(matches!(
            open_headless(&path, true, false),
            Err(StoreError::DocumentConflict(_))
        ));
        assert!(open_headless(&path, true, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn sequential_mutations_and_history_share_one_advancing_save_baseline() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut editor, mut document) = open_headless(&path, true, false).unwrap();
        for name in ["First saved edit", "Second saved edit"] {
            let request = rename(&editor, name);
            let response = execute_headless(&mut editor, Some(&mut document), request).unwrap();
            assert!(matches!(
                response.result,
                AgentResult::Applied { changed: true, .. }
            ));
            assert!(!editor.is_dirty());
            assert_eq!(editor_store::load_project(&path).unwrap().name, name);
        }
        let undo = mutation(&editor, AgentOperation::Undo);
        let response = execute_headless(&mut editor, Some(&mut document), undo).unwrap();
        assert!(matches!(
            response.result,
            AgentResult::History { changed: true }
        ));
        assert_eq!(
            editor_store::load_project(&path).unwrap().name,
            "First saved edit"
        );
        let redo = mutation(&editor, AgentOperation::Redo);
        let response = execute_headless(&mut editor, Some(&mut document), redo).unwrap();
        assert!(matches!(
            response.result,
            AgentResult::History { changed: true }
        ));
        assert_eq!(
            editor_store::load_project(&path).unwrap().name,
            "Second saved edit"
        );
        let recovered = editor_store::load_recovery(&path).unwrap();
        assert_eq!(recovered.revision(), editor.revision());
        assert!(recovered.can_undo());
        assert!(!recovered.is_dirty());
    }

    #[test]
    fn stale_headless_writer_reports_conflict_and_rolls_back_only_the_failed_mutation() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut first, mut first_document) = open_headless(&path, true, false).unwrap();
        let (mut stale, mut stale_document) = open_headless(&path, false, false).unwrap();
        let previous = stale.session_state();
        let previous_context = stale.instance_id();
        let request = rename(&first, "Winning saved edit");
        execute_headless(&mut first, Some(&mut first_document), request).unwrap();
        let saved = fs::read(&path).unwrap();
        let request = rename(&stale, "Losing attempted edit");
        let response = execute_headless(&mut stale, Some(&mut stale_document), request).unwrap();
        assert_code(response, "storage_conflict");
        assert_eq!(stale.project(), previous.current.as_ref());
        assert_eq!(stale.revision(), previous.revision);
        assert_ne!(stale.instance_id(), previous_context);
        assert!(!stale.can_undo());
        assert!(!stale.is_dirty());
        assert_eq!(fs::read(&path).unwrap(), saved);
        // Retrying cannot silently capture the winner's new baseline.
        let request = rename(&stale, "A retry remains stale");
        assert_code(
            execute_headless(&mut stale, Some(&mut stale_document), request).unwrap(),
            "storage_conflict",
        );
        assert_eq!(fs::read(&path).unwrap(), saved);
    }

    #[test]
    fn busy_headless_save_is_distinct_and_can_retry_after_the_other_writer_releases() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut editor, mut document) = open_headless(&path, true, false).unwrap();
        let original = editor.snapshot();
        let mut sidecar = document
            .session
            .as_ref()
            .unwrap()
            .path()
            .as_os_str()
            .to_os_string();
        sidecar.push(".write-lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(PathBuf::from(sidecar))
            .unwrap();
        lock.try_lock().unwrap();
        let request = rename(&editor, "While busy");
        assert_code(
            execute_headless(&mut editor, Some(&mut document), request).unwrap(),
            "storage_busy",
        );
        assert_eq!(editor.project(), original.as_ref());
        assert_eq!(editor.revision(), 0);
        drop(lock);
        let request = rename(&editor, "A safe retry");
        assert!(matches!(
            execute_headless(&mut editor, Some(&mut document), request)
                .unwrap()
                .result,
            AgentResult::Applied { changed: true, .. }
        ));
        assert_eq!(
            editor_store::load_project(&path).unwrap().name,
            "A safe retry"
        );
    }

    #[test]
    fn an_active_recovery_owner_is_checked_before_the_archive_is_read() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (_, _document) = open_headless(&path, true, false).unwrap();
        let copy = editor_store::recovery_path(&path);
        fs::write(
            &copy,
            b"Active owner is still writing this incomplete archive",
        )
        .unwrap();
        let _active = RecoveryLease::acquire(&path).unwrap();
        let Err(error) = open_headless(&path, false, true) else {
            panic!("An active archive was opened");
        };
        assert!(matches!(error, StoreError::RecoveryInUse(_)));
        assert_eq!(
            fs::read(copy).unwrap(),
            b"Active owner is still writing this incomplete archive"
        );
    }

    #[test]
    fn ordinary_commit_does_not_overwrite_another_editors_owned_recovery_copy() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut editor, mut document) = open_headless(&path, true, false).unwrap();
        let active = RecoveryLease::acquire(&path).unwrap();
        editor_store::save_recovery(&path, &editor).unwrap();
        let original_copy = fs::read(editor_store::recovery_path(&path)).unwrap();
        let request = rename(&editor, "My durable project edit");
        assert_code(
            execute_headless(&mut editor, Some(&mut document), request).unwrap(),
            "recovery_busy",
        );
        assert_eq!(
            editor_store::load_project(&path).unwrap().name,
            "My durable project edit"
        );
        assert!(!editor.is_dirty());
        assert_eq!(
            fs::read(editor_store::recovery_path(&path)).unwrap(),
            original_copy
        );
        drop(active);
        let request = rename(&editor, "My durable next edit");
        assert!(matches!(
            execute_headless(&mut editor, Some(&mut document), request)
                .unwrap()
                .result,
            AgentResult::Applied { changed: true, .. }
        ));
        assert_eq!(
            editor_store::load_recovery(&path).unwrap().project().name,
            "My durable next edit"
        );
        assert!(RecoveryLease::is_active(&path).unwrap());
    }

    #[test]
    fn a_recovered_editors_prior_unsaved_history_survives_a_later_storage_conflict() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut archived, document) = open_headless(&path, true, false).unwrap();
        archived
            .dispatch(Command::RenameProject {
                name: "Previous unsaved work".into(),
            })
            .unwrap();
        editor_store::save_recovery_with_document(document.session.as_ref().unwrap(), &archived)
            .unwrap();
        drop(document);
        let (mut recovered, mut recovered_document) = open_headless(&path, false, true).unwrap();
        assert!(recovered.is_dirty());
        let (mut winner, winner_session) = editor_store::load_project_with_session(&path).unwrap();
        winner.name = "External saved edit".into();
        editor_store::save_project_with_session(&winner_session, &winner).unwrap();
        let original_copy = fs::read(editor_store::recovery_path(&path)).unwrap();
        let before = recovered.session_state();
        let request = rename(&recovered, "Failed new work");
        assert_code(
            execute_headless(&mut recovered, Some(&mut recovered_document), request).unwrap(),
            "storage_conflict",
        );
        assert_eq!(recovered.project(), before.current.as_ref());
        assert_eq!(recovered.revision(), before.revision);
        assert!(recovered.is_dirty());
        assert!(recovered.can_undo());
        assert_eq!(
            fs::read(editor_store::recovery_path(&path)).unwrap(),
            original_copy
        );
        assert_eq!(editor_store::load_project(&path).unwrap(), winner);
    }

    #[test]
    fn a_writer_changing_disk_between_crash_and_restore_never_refreshes_archived_authority() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut archived, original_document) = open_headless(&path, true, false).unwrap();
        archived
            .dispatch(Command::RenameProject {
                name: "Work before crash".into(),
            })
            .unwrap();
        let lease = RecoveryLease::acquire(&path).unwrap();
        editor_store::save_recovery_with_document(
            original_document.session.as_ref().unwrap(),
            &archived,
        )
        .unwrap();
        let copy = fs::read(editor_store::recovery_path(&path)).unwrap();
        drop(lease);
        drop(original_document);
        let (mut winner, winner_session) = editor_store::load_project_with_session(&path).unwrap();
        winner.name = "Saved after the crash".into();
        editor_store::save_project_with_session(&winner_session, &winner).unwrap();
        // Restoration happens after the other writer's save. It must retain
        // the original archived baseline, not grant authority over this one.
        let (mut recovered, mut document) = open_headless(&path, false, true).unwrap();
        assert!(document.session.is_some());
        assert!(recovered.is_dirty());
        let before = recovered.session_state();
        let request = rename(&recovered, "A failed post-restore edit");
        assert_code(
            execute_headless(&mut recovered, Some(&mut document), request).unwrap(),
            "storage_conflict",
        );
        assert_eq!(recovered.project(), before.current.as_ref());
        assert_eq!(recovered.revision(), before.revision);
        assert!(recovered.can_undo());
        assert_eq!(editor_store::load_project(&path).unwrap(), winner);
        assert_eq!(fs::read(editor_store::recovery_path(&path)).unwrap(), copy);
    }

    #[test]
    fn legacy_recovery_stays_unsaved_until_explicit_new_destination_retains_its_history() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut archived, _original_document) = open_headless(&path, true, false).unwrap();
        archived
            .dispatch(Command::RenameProject {
                name: "Legacy recovered work".into(),
            })
            .unwrap();
        editor_store::save_recovery(&path, &archived).unwrap();
        let ordinary = fs::read(&path).unwrap();
        let copy = fs::read(editor_store::recovery_path(&path)).unwrap();
        let (mut recovered, mut document) = open_headless(&path, false, true).unwrap();
        assert!(document.session.is_none());
        assert!(recovered.is_dirty());
        assert!(recovered.can_undo());
        let inspect = mutation(&recovered, AgentOperation::Inspect);
        assert!(matches!(
            execute_headless(&mut recovered, Some(&mut document), inspect)
                .unwrap()
                .result,
            AgentResult::Project {
                dirty: true,
                can_undo: true,
                ..
            }
        ));
        let before = recovered.session_state();
        let request = rename(&recovered, "Unbound mutation must not be acknowledged");
        assert_code(
            execute_headless(&mut recovered, Some(&mut document), request).unwrap(),
            "storage_unsaved",
        );
        assert_eq!(recovered.project(), before.current.as_ref());
        assert_eq!(recovered.revision(), before.revision);
        assert_eq!(fs::read(&path).unwrap(), ordinary);
        assert_eq!(fs::read(editor_store::recovery_path(&path)).unwrap(), copy);

        let destination = dir.0.join("restored/kept.aster");
        assert!(
            configure_save_as(&mut recovered, &mut document, &destination)
                .unwrap()
                .is_none()
        );
        assert!(!recovered.is_dirty());
        assert!(recovered.can_undo());
        assert_eq!(
            editor_store::load_project(&destination).unwrap().name,
            "Legacy recovered work"
        );
        let kept = editor_store::load_recovery_with_document(&destination).unwrap();
        assert!(kept.document.is_some());
        assert!(kept.editor.can_undo());
        let undo = mutation(&recovered, AgentOperation::Undo);
        assert!(matches!(
            execute_headless(&mut recovered, Some(&mut document), undo)
                .unwrap()
                .result,
            AgentResult::History { changed: true }
        ));
        assert_ne!(
            editor_store::load_project(&destination).unwrap().name,
            "Legacy recovered work"
        );
        assert_eq!(fs::read(&path).unwrap(), ordinary);
        assert_eq!(fs::read(editor_store::recovery_path(&path)).unwrap(), copy);
    }

    #[test]
    fn recovery_save_as_refuses_existing_target_without_losing_unsaved_state() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut archived, _original_document) = open_headless(&path, true, false).unwrap();
        archived
            .dispatch(Command::RenameProject {
                name: "Recovered work".into(),
            })
            .unwrap();
        editor_store::save_recovery(&path, &archived).unwrap();
        let (mut recovered, mut document) = open_headless(&path, false, true).unwrap();
        let destination = dir.0.join("existing.aster");
        let target = DocumentSession::create(&destination).unwrap();
        editor_store::save_project_with_session(&target, &Project::new("Other work")).unwrap();
        let original = fs::read(&destination).unwrap();
        let before = recovered.snapshot();
        assert!(matches!(
            configure_save_as(&mut recovered, &mut document, &destination),
            Err(StoreError::DocumentConflict(_))
        ));
        assert!(Arc::ptr_eq(&before, &recovered.snapshot()));
        assert!(recovered.is_dirty());
        assert!(recovered.can_undo());
        assert!(document.session.is_none());
        assert_eq!(fs::read(destination).unwrap(), original);
    }

    #[test]
    fn unchanged_and_read_only_requests_do_not_attempt_disk_writes() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut editor, mut document) = open_headless(&path, true, false).unwrap();
        fs::remove_file(&path).unwrap();
        let request = rename(&editor, &editor.project().name);
        let response = execute_headless(&mut editor, Some(&mut document), request).unwrap();
        assert!(matches!(
            response.result,
            AgentResult::Applied { changed: false, .. }
        ));
        let request = mutation(&editor, AgentOperation::Inspect);
        assert!(matches!(
            execute_headless(&mut editor, Some(&mut document), request)
                .unwrap()
                .result,
            AgentResult::Project { .. }
        ));
        assert!(!path.exists());
    }

    #[test]
    fn render_operations_are_unsupported_and_touch_nothing() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let (mut editor, mut document) = open_headless(&path, true, false).unwrap();
        let original = fs::read(&path).unwrap();
        let destination = dir.0.join("film.mp4");
        let request = mutation(
            &editor,
            AgentOperation::QueueExport {
                settings: editor_core::export::ExportSettings::default(),
                destination: destination.clone(),
                overwrite: false,
            },
        );
        let response = execute_headless(&mut editor, Some(&mut document), request).unwrap();
        assert_code(response, editor_agent::codes::UNSUPPORTED);
        let request = mutation(&editor, AgentOperation::RenderJobs);
        let response = execute_headless(&mut editor, Some(&mut document), request).unwrap();
        assert_code(response, editor_agent::codes::UNSUPPORTED);
        assert!(!destination.exists());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(editor.revision(), 0);
    }

    #[test]
    fn headless_load_normalizes_the_destination_and_keeps_project_identity() {
        let dir = OwnedDirectory::new();
        let path = dir.project();
        let session = DocumentSession::create(&path).unwrap();
        let project = Project::new("Existing document");
        editor_store::save_project_with_session(&session, &project).unwrap();
        let (editor, document) = open_headless(&path, false, false).unwrap();
        assert_eq!(editor.project().id, project.id);
        assert_eq!(
            document.session.as_ref().unwrap().path(),
            path.canonicalize().unwrap()
        );
    }
}
