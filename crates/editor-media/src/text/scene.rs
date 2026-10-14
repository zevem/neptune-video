//! What a title draws, as a list of shapes in title space (canvas pixels
//! from the title's anchor), and how that list is rasterised.
use super::fonts::Chosen;
use vello_cpu::{
    Glyph, Pixmap, RenderContext, Resources,
    kurbo::{Affine, BezPath, Cap, Join, Rect, Shape as _, Stroke, Vec2},
    peniko::Color,
};

#[derive(Clone)]
pub(crate) enum Shape {
    Path(BezPath),
    Glyphs {
        face: usize,
        size: f32,
        glyphs: Vec<Glyph>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum Paint {
    Fill([u8; 4]),
    /// A colour and a width in the item's own units.
    Stroke([u8; 4], f32),
}

#[derive(Clone)]
pub(crate) struct Item {
    /// From the item's own units into title space.
    pub transform: Affine,
    pub shape: Shape,
    pub paint: Paint,
    /// Where its ink may reach, in title space.
    pub bounds: Rect,
}

#[derive(Clone)]
pub(crate) enum Node {
    Item(Item),
    Group(Group),
}

/// Nodes drawn together: faded as one and cut to its clips.
#[derive(Clone, Default)]
pub(crate) struct Group {
    pub opacity: f32,
    /// Title space.
    pub clips: Vec<BezPath>,
    pub children: Vec<Node>,
}

impl Group {
    pub fn new(opacity: f32) -> Self {
        Self {
            opacity,
            clips: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn push(&mut self, node: Node) {
        self.children.push(node);
    }

    pub fn item(&mut self, item: Item) {
        self.children.push(Node::Item(item));
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    fn bounds(&self) -> Option<Rect> {
        if self.opacity <= 0.0 {
            return None;
        }
        let mut bounds: Option<Rect> = None;
        for child in &self.children {
            let rect = match child {
                Node::Item(item) => Some(item.bounds),
                Node::Group(group) => group.bounds(),
            };
            if let Some(rect) = rect {
                bounds = Some(bounds.map_or(rect, |known| known.union(rect)));
            }
        }
        // A clip only ever takes away.
        for clip in &self.clips {
            bounds = bounds.map(|rect| rect.intersect(clip.bounding_box()));
        }
        bounds.filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
    }
}

/// A blurred copy of a pass's ink in one colour, moved and under what
/// follows.
#[derive(Clone, Copy)]
pub(crate) struct Shadow {
    pub color: [u8; 4],
    /// Title space.
    pub offset: [f32; 2],
    /// The Gaussian's deviation, title space.
    pub sigma: f32,
}

pub(crate) struct Pass {
    pub group: Group,
    pub shadow: Option<Shadow>,
}

#[derive(Default)]
pub(crate) struct Scene {
    pub passes: Vec<Pass>,
    pub faces: Vec<Chosen>,
}

impl Scene {
    pub fn plain(&mut self, group: Group) {
        if !group.is_empty() {
            self.passes.push(Pass {
                group,
                shadow: None,
            });
        }
    }

    pub fn shadowed(&mut self, group: Group, shadow: Shadow) {
        if !group.is_empty() && shadow.color[3] > 0 {
            self.passes.push(Pass {
                group,
                shadow: Some(shadow),
            });
        }
    }

    /// Where any ink may reach, title space.
    pub fn bounds(&self) -> Option<Rect> {
        let mut bounds: Option<Rect> = None;
        for pass in &self.passes {
            let Some(mut rect) = pass.group.bounds() else {
                continue;
            };
            if let Some(shadow) = pass.shadow {
                let reach = f64::from(shadow.sigma * 3.0);
                rect = (rect + Vec2::new(f64::from(shadow.offset[0]), f64::from(shadow.offset[1])))
                    .inflate(reach, reach);
            }
            bounds = Some(bounds.map_or(rect, |known| known.union(rect)));
        }
        bounds
    }

    /// Draws the scene into a `width` by `height` premultiplied picture
    /// through `base` (title space to pixels, `scale` pixels a unit).
    pub fn rasterise(&self, width: u16, height: u16, base: Affine, scale: f32) -> Pixmap {
        let mut target = Pixmap::new(width, height);
        let mut resources = Resources::new();
        let mut context = RenderContext::new(width, height);
        let mut pending = false;
        for pass in &self.passes {
            match pass.shadow {
                None => {
                    self.draw(&mut context, &mut resources, base, &pass.group);
                    pending = true;
                }
                Some(shadow) => {
                    if pending {
                        flush(&mut context, &mut resources, &mut target);
                    }
                    let moved = base
                        * Affine::translate((
                            f64::from(shadow.offset[0]),
                            f64::from(shadow.offset[1]),
                        ));
                    let mut ink = Pixmap::new(width, height);
                    let mut shade = RenderContext::new(width, height);
                    self.draw(&mut shade, &mut resources, moved, &pass.group);
                    shade.flush();
                    shade.render(&mut ink, &mut resources);
                    let alpha: Vec<u8> = ink.data().iter().map(|pixel| pixel.a).collect();
                    let alpha = super::blur::gaussian(
                        alpha,
                        usize::from(width),
                        usize::from(height),
                        shadow.sigma * scale,
                    );
                    composite_shade(&mut target, &alpha, shadow.color);
                    // The ink itself, over its shadow.
                    self.draw(&mut context, &mut resources, base, &pass.group);
                    pending = true;
                }
            }
        }
        if pending {
            flush(&mut context, &mut resources, &mut target);
        }
        target
    }

    fn draw(
        &self,
        context: &mut RenderContext,
        resources: &mut Resources,
        base: Affine,
        group: &Group,
    ) {
        if group.opacity <= 0.0 {
            return;
        }
        context.set_transform(base);
        for clip in &group.clips {
            context.push_clip_layer(clip);
        }
        let faded = group.opacity < 1.0;
        if faded {
            context.push_opacity_layer(group.opacity);
        }
        for child in &group.children {
            match child {
                Node::Group(inner) => self.draw(context, resources, base, inner),
                Node::Item(item) => self.item(context, resources, base, item),
            }
        }
        if faded {
            context.pop_layer();
        }
        for _ in &group.clips {
            context.pop_layer();
        }
    }

    fn item(
        &self,
        context: &mut RenderContext,
        resources: &mut Resources,
        base: Affine,
        item: &Item,
    ) {
        let ([r, g, b, a], stroke) = match item.paint {
            Paint::Fill(color) => (color, None),
            Paint::Stroke(color, width) => (color, Some(width)),
        };
        if a == 0 {
            return;
        }
        context.set_transform(base * item.transform);
        context.set_paint(Color::from_rgba8(r, g, b, a));
        if let Some(width) = stroke {
            context.set_stroke(
                Stroke::new(f64::from(width))
                    .with_join(Join::Round)
                    .with_caps(Cap::Round),
            );
        }
        match &item.shape {
            Shape::Path(path) => match stroke {
                Some(_) => context.stroke_path(path),
                None => context.fill_path(path),
            },
            Shape::Glyphs { face, size, glyphs } => {
                let chosen = &self.faces[*face];
                let mut run = context
                    .glyph_run(resources, &chosen.face.data)
                    .font_size(*size)
                    .hint(false)
                    .normalized_coords(&chosen.bits);
                if chosen.slant {
                    run = run.glyph_transform(Affine::skew(-0.2, 0.0));
                }
                match stroke {
                    Some(_) => run.stroke_glyphs(glyphs.iter().copied()),
                    None => run.fill_glyphs(glyphs.iter().copied()),
                }
            }
        }
    }
}

fn flush(context: &mut RenderContext, resources: &mut Resources, target: &mut Pixmap) {
    context.flush();
    context.render_with(
        &mut *target,
        resources,
        vello_cpu::RasterizerSettings {
            composite_mode: vello_cpu::CompositeMode::SrcOver,
            ..Default::default()
        },
    );
    context.reset();
}

/// `color` at each pixel's `alpha`, over what is already drawn: a shadow
/// lies over what came before its pass and under its own ink.
fn composite_shade(target: &mut Pixmap, alpha: &[u8], [r, g, b, a]: [u8; 4]) {
    let color = [u32::from(r), u32::from(g), u32::from(b)];
    for (pixel, &coverage) in target.data_mut().iter_mut().zip(alpha) {
        let sa = u32::from(coverage) * u32::from(a) / 255;
        if sa == 0 {
            continue;
        }
        let keep = 255 - sa;
        let over = |source: u32, dest: u8| -> u8 {
            ((source * sa + 127) / 255 + (u32::from(dest) * keep + 127) / 255).min(255) as u8
        };
        pixel.r = over(color[0], pixel.r);
        pixel.g = over(color[1], pixel.g);
        pixel.b = over(color[2], pixel.b);
        pixel.a = (sa + (u32::from(pixel.a) * keep + 127) / 255).min(255) as u8;
    }
}
