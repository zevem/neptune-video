//! Captions made from the timeline's sound, as the desktop asks for them:
//! the person's choices, where models are kept, and the one command the
//! words heard become. The hearing itself is the media crate's speech
//! worker, and nothing is edited until its words come back.
use crate::{languages, ui::captions_tab};
use editor_core::{
    Command, Project, Time, TrackId,
    captions::{self, Chunking, SpokenWord},
};
use editor_media::speech::{self, DETECTOR, MODELS, Model};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How much of what is said one caption shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueLength {
    /// A few words at a time, as short vertical videos show them.
    #[default]
    Phrases,
    /// Full lines, as subtitles are.
    Lines,
    /// One word at a time.
    Words,
}

impl CueLength {
    pub const ALL: [Self; 3] = [Self::Phrases, Self::Lines, Self::Words];

    pub fn chunking(self) -> Chunking {
        match self {
            Self::Phrases => Chunking::PHRASES,
            Self::Lines => Chunking::LINES,
            Self::Words => Chunking::WORDS,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Phrases => "A few words",
            Self::Lines => "Full lines",
            Self::Words => "One word",
        }
    }

    /// What the length is for.
    pub fn note(self) -> &'static str {
        match self {
            Self::Phrases => "Shorts, Reels",
            Self::Lines => "Subtitles",
            Self::Words => "Fast cuts",
        }
    }
}

/// What the person chose for captions from speech, kept with their
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    /// The model of the catalog, by id.
    pub model: String,
    /// A model file chosen by hand, used instead of the catalog's.
    pub file: Option<PathBuf>,
    /// The language spoken, as a tag; `None` tells it from the sound.
    pub language: Option<String>,
    pub length: CueLength,
    /// Write English whatever is spoken.
    pub translate: bool,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            model: speech::DEFAULT_MODEL.to_owned(),
            file: None,
            language: None,
            length: CueLength::default(),
            translate: false,
        }
    }
}

impl SpeechSettings {
    /// The catalog's model chosen, or its default for an id it no longer
    /// offers.
    pub fn catalog(&self) -> &'static Model {
        Model::by_id(&self.model)
            .or_else(|| Model::by_id(speech::DEFAULT_MODEL))
            .unwrap_or(&MODELS[0])
    }

    /// The model the next captions are made with.
    pub fn choice(&self) -> ModelChoice {
        match &self.file {
            Some(file) => ModelChoice::File(file.clone()),
            None => ModelChoice::Catalog(self.catalog()),
        }
    }
}

/// Where downloaded models are kept: beside the settings a window was told
/// to use, else in the person's local data folder. `None` on a system with
/// no home.
pub fn models_dir(told_settings: Option<&Path>) -> Option<PathBuf> {
    match told_settings {
        Some(settings) => settings.parent().map(|dir| dir.join("models")),
        None => directories::ProjectDirs::from("app", "Aster", "Aster")
            .map(|dirs| dirs.data_local_dir().join("models")),
    }
}

/// Which models are in the models folder, as a worker last saw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inventory {
    pub installed: Vec<&'static str>,
    /// The speech detector is there.
    pub detector: bool,
}

impl Inventory {
    /// Looks in `dir`. File I/O: a worker's.
    pub fn look(dir: &Path) -> Self {
        Self {
            installed: (MODELS.iter())
                .filter(|model| model.installed(dir))
                .map(|model| model.id)
                .collect(),
            detector: DETECTOR.installed(dir),
        }
    }

    pub fn has(&self, model: &Model) -> bool {
        self.installed.contains(&model.id)
    }
}

/// Which model hears the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelChoice {
    /// One of the catalog's, downloaded first when it is not there.
    Catalog(&'static Model),
    /// A file the person chose by hand. Nothing is downloaded for it.
    File(PathBuf),
}

/// Captions to make, as they were asked for: everything is captured when
/// the person presses Generate.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeechRequest {
    pub start: Time,
    pub end: Time,
    pub model: ModelChoice,
    /// The language tag chosen, or `None` to tell it from the sound.
    pub language: Option<String>,
    pub translate: bool,
    pub length: CueLength,
}

impl SpeechRequest {
    /// The language Whisper is told, when it knows the one chosen.
    pub fn whisper_language(&self) -> Option<String> {
        (self.language.as_deref())
            .and_then(speech::whisper_language)
            .map(str::to_owned)
    }
}

/// What the speech worker is doing, as the Captions tab shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SpeechView {
    #[default]
    Idle,
    /// A download or a transcription runs. `stage` is `None` until the
    /// worker first says how far it is.
    Working {
        stage: Option<speech::Stage>,
        done: f32,
        /// A cancel was asked for and the worker has not stopped yet.
        stopping: bool,
    },
    /// Captions came while adding them would have cost something, and wait
    /// for the person: `why` says what.
    Ready {
        count: usize,
        language: String,
        why: String,
    },
}

impl SpeechView {
    pub fn working(&self) -> bool {
        matches!(self, Self::Working { .. })
    }
}

/// What a job under way is doing, in a few words.
pub fn stage_text(stage: Option<speech::Stage>, stopping: bool) -> String {
    use speech::Stage;
    if stopping {
        return "Stopping…".to_owned();
    }
    match stage {
        None => "Starting…".to_owned(),
        Some(Stage::Downloading(name)) => format!("Downloading {name}…"),
        Some(Stage::Checking) => "Checking the download…".to_owned(),
        Some(Stage::Loading) => "Opening the model…".to_owned(),
        Some(Stage::Listening) => "Listening to the timeline…".to_owned(),
        Some(Stage::Recognizing) => "Writing what is said…".to_owned(),
    }
}

/// The language of captions made: English when translated, else the one
/// chosen, else the one Whisper heard, else English.
pub fn caption_language(chosen: Option<&str>, translate: bool, heard: Option<&str>) -> String {
    if translate {
        return "en".to_owned();
    }
    if let Some(chosen) = chosen.and_then(languages::tag) {
        return chosen;
    }
    // Whisper calls Javanese "jw"; its tag is "jv".
    let heard = heard.map(|code| if code == "jw" { "jv" } else { code });
    heard
        .and_then(languages::tag)
        .unwrap_or_else(|| "en".to_owned())
}

/// The one command the words heard become: a new caption track in
/// `language` of captions as long as `length` asks, in the caption look,
/// with each word's time for karaoke styles. One step of history.
pub fn generated(
    project: &Project,
    words: &[SpokenWord],
    language: &str,
    length: CueLength,
) -> Result<(Command, TrackId, usize), String> {
    if words.iter().all(|word| word.text.trim().is_empty()) {
        return Err("No speech was heard there.".to_owned());
    }
    let cues = captions::cues(words, &length.chunking());
    captions_tab::from_cues(project, &cues, language, "Generate captions")
        .map_err(|_| "What was heard was too short to show as captions.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{ClipContent, Editor, TrackKind};

    fn word(text: &str, at: f64, end: f64) -> SpokenWord {
        SpokenWord {
            text: text.to_owned(),
            at: Time::from_seconds(at),
            end: Time::from_seconds(end),
        }
    }

    #[test]
    fn heard_words_become_one_step_that_adds_a_caption_track() {
        let mut editor = Editor::new(Project::new("Speech")).unwrap();
        let words = [
            word(" Three", 1.0, 1.4),
            word(" things", 1.4, 1.8),
            word(" nobody", 1.8, 2.2),
            word(" tells", 2.2, 2.5),
            word(" you", 2.5, 2.7),
            word(".", 2.7, 2.75),
        ];
        let (command, track_id, count) =
            generated(editor.project(), &words, "en", CueLength::Phrases).unwrap();
        assert_eq!(count, 2);
        let outcome = editor.dispatch(command).unwrap();
        assert_eq!(outcome.label, "Generate captions");
        let track = (editor.project().tracks.iter())
            .find(|track| track.id == track_id)
            .unwrap();
        assert_eq!(track.kind, TrackKind::Text);
        assert_eq!(track.language.as_deref(), Some("en"));
        let texts: Vec<_> = (track.clips.iter())
            .map(|clip| match &clip.content {
                ClipContent::Text { text, .. } => text.clone(),
                _ => String::new(),
            })
            .collect();
        // A phrase holds 24 characters at most.
        assert_eq!(texts, ["Three things nobody", "tells you."]);
        // Each caption keeps its words' times for karaoke.
        assert!(track.clips.iter().all(|clip| match &clip.content {
            ClipContent::Text { words, text, .. } =>
                words.len() == editor_core::title::word_ranges(text).len(),
            _ => false,
        }));
        assert_eq!(track.clips[0].start, Time::from_seconds(1.0));
        assert!(editor.undo());
        assert!(
            editor
                .project()
                .tracks
                .iter()
                .all(|track| track.id != track_id)
        );
    }

    #[test]
    fn longer_or_shorter_captions_come_from_the_same_words() {
        let project = Project::new("Speech");
        let words: Vec<_> = (0..12)
            .map(|index| {
                word(
                    " word",
                    f64::from(index) * 0.3,
                    f64::from(index) * 0.3 + 0.25,
                )
            })
            .collect();
        let count = |length| generated(&project, &words, "en", length).unwrap().2;
        assert_eq!(count(CueLength::Words), 12);
        assert!(count(CueLength::Phrases) < 12);
        assert!(count(CueLength::Lines) < count(CueLength::Phrases));
    }

    #[test]
    fn nothing_heard_is_said_and_adds_nothing() {
        let project = Project::new("Speech");
        assert_eq!(
            generated(&project, &[], "en", CueLength::Lines).unwrap_err(),
            "No speech was heard there."
        );
        assert!(generated(&project, &[word(" ", 1.0, 2.0)], "en", CueLength::Lines).is_err());
    }

    #[test]
    fn captions_are_in_the_language_chosen_translated_or_heard() {
        assert_eq!(caption_language(Some("pt-BR"), false, Some("pt")), "pt-BR");
        assert_eq!(caption_language(Some("pt-BR"), true, Some("pt")), "en");
        assert_eq!(caption_language(None, false, Some("ja")), "ja");
        assert_eq!(caption_language(None, false, Some("jw")), "jv");
        assert_eq!(caption_language(None, false, None), "en");
        let request = SpeechRequest {
            start: Time::ZERO,
            end: Time::from_seconds(1.0),
            model: ModelChoice::Catalog(&MODELS[0]),
            language: Some("pt-BR".to_owned()),
            translate: false,
            length: CueLength::Lines,
        };
        assert_eq!(request.whisper_language().as_deref(), Some("pt"));
        let unknown = SpeechRequest {
            language: Some("x-klingon".to_owned()),
            ..request
        };
        assert_eq!(unknown.whisper_language(), None);
    }

    #[test]
    fn the_choices_survive_a_round_trip_and_an_unknown_model_is_the_default() {
        let settings = SpeechSettings {
            model: "small".to_owned(),
            file: None,
            language: Some("es".to_owned()),
            length: CueLength::Words,
            translate: true,
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<SpeechSettings>(&text).unwrap(),
            settings
        );
        let read: SpeechSettings = serde_json::from_str(r#"{"model":"huge"}"#).unwrap();
        assert_eq!(read.catalog().id, speech::DEFAULT_MODEL);
        assert_eq!(read.length, CueLength::Phrases);
        let chosen = SpeechSettings {
            file: Some(PathBuf::from("/models/mine.bin")),
            ..SpeechSettings::default()
        };
        assert_eq!(
            chosen.choice(),
            ModelChoice::File(PathBuf::from("/models/mine.bin"))
        );
    }

    #[test]
    fn models_live_beside_settings_a_window_was_told_to_use() {
        assert_eq!(
            models_dir(Some(Path::new("/tmp/run/settings.json"))),
            Some(PathBuf::from("/tmp/run/models"))
        );
        let dir = std::env::temp_dir().join(format!("aster-test-{}", uuid::Uuid::new_v4()));
        assert_eq!(Inventory::look(&dir), Inventory::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(DETECTOR.path(&dir), vec![0; DETECTOR.bytes as usize]).unwrap();
        let seen = Inventory::look(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(seen.detector);
        assert!(!seen.has(&MODELS[0]));
    }
}
