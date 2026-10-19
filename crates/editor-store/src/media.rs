//! Media the editor itself writes for a saved document: voiceover takes and
//! separated stems. They are the person's files, kept in `<stem> media/` beside
//! the document. The folder is made on first use and the editor never empties,
//! moves or deletes it or anything in it, also not on undo. Assets in it are
//! saved by absolute path like any other, so they do not move with the document.

use crate::StoreError;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Names tried for takes started in one second before giving up.
const MAX_SAME_SECOND: u32 = 999;

/// The media folder of the document at `document`: `film.nvp` keeps its
/// recordings in `film media/` in the same folder. Nothing is created.
pub fn media_folder(document: &Path) -> Result<PathBuf, StoreError> {
    let stem = document
        .file_stem()
        .ok_or_else(|| StoreError::Invalid("The project has no file to record beside".into()))?;
    let mut name = stem.to_os_string();
    name.push(" media");
    Ok(document.with_file_name(name))
}

/// A path for a new voiceover take of the document at `document`, started at
/// `now`: `Voiceover 2026-10-14 15.30.12 UTC.wav`, with ` 2`, ` 3`… when the
/// name is taken. The media folder exists on return; the take does not, and
/// its writer opens it with `create_new`, which refuses one made meanwhile.
/// A name is free only when nothing, not even a dangling link, holds it.
/// Filesystem work, possibly on a slow volume: call it off the frame loop.
pub fn new_take(document: &Path, now: SystemTime) -> Result<PathBuf, StoreError> {
    let folder = media_folder(document)?;
    match fs::metadata(&folder) {
        Ok(found) if !found.is_dir() => {
            return Err(StoreError::Invalid(format!(
                "Recordings go in {}, which is not a folder",
                folder.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir_all(&folder)?,
        Err(error) => return Err(error.into()),
    }
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::Invalid("System clock precedes Unix epoch".into()))?
        .as_secs();
    let stamp = utc_stamp(seconds);
    for n in 1..=MAX_SAME_SECOND {
        let path = match n {
            1 => folder.join(format!("Voiceover {stamp}.wav")),
            n => folder.join(format!("Voiceover {stamp} {n}.wav")),
        };
        // `create_new` refuses a link, dangling or not, so one is a taken name.
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(path),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
    }
    Err(StoreError::Invalid(format!(
        "Too many takes named for {stamp} in {}",
        folder.display()
    )))
}

/// `YYYY-MM-DD HH.MM.SS UTC`. The workspace has no time zone database, so the
/// name says which clock it is; dots, not colons, which Windows refuses.
fn utc_stamp(seconds: u64) -> String {
    let (days, time) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}.{:02}.{:02} UTC",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Hinnant's
/// `civil_from_days`, for days on or after the epoch).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::utc_stamp;

    #[test]
    fn stamps_are_utc_dates_and_times_without_colons() {
        assert_eq!(utc_stamp(0), "1970-01-01 00.00.00 UTC");
        assert_eq!(utc_stamp(1_791_991_812), "2026-10-14 15.30.12 UTC");
        assert_eq!(utc_stamp(1_709_251_199), "2024-02-29 23.59.59 UTC");
        assert_eq!(utc_stamp(1_709_251_200), "2024-03-01 00.00.00 UTC");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29 00.00.00 UTC");
        assert_eq!(utc_stamp(4_107_542_400), "2100-03-01 00.00.00 UTC");
    }
}
