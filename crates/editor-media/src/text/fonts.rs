//! Which face draws each character: the project's font files, the faces the
//! program bundles and the system's. System faces are found once, from the
//! few tables that name them, and a face's whole file is read only when a
//! title draws with it; read faces are held to a byte budget.
use crate::MediaError;
use editor_core::FontFace;
use harfrust::ShaperData;
use skrifa::{
    FontRef, MetadataProvider, Tag,
    attribute::Style,
    instance::{LocationRef, NormalizedCoord, Size},
    raw::FileRef,
    string::StringId,
};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};
use vello_cpu::peniko::{Blob, FontData};

/// The largest font file read.
pub const MAX_FONT_FILE: u64 = 128 * 1024 * 1024;
/// Font files held read at once, in file bytes, beyond the bundled faces;
/// the faces of the title being drawn are always kept.
pub const FONT_CACHE_BYTES: usize = 192 * 1024 * 1024;
/// Font files looked at when the system's faces are found.
const MAX_SYSTEM_FILES: usize = 8192;
/// Directory levels searched below each system font directory.
const MAX_DEPTH: usize = 8;
/// Faces read only to learn whether they have one character, per drawing.
const MAX_PROBES: usize = 48;
/// Characters whose fallback face is remembered.
const MAX_REMEMBERED: usize = 65_536;

/// System families tried, in order, for a character the title's face lacks.
const FALLBACK_FAMILIES: &[&str] = &[
    "Noto Sans",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Noto Sans CJK KR",
    "Noto Sans CJK TC",
    "Source Han Sans SC",
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Apple SD Gothic Neo",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "Segoe UI",
    "Segoe UI Symbol",
    "Segoe UI Historic",
    "Nirmala UI",
    "Leelawadee UI",
    "Noto Sans Arabic",
    "Noto Naskh Arabic",
    "Geeza Pro",
    "Noto Sans Hebrew",
    "Arial Hebrew",
    "Noto Sans Devanagari",
    "Kohinoor Devanagari",
    "Noto Sans Thai",
    "Thonburi",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "Apple Symbols",
    "DejaVu Sans",
    "FreeSans",
    "Arial Unicode MS",
    "Arial",
    "Helvetica",
    "Tahoma",
    "WenQuanYi Zen Hei",
    "WenQuanYi Micro Hei",
    "Droid Sans Fallback",
    "Unifont",
];

/// Colour emoji families, tried first for an emoji.
const EMOJI_FAMILIES: &[&str] = &[
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Twemoji",
    "Twemoji Mozilla",
    "EmojiOne Color",
    "JoyPixels",
];

#[derive(Clone, Debug)]
enum Source {
    Bundled(&'static [u8]),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Project,
    Bundled,
    System,
}

/// One face the book knows of, by the tables that name it.
#[derive(Clone, Debug)]
struct Entry {
    /// The family as the face names it.
    family: String,
    /// Lower-case names the face answers to: its typographic family and its
    /// legacy family.
    keys: Vec<String>,
    /// The default instance's weight.
    weight: f32,
    /// The weights a variable face spans.
    weights: Option<(f32, f32)>,
    italic: bool,
    source: Source,
    index: u32,
    origin: Origin,
    /// File bytes it costs when read; nothing for a bundled face.
    bytes: usize,
}

/// A face read and ready to shape and draw.
pub(crate) struct Face {
    pub data: FontData,
    pub shaper: ShaperData,
    pub upem: f32,
    /// Font units above and below (negative) the baseline.
    pub ascent: f32,
    pub descent: f32,
    /// Drawn in its own colours: emoji.
    pub color: bool,
}

impl Face {
    pub fn font(&self) -> FontRef<'_> {
        // Checked when the face was read.
        FontRef::from_index(self.data.data.data(), self.data.index).expect("a read face parses")
    }

    pub fn has(&self, c: char) -> bool {
        self.font().charmap().map(c).is_some()
    }
}

/// A face as one style draws it: the face, and what is made up for what
/// its family lacks.
#[derive(Clone)]
pub(crate) struct Chosen {
    pub entry: usize,
    pub face: Arc<Face>,
    /// Variation coordinates for a variable face's weight, and the same
    /// as the rasteriser takes them.
    pub coords: Vec<NormalizedCoord>,
    pub bits: Vec<i16>,
    /// Synthetic emboldening: how far strokes are thickened on each side,
    /// in em.
    pub embolden: f32,
    /// Slanted, for an italic its family does not have.
    pub slant: bool,
}

impl Chosen {
    /// Whether two choices draw alike, so their glyphs may share a run.
    pub fn same(&self, other: &Self) -> bool {
        self.entry == other.entry
            && self.coords == other.coords
            && self.embolden == other.embolden
            && self.slant == other.slant
    }
}

/// Every face titles may draw with, and the ones read.
pub struct FontBook {
    entries: Vec<Entry>,
    project: Vec<PathBuf>,
    scanned: bool,
    loaded: HashMap<usize, (Arc<Face>, u64)>,
    loaded_bytes: usize,
    clock: u64,
    /// Faces that could not be read, never tried again.
    broken: HashSet<usize>,
    /// The fallback entry found for a character, by family and slant asked.
    fallback: HashMap<(char, bool), Option<usize>>,
    probes: usize,
    system_roots: Option<Vec<PathBuf>>,
}

impl FontBook {
    /// A book of the bundled faces. The system's are found when first
    /// needed.
    pub fn new(bundled: &[&'static [u8]]) -> Self {
        let mut book = Self {
            entries: Vec::new(),
            project: Vec::new(),
            scanned: false,
            loaded: HashMap::new(),
            loaded_bytes: 0,
            clock: 0,
            broken: HashSet::new(),
            fallback: HashMap::new(),
            probes: 0,
            system_roots: None,
        };
        for bytes in bundled {
            for (index, names) in describe(bytes).into_iter().enumerate() {
                if let Some(names) = names {
                    book.entries.push(names.entry(
                        Source::Bundled(bytes),
                        index as u32,
                        Origin::Bundled,
                        0,
                    ));
                }
            }
        }
        book
    }

    /// A book that finds system faces under `roots` and nowhere else; empty
    /// roots keep it to the bundled and project faces.
    pub fn with_system_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.system_roots = Some(roots);
        self
    }

    /// The project's font files. Faces of files no longer listed are
    /// forgotten; a file that cannot be read is left out.
    pub fn set_project_fonts(&mut self, paths: &[PathBuf]) {
        if paths == self.project.as_slice() {
            return;
        }
        self.project = paths.to_vec();
        let old: Vec<usize> = (self.entries.iter().enumerate())
            .filter(|(_, entry)| entry.origin == Origin::Project)
            .map(|(index, _)| index)
            .collect();
        // Entries are addressed by index: a removed project face leaves a
        // hole no family answers to.
        for index in old {
            self.entries[index].keys.clear();
            if let Some((_, _)) = self.loaded.remove(&index) {
                self.loaded_bytes = self.loaded_bytes.saturating_sub(self.entries[index].bytes);
            }
        }
        for path in paths {
            let Ok(found) = read_faces(path) else {
                continue;
            };
            for (index, names, bytes) in found {
                self.entries.push(names.entry(
                    Source::File(path.clone()),
                    index,
                    Origin::Project,
                    bytes,
                ));
            }
        }
        self.fallback.clear();
    }

    /// Every family a title may name, sorted, each once.
    pub fn families(&mut self) -> Vec<String> {
        self.scan();
        let mut seen = HashSet::new();
        let mut families: Vec<String> = (self.entries.iter())
            .filter(|entry| !entry.keys.is_empty())
            .filter(|entry| seen.insert(entry.family.to_lowercase()))
            .map(|entry| entry.family.clone())
            .collect();
        families.sort_by_key(|family| family.to_lowercase());
        families
    }

    /// Starts a drawing: the probe allowance is renewed.
    pub(crate) fn begin(&mut self) {
        self.probes = 0;
    }

    /// The face `face` names, nearest in weight and slant; the default
    /// family when its family is not known.
    pub(crate) fn select(&mut self, face: &FontFace) -> Option<Chosen> {
        let key = face.family.to_lowercase();
        let mut found = self.find(&key, face);
        if found.is_none() && !self.scanned {
            self.scan();
            found = self.find(&key, face);
        }
        let found = found.or_else(|| {
            let key = editor_core::title::DEFAULT_FAMILY.to_lowercase();
            self.find(&key, face)
        });
        match found {
            Some(entry) => self.choose(entry, face),
            // No Geist: the first face that reads.
            None => {
                let known: Vec<usize> = (0..self.entries.len())
                    .filter(|&entry| !self.entries[entry].keys.is_empty())
                    .collect();
                known.into_iter().find_map(|entry| self.choose(entry, face))
            }
        }
    }

    /// A face for `c`, which `primary` lacks: colour emoji first for an
    /// emoji, then the bundled faces, the usual system families and at last
    /// any system face found to have it.
    pub(crate) fn fallback(&mut self, c: char, emoji: bool, face: &FontFace) -> Option<Chosen> {
        let remembered = (c, face.italic);
        if let Some(entry) = self.fallback.get(&remembered).copied() {
            return entry.and_then(|entry| self.choose_near(entry, c, face));
        }
        self.scan();
        let mut families: Vec<String> = Vec::new();
        if emoji {
            families.extend(EMOJI_FAMILIES.iter().map(|family| family.to_lowercase()));
        }
        let mut bundled: Vec<String> = Vec::new();
        for entry in &self.entries {
            if entry.origin == Origin::Bundled && !bundled.contains(&entry.keys[0]) {
                bundled.push(entry.keys[0].clone());
            }
        }
        families.extend(bundled);
        families.extend(FALLBACK_FAMILIES.iter().map(|family| family.to_lowercase()));
        let mut found = None;
        for family in &families {
            let Some(entry) = self.find(family, face) else {
                continue;
            };
            if let Some(read) = self.read(entry)
                && read.has(c)
            {
                found = Some(entry);
                break;
            }
        }
        if found.is_none() {
            // Any face at all, a few a drawing.
            let candidates: Vec<usize> = (0..self.entries.len())
                .filter(|&entry| {
                    let entry = &self.entries[entry];
                    !entry.keys.is_empty() && entry.origin != Origin::Bundled
                })
                .collect();
            for entry in candidates {
                let loaded = self.loaded.contains_key(&entry);
                if !loaded {
                    if self.probes >= MAX_PROBES {
                        // Not remembered: a later drawing looks further.
                        return None;
                    }
                    self.probes += 1;
                }
                if self.read(entry).is_some_and(|read| read.has(c)) {
                    found = Some(entry);
                    break;
                }
            }
        }
        if self.fallback.len() >= MAX_REMEMBERED {
            self.fallback.clear();
        }
        self.fallback.insert(remembered, found);
        self.choose_near(found?, c, face)
    }

    /// The face of `found`'s family nearest `face` in weight and slant when
    /// it has `c` too, else `found`.
    fn choose_near(&mut self, found: usize, c: char, face: &FontFace) -> Option<Chosen> {
        let family = self.entries[found].keys[0].clone();
        let near = self
            .find(&family, face)
            .filter(|&near| near != found)
            .and_then(|near| self.choose(near, face))
            .filter(|chosen| chosen.face.has(c));
        near.or_else(|| self.choose(found, face))
    }

    fn find(&self, key: &str, face: &FontFace) -> Option<usize> {
        let weight = f32::from(face.weight);
        (self.entries.iter().enumerate())
            .filter(|(index, entry)| {
                entry.keys.iter().any(|known| known == key) && !self.broken.contains(index)
            })
            .min_by(|(_, a), (_, b)| {
                let rank = |entry: &Entry| {
                    let distance = match entry.weights {
                        Some((low, high)) => (low - weight).max(0.0) + (weight - high).max(0.0),
                        None => (entry.weight - weight).abs(),
                    };
                    // Slant matters more than weight; past a match, a
                    // heavier face beats a lighter one for a bold, as CSS
                    // would have it.
                    let slant = if entry.italic == face.italic {
                        0.0
                    } else {
                        10_000.0
                    };
                    let lean = if entry.weight < weight && weight > 500.0 {
                        0.5
                    } else {
                        0.0
                    };
                    (entry.origin, slant + distance + lean)
                };
                let (a, b) = (rank(a), rank(b));
                a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
            })
            .map(|(index, _)| index)
    }

    fn choose(&mut self, entry: usize, face: &FontFace) -> Option<Chosen> {
        let read = self.read(entry)?;
        let known = &self.entries[entry];
        let weight = f32::from(face.weight);
        let mut coords = Vec::new();
        let mut drawn_weight = known.weight;
        if let Some((low, high)) = known.weights {
            let target = weight.clamp(low, high);
            let font = read.font();
            let location = font.axes().location([("wght", target)]);
            coords = location.coords().to_vec();
            drawn_weight = target;
        }
        // Each hundred of weight missing thickens strokes by a hundredth
        // of an em on each side, past a margin no one would see.
        let missing = (weight - drawn_weight - 50.0).max(0.0);
        let embolden = if read.color {
            0.0
        } else {
            missing / 100.0 * 0.012
        };
        let bits = coords.iter().map(|coord| coord.to_bits()).collect();
        Some(Chosen {
            entry,
            face: read,
            coords,
            bits,
            embolden,
            slant: face.italic && !known.italic,
        })
    }

    /// The face of an entry, read when not yet.
    fn read(&mut self, entry: usize) -> Option<Arc<Face>> {
        self.clock += 1;
        if let Some((face, used)) = self.loaded.get_mut(&entry) {
            *used = self.clock;
            return Some(face.clone());
        }
        if self.broken.contains(&entry) {
            return None;
        }
        let known = &self.entries[entry];
        let blob = match &known.source {
            Source::Bundled(bytes) => Blob::new(Arc::new(*bytes)),
            Source::File(path) => match read_file(path) {
                Ok(bytes) => Blob::new(Arc::new(bytes)),
                Err(_) => {
                    self.broken.insert(entry);
                    return None;
                }
            },
        };
        let Some(face) = make_face(blob, known.index) else {
            self.broken.insert(entry);
            return None;
        };
        let face = Arc::new(face);
        let bytes = known.bytes;
        self.loaded.insert(entry, (face.clone(), self.clock));
        self.loaded_bytes += bytes;
        self.evict();
        Some(face)
    }

    /// Lets go of the faces used longest ago past the budget; one still
    /// held by a drawing stays alive until it is done.
    fn evict(&mut self) {
        while self.loaded_bytes > FONT_CACHE_BYTES {
            let oldest = (self.loaded.iter())
                .filter(|(entry, _)| self.entries[**entry].bytes > 0)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(entry, _)| *entry);
            let Some(entry) = oldest else {
                break;
            };
            self.loaded.remove(&entry);
            self.loaded_bytes = self.loaded_bytes.saturating_sub(self.entries[entry].bytes);
        }
    }

    /// Finds the system's faces, once.
    fn scan(&mut self) {
        if self.scanned {
            return;
        }
        self.scanned = true;
        let roots = self.system_roots.clone().unwrap_or_else(system_roots);
        let mut files = Vec::new();
        for root in roots {
            walk(&root, 0, &mut files);
        }
        for path in files {
            for (index, names, bytes) in read_names(&path).unwrap_or_default() {
                self.entries.push(names.entry(
                    Source::File(path.clone()),
                    index,
                    Origin::System,
                    bytes,
                ));
            }
        }
    }
}

/// What the naming tables say of a face.
struct Names {
    family: String,
    legacy: Option<String>,
    weight: f32,
    weights: Option<(f32, f32)>,
    italic: bool,
}

impl Names {
    fn entry(self, source: Source, index: u32, origin: Origin, bytes: usize) -> Entry {
        let mut keys = vec![self.family.to_lowercase()];
        if let Some(legacy) = self.legacy.map(|legacy| legacy.to_lowercase())
            && !keys.contains(&legacy)
        {
            keys.push(legacy);
        }
        Entry {
            family: self.family,
            keys,
            weight: self.weight,
            weights: self.weights,
            italic: self.italic,
            source,
            index,
            origin,
            bytes,
        }
    }
}

fn names_of(font: &FontRef<'_>) -> Option<Names> {
    let name = |id| {
        font.localized_strings(id)
            .english_or_first()
            .map(|name| name.chars().collect::<String>())
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty() && name.len() <= 128)
    };
    let legacy = name(StringId::FAMILY_NAME);
    let family = name(StringId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| legacy.clone())?;
    let attributes = font.attributes();
    let weights = font
        .axes()
        .get_by_tag(Tag::new(b"wght"))
        .map(|axis| (axis.min_value(), axis.max_value()))
        .filter(|(low, high)| low < high);
    Some(Names {
        family,
        legacy,
        weight: attributes.weight.value(),
        weights,
        italic: !matches!(attributes.style, Style::Normal),
    })
}

/// The names of each face of a font file in memory, by index.
fn describe(bytes: &[u8]) -> Vec<Option<Names>> {
    match FileRef::new(bytes) {
        Ok(file) => file
            .fonts()
            .map(|font| font.ok().and_then(|font| names_of(&font)))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn make_face(blob: Blob<u8>, index: u32) -> Option<Face> {
    let font = FontRef::from_index(blob.data(), index).ok()?;
    let has_glyphs = font.charmap().has_map();
    if !has_glyphs {
        return None;
    }
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let upem = f32::from(metrics.units_per_em.max(1));
    let color = [b"COLR", b"CBDT", b"sbix"]
        .iter()
        .any(|tag| font.table_data(Tag::new(tag)).is_some());
    let (mut ascent, mut descent) = (metrics.ascent, metrics.descent);
    if ascent <= 0.0 || ascent - descent <= 0.0 {
        ascent = upem * 0.8;
        descent = -upem * 0.2;
    }
    let shaper = ShaperData::new(&font);
    Some(Face {
        data: FontData::new(blob.clone(), index),
        shaper,
        upem,
        ascent,
        descent,
        color,
    })
}

fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    if length > MAX_FONT_FILE {
        return Err(std::io::Error::other("Font file is too large"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_FONT_FILE).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Every face of a font file read whole: a project's font.
fn read_faces(path: &Path) -> Result<Vec<(u32, Names, usize)>, MediaError> {
    let bytes = read_file(path)
        .map_err(|error| crate::io(format!("Cannot read {}", path.display()), error))?;
    let size = bytes.len();
    let blob = Blob::new(Arc::new(bytes));
    let faces: Vec<(u32, Names, usize)> = describe(blob.data())
        .into_iter()
        .enumerate()
        .filter_map(|(index, names)| {
            let names = names?;
            // A face that names itself but cannot draw is no font.
            make_face(blob.clone(), index as u32)?;
            Some((index as u32, names, size))
        })
        .collect();
    if faces.is_empty() {
        return Err(MediaError::Invalid(format!(
            "{} is not a font Neptune Video can draw with",
            path.display()
        )));
    }
    Ok(faces)
}

/// The family a font file is imported as: its first face's.
pub fn font_family(path: &Path) -> Result<String, MediaError> {
    let faces = read_faces(path)?;
    Ok(faces
        .into_iter()
        .next()
        .map(|(_, names, _)| names.family)
        .unwrap_or_default())
}

/// The names of each face of a font file from its naming tables alone,
/// without reading the rest of the file.
fn read_names(path: &Path) -> Option<Vec<(u32, Names, usize)>> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    if !(12..=MAX_FONT_FILE).contains(&length) {
        return None;
    }
    let mut head = [0_u8; 12];
    file.read_exact(&mut head).ok()?;
    let starts: Vec<u32> = if &head[0..4] == b"ttcf" {
        let count = u32::from_be_bytes(head[8..12].try_into().ok()?).min(256);
        let mut offsets = vec![0_u8; count as usize * 4];
        file.read_exact(&mut offsets).ok()?;
        offsets
            .chunks_exact(4)
            .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    } else {
        vec![0]
    };
    let mut faces = Vec::new();
    for (index, start) in starts.into_iter().enumerate() {
        let Some(skeleton) = skeleton(&mut file, u64::from(start), length) else {
            continue;
        };
        if let Ok(font) = FontRef::new(&skeleton)
            && let Some(names) = names_of(&font)
        {
            faces.push((index as u32, names, length as usize));
        }
    }
    Some(faces)
}

/// The tables that name a face, copied into a font of their own.
fn skeleton(file: &mut File, start: u64, length: u64) -> Option<Vec<u8>> {
    const WANTED: [&[u8; 4]; 4] = [b"name", b"OS/2", b"head", b"fvar"];
    const MAX_TABLE: u32 = 4 * 1024 * 1024;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).ok()?;
    let count = u16::from_be_bytes([header[4], header[5]]).min(512);
    let mut records = vec![0_u8; usize::from(count) * 16];
    file.read_exact(&mut records).ok()?;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for record in records.chunks_exact(16) {
        let tag: [u8; 4] = record[0..4].try_into().ok()?;
        if !WANTED.contains(&&tag) {
            continue;
        }
        let offset = u32::from_be_bytes(record[8..12].try_into().ok()?);
        let size = u32::from_be_bytes(record[12..16].try_into().ok()?);
        if size > MAX_TABLE || u64::from(offset) + u64::from(size) > length {
            return None;
        }
        let mut data = vec![0_u8; size as usize];
        file.seek(SeekFrom::Start(u64::from(offset))).ok()?;
        file.read_exact(&mut data).ok()?;
        tables.push((tag, data));
    }
    if !tables.iter().any(|(tag, _)| tag == b"name") {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&header[0..4]);
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    let mut offset = 12 + tables.len() * 16;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Some(out)
}

fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || files.len() >= MAX_SYSTEM_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= MAX_SYSTEM_FILES {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, depth + 1, files);
        } else if is_font_file(&path) {
            files.push(path);
        }
    }
}

/// Whether a path names a font file by its extension.
pub fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

/// Where this system keeps its fonts.
fn system_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push(PathBuf::from("/System/Library/Fonts"));
        roots.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = &home {
            roots.push(home.join("Library/Fonts"));
        }
    } else if cfg!(windows) {
        let windows =
            std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), PathBuf::from);
        roots.push(windows.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        roots.push(PathBuf::from("/usr/share/fonts"));
        roots.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(data).join("fonts"));
        } else if let Some(home) = &home {
            roots.push(home.join(".local/share/fonts"));
        }
        if let Some(home) = &home {
            roots.push(home.join(".fonts"));
        }
    }
    roots
}

/// Whether `c` is drawn as an emoji when nothing says otherwise.
pub(crate) fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF
        | 0x2600..=0x27BF
        | 0x2300..=0x23FF
        | 0x2B00..=0x2BFF)
}

/// Characters that belong to the one before them: marks, joiners,
/// selectors and modifiers draw with its face.
pub(crate) fn clings(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0x200C..=0x200D
        | 0x1F3FB..=0x1F3FF
        | 0xE0020..=0xE007F
        | 0xE0100..=0xE01EF)
}
