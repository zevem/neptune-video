use editor_core::{AnimationTrim, Clip, Command, Editor, Time};
use editor_store::{load_project, load_recovery, save_project, save_recovery};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn patched_clip_style_and_identity_survive_project_and_recovery_round_trips() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("patch.nvp");
    let mut editor = Editor::default();
    let title = Clip::text("Hello", Time::ZERO, Time(4_000_000));
    let id = title.id;
    editor
        .dispatch(Command::AddClip {
            track_id: editor.project().tracks[2].id,
            clip: title,
        })
        .unwrap();
    let before = editor.snapshot();
    editor
        .dispatch(Command::PatchClip {
            clip_id: id,
            patch: json!({
                "start": 2_000_000,
                "content": {"style": {"font": {"weight": 800}, "tracking": 0.1}}
            })
            .as_object()
            .unwrap()
            .clone(),
            trim_animation: AnimationTrim::Crop,
        })
        .unwrap();
    save_project(&path, editor.project()).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(loaded, *editor.project());
    assert!(loaded.clip(id).is_some());

    save_recovery(&path, &editor).unwrap();
    let mut restored = load_recovery(&path).unwrap();
    assert_eq!(restored.project(), editor.project());
    assert!(restored.undo());
    assert_eq!(restored.project(), before.as_ref());
    assert!(restored.redo());
    assert_eq!(restored.project(), editor.project());
}
