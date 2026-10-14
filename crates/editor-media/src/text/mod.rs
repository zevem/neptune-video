//! Titles and graphics drawn on the CPU into pictures the viewer shows as
//! one quad: fonts found and read lazily, text shaped with fallback faces,
//! set in lines, animated by unit and lit word by word.
mod blur;
mod draw;
mod fonts;
mod lane;
mod layout;
mod scene;

pub use fonts::{FONT_CACHE_BYTES, FontBook, MAX_FONT_FILE, font_family, is_font_file};
pub use lane::{TEXT_QUEUE, TITLE_BUDGET, TextEvent, TextService, TitleDrawn, TitleRequest};

use editor_core::LayerSource;
use std::sync::Arc;
use vello_cpu::kurbo::{Affine, Rect};

/// The longest side of a picture, in pixels.
pub const MAX_TITLE_SIDE: u32 = 4096;
/// The fewest and most picture pixels a canvas pixel may have.
pub const MIN_TITLE_SCALE: f32 = 1.0 / 64.0;
pub const MAX_TITLE_SCALE: f32 = 16.0;

/// A title or graphic drawn at one scale: premultiplied RGBA8, its left
/// top `origin` from the title's anchor (the centre of its text or design
/// box) in canvas pixels at the layer's scale 1.
#[derive(Clone, Debug)]
pub struct TitlePicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub origin: [f32; 2],
    /// Picture pixels per canvas pixel.
    pub scale: f32,
    /// The text or design box, left, top, right and bottom from the anchor:
    /// what is selected and dragged.
    pub bounds: [f32; 4],
}

impl TitlePicture {
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// The picture's size in canvas pixels at the layer's scale 1.
    pub fn size(&self) -> [f32; 2] {
        [
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        ]
    }
}

/// `scale` held to the steps pictures are drawn at: eighths of an octave,
/// so a zoom redraws a title at most eight times per doubling.
pub fn quantize_scale(scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 1.0;
    }
    let steps = (scale.log2() * 8.0).round() / 8.0;
    2_f32.powf(steps).clamp(MIN_TITLE_SCALE, MAX_TITLE_SCALE)
}

/// Draws a title or graphic layer at `scale` picture pixels per canvas
/// pixel; `None` for a media layer. A picture that would be wider or taller
/// than [`MAX_TITLE_SIDE`] is drawn at a smaller scale, which it reports.
pub fn draw_title(book: &mut FontBook, source: &LayerSource, scale: f32) -> Option<TitlePicture> {
    book.begin();
    let mut scene = scene::Scene::default();
    let bounds = match source {
        LayerSource::Media { .. } => return None,
        LayerSource::Text {
            text,
            font_size,
            color,
            style,
            enter,
            exit,
            words,
        } => {
            let spec = draw::TextSpec {
                text,
                font_size: *font_size,
                color: *color,
                style,
                enter: *enter,
                exit: *exit,
                words: *words,
                wrap: style.wrap,
            };
            draw::text_scene(
                &mut scene,
                book,
                &spec,
                |laid| {
                    Affine::translate((-f64::from(laid.width) / 2.0, -f64::from(laid.height) / 2.0))
                },
                1.0,
            )
        }
        LayerSource::Graphic { size, elements } => {
            draw::graphic_scene(&mut scene, book, *size, elements);
            let [width, height] = size.map(f64::from);
            Rect::new(-width / 2.0, -height / 2.0, width / 2.0, height / 2.0)
        }
    };
    let bounds_of = [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(|v| v as f32);
    let Some(ink) = scene.bounds() else {
        return Some(empty(bounds_of));
    };
    let mut scale = if scale.is_finite() {
        scale.clamp(MIN_TITLE_SCALE, MAX_TITLE_SCALE)
    } else {
        1.0
    };
    let longest = ink.width().max(ink.height()) as f32 * scale + 2.0;
    if longest > MAX_TITLE_SIDE as f32 {
        scale *= (MAX_TITLE_SIDE as f32 - 2.0) / longest;
    }
    let s = f64::from(scale);
    let left = (ink.x0 * s).floor();
    let top = (ink.y0 * s).floor();
    let width = ((ink.x1 * s).ceil() - left).clamp(1.0, f64::from(MAX_TITLE_SIDE)) as u16;
    let height = ((ink.y1 * s).ceil() - top).clamp(1.0, f64::from(MAX_TITLE_SIDE)) as u16;
    let base = Affine::translate((-left, -top)) * Affine::scale(s);
    let pixmap = scene.rasterise(width, height, base, scale);
    Some(TitlePicture {
        width: u32::from(width),
        height: u32::from(height),
        rgba: pixmap.data_as_u8_slice().into(),
        origin: [(left / s) as f32, (top / s) as f32],
        scale,
        bounds: bounds_of,
    })
}

/// Nothing to draw: one clear pixel, its box still where the text is.
fn empty(bounds: [f32; 4]) -> TitlePicture {
    TitlePicture {
        width: 1,
        height: 1,
        rgba: vec![0; 4].into(),
        origin: [0.0, 0.0],
        scale: 1.0,
        bounds,
    }
}

#[cfg(test)]
mod tests;
