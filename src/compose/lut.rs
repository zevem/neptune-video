//! Lookup tables for the compositor: how a table is laid out as a texture,
//! and the few tables the viewer keeps.
//!
//! A table is read from its file by the media worker and kept here for as
//! long as a layer on screen looks colours up in it. [`Luts`] asks, keeps and
//! forgets; it never reads a file and never waits. Until a table has come, and
//! when its file cannot be read, the effect changes nothing.
use super::pass;
use crate::textures::LAYER_IDLE_FRAMES;
use editor_core::{Asset, AssetId, MediaKind};
use editor_media::{Cube, MediaError, MediaService};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// The tables kept at once, and so the most one frame can look up in.
pub const MAX_LUTS: usize = 4;
/// The bytes of one texel of a table's texture: four halves.
const TEXEL: usize = 8;

/// A table to draw with, and a name for it no other table has had.
#[derive(Clone, Copy)]
pub struct Lut<'a> {
    pub serial: u64,
    pub table: &'a Cube,
}

impl Lut<'_> {
    /// What a pass is told of the table.
    pub fn told(&self) -> pass::Table {
        pass::Table {
            serial: self.serial,
            size: self.table.size,
            across: layout(self.table.size).0,
            domain_min: self.table.domain_min,
            domain_max: self.table.domain_max,
        }
    }
}

/// How a table of `size` points an edge lies in a two-dimensional texture:
/// one square tile of `size` texels for each step of blue, red across it and
/// green down, the tiles in rows of the first number returned. The second is
/// the texture's size.
///
/// A table of 65 comes to 585 by 520 texels, which every device has room
/// for; a volume texture, or one row of tiles, would not fit them all.
pub fn layout(size: u32) -> (u32, [u32; 2]) {
    let size = size.max(1);
    let across = (1..=size).find(|n| n * n >= size).unwrap_or(size);
    (across, [across * size, size.div_ceil(across) * size])
}

/// The logical bytes a table is counted as while it is drawn with: its
/// colours as they are kept, and its texture.
pub fn bytes(table: &Cube) -> usize {
    let [width, height] = layout(table.size).1;
    table.byte_len() + width as usize * height as usize * TEXEL
}

/// A float as the nearest sixteen-bit one, and the largest of those for what
/// is past them all.
fn half(value: f32) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    let bits = value.clamp(-65504.0, 65504.0).to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x007f_ffff;
    if exponent <= 0 {
        // Too small for a normal half: a subnormal, or nothing.
        if exponent < -10 {
            return sign;
        }
        let mantissa = (mantissa | 0x0080_0000) >> (1 - exponent);
        return sign | ((mantissa + 0x1000) >> 13) as u16;
    }
    // A carry out of the mantissa goes on into the exponent, as it should.
    let half = ((exponent as u32) << 10) | (mantissa >> 13);
    sign | (half + ((mantissa >> 12) & 1)) as u16
}

/// The texels of a table's texture, row after row, as [`layout`] has them.
/// A tile the table has no step of blue for is left clear.
pub fn texels(table: &Cube) -> Vec<u8> {
    let size = table.size;
    let (across, [width, height]) = layout(size);
    let mut bytes = vec![0u8; width as usize * height as usize * TEXEL];
    for blue in 0..size {
        let (left, top) = ((blue % across) * size, (blue / across) * size);
        for green in 0..size {
            let row = ((top + green) * width + left) as usize * TEXEL;
            for red in 0..size {
                let [r, g, b] = table.at(red, green, blue);
                let texel = row + red as usize * TEXEL;
                for (channel, value) in [r, g, b, 1.0].into_iter().enumerate() {
                    bytes[texel + channel * 2..texel + channel * 2 + 2]
                        .copy_from_slice(&half(value).to_ne_bytes());
                }
            }
        }
    }
    bytes
}

enum State {
    /// Asked of the worker while it was at this generation.
    Asked(u64),
    Ready {
        serial: u64,
        table: Arc<Cube>,
    },
    /// Why there is no table, and the worker's generation when that was
    /// found out.
    Failed(String, u64),
}

struct Entry {
    /// The file the table is of, as the asset named it when it was asked for.
    path: PathBuf,
    file_size: u64,
    state: State,
    /// The frame a layer last wanted it in.
    used: u64,
}

impl Entry {
    fn is_of(&self, asset: &Asset) -> bool {
        self.path == asset.path && self.file_size == asset.file_size
    }
}

/// What became of a table the worker answered with.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken {
    /// It is kept, and the next frame is drawn with it.
    Kept,
    /// There is no table: why, in words for the person.
    Failed(String),
    /// The answer is about nothing that is waited for.
    Dropped,
}

/// The tables the viewer keeps: [`MAX_LUTS`] at the most, each for one LUT
/// asset as its file was when the table was asked for.
#[derive(Default)]
pub struct Luts {
    entries: HashMap<AssetId, Entry>,
    frame: u64,
    /// The frame a table was last wanted in.
    used: u64,
    serials: u64,
}

impl Luts {
    /// Begins a frame of the application. Every table is given up once none
    /// has been wanted for as many frames as an undrawn viewer layer is kept:
    /// it is read again when a layer next wants it.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if !self.entries.is_empty() && self.frame.saturating_sub(self.used) > LAYER_IDLE_FRAMES {
            self.entries.clear();
        }
    }

    /// Gives up every table: the project is another.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Gives up the table of an asset that has left the library or is now
    /// of another file.
    pub fn forget(&mut self, asset: AssetId) {
        self.entries.remove(&asset);
    }

    /// Whether a table is kept or waited for.
    pub fn holds(&self) -> bool {
        !self.entries.is_empty()
    }

    /// How many tables are kept or waited for.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Says a layer drawn this frame looks colours up in the table of
    /// `asset`, and asks `media` for it unless it is kept, on its way, or
    /// known not to be readable. Asking returns at once.
    ///
    /// A fifth table in one frame is not asked for. Otherwise the table
    /// least lately wanted makes room.
    pub fn want(&mut self, media: Option<&MediaService>, asset: &Asset) {
        if asset.kind != MediaKind::Lut {
            return;
        }
        self.used = self.frame;
        let generation = media.map(MediaService::generation);
        if let Some(entry) = self.entries.get_mut(&asset.id) {
            if entry.is_of(asset) {
                entry.used = self.frame;
                let waits = match &entry.state {
                    State::Ready { .. } => true,
                    // The worker started over: what it was asked is gone,
                    // and what it could not read may be readable now.
                    State::Asked(from) | State::Failed(_, from) => {
                        generation.is_none_or(|now| now == *from)
                    }
                };
                if waits {
                    return;
                }
            }
            self.entries.remove(&asset.id);
        }
        if self.entries.len() >= MAX_LUTS {
            let oldest = (self.entries.iter())
                .filter(|(_, entry)| entry.used < self.frame)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else { return };
            self.entries.remove(&oldest);
        }
        let Some(media) = media else { return };
        let state = match media.request_lut(asset) {
            Ok(generation) => State::Asked(generation),
            // It is asked for again on the next frame that is drawn.
            Err(MediaError::Busy) => return,
            Err(error) => State::Failed(error.to_string(), media.generation()),
        };
        self.entries.insert(
            asset.id,
            Entry {
                path: asset.path.clone(),
                file_size: asset.file_size,
                state,
                used: self.frame,
            },
        );
    }

    /// The table of `asset`, once it has come.
    pub fn get(&self, asset: &Asset) -> Option<Lut<'_>> {
        let entry = self.entries.get(&asset.id).filter(|e| e.is_of(asset))?;
        match &entry.state {
            State::Ready { serial, table } => Some(Lut {
                serial: *serial,
                table,
            }),
            _ => None,
        }
    }

    /// Why a table that was wanted will not come as things are: its file
    /// could not be read, or four others are in use. Nothing while it is
    /// kept or on its way.
    pub fn problem(&self, asset: &Asset) -> Option<String> {
        match self.entries.get(&asset.id).filter(|e| e.is_of(asset)) {
            Some(Entry {
                state: State::Failed(why, _),
                ..
            }) => Some(format!("The LUT {} could not be read: {why}", asset.name)),
            Some(_) => None,
            None if self.entries.len() >= MAX_LUTS => Some(format!(
                "The LUT {} is left out: a frame is drawn with {MAX_LUTS} tables at the most.",
                asset.name
            )),
            None => None,
        }
    }

    /// Takes the worker's answer for the table of `asset_id`, read from
    /// `path` when that file was `file_size` long, by a worker at
    /// `generation`. `now` is the asset as the library has it.
    ///
    /// An answer is kept only if that very read is still waited for: the
    /// asset is in the library and still of that file, and nothing has
    /// asked again since. Whatever else arrives is dropped.
    pub fn take(
        &mut self,
        now: Option<&Asset>,
        asset_id: AssetId,
        generation: u64,
        path: &std::path::Path,
        file_size: u64,
        table: Result<Arc<Cube>, String>,
    ) -> Taken {
        let Some(asset) = now.filter(|asset| asset.id == asset_id) else {
            self.entries.remove(&asset_id);
            return Taken::Dropped;
        };
        let Some(entry) = self.entries.get_mut(&asset_id) else {
            return Taken::Dropped;
        };
        let waited = matches!(entry.state, State::Asked(from) if from == generation)
            && entry.is_of(asset)
            && entry.path == path
            && entry.file_size == file_size
            && asset.kind == MediaKind::Lut;
        if !waited {
            return Taken::Dropped;
        }
        match table {
            Ok(table) => {
                self.serials += 1;
                entry.state = State::Ready {
                    serial: self.serials,
                    table,
                };
                Taken::Kept
            }
            Err(why) => {
                entry.state = State::Failed(why.clone(), generation);
                Taken::Failed(format!("{}: {why}", asset.name))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use editor_core::Time;

    /// A table that maps each colour by `map`.
    pub(crate) fn cube(size: u32, map: impl Fn([f32; 3]) -> [f32; 3]) -> Cube {
        let last = (size - 1) as f32;
        let mut table = Vec::with_capacity((size as usize).pow(3));
        for blue in 0..size {
            for green in 0..size {
                for red in 0..size {
                    table.push(map([
                        red as f32 / last,
                        green as f32 / last,
                        blue as f32 / last,
                    ]));
                }
            }
        }
        Cube {
            title: None,
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            table,
        }
    }

    fn asset(name: &str) -> Asset {
        let mut asset = Asset::new(format!("/luts/{name}").into(), MediaKind::Lut, Time(1));
        asset.width = 2;
        asset.file_size = 64;
        asset
    }

    /// An entry as `want` leaves it once the worker took the request.
    fn asked(luts: &mut Luts, asset: &Asset, generation: u64) {
        luts.used = luts.frame;
        luts.entries.insert(
            asset.id,
            Entry {
                path: asset.path.clone(),
                file_size: asset.file_size,
                state: State::Asked(generation),
                used: luts.frame,
            },
        );
    }

    fn answer(luts: &mut Luts, now: Option<&Asset>, of: &Asset, generation: u64) -> Taken {
        let table = Ok(Arc::new(cube(2, |c| c)));
        luts.take(now, of.id, generation, &of.path, of.file_size, table)
    }

    #[test]
    fn halves_are_the_nearest_sixteen_bit_floats() {
        for (value, bits) in [
            (0.0, 0x0000),
            (1.0, 0x3c00),
            (0.5, 0x3800),
            (-2.0, 0xc000),
            (1.0 / 3.0, 0x3555),
            (65504.0, 0x7bff),
            (1.0e9, 0x7bff),
            (-1.0e9, 0xfbff),
            (5.960_464_5e-8, 0x0001),
            (1.0e-10, 0x0000),
            (6.103_515_6e-5, 0x0400),
            (f32::NAN, 0x0000),
        ] {
            assert_eq!(half(value), bits, "{value}");
        }
        // Every byte value over 255 comes back as itself within half a step
        // of sixteen-bit precision.
        for byte in 0..=255u16 {
            let value = f32::from(byte) / 255.0;
            let bits = half(value);
            let (exponent, mantissa) = (i32::from(bits >> 10), f32::from(bits & 0x3ff));
            let back = if exponent == 0 {
                mantissa * 2f32.powi(-24)
            } else {
                (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
            };
            assert!((back - value).abs() <= value * 0.0006, "{byte}: {back}");
        }
    }

    #[test]
    fn a_table_is_laid_out_in_tiles_no_wider_than_a_device_takes() {
        assert_eq!(layout(2), (2, [4, 2]));
        assert_eq!(layout(17), (5, [85, 68]));
        assert_eq!(layout(33), (6, [198, 198]));
        assert_eq!(layout(65), (9, [585, 520]));
        for size in 2..=65 {
            let (across, [width, height]) = layout(size);
            assert!(
                across * (height / size) >= size,
                "{size}: a tile for every blue"
            );
            assert!(width <= 2048 && height <= 2048, "{size}");
        }
        // Red goes across a tile, green down it, and blue from tile to tile.
        let table = cube(3, |c| c);
        let bytes = texels(&table);
        let (across, [width, _]) = layout(3);
        assert_eq!(bytes.len(), 6 * 6 * TEXEL);
        assert_eq!(bytes.len() + table.byte_len(), super::bytes(&table));
        let at = |red: u32, green: u32, blue: u32| {
            let x = (blue % across) * 3 + red;
            let y = (blue / across) * 3 + green;
            let from = ((y * width + x) as usize) * TEXEL;
            let word =
                |n: usize| u16::from_ne_bytes([bytes[from + 2 * n], bytes[from + 2 * n + 1]]);
            [word(0), word(1), word(2), word(3)]
        };
        assert_eq!(at(2, 0, 0), [0x3c00, 0, 0, 0x3c00]);
        assert_eq!(at(0, 1, 0), [0, 0x3800, 0, 0x3c00]);
        assert_eq!(at(0, 0, 2), [0, 0, 0x3c00, 0x3c00]);
        assert_eq!(at(1, 2, 1), [0x3800, 0x3c00, 0x3800, 0x3c00]);
    }

    #[test]
    fn a_table_is_kept_only_for_the_read_that_is_waited_for() {
        let look = asset("look.cube");
        let mut luts = Luts::default();
        luts.begin_frame();

        // Nothing was asked: nothing is kept.
        assert_eq!(answer(&mut luts, Some(&look), &look, 1), Taken::Dropped);
        asked(&mut luts, &look, 1);
        assert!(luts.get(&look).is_none() && luts.problem(&look).is_none());

        // An answer from before the worker started over is not this read's.
        assert_eq!(answer(&mut luts, Some(&look), &look, 0), Taken::Dropped);
        // Nor is one for the file the asset used to be of.
        let mut replaced = look.clone();
        replaced.path = "/luts/other.cube".into();
        assert_eq!(answer(&mut luts, Some(&replaced), &look, 1), Taken::Dropped);
        let mut grown = look.clone();
        grown.file_size += 1;
        assert_eq!(answer(&mut luts, Some(&grown), &look, 1), Taken::Dropped);
        assert_eq!(answer(&mut luts, Some(&look), &grown, 1), Taken::Dropped);
        assert!(luts.get(&look).is_none());
        // The replaced asset has no table of the old file either.
        assert!(luts.get(&replaced).is_none());

        assert_eq!(answer(&mut luts, Some(&look), &look, 1), Taken::Kept);
        let first = luts.get(&look).expect("the table came").serial;
        // The same read answered twice changes nothing.
        assert_eq!(answer(&mut luts, Some(&look), &look, 1), Taken::Dropped);
        assert_eq!(luts.get(&look).map(|lut| lut.serial), Some(first));

        // An asset that has left the library takes its table with it.
        asked(&mut luts, &look, 1);
        assert_eq!(answer(&mut luts, None, &look, 1), Taken::Dropped);
        assert_eq!(luts.len(), 0);

        // A table read anew has a new name, so a canvas is drawn again.
        asked(&mut luts, &look, 2);
        assert_eq!(answer(&mut luts, Some(&look), &look, 2), Taken::Kept);
        assert_ne!(luts.get(&look).map(|lut| lut.serial), Some(first));
        luts.forget(look.id);
        assert!(luts.get(&look).is_none());
    }

    #[test]
    fn a_file_that_cannot_be_read_leaves_no_table_and_says_why() {
        let look = asset("look.cube");
        let mut luts = Luts::default();
        luts.begin_frame();
        asked(&mut luts, &look, 1);
        let taken = luts.take(
            Some(&look),
            look.id,
            1,
            &look.path,
            look.file_size,
            Err("No such file".into()),
        );
        assert_eq!(taken, Taken::Failed("look.cube: No such file".into()));
        assert!(luts.get(&look).is_none());
        let why = luts.problem(&look).expect("it is known to be unreadable");
        assert!(why.contains("look.cube") && why.contains("No such file"));
        // It is not asked for again while nothing has changed: with no
        // worker to ask, wanting it leaves the failure as it is.
        luts.want(None, &look);
        assert!(luts.problem(&look).is_some());
    }

    #[test]
    fn four_tables_are_kept_and_the_one_least_lately_wanted_makes_room() {
        let service = MediaService::new(|| {}).unwrap();
        let generation = service.generation();
        let tables: Vec<Asset> = (0..6).map(|n| asset(&format!("{n}.cube"))).collect();
        let mut luts = Luts::default();
        luts.begin_frame();
        // A frame that wants five: the fifth is not asked for, and says so.
        for table in &tables[..5] {
            luts.want(Some(&service), table);
        }
        assert_eq!(luts.len(), MAX_LUTS);
        assert!(
            tables[..4]
                .iter()
                .all(|table| luts.problem(table).is_none())
        );
        let why = luts.problem(&tables[4]).expect("there was no room");
        assert!(why.contains("4.cube") && why.contains("4 tables"), "{why}");

        // The next frame wants three of them and a new one: the one left
        // unwanted goes.
        luts.begin_frame();
        for table in [&tables[0], &tables[2], &tables[3], &tables[5]] {
            luts.want(Some(&service), table);
        }
        assert_eq!(luts.len(), MAX_LUTS);
        assert!(!luts.entries.contains_key(&tables[1].id));
        assert!(luts.entries.contains_key(&tables[5].id));
        assert!(matches!(
            luts.entries[&tables[5].id].state,
            State::Asked(from) if from == generation
        ));
        // Something that is not a table is never asked for.
        let still = Asset::new("/media/still.png".into(), MediaKind::Image, Time(1));
        luts.want(Some(&service), &still);
        assert_eq!(luts.len(), MAX_LUTS);

        // The worker started over: what was asked of it is asked again.
        let restarted = service.cancel();
        luts.begin_frame();
        luts.want(Some(&service), &tables[0]);
        assert!(matches!(
            luts.entries[&tables[0].id].state,
            State::Asked(from) if from == restarted
        ));
    }

    #[test]
    fn tables_are_given_up_when_none_is_wanted_for_long_or_the_project_is_another() {
        let look = asset("look.cube");
        let mut luts = Luts::default();
        luts.begin_frame();
        asked(&mut luts, &look, 1);
        assert_eq!(answer(&mut luts, Some(&look), &look, 1), Taken::Kept);
        for _ in 0..LAYER_IDLE_FRAMES {
            luts.begin_frame();
        }
        assert!(luts.get(&look).is_some(), "kept for as long as a layer is");
        luts.begin_frame();
        assert_eq!(luts.len(), 0);

        asked(&mut luts, &look, 1);
        luts.clear();
        assert_eq!(luts.len(), 0);
    }
}
