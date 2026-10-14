use crate::{
    ImportError, MediaError, io,
    process::{Processes, capture, tool_command},
};
use editor_core::{Asset, FolderId, FrameRate, MediaFolder, MediaKind, Time};
use image::{AnimationDecoder, ImageDecoder, ImageReader, codecs::gif::GifDecoder};
use serde_json::Value;
use std::{
    collections::{BTreeSet, VecDeque},
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

const MAX_IMPORT_FILES: usize = 10_000;
const MAX_SOURCE_PIXELS: u64 = 64_000_000;

pub(crate) struct Imported {
    pub assets: Vec<Asset>,
    pub folders: Vec<MediaFolder>,
    pub errors: Vec<ImportError>,
}

pub(crate) fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "mp4"
                    | "mov"
                    | "mkv"
                    | "webm"
                    | "avi"
                    | "m4v"
                    | "mpeg"
                    | "mpg"
                    | "mts"
                    | "m2ts"
                    | "ts"
                    | "ogv"
                    | "wav"
                    | "mp3"
                    | "m4a"
                    | "aac"
                    | "flac"
                    | "ogg"
                    | "opus"
                    | "aiff"
                    | "aif"
                    | "wma"
                    | "cube"
            )
        })
}

pub(crate) fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits
}

pub(crate) fn image_reader(path: &Path) -> Result<ImageReader<BufReader<File>>, MediaError> {
    let mut reader = ImageReader::open(path)
        .map_err(|error| io(format!("Cannot open {}", path.display()), error))?
        .with_guessed_format()
        .map_err(|error| io("Cannot recognize image", error))?;
    reader.limits(limits());
    Ok(reader)
}

pub(crate) fn import_paths(
    paths: Vec<PathBuf>,
    processes: &Processes,
    cancelled: impl Fn() -> bool,
) -> Imported {
    let mut result = Imported {
        assets: Vec::new(),
        folders: Vec::new(),
        errors: Vec::new(),
    };
    let mut queue: VecDeque<_> = paths.into_iter().map(|path| (path, None, true)).collect();
    let mut seen = BTreeSet::new();
    let mut visited = 0;
    while let Some((path, parent, explicit)) = queue.pop_front() {
        if cancelled() {
            break;
        }
        visited += 1;
        if visited > MAX_IMPORT_FILES {
            result.errors.push(ImportError { path, message: "Import is limited to 10,000 entries per batch; import additional folders separately".into() });
            break;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                result.errors.push(ImportError {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        // Do not follow directory symlinks: a tree may point back to its ancestors.
        if metadata.file_type().is_symlink() && path.is_dir() {
            continue;
        }
        let absolute = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                result.errors.push(ImportError {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !seen.insert(absolute.clone()) {
            // A directly selected file may be encountered again through an
            // imported folder. Preserve that folder membership without duplicates.
            if parent.is_some()
                && let Some(asset) = result
                    .assets
                    .iter_mut()
                    .find(|asset| asset.path == absolute)
            {
                asset.folder_id = parent;
            }
            continue;
        }
        if metadata.is_dir() {
            let id = FolderId::new_v4();
            result.folders.push(MediaFolder {
                id,
                parent_id: parent,
                name: absolute
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Media".into()),
            });
            match fs::read_dir(&absolute) {
                Ok(entries) => {
                    let mut children = Vec::new();
                    for entry in entries.take(MAX_IMPORT_FILES + 1) {
                        match entry {
                            Ok(entry) => children.push(entry.path()),
                            Err(error) => result.errors.push(ImportError {
                                path: absolute.clone(),
                                message: error.to_string(),
                            }),
                        }
                    }
                    children.sort();
                    for child in children {
                        if queue.len() < MAX_IMPORT_FILES {
                            queue.push_back((child, Some(id), false));
                        } else {
                            result.errors.push(ImportError {
                                path: absolute.clone(),
                                message: "Folder exceeds the 10,000-entry import budget".into(),
                            });
                            break;
                        }
                    }
                }
                Err(error) => result.errors.push(ImportError {
                    path: absolute,
                    message: error.to_string(),
                }),
            }
        } else if supported(&absolute) {
            match probe(&absolute, processes, &cancelled) {
                Ok(mut asset) => {
                    asset.folder_id = parent;
                    result.assets.push(asset);
                }
                Err(error) => result.errors.push(ImportError {
                    path: absolute,
                    message: error.to_string(),
                }),
            }
        } else if explicit {
            result.errors.push(ImportError {
                path: absolute,
                message: "Unsupported media file".into(),
            });
        }
    }
    result
}

/// One media file as an asset: what ffprobe, or the image decoder, reads of
/// it. The import's own step, called directly by jobs that make media.
pub(crate) fn probe(
    path: &Path,
    processes: &Processes,
    cancelled: &impl Fn() -> bool,
) -> Result<Asset, MediaError> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut asset = if ext == "cube" {
        // The whole table is read here, so that one that cannot be used is
        // refused at the door and not when an effect first needs it.
        let cube = crate::lut::read(path)?;
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        // A table has no picture and no time: the shortest duration an
        // asset may have, and its points along an edge where a width goes.
        let mut asset = Asset::new(path.to_owned(), MediaKind::Lut, Time(1));
        asset.width = cube.size;
        asset
    } else if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
    ) {
        let (width, height) = image_reader(path)?
            .into_dimensions()
            .map_err(|error| MediaError::Decode(format!("{}: {error}", path.display())))?;
        if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
            return Err(MediaError::Invalid(
                "Image exceeds the 64 megapixel decode budget".into(),
            ));
        }
        let (kind, duration, frame_rate) = if ext == "gif" {
            let file = File::open(path).map_err(|error| io("Cannot open GIF", error))?;
            let mut decoder = GifDecoder::new(BufReader::new(file))
                .map_err(|error| MediaError::Decode(error.to_string()))?;
            decoder
                .set_limits(limits())
                .map_err(|error| MediaError::Decode(error.to_string()))?;
            let mut micros: i64 = 0;
            let mut first_delay = None;
            let mut uniform = true;
            for (index, frame) in decoder.into_frames().enumerate() {
                if cancelled() {
                    return Err(MediaError::Cancelled);
                }
                if index >= 10_000 {
                    return Err(MediaError::Invalid("GIF exceeds 10,000 frames".into()));
                }
                let frame = frame.map_err(|error| MediaError::Decode(error.to_string()))?;
                let delay = gif_delay_us(frame.delay());
                if let Some(first) = first_delay {
                    uniform &= first == delay;
                } else {
                    first_delay = Some(delay);
                }
                micros = micros.saturating_add(delay);
            }
            if first_delay.is_none() {
                return Err(MediaError::Invalid("GIF has no frames".into()));
            }
            (
                MediaKind::Gif,
                Time(micros),
                first_delay.filter(|_| uniform).and_then(uniform_gif_rate),
            )
        } else {
            (MediaKind::Image, Time::from_seconds(5.0), None)
        };
        let mut asset = Asset::new(path.to_owned(), kind, duration);
        asset.width = width;
        asset.height = height;
        asset.frame_rate = frame_rate;
        asset
    } else {
        let mut command = tool_command("ffprobe");
        command.args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height,duration,avg_frame_rate,r_frame_rate:stream_disposition=attached_pic:stream_tags=rotate:stream_side_data=rotation", "-of", "json"])
            .arg(path);
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let bytes = capture("ffprobe", &mut command, processes, 1024 * 1024)?;
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        asset_from_probe(path, &bytes)?
    };
    asset.file_size = fs::metadata(path)
        .map_err(|error| io("Cannot read media size", error))?
        .len();
    Ok(asset)
}

/// GIF's zero/very short delays are displayed for at least one centisecond.
/// Import duration and preview interval lookup use this same effective delay.
pub(crate) fn gif_delay_us(delay: image::Delay) -> i64 {
    let (numerator, denominator) = delay.numer_denom_ms();
    (i64::from(numerator) * 1000 / i64::from(denominator.max(1))).max(10_000)
}

fn uniform_gif_rate(delay_us: i64) -> Option<FrameRate> {
    let mut a = 1_000_000_u32;
    let mut b = u32::try_from(delay_us).ok()?;
    let denominator = b;
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    let rate = FrameRate {
        numerator: 1_000_000 / a,
        denominator: denominator / a,
    };
    rate.validate().then_some(rate)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| value.is_finite())
}

pub(crate) fn asset_from_probe(path: &Path, bytes: &[u8]) -> Result<Asset, MediaError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| MediaError::Decode(format!("Invalid ffprobe response: {error}")))?;
    let streams = value["streams"]
        .as_array()
        .ok_or_else(|| MediaError::Invalid("File contains no media streams".into()))?;
    let audio = streams.iter().any(|stream| stream["codec_type"] == "audio");
    let video = streams.iter().find(|stream| {
        stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
    });
    if video.is_none() && !audio {
        return Err(MediaError::Invalid(
            "File contains no usable video or audio stream".into(),
        ));
    }
    let seconds = number(&value["format"]["duration"]).or_else(|| {
        streams
            .iter()
            .filter_map(|stream| number(&stream["duration"]))
            .max_by(f64::total_cmp)
    });
    let seconds = seconds
        .filter(|seconds| *seconds > 0.0 && *seconds <= Time::MAX.as_seconds())
        .ok_or_else(|| MediaError::Invalid("Media has no finite positive duration".into()))?;
    let mut asset = Asset::new(
        path.to_owned(),
        if video.is_some() {
            MediaKind::Video
        } else {
            MediaKind::Audio
        },
        Time::from_seconds(seconds),
    );
    asset.has_audio = audio;
    if let Some(video) = video {
        asset.width = video["width"].as_u64().unwrap_or(0) as u32;
        asset.height = video["height"].as_u64().unwrap_or(0) as u32;
        let rotation = video["side_data_list"]
            .as_array()
            .and_then(|values| values.iter().find_map(|value| number(&value["rotation"])))
            .or_else(|| number(&video["tags"]["rotate"]))
            .unwrap_or(0.0);
        if (rotation.abs() % 180.0 - 90.0).abs() < 1.0 {
            std::mem::swap(&mut asset.width, &mut asset.height);
        }
        asset.frame_rate = ["avg_frame_rate", "r_frame_rate"].iter().find_map(|key| {
            let (numerator, denominator) = video[*key].as_str()?.split_once('/')?;
            let (numerator, denominator) = (
                numerator.parse::<u32>().ok()?,
                denominator.parse::<u32>().ok()?,
            );
            (numerator != 0
                && denominator != 0
                && numerator >= denominator
                && f64::from(numerator) / f64::from(denominator) <= 240.0)
                .then_some(FrameRate {
                    numerator,
                    denominator,
                })
        });
        if asset.width == 0 || asset.height == 0 {
            return Err(MediaError::Invalid("Video has no usable dimensions".into()));
        }
    }
    Ok(asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_distinguishes_audio_cover_art_and_honors_rotation_and_rational_rate() {
        let bytes = br#"{"format":{"duration":"2.5"},"streams":[{"codec_type":"video","width":1080,"height":1920,"avg_frame_rate":"30000/1001","side_data_list":[{"rotation":90}]},{"codec_type":"audio"}]}"#;
        let asset = asset_from_probe(Path::new("clip.mp4"), bytes).unwrap();
        assert_eq!((asset.width, asset.height), (1920, 1080));
        assert_eq!(asset.duration, Time(2_500_000));
        assert!(asset.has_audio);
        assert_eq!(asset.frame_rate.unwrap().denominator, 1001);
        let audio = asset_from_probe(Path::new("song.mp3"), br#"{"format":{"duration":"1"},"streams":[{"codec_type":"audio"},{"codec_type":"video","disposition":{"attached_pic":1}}]}"#).unwrap();
        assert_eq!(audio.kind, MediaKind::Audio);
    }

    #[test]
    fn folder_import_ignores_unrelated_files_and_reuses_canonical_paths() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("nested");
        fs::create_dir(&folder).unwrap();
        let path = folder.join("still.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        fs::write(folder.join("notes.txt"), "ignore").unwrap();
        let imported = import_paths(
            vec![temp.path().into(), path],
            &Processes::default(),
            || false,
        );
        assert_eq!(imported.assets.len(), 1);
        assert_eq!(imported.folders.len(), 2);
        assert_eq!(imported.assets[0].folder_id, Some(imported.folders[1].id));
        assert!(imported.errors.is_empty());
    }

    #[test]
    fn a_cube_file_is_a_lut_asset_and_a_bad_one_is_refused_with_its_reason() {
        let temp = tempfile::tempdir().unwrap();
        let good = temp.path().join("Look.CUBE");
        fs::write(&good, crate::lut::tests::text(17, |c| c)).unwrap();
        let flat = temp.path().join("flat.cube");
        fs::write(&flat, "LUT_1D_SIZE 4\n0 0 0\n").unwrap();
        let huge = temp.path().join("huge.cube");
        fs::write(&huge, "LUT_3D_SIZE 128\n").unwrap();
        let holed = temp.path().join("holed.cube");
        fs::write(&holed, "LUT_3D_SIZE 2\n0 0 0\n0 nan 0\n").unwrap();
        let short = temp.path().join("short.cube");
        fs::write(&short, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        let imported = import_paths(
            vec![good.clone(), flat, huge, holed, short],
            &Processes::default(),
            || false,
        );
        assert_eq!(imported.assets.len(), 1);
        let asset = &imported.assets[0];
        assert_eq!(asset.kind, MediaKind::Lut);
        assert_eq!(asset.name, "Look.CUBE");
        assert_eq!((asset.width, asset.height), (17, 0));
        assert_eq!(asset.duration, Time(1));
        assert!(!asset.has_audio && asset.frame_rate.is_none());
        assert_eq!(asset.file_size, fs::metadata(&good).unwrap().len());
        // What import describes is what a project accepts.
        let mut project = editor_core::Project::new("Looks");
        std::sync::Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.validate().unwrap();
        let said: Vec<_> = (imported.errors.iter())
            .map(|error| error.message.as_str())
            .collect();
        assert_eq!(said.len(), 4);
        assert!(said[0].contains("1D LUT"), "{said:?}");
        assert!(said[1].contains("outside 2 to 65"), "{said:?}");
        assert!(said[2].contains("three finite numbers"), "{said:?}");
        assert!(said[3].contains("fewer rows"), "{said:?}");
    }

    #[test]
    fn gif_metadata_only_exposes_a_frame_grid_when_all_delays_are_uniform() {
        let temp = tempfile::tempdir().unwrap();
        for (name, delays, expected) in [
            (
                "uniform.gif",
                [30, 30],
                Some(FrameRate {
                    numerator: 100,
                    denominator: 3,
                }),
            ),
            ("variable.gif", [100, 200], None),
        ] {
            let path = temp.path().join(name);
            let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
            for delay in delays {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
            drop(encoder);
            let asset = probe(&path, &Processes::default(), &|| false).unwrap();
            assert_eq!(asset.frame_rate, expected);
            assert_eq!(
                asset.duration,
                Time(i64::from(delays.iter().sum::<u32>()) * 1000)
            );
        }
    }
}
