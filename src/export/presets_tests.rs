//! The export presets file: tolerant per entry, written back whole.
use super::*;
use editor_core::{
    TrackId,
    export::{Container, SidecarFormat},
};

/// A folder of the test's own, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("neptune-video-export-presets-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("FIXTURE: the scratch folder is made");
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join(FILE_NAME)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> ExportPresets {
    match ExportPresets::read(path) {
        Read::Presets(presets) => presets,
        other => panic!("the file is read: {other:?}"),
    }
}

/// An entry a newer Neptune Video might write: a container this build has never
/// heard of.
fn from_the_future() -> serde_json::Value {
    serde_json::json!({
        "id": "later-1",
        "name": "Hologram",
        "destination": "custom",
        "note": "",
        "settings": { "container": "holo", "video": null, "audio": null },
        "depth": 7
    })
}

#[test]
fn a_saved_preset_and_an_unknown_entry_survive_a_round_trip() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let mut kept = ExportSettings::default();
    kept.subtitles.sidecar = Some(SidecarFormat::Vtt);
    let file = serde_json::json!({
        "version": 1,
        "presets": [
            from_the_future(),
            {
                "id": "mine",
                "name": "  Mine  ",
                "destination": "general",
                "note": "",
                "settings": kept,
            },
            "not even an object",
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

    let mut presets = read(&path);
    assert_eq!(presets.entries.len(), 3, "every entry is held");
    let readable: Vec<&ExportPreset> = presets.presets().collect();
    assert_eq!(readable.len(), 1);
    assert_eq!(readable[0].name, "Mine", "the name is trimmed");
    assert_eq!(
        readable[0].destination,
        Destination::Custom,
        "the person's presets are offered as theirs"
    );

    let audio = ExportSettings {
        container: Container::Wav,
        video: None,
        ..ExportSettings::default()
    };
    let id = presets.keep("Sound", &audio).unwrap();
    presets.save(&path, false).unwrap();

    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["version"], 1);
    let listed = written["presets"].as_array().unwrap();
    assert_eq!(listed.len(), 4);
    assert_eq!(
        listed[0],
        from_the_future(),
        "an unknown entry is unchanged"
    );
    assert_eq!(listed[2], serde_json::json!("not even an object"));

    let again = read(&path);
    assert_eq!(again, presets, "what was written reads back the same");
    let names: Vec<&str> = again.presets().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mine", "Sound"]);

    let mut again = again;
    assert!(again.remove(&id).is_some());
    assert!(
        again.remove("later-1").is_none(),
        "an unknown entry is not a preset"
    );
    again.save(&path, false).unwrap();
    assert_eq!(read(&path).entries.len(), 3);
    assert!(
        std::fs::read_dir(&scratch.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")),
        "no temporary file is left"
    );
}

#[test]
fn a_kept_preset_carries_neither_range_nor_caption_track() {
    let mut settings = ExportSettings {
        range: editor_core::export::ExportRange::Span {
            start: editor_core::Time::ZERO,
            end: editor_core::Time::from_seconds(2.0),
        },
        ..ExportSettings::default()
    };
    settings.subtitles.track = Some(TrackId::new_v4());
    let mut presets = ExportPresets::default();
    presets.keep("Short", &settings).unwrap();
    let preset = presets.presets().next().unwrap();
    assert_eq!(
        preset.settings.range,
        editor_core::export::ExportRange::Timeline
    );
    assert_eq!(preset.settings.subtitles.track, None);
    assert!(preset.note.contains("H.264"), "{}", preset.note);
    // The same name replaces, rather than repeats.
    let id = preset.id.clone();
    assert_eq!(
        presets.keep("short", &ExportSettings::default()).unwrap(),
        id
    );
    assert_eq!(presets.entries.len(), 1);
}

#[test]
fn names_are_bounded_and_so_is_the_list() {
    assert_eq!(name_of("   "), None);
    assert_eq!(name_of("a\u{7}b"), Some("ab".to_owned()));
    let long = "x".repeat(200);
    assert_eq!(name_of(&long).unwrap().chars().count(), NAME_LENGTH);
    let mut presets = ExportPresets::default();
    assert!(presets.keep(" ", &ExportSettings::default()).is_err());
    for n in 0..MOST_PRESETS {
        presets
            .keep(&format!("P{n}"), &ExportSettings::default())
            .unwrap();
    }
    let refused = presets.keep("One more", &ExportSettings::default());
    assert!(refused.unwrap_err().contains("100"));
}

#[test]
fn a_newer_or_damaged_file_is_never_written_over() {
    let scratch = Scratch::new();
    let path = scratch.file();
    assert_eq!(ExportPresets::read(&path), Read::Missing);
    for (text, why) in [
        (r#"{"version":2,"presets":[]}"#, "newer"),
        ("{not json", "damaged"),
        (r#"{"presets":[]}"#, "version"),
        (r#"{"version":1,"presets":{}}"#, "list"),
    ] {
        std::fs::write(&path, text).unwrap();
        match ExportPresets::read(&path) {
            Read::Unreadable(said) => assert!(said.contains(why), "{text}: {said}"),
            other => panic!("{text}: {other:?}"),
        }
    }
    // Saving over an unreadable file moves it aside first.
    let mut presets = ExportPresets::default();
    presets.keep("Fresh", &ExportSettings::default()).unwrap();
    let aside = presets
        .save(&path, true)
        .unwrap()
        .expect("the old file is kept");
    assert_eq!(
        std::fs::read_to_string(&aside).unwrap(),
        r#"{"version":1,"presets":{}}"#
    );
    assert_eq!(read(&path).presets().count(), 1);
}

#[test]
fn an_unread_file_stays_protected_until_a_write_has_moved_it() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let unread = r#"{"version":2,"presets":[]}"#;
    let mut presets = ExportPresets::default();
    presets.keep("Fresh", &ExportSettings::default()).unwrap();

    // The newer write lands first: it moves the file aside, and the older
    // one is not made.
    std::fs::write(&path, unread).unwrap();
    let mut writes = Writes::new(true);
    let kept = writes
        .save(&presets, &path, 2)
        .expect("the newest write is made")
        .unwrap()
        .expect("the unread file is kept");
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), unread);
    assert!(writes.save(&presets, &path, 1).is_none());
    assert_eq!(read(&path).presets().count(), 1);

    // A write that fails leaves the file to be moved by the next one.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::remove_file(&kept).unwrap();
        std::fs::write(&path, unread).unwrap();
        let mut writes = Writes::new(true);
        let folder = scratch.0.clone();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let failed = writes.save(&presets, &path, 1).expect("the write is tried");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if failed.is_ok() {
            eprintln!("SKIP: this user writes into read-only folders");
            return;
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), unread);
        let kept = writes
            .save(&presets, &path, 2)
            .expect("the next write is made")
            .unwrap()
            .expect("the unread file is moved aside, not written over");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), unread);
        assert_eq!(read(&path).presets().count(), 1);
        // Once moved, later writes replace the file as usual.
        assert_eq!(writes.save(&presets, &path, 3).unwrap().unwrap(), None);
    }
}
