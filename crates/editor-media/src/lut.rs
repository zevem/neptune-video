//! Colour lookup tables in the `.cube` text format.
//!
//! [`Cube::parse`] is pure: it is given the text and touches nothing else.
//! [`read`] is the one place a table's file is opened, and only a worker
//! calls it.
use crate::{MediaError, io};
use std::{fmt, fs::File, io::Read, path::Path};

/// The fewest and the most points along an edge of a table.
pub const MIN_CUBE_SIZE: u32 = 2;
pub const MAX_CUBE_SIZE: u32 = 65;
/// The largest `.cube` file that is read. A table of the largest size takes
/// some ten megabytes of text; nothing honest comes near this.
pub const MAX_CUBE_BYTES: u64 = 32 * 1024 * 1024;

/// Why a `.cube` text is not a table, and the line that says so: counted
/// from one, or nought for what the whole text lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CubeError {
    pub line: usize,
    pub message: &'static str,
}

impl fmt::Display for CubeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(self.message)
        } else {
            write!(f, "Line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for CubeError {}

/// A three-dimensional colour lookup table: `size` points along each of red,
/// green and blue, over the inputs from `domain_min` to `domain_max`.
///
/// Both what is looked up and what is found are display-referred colour as
/// a file stores it: gamma-encoded, not linear light.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub title: Option<String>,
    pub size: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size³` colours with red changing fastest, then green, then blue:
    /// the order of the file's rows.
    pub table: Vec<[f32; 3]>,
}

impl Cube {
    /// Reads the text of a `.cube` file.
    ///
    /// `TITLE`, `DOMAIN_MIN`, `DOMAIN_MAX` and `LUT_3D_SIZE` are understood,
    /// in any order before the first row, and `LUT_3D_INPUT_RANGE` as the
    /// one domain of all three channels. Blank lines and `#` comments are
    /// passed over, and lines may end in a carriage return. Anything else is
    /// refused with its line: a one-dimensional table, an unknown keyword,
    /// a size outside `2..=65`, a number that is not finite, a domain that
    /// is empty, and more or fewer rows than the size has points.
    pub fn parse(text: &str) -> Result<Self, CubeError> {
        let fail = |line: usize, message: &'static str| Err(CubeError { line, message });
        let mut title = None;
        let mut size: Option<u32> = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut table: Vec<[f32; 3]> = Vec::new();
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let Some(first) = words.next() else {
                continue;
            };
            // Finite numbers, exactly as many as are asked for.
            let mut numbers = |into: &mut [f32]| -> bool {
                for slot in into.iter_mut() {
                    match words.next().and_then(|word| word.parse::<f32>().ok()) {
                        Some(value) if value.is_finite() => *slot = value,
                        _ => return false,
                    }
                }
                words.next().is_none()
            };
            let keyword = first.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
            if !keyword {
                let Some(size) = size else {
                    return fail(number, "A row comes before LUT_3D_SIZE");
                };
                let mut row = [0.0f32; 3];
                let mut words = line.split_whitespace();
                for slot in &mut row {
                    match words.next().and_then(|word| word.parse::<f32>().ok()) {
                        Some(value) if value.is_finite() => *slot = value,
                        _ => return fail(number, "A row is not three finite numbers"),
                    }
                }
                if words.next().is_some() {
                    return fail(number, "A row is not three finite numbers");
                }
                if table.len() >= points(size) {
                    return fail(number, "There are more rows than the table has points");
                }
                table.push(row);
                continue;
            }
            if !table.is_empty() {
                return fail(number, "A keyword comes after the first row");
            }
            match first {
                "TITLE" => {
                    let rest = line["TITLE".len()..].trim();
                    let rest = rest
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                        .unwrap_or(rest);
                    title = Some(rest.chars().take(256).collect());
                }
                "LUT_3D_SIZE" => {
                    let mut edge = [0.0f32];
                    if !numbers(&mut edge) || edge[0].fract() != 0.0 {
                        return fail(number, "LUT_3D_SIZE is not a whole number");
                    }
                    if !(MIN_CUBE_SIZE as f32..=MAX_CUBE_SIZE as f32).contains(&edge[0]) {
                        return fail(number, "LUT_3D_SIZE is outside 2 to 65");
                    }
                    if size.is_some() {
                        return fail(number, "LUT_3D_SIZE is given twice");
                    }
                    size = Some(edge[0] as u32);
                    table.reserve_exact(points(edge[0] as u32));
                }
                "DOMAIN_MIN" => {
                    if !numbers(&mut domain_min) {
                        return fail(number, "DOMAIN_MIN is not three finite numbers");
                    }
                }
                "DOMAIN_MAX" => {
                    if !numbers(&mut domain_max) {
                        return fail(number, "DOMAIN_MAX is not three finite numbers");
                    }
                }
                "LUT_3D_INPUT_RANGE" => {
                    let mut range = [0.0f32; 2];
                    if !numbers(&mut range) {
                        return fail(number, "LUT_3D_INPUT_RANGE is not two finite numbers");
                    }
                    domain_min = [range[0]; 3];
                    domain_max = [range[1]; 3];
                }
                "LUT_1D_SIZE" | "LUT_1D_INPUT_RANGE" => {
                    return fail(
                        number,
                        "This is a 1D LUT; only 3D tables (LUT_3D_SIZE) are supported",
                    );
                }
                _ => return fail(number, "Unknown keyword"),
            }
        }
        let Some(size) = size else {
            return fail(0, "The file has no LUT_3D_SIZE");
        };
        if (0..3).any(|c| domain_max[c] <= domain_min[c]) {
            return fail(0, "DOMAIN_MAX is not above DOMAIN_MIN");
        }
        if table.len() != points(size) {
            return fail(0, "There are fewer rows than the table has points");
        }
        Ok(Self {
            title,
            size,
            domain_min,
            domain_max,
            table,
        })
    }

    /// The colour at one point of the table.
    pub fn at(&self, red: u32, green: u32, blue: u32) -> [f32; 3] {
        let n = self.size as usize;
        self.table[red as usize + n * (green as usize + n * blue as usize)]
    }

    /// What the table makes of `colour`: the eight points around it, mixed
    /// by how near each is. A colour outside the domain is looked up at the
    /// domain's edge. This is the lookup a renderer is checked against.
    pub fn sample(&self, colour: [f32; 3]) -> [f32; 3] {
        let last = (self.size - 1) as f32;
        let mut low = [0u32; 3];
        let mut part = [0.0f32; 3];
        for c in 0..3 {
            let span = self.domain_max[c] - self.domain_min[c];
            let at = ((colour[c] - self.domain_min[c]) / span).clamp(0.0, 1.0) * last;
            low[c] = (at.floor() as u32).min(self.size - 2);
            part[c] = at - low[c] as f32;
        }
        let mut out = [0.0f32; 3];
        for corner in 0..8u32 {
            let step = [corner & 1, (corner >> 1) & 1, corner >> 2];
            let weight: f32 = (0..3)
                .map(|c| if step[c] == 1 { part[c] } else { 1.0 - part[c] })
                .product();
            let point = self.at(low[0] + step[0], low[1] + step[1], low[2] + step[2]);
            for c in 0..3 {
                out[c] += weight * point[c];
            }
        }
        out
    }

    /// The bytes the table's colours take.
    pub fn byte_len(&self) -> usize {
        std::mem::size_of_val(self.table.as_slice())
    }
}

fn points(size: u32) -> usize {
    (size as usize).pow(3)
}

/// Opens and reads a table. It reads the whole file, so only a worker calls
/// it.
pub(crate) fn read(path: &Path) -> Result<Cube, MediaError> {
    let file =
        File::open(path).map_err(|error| io(format!("Cannot open {}", path.display()), error))?;
    let length = file
        .metadata()
        .map_err(|error| io("Cannot read LUT size", error))?
        .len();
    if length > MAX_CUBE_BYTES {
        return Err(MediaError::Invalid(
            "LUT file exceeds the 32 MiB read budget".into(),
        ));
    }
    let mut text = String::with_capacity(length as usize);
    // A file that grew since it was measured is still read no further.
    file.take(MAX_CUBE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| io("Cannot read LUT", error))?;
    if text.len() as u64 > MAX_CUBE_BYTES {
        return Err(MediaError::Invalid(
            "LUT file exceeds the 32 MiB read budget".into(),
        ));
    }
    Cube::parse(&text).map_err(|error| MediaError::Invalid(error.to_string()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The text of a table of `size` that maps each colour by `map`.
    pub(crate) fn text(size: u32, map: impl Fn([f32; 3]) -> [f32; 3]) -> String {
        let mut text = format!("LUT_3D_SIZE {size}\n");
        let last = (size - 1) as f32;
        for blue in 0..size {
            for green in 0..size {
                for red in 0..size {
                    let [r, g, b] =
                        map([red as f32 / last, green as f32 / last, blue as f32 / last]);
                    text.push_str(&format!("{r:.6} {g:.6} {b:.6}\n"));
                }
            }
        }
        text
    }

    #[track_caller]
    fn refused(text: &str, line: usize, says: &str) {
        let error = Cube::parse(text).expect_err("the text is not a table");
        assert_eq!(error.line, line, "{error}");
        assert!(error.message.contains(says), "{error}");
    }

    #[test]
    fn a_table_is_read_with_red_changing_fastest() {
        let cube = Cube::parse(&text(3, |c| c)).unwrap();
        assert_eq!(cube.size, 3);
        assert_eq!(cube.title, None);
        assert_eq!((cube.domain_min, cube.domain_max), ([0.0; 3], [1.0; 3]));
        assert_eq!(cube.table.len(), 27);
        assert_eq!(cube.byte_len(), 27 * 12);
        // The second row is one step of red, the fourth one of green and
        // the tenth one of blue.
        assert_eq!(cube.table[1], [0.5, 0.0, 0.0]);
        assert_eq!(cube.table[3], [0.0, 0.5, 0.0]);
        assert_eq!(cube.table[9], [0.0, 0.0, 0.5]);
        assert_eq!(cube.at(2, 1, 0), [1.0, 0.5, 0.0]);
        assert_eq!(cube.at(0, 1, 2), [0.0, 0.5, 1.0]);
    }

    #[test]
    fn header_comments_blank_lines_and_carriage_returns_are_understood() {
        let text = "\u{feff}# made by hand\r\n\r\nTITLE \"Warm look\"\r\n\
                    DOMAIN_MIN 0.0 0.0 0.0\r\nDOMAIN_MAX\t2.0 2.0 4.0\r\n\
                    LUT_3D_SIZE 2\r\n\r\n\
                    0 0 0\r\n1 0 0\r\n0 1 0\r\n1 1 0\r\n  # half way\r\n\
                    0 0 1\r\n1 0 1\r\n0 1 1\r\n1e0 1.0 +1\r\n";
        let cube = Cube::parse(text).unwrap();
        assert_eq!(cube.title.as_deref(), Some("Warm look"));
        assert_eq!(cube.domain_min, [0.0; 3]);
        assert_eq!(cube.domain_max, [2.0, 2.0, 4.0]);
        assert_eq!(cube.table[7], [1.0, 1.0, 1.0]);
        // Keywords come in any order before the rows.
        let swapped = "LUT_3D_SIZE 2\nTITLE plain\nLUT_3D_INPUT_RANGE -1 3\n\
                       0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1";
        let cube = Cube::parse(swapped).unwrap();
        assert_eq!(cube.title.as_deref(), Some("plain"));
        assert_eq!((cube.domain_min, cube.domain_max), ([-1.0; 3], [3.0; 3]));
    }

    #[test]
    fn what_is_not_a_3d_table_is_refused_with_its_line() {
        let rows = "0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        refused("", 0, "no LUT_3D_SIZE");
        refused(rows, 1, "before LUT_3D_SIZE");
        refused("LUT_1D_SIZE 16\n0 0 0\n", 1, "1D LUT");
        refused("# x\nLUT_1D_INPUT_RANGE 0 1\n", 2, "1D LUT");
        refused("LUT_3D_SIZE 1\n0 0 0\n", 1, "outside 2 to 65");
        refused("LUT_3D_SIZE 66\n", 1, "outside 2 to 65");
        refused("LUT_3D_SIZE 2.5\n", 1, "whole number");
        refused("LUT_3D_SIZE two\n", 1, "whole number");
        refused("LUT_3D_SIZE 2 2\n", 1, "whole number");
        refused("LUT_3D_SIZE 2\nLUT_3D_SIZE 2\n", 2, "twice");
        refused("LUT_3D_SIZE 2\nGAMMA 2.2\n", 2, "Unknown keyword");
        refused(
            &format!("LUT_3D_SIZE 2\n{rows}TITLE late\n"),
            10,
            "after the first row",
        );
        refused(&format!("LUT_3D_SIZE 2\n{rows}0 0 0\n"), 10, "more rows");
        refused("LUT_3D_SIZE 2\n0 0 0\n1 0 0\n", 0, "fewer rows");
        refused("LUT_3D_SIZE 2\n0 0\n", 2, "three finite numbers");
        refused("LUT_3D_SIZE 2\n0 0 0 0\n", 2, "three finite numbers");
        refused("LUT_3D_SIZE 2\n0 0 x\n", 2, "three finite numbers");
        refused("LUT_3D_SIZE 2\n0 NaN 0\n", 2, "three finite numbers");
        refused("LUT_3D_SIZE 2\n0 0 inf\n", 2, "three finite numbers");
        refused("LUT_3D_SIZE 2\n1e39 0 0\n", 2, "three finite numbers");
        refused("DOMAIN_MIN 0 0\n", 1, "DOMAIN_MIN");
        refused("DOMAIN_MAX 1 1 inf\n", 1, "DOMAIN_MAX");
        refused(
            &format!("DOMAIN_MAX 1 0 1\nLUT_3D_SIZE 2\n{rows}"),
            0,
            "not above DOMAIN_MIN",
        );
        assert_eq!(
            Cube::parse("LUT_3D_SIZE 66").unwrap_err().to_string(),
            "Line 1: LUT_3D_SIZE is outside 2 to 65"
        );
    }

    #[test]
    fn the_largest_table_is_read_and_one_larger_is_not() {
        let cube = Cube::parse(&text(MAX_CUBE_SIZE, |c| c)).unwrap();
        assert_eq!(cube.table.len(), 65 * 65 * 65);
        assert_eq!(cube.at(64, 64, 64), [1.0; 3]);
    }

    #[test]
    fn a_lookup_mixes_the_points_around_a_colour() {
        let identity = Cube::parse(&text(5, |c| c)).unwrap();
        let swap = Cube::parse(&text(2, |[r, g, b]| [b, g, r])).unwrap();
        for colour in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.3, 0.61, 0.07],
            [0.99, 0.5, 0.26],
        ] {
            let found = identity.sample(colour);
            let turned = swap.sample(colour);
            for c in 0..3 {
                assert!((found[c] - colour[c]).abs() < 1e-5, "{found:?}");
                assert!((turned[c] - colour[2 - c]).abs() < 1e-5, "{turned:?}");
            }
        }
        // Outside the domain is its edge, and the domain scales the input.
        assert_eq!(identity.sample([2.0, -1.0, 0.5]), [1.0, 0.0, 0.5]);
        let mut wide = identity.clone();
        wide.domain_max = [2.0; 3];
        let found = wide.sample([1.0, 0.5, 2.0]);
        assert!((found[0] - 0.5).abs() < 1e-5 && (found[1] - 0.25).abs() < 1e-5);
        assert!((found[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_file_is_read_whole_or_refused() {
        let temp = tempfile::tempdir().unwrap();
        let good = temp.path().join("good.cube");
        std::fs::write(&good, text(2, |c| c)).unwrap();
        assert_eq!(read(&good).unwrap().size, 2);
        let bad = temp.path().join("bad.cube");
        std::fs::write(&bad, "LUT_1D_SIZE 4\n").unwrap();
        let error = read(&bad).unwrap_err().to_string();
        assert!(
            error.contains("Line 1") && error.contains("1D LUT"),
            "{error}"
        );
        std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();
        assert!(read(&bad).is_err());
        assert!(read(&temp.path().join("gone.cube")).is_err());
        // Too long to be a table is not read at all.
        let long = temp.path().join("long.cube");
        let file = std::fs::File::create(&long).unwrap();
        file.set_len(MAX_CUBE_BYTES + 1).unwrap();
        let error = read(&long).unwrap_err().to_string();
        assert!(error.contains("32 MiB"), "{error}");
    }
}
