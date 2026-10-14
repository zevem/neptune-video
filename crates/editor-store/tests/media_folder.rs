use editor_core::{Asset, MediaKind, Project, Time};
use editor_store::{StoreError, checkpoint, load_project, media_folder, new_take, save_project};
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;

/// 2026-10-14 15:30:12 UTC.
fn at() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_791_991_812)
}

fn with_take(path: &Path) -> Project {
    let mut project = Project::new("Narrated");
    let mut asset = Asset::new(
        path.to_path_buf(),
        MediaKind::Audio,
        Time::from_seconds(5.0),
    );
    asset.has_audio = true;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    project
}

#[test]
fn the_media_folder_is_beside_the_document_and_named_by_its_stem() {
    let directory = TempDir::new().unwrap();
    let document = directory.path().join("Film cut.aster");

    let folder = media_folder(&document).unwrap();

    assert_eq!(folder, directory.path().join("Film cut media"));
    assert!(!folder.exists(), "naming the folder creates nothing");
    assert_eq!(
        media_folder(&directory.path().join("notes")).unwrap(),
        directory.path().join("notes media")
    );
    assert!(matches!(
        media_folder(Path::new("/")),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn a_take_is_named_by_its_start_and_never_over_an_existing_file() {
    let directory = TempDir::new().unwrap();
    let document = directory.path().join("film.aster");
    let folder = directory.path().join("film media");

    let first = new_take(&document, at()).unwrap();

    assert!(folder.is_dir(), "the first take makes the folder");
    assert_eq!(first, folder.join("Voiceover 2026-10-14 15.30.12 UTC.wav"));
    assert!(!first.exists(), "the writer creates the take, not the name");

    fs::write(&first, b"first").unwrap();
    let second = new_take(&document, at()).unwrap();
    assert_eq!(
        second,
        folder.join("Voiceover 2026-10-14 15.30.12 UTC 2.wav")
    );
    fs::create_dir(&second).unwrap();
    let third = new_take(&document, at()).unwrap();
    assert_eq!(
        third,
        folder.join("Voiceover 2026-10-14 15.30.12 UTC 3.wav")
    );

    let later = new_take(&document, at() + Duration::from_secs(1)).unwrap();
    assert_eq!(later, folder.join("Voiceover 2026-10-14 15.30.13 UTC.wav"));
    assert_eq!(
        fs::read(&first).unwrap(),
        b"first",
        "earlier takes are kept"
    );
}

#[test]
fn a_file_where_the_folder_goes_is_refused_and_kept() {
    let directory = TempDir::new().unwrap();
    let document = directory.path().join("film.aster");
    let blocker = directory.path().join("film media");
    fs::write(&blocker, b"mine").unwrap();

    let refused = new_take(&document, at());

    assert!(
        matches!(&refused, Err(StoreError::Invalid(words)) if words.contains("not a folder")),
        "{refused:?}"
    );
    assert_eq!(fs::read(&blocker).unwrap(), b"mine");
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_made_is_an_error_and_names_no_take() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TempDir::new().unwrap();
    let locked = directory.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    let refused = new_take(&locked.join("film.aster"), at());

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(refused, Err(StoreError::Io(_))), "{refused:?}");
    assert!(!locked.join("film media").exists());
}

#[cfg(unix)]
#[test]
fn a_dangling_link_holds_its_name() {
    let directory = TempDir::new().unwrap();
    let document = directory.path().join("film.aster");
    let folder = directory.path().join("film media");
    fs::create_dir(&folder).unwrap();
    let link = folder.join("Voiceover 2026-10-14 15.30.12 UTC.wav");
    std::os::unix::fs::symlink(directory.path().join("gone.wav"), &link).unwrap();

    let take = new_take(&document, at()).unwrap();

    assert_eq!(take, folder.join("Voiceover 2026-10-14 15.30.12 UTC 2.wav"));
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
}

#[cfg(unix)]
#[test]
fn a_name_that_cannot_be_looked_at_is_an_error_not_free() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TempDir::new().unwrap();
    let folder = directory.path().join("film media");
    fs::create_dir(&folder).unwrap();
    // Readable but not searchable: the folder is there, its names cannot be
    // checked.
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o644)).unwrap();

    let refused = new_take(&directory.path().join("film.aster"), at());

    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(refused, Err(StoreError::Io(_))), "{refused:?}");
}

#[test]
fn a_clock_before_the_epoch_is_refused() {
    let directory = TempDir::new().unwrap();
    let early = UNIX_EPOCH - Duration::from_secs(1);

    assert!(matches!(
        new_take(&directory.path().join("film.aster"), early),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn a_take_is_saved_by_its_absolute_path_through_save_as_and_checkpoints() {
    let directory = TempDir::new().unwrap();
    let document = directory.path().join("film.aster");
    let take = new_take(&document, at()).unwrap();
    fs::write(&take, b"RIFF").unwrap();
    let take = take.canonicalize().unwrap();
    let project = with_take(&take);

    save_project(&document, &project).unwrap();
    let text = fs::read_to_string(&document).unwrap();
    let written = serde_json::to_string(&take.to_string_lossy()).unwrap();
    assert!(text.contains(&written), "the path is written as held");
    assert_eq!(load_project(&document).unwrap(), project);

    // Save as to another folder: the take stays where it was recorded.
    let elsewhere = directory.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let copy = elsewhere.join("film.aster");
    save_project(&copy, &project).unwrap();
    assert_eq!(load_project(&copy).unwrap(), project);
    assert!(!elsewhere.join("film media").exists(), "nothing is copied");

    let snapshot = checkpoint(&document, &project, "Before narration").unwrap();
    assert_eq!(load_project(&snapshot).unwrap(), project);
    assert_eq!(fs::read(&take).unwrap(), b"RIFF");
}
