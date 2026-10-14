//! A Gaussian blur of one coverage plane, as three box blurs each way.

/// `plane` (`width` by `height`) blurred with deviation `sigma` pixels.
pub(crate) fn gaussian(mut plane: Vec<u8>, width: usize, height: usize, sigma: f32) -> Vec<u8> {
    if sigma.is_nan() || sigma < 0.5 || width == 0 || height == 0 {
        return plane;
    }
    let mut scratch = vec![0_u8; plane.len()];
    for size in box_sizes(sigma) {
        let radius = (size - 1) / 2;
        if radius == 0 {
            continue;
        }
        horizontal(&plane, &mut scratch, width, height, radius);
        vertical(&scratch, &mut plane, width, height, radius);
    }
    plane
}

/// Three box widths whose passes approach a Gaussian of `sigma`: the odd
/// widths either side of the ideal, the narrower for the first `m` passes.
fn box_sizes(sigma: f32) -> [usize; 3] {
    let passes = 3.0_f32;
    let variance = 12.0 * sigma * sigma;
    let ideal = (variance / passes + 1.0).sqrt();
    let mut lower = ideal.floor() as usize;
    if lower.is_multiple_of(2) {
        lower = lower.saturating_sub(1);
    }
    let lower = lower.max(1);
    let l = lower as f32;
    let m =
        ((variance - passes * l * l - 4.0 * passes * l - 3.0 * passes) / (-4.0 * l - 4.0)).round();
    let mut sizes = [lower; 3];
    for (index, size) in sizes.iter_mut().enumerate() {
        if index as f32 >= m {
            *size = lower + 2;
        }
    }
    sizes
}

fn horizontal(from: &[u8], to: &mut [u8], width: usize, height: usize, radius: usize) {
    let span = (2 * radius + 1) as u32;
    for row in 0..height {
        let line = &from[row * width..(row + 1) * width];
        let out = &mut to[row * width..(row + 1) * width];
        blur_line(line.iter().copied(), width, radius, span, |index, value| {
            out[index] = value;
        });
    }
}

fn vertical(from: &[u8], to: &mut [u8], width: usize, height: usize, radius: usize) {
    let span = (2 * radius + 1) as u32;
    for column in 0..width {
        let line = (0..height).map(|row| from[row * width + column]);
        blur_line(line, height, radius, span, |index, value| {
            to[index * width + column] = value;
        });
    }
}

/// A running box mean along one line, with nothing past its ends.
fn blur_line(
    line: impl Iterator<Item = u8> + Clone,
    length: usize,
    radius: usize,
    span: u32,
    mut put: impl FnMut(usize, u8),
) {
    let values: Vec<u32> = line.map(u32::from).collect();
    let at = |index: isize| -> u32 {
        if index < 0 || index as usize >= length {
            0
        } else {
            values[index as usize]
        }
    };
    let radius = radius as isize;
    let mut sum: u32 = (-radius..=radius).map(at).sum();
    for index in 0..length as isize {
        put(index as usize, ((sum + span / 2) / span).min(255) as u8);
        sum += at(index + radius + 1);
        sum -= at(index - radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_keeps_coverage_and_spreads_it() {
        let (width, height) = (41, 41);
        let mut plane = vec![0_u8; width * height];
        for row in 15..26 {
            for column in 15..26 {
                plane[row * width + column] = 255;
            }
        }
        let before: u32 = plane.iter().map(|v| u32::from(*v)).sum();
        let blurred = gaussian(plane, width, height, 3.0);
        let after: u32 = blurred.iter().map(|v| u32::from(*v)).sum();
        // Rounding moves a little; nothing is lost past the edges here.
        assert!(before.abs_diff(after) < before / 50, "{before} {after}");
        assert!(blurred[20 * width + 20] > 200);
        assert!(blurred[20 * width + 12] > 0, "spread past the square");
        assert_eq!(blurred[0], 0);
    }

    #[test]
    fn a_small_deviation_changes_nothing() {
        let plane = vec![7_u8; 16];
        assert_eq!(gaussian(plane.clone(), 4, 4, 0.2), plane);
    }
}
