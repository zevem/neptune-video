use editor_core::{Asset, Clip, Command, Editor, MediaKind, Project, Time};
use std::sync::Arc;

#[test]
fn cross_kind_identity_collision_keeps_the_exact_root_revision_and_redo() {
    let mut editor = Editor::default();
    editor
        .dispatch(Command::RenameProject {
            name: "Future".into(),
        })
        .unwrap();
    assert!(editor.undo());
    let before = editor.snapshot();
    let revision = editor.revision();
    let mut asset = Asset::new("owned.mp4".into(), MediaKind::Video, Time(1_000_000));
    asset.id = before.tracks[0].id;
    assert!(
        editor
            .dispatch(Command::AddAssets {
                assets: vec![asset]
            })
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert_eq!(editor.revision(), revision);
    assert!(editor.can_redo());
}

#[test]
fn reloaded_clip_cannot_reuse_the_project_identity() {
    let mut project = Project::new("Collision");
    let mut clip = Clip::text("Title", Time::ZERO, Time(1_000_000));
    clip.id = project.id;
    Arc::make_mut(&mut project.tracks[2]).clips.push(clip);
    let encoded = serde_json::to_string(&project).unwrap();
    let reloaded: Project = serde_json::from_str(&encoded).unwrap();
    assert!(Editor::new(reloaded).is_err());
}
