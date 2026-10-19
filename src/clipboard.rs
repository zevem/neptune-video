//! Copied clips as text on the system clipboard.
//!
//! The window is only told about a paste when the system clipboard holds
//! text, so a copy publishes the clips there. The text names itself, and
//! anything else on the clipboard is not taken for clips.
use editor_core::{Asset, AssetId, Clip, Clipboard, ClipboardClip, Time, TrackKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const PREFIX: &str = "{\"neptune_video_clips\":";
/// Written as 2 since clips carry looks, motion and time maps, and as 3 since
/// they carry fades, so a build that would strip them does not take the text
/// for clips. Version 1 and 2 text is read.
const VERSION: u32 = 3;
/// Longer text is not looked at: no copy of clips comes near it.
const LIMIT: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Payload {
    neptune_video_clips: u32,
    origin: Time,
    clips: Vec<Item>,
}

#[derive(Serialize, Deserialize)]
struct Item {
    track_index: usize,
    track_kind: TrackKind,
    clip: Clip,
}

/// Whether a copied clip uses media that is not among `assets`: the file
/// it plays, or one an effect of it reads, such as a LUT or the clean plate
/// of a difference matte. Clips copied in another project may, and so may
/// clips whose media has since left the library; pasted, they would be
/// refused for an ID that says nothing to the person.
pub fn lacks_media(clipboard: &Clipboard, assets: &BTreeMap<AssetId, Asset>) -> bool {
    clipboard.clips.iter().any(|item| {
        (item.clip.asset_id().into_iter())
            .chain(
                item.clip
                    .effects
                    .iter()
                    .filter_map(|effect| effect.asset_id),
            )
            .any(|id| !assets.contains_key(&id))
    })
}

pub fn encode(clipboard: &Clipboard) -> Option<String> {
    serde_json::to_string(&Payload {
        neptune_video_clips: VERSION,
        origin: clipboard.origin,
        clips: clipboard
            .clips
            .iter()
            .map(|item| Item {
                track_index: item.track_index,
                track_kind: item.track_kind,
                clip: item.clip.clone(),
            })
            .collect(),
    })
    .ok()
}

/// The clips in `text`, when it is a copy made by [`encode`].
pub fn decode(text: &str) -> Option<Clipboard> {
    if text.len() > LIMIT || !text.starts_with(PREFIX) {
        return None;
    }
    let payload: Payload = serde_json::from_str(text).ok()?;
    if !(1..=VERSION).contains(&payload.neptune_video_clips) || payload.clips.is_empty() {
        return None;
    }
    Some(Clipboard {
        origin: payload.origin,
        clips: payload
            .clips
            .into_iter()
            .map(|item| ClipboardClip {
                track_index: item.track_index,
                track_kind: item.track_kind,
                clip: item.clip,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied() -> Clipboard {
        let mut clip = Clip::text("Title", Time::from_seconds(2.0), Time::from_seconds(3.0));
        clip.group_id = Some(uuid::Uuid::new_v4());
        Clipboard {
            origin: Time::from_seconds(2.0),
            clips: vec![ClipboardClip {
                track_index: 2,
                track_kind: TrackKind::Text,
                clip,
            }],
        }
    }

    #[test]
    fn copied_clips_lack_media_an_effect_reads_as_well_as_media_they_play() {
        use editor_core::{Effect, EffectKind, MediaKind};
        let asset = |name: &str, kind| {
            Asset::new(
                std::path::PathBuf::from(name),
                kind,
                Time::from_seconds(4.0),
            )
        };
        let (shot, lut) = (
            asset("/media/shot.mp4", MediaKind::Video),
            asset("/media/look.cube", MediaKind::Lut),
        );
        let mut clip = Clip::from_asset(&shot, Time::ZERO);
        let mut lookup = Effect::new(EffectKind::Lut);
        lookup.asset_id = Some(lut.id);
        clip.effects.push(lookup);
        let clipboard = Clipboard {
            origin: Time::ZERO,
            clips: vec![ClipboardClip {
                track_index: 0,
                track_kind: TrackKind::Video,
                clip,
            }],
        };
        let library = |assets: &[&Asset]| -> BTreeMap<AssetId, Asset> {
            (assets.iter().map(|asset| (asset.id, (*asset).clone()))).collect()
        };
        assert!(!lacks_media(&clipboard, &library(&[&shot, &lut])));
        assert!(
            lacks_media(&clipboard, &library(&[&shot])),
            "the clip's own media is there, and the table its effect reads is not"
        );
        assert!(lacks_media(&clipboard, &library(&[&lut])));
        // A title plays nothing and reads nothing.
        assert!(!lacks_media(&copied(), &library(&[])));
    }

    #[test]
    fn copied_clips_survive_the_system_clipboard() {
        let clipboard = copied();
        let text = encode(&clipboard).expect("clips encode");
        let back = decode(&text).expect("clips decode");
        assert_eq!(back.origin, clipboard.origin);
        assert_eq!(back.clips.len(), 1);
        assert_eq!(back.clips[0].track_index, 2);
        assert_eq!(back.clips[0].track_kind, TrackKind::Text);
        assert_eq!(back.clips[0].clip, clipboard.clips[0].clip);
    }

    #[test]
    fn copied_fades_survive_the_system_clipboard() {
        let song = Asset::new(
            std::path::PathBuf::from("/media/song.wav"),
            editor_core::MediaKind::Audio,
            Time::from_seconds(30.0),
        );
        let mut clip = Clip::from_asset(&song, Time::from_seconds(2.0));
        clip.fade_in = Time::from_seconds(0.5);
        clip.fade_out = Time::from_seconds(2.0);
        let mut clipboard = copied();
        clipboard.clips.push(ClipboardClip {
            track_index: 1,
            track_kind: TrackKind::Audio,
            clip: clip.clone(),
        });
        let text = encode(&clipboard).expect("clips encode");
        let back = decode(&text).expect("clips decode");
        assert_eq!(back.clips.len(), 2);
        assert_eq!(back.clips[0].clip, clipboard.clips[0].clip);
        assert_eq!(back.clips[1].track_kind, TrackKind::Audio);
        assert_eq!(back.clips[1].clip, clip);
        assert_eq!(back.clips[1].clip.fade_in, Time::from_seconds(0.5));
        assert_eq!(back.clips[1].clip.fade_out, Time::from_seconds(2.0));
    }

    #[test]
    fn a_copied_audio_effect_survives_the_system_clipboard() {
        use editor_core::{Effect, EffectKind, ParamKey, ParamValue};
        let song = Asset::new(
            std::path::PathBuf::from("/media/song.wav"),
            editor_core::MediaKind::Audio,
            Time::from_seconds(30.0),
        );
        let mut clip = Clip::from_asset(&song, Time::from_seconds(2.0));
        let mut band = Effect::new(EffectKind::EqBand);
        band.params = vec![
            ParamValue {
                key: ParamKey::Frequency,
                value: 3_500.0,
            },
            ParamValue {
                key: ParamKey::Gain,
                value: 3.0,
            },
        ];
        let mut off = Effect::new(EffectKind::Denoise);
        off.enabled = false;
        clip.effects = vec![Effect::new(EffectKind::HighPass), band, off];
        let mut clipboard = copied();
        clipboard.clips.push(ClipboardClip {
            track_index: 1,
            track_kind: TrackKind::Audio,
            clip: clip.clone(),
        });
        let text = encode(&clipboard).expect("clips encode");
        assert!(text.contains(r#""kind":"eq_band""#), "{text}");
        let back = decode(&text).expect("clips decode");
        assert_eq!(back.clips[1].track_kind, TrackKind::Audio);
        assert_eq!(back.clips[1].clip.effects, clip.effects);
        assert_eq!(back.clips[1].clip, clip);
        // Neither plays nor reads media the library lacks.
        let library: BTreeMap<AssetId, Asset> = [(song.id, song)].into();
        assert!(!lacks_media(&back, &library));

        // Pasted into a project that has the song, the clip is as it was copied.
        let mut project = editor_core::Project::new("Paste");
        std::sync::Arc::make_mut(&mut project.assets).extend(library);
        let mut editor = editor_core::Editor::new(project).unwrap();
        editor.paste(&back, Time::from_seconds(2.0), None).unwrap();
        let pasted = &editor.project().tracks[1].clips[0];
        assert_eq!(pasted.effects.len(), 3);
        assert_eq!(pasted.effects[1].param(ParamKey::Frequency), Some(3_500.0));
    }

    #[test]
    fn a_copied_time_map_plays_the_same_source_when_pasted() {
        use editor_core::Ease::{Bezier, Linear};
        let ramp = Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        // Forwards, a freeze, backwards and a ramp in one map.
        let keys = [
            (0.0, 1.0, Linear),
            (1.0, 3.0, Linear),
            (2.0, 3.0, Linear),
            (3.0, 0.0, ramp),
            (4.0, 2.0, Linear),
        ];
        let mut clipboard = copied();
        let clip = &mut clipboard.clips[0].clip;
        clip.source_in = Time::from_seconds(5.0);
        clip.duration = Time::from_seconds(4.0);
        clip.retime = Some(editor_core::Retime {
            keys: keys
                .map(|(at, source, ease)| editor_core::TimeKey {
                    at: Time::from_seconds(at),
                    source: Time::from_seconds(source),
                    ease,
                })
                .to_vec(),
        });
        let clip = clip.clone();
        let text = encode(&clipboard).expect("clips encode");
        let back = decode(&text).expect("clips decode").clips.remove(0).clip;
        assert_eq!(back.source_span(), clip.source_span());
        for step in 0..=64 {
            let at = clip.start + Time(clip.duration.0 * step / 64);
            assert_eq!(back.source_time(at), clip.source_time(at));
            assert_eq!(back.speed_at(at), clip.speed_at(at));
        }
    }

    #[test]
    fn older_text_is_read_and_newer_text_is_left_alone() {
        let text = encode(&copied()).expect("clips encode");
        assert!(text.starts_with("{\"neptune_video_clips\":3,"));
        let as_version = |version: u32| {
            text.replacen(
                "\"neptune_video_clips\":3",
                &format!("\"neptune_video_clips\":{version}"),
                1,
            )
        };
        for version in [1, 2] {
            let older = decode(&as_version(version)).expect("an older version");
            assert_eq!(older.clips.len(), 1);
        }
        for version in [0, 4] {
            assert!(decode(&as_version(version)).is_none());
        }
    }

    #[test]
    fn other_text_is_not_taken_for_clips() {
        assert!(decode("a sentence someone copied").is_none());
        assert!(decode("{\"type\":\"rename_project\",\"name\":\"Film\"}").is_none());
        assert!(decode("{\"neptune_video_clips\":1,\"origin\":0,\"clips\":[]}").is_none());
        assert!(decode("{\"neptune_video_clips\":2,\"origin\":0,\"clips\":[]}").is_none());
        assert!(decode("{\"neptune_video_clips\":3,\"origin\":0,\"clips\":[]}").is_none());
        assert!(decode("{\"neptune_video_clips\":").is_none());
    }
}
