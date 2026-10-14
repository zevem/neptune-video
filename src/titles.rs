//! Titles and graphics on the canvas: pictures the title lane draws off the
//! frame, each held as one texture for as long as its layer is shown.
//!
//! A layer asks again only when what it shows, the scale it is drawn at or
//! the project's fonts change, and goes on showing its last picture until a
//! newer one comes: typing, an animation and a zoom never wait on a drawing.
use editor_core::{Asset, ClipId, LayerSource, MediaKind};
use editor_media::text::{TextEvent, TextService, TitlePicture, TitleRequest, quantize_scale};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::{
    collections::{HashMap, hash_map::Entry},
    path::PathBuf,
    sync::Arc,
};

/// Font files drawn with whatever the system has: the interface's own faces
/// and egui's monochrome emoji, the last resort for a picture character.
pub fn bundled_fonts() -> Vec<&'static [u8]> {
    let mut fonts = vec![
        include_bytes!("../assets/fonts/Geist-Regular.ttf").as_slice(),
        include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice(),
        include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice(),
    ];
    let definitions = egui::FontDefinitions::default();
    if let Some(data) = definitions.font_data.get("NotoEmoji-Regular")
        && let std::borrow::Cow::Borrowed(bytes) = &data.font
    {
        fonts.push(bytes);
    }
    fonts
}

/// One layer's picture: what was last asked for it and what is shown.
struct Held {
    asked: Asked,
    /// The serial of the newest request.
    serial: u64,
    /// The first serial asked for in this holding: an answer from before it
    /// is of a layer this one replaced.
    since: u64,
    shown: Option<Shown>,
    /// The frame the layer was last drawn in.
    seen: u64,
}

#[derive(PartialEq)]
struct Asked {
    source: LayerSource,
    scale: f32,
    fonts: Arc<[PathBuf]>,
}

struct Shown {
    serial: u64,
    picture: Arc<TitlePicture>,
    texture: TextureHandle,
}

#[derive(Default)]
pub struct Titles {
    service: Option<TextService>,
    held: HashMap<ClipId, Held>,
    serial: u64,
    frame: u64,
    /// The project's font files, as last asked with.
    fonts: Arc<[PathBuf]>,
    /// Every family a title may name, once the lane has said, and whether
    /// it was asked.
    families: Option<Arc<[String]>>,
    families_asked: bool,
}

impl Titles {
    pub fn new(service: Option<TextService>) -> Self {
        Self {
            service,
            ..Self::default()
        }
    }

    /// Begins a frame of the application: a layer drawn in neither the last
    /// frame nor this one gives its picture up.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.held.retain(|_, held| frame - held.seen <= 1);
    }

    /// Takes what the lane has drawn. A picture is shown when it answers a
    /// newer request than the one shown; one of a layer no longer drawn, or
    /// drawn again since, is let go.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(service) = &self.service else {
            return;
        };
        while let Some(event) = service.poll() {
            match event {
                TextEvent::Families(families) => self.families = Some(families),
                TextEvent::Drawn(drawn) => {
                    let Some(held) = self.held.get_mut(&drawn.clip) else {
                        continue;
                    };
                    let newer = held.shown.as_ref().is_none_or(|s| drawn.serial > s.serial);
                    if drawn.serial < held.since || !newer {
                        continue;
                    }
                    let picture = drawn.picture;
                    let image = ColorImage::from_rgba_premultiplied(
                        [picture.width as usize, picture.height as usize],
                        &picture.rgba,
                    );
                    let texture = match held.shown.take() {
                        // A picture of the same size is written over the
                        // last one's texture.
                        Some(mut shown)
                            if shown.picture.width == picture.width
                                && shown.picture.height == picture.height =>
                        {
                            shown.texture.set(image, TextureOptions::LINEAR);
                            shown.texture
                        }
                        _ => ctx.load_texture(
                            format!("title-{}", drawn.clip),
                            image,
                            TextureOptions::LINEAR,
                        ),
                    };
                    held.shown = Some(Shown {
                        serial: drawn.serial,
                        picture,
                        texture,
                    });
                }
            }
        }
    }

    /// Says the project's font files: titles naming their families are
    /// drawn with them.
    pub fn set_fonts<'a>(&mut self, assets: impl Iterator<Item = &'a Asset>) {
        let mut fonts: Vec<PathBuf> = assets
            .filter(|asset| asset.kind == MediaKind::Font)
            .map(|asset| asset.path.clone())
            .collect();
        fonts.sort();
        if *self.fonts != *fonts {
            self.fonts = fonts.into();
        }
    }

    /// The layer of `clip` is drawn this frame showing `source` at `scale`
    /// picture pixels per canvas pixel: asks for a picture when that is not
    /// what was last asked.
    pub fn want(&mut self, clip: ClipId, source: &LayerSource, scale: f32) {
        let Some(service) = &self.service else {
            return;
        };
        let scale = quantize_scale(scale);
        let frame = self.frame;
        let fonts = &self.fonts;
        let same = |asked: &Asked| {
            asked.scale == scale && Arc::ptr_eq(&asked.fonts, fonts) && asked.source == *source
        };
        if let Some(held) = self.held.get_mut(&clip) {
            held.seen = frame;
            if same(&held.asked) {
                return;
            }
        }
        self.serial += 1;
        let serial = self.serial;
        let asked = Asked {
            source: source.clone(),
            scale,
            fonts: Arc::clone(fonts),
        };
        service.request(TitleRequest {
            clip,
            serial,
            source: asked.source.clone(),
            scale,
            fonts: Arc::clone(fonts),
        });
        match self.held.entry(clip) {
            Entry::Occupied(mut entry) => {
                let held = entry.get_mut();
                held.asked = asked;
                held.serial = serial;
            }
            Entry::Vacant(entry) => {
                entry.insert(Held {
                    asked,
                    serial,
                    since: serial,
                    shown: None,
                    seen: frame,
                });
            }
        }
    }

    /// The picture shown for the layer of `clip`, and its texture.
    pub fn picture(&self, clip: ClipId) -> Option<(&TitlePicture, egui::TextureId)> {
        let shown = self.held.get(&clip)?.shown.as_ref()?;
        Some((&shown.picture, shown.texture.id()))
    }

    /// Whether the layer of `clip` shows an older picture than it asked for.
    #[cfg(test)]
    pub fn behind(&self, clip: ClipId) -> bool {
        self.held
            .get(&clip)
            .is_some_and(|held| held.shown.as_ref().is_none_or(|s| s.serial < held.serial))
    }

    /// The families a title may name, once they are known.
    pub fn families(&self) -> Option<&[String]> {
        self.families.as_deref()
    }

    /// Asks the lane once for the families: the system's faces are looked
    /// for only when a font is first chosen, or a title first drawn.
    pub fn want_families(&mut self) {
        if !self.families_asked
            && let Some(service) = &self.service
        {
            service.want_families();
            self.families_asked = true;
        }
    }

    /// Gives up every picture: the project is another.
    pub fn clear(&mut self) {
        self.held.clear();
    }
}
