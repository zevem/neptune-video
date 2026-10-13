use editor_store::{RecoveryLease, StoreError};
use std::{fs, path::Path};

#[test]
fn independent_handles_cannot_own_the_same_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("untitled.aster");
    let lease = RecoveryLease::acquire(&path).unwrap();
    assert!(RecoveryLease::is_active(&path).unwrap());
    assert!(matches!(
        RecoveryLease::acquire(&path),
        Err(StoreError::RecoveryInUse(_))
    ));
    assert!(
        !path.exists(),
        "acquiring ownership must not create a recovery document"
    );
    drop(lease);
    assert!(!RecoveryLease::is_active(&path).unwrap());
    let next = RecoveryLease::acquire(&path).unwrap();
    assert!(RecoveryLease::is_active(&path).unwrap());
    drop(next);
}

#[test]
fn missing_probe_does_not_create_a_sidecar_or_parent_directory() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing").join("untitled.aster");
    assert!(!RecoveryLease::is_active(&path).unwrap());
    assert!(!temp.path().join("missing").exists());
    assert!(fs::read_dir(temp.path()).unwrap().next().is_none());
}

#[test]
fn probes_ignore_invalid_contents_and_acquire_never_truncates_the_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.aster.recovery");
    let sidecar = temp.path().join("broken.aster.recovery.lock");
    let invalid_document = b"not JSON or a project";
    let invalid_owner = b"not a PID or owner record\0\xff";
    fs::write(&path, invalid_document).unwrap();
    assert!(!RecoveryLease::is_active(&path).unwrap());
    assert!(!sidecar.exists());
    fs::write(&sidecar, invalid_owner).unwrap();
    assert!(!RecoveryLease::is_active(&path).unwrap());
    let lease = RecoveryLease::acquire(&path).unwrap();
    assert!(RecoveryLease::is_active(&path).unwrap());
    drop(lease);
    assert!(sidecar.exists(), "never unlink a lock sidecar on release");
    assert_eq!(fs::read(sidecar).unwrap(), invalid_owner);
    assert_eq!(fs::read(path).unwrap(), invalid_document);
}

#[test]
fn invalid_identity_fails_without_touching_the_filesystem() {
    assert!(matches!(
        RecoveryLease::is_active(Path::new("")),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        RecoveryLease::acquire(Path::new("")),
        Err(StoreError::Invalid(_))
    ));
}
