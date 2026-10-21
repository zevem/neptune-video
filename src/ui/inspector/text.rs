//! The Text, Graphic and Animate sections: what a title says and how it
//! looks, a graphic's words, colours and font, and how either comes and goes.
//!
//! Every row writes the draft's content, as the rest of the inspector writes
//! its values, and the application commits it as one undo step when the
//! gesture ends: a word typed, a colour picked, a value dragged. Sizes the
//! project keeps in ems (outline, shadow, padding, spacing) are shown in
//! canvas pixels at the title's size, which is how they look.
use super::{
    FIELD, LEAD, LIST_HEIGHT, PAIRED_GAP, button, card, clip::Pane, controls, fact, number, row,
    section, select_list, select_popup, tall_row, text_field, well,
};
use crate::{
    theme::{self, Palette, metrics},
    ui::{
        Action, PresetChange, kit,
        widgets::{self, Kind},
    },
};
use editor_core::{
    BackgroundShape, ClipContent, Element, ElementKind, FontFace, FrameRate, Highlight,
    HighlightMode, TextAlign, TextAnimation, TextAnimationKind, TextBackground, TextCase,
    TextOutline, TextShadow, TextStyle, TextUnit, Time, title,
};
use eframe::egui::{self, Ui, WidgetInfo, WidgetType, vec2};

const WEIGHTS: [(u16, &str); 9] = [
    (100, "Thin"),
    (200, "Extra light"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "Semibold"),
    (700, "Bold"),
    (800, "Extra bold"),
    (900, "Black"),
];

const ALIGNS: [(TextAlign, &str); 3] = [
    (TextAlign::Left, "Left"),
    (TextAlign::Center, "Centre"),
    (TextAlign::Right, "Right"),
];

const CASES: [(TextCase, &str); 3] = [
    (TextCase::AsTyped, "As typed"),
    (TextCase::Upper, "UPPERCASE"),
    (TextCase::Lower, "lowercase"),
];

const UNITS: [(TextUnit, &str); 4] = [
    (TextUnit::Whole, "All at once"),
    (TextUnit::Line, "By line"),
    (TextUnit::Word, "By word"),
    (TextUnit::Letter, "By letter"),
];

const HIGHLIGHTS: [(HighlightMode, &str); 3] = [
    (HighlightMode::Word, "Word"),
    (HighlightMode::Box, "Box"),
    (HighlightMode::Fill, "Fill"),
];

const SHAPES: [(BackgroundShape, &str); 2] = [
    (BackgroundShape::Block, "One box"),
    (BackgroundShape::Lines, "Each line"),
];

/// The longest an entrance or an exit may take.
const LONGEST: f64 = 10.0;
/// What a list of fonts shows at most: a search finds the rest.
const LISTED: usize = 300;

pub(super) fn kind_name(kind: TextAnimationKind) -> &'static str {
    match kind {
        TextAnimationKind::Fade => "Fade",
        TextAnimationKind::Pop => "Pop",
        TextAnimationKind::Bounce => "Bounce",
        TextAnimationKind::Typewriter => "Typewriter",
        TextAnimationKind::SlideUp => "Slide up",
        TextAnimationKind::SlideDown => "Slide down",
        TextAnimationKind::SlideLeft => "Slide left",
        TextAnimationKind::SlideRight => "Slide right",
        TextAnimationKind::Wipe => "Wipe",
    }
}

/// The width of a select of a title's card: the span of a pair of fields,
/// so that a card's values start at one place.
const SELECT: f32 = 132.0;

/// A select of named values in a row, named by the row.
fn choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    value: &mut T,
    options: &[(T, &'static str)],
    salt: &str,
) {
    tall_row(ui, p, label, |ui| {
        let width = ui.available_width().min(SELECT);
        select_list(ui, p, salt, label, value, options, width);
    });
}

/// A length the project keeps in ems, shown in canvas pixels at `size`.
fn em_field(ui: &mut Ui, p: &Palette, value: &mut f32, size: f32, range: [f32; 2], name: &str) {
    em_field_sized(ui, p, value, size, range, (name, FIELD));
}

fn em_field_sized(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    size: f32,
    range: [f32; 2],
    (name, width): (&str, f32),
) {
    let mut shown = *value * size;
    let field = number(
        ui,
        p,
        &mut shown,
        range[0] * size..=range[1] * size,
        0.25,
        " px",
        1,
        width,
        name,
    );
    if field.changed() {
        *value = (shown / size.max(1.0)).clamp(range[0], range[1]);
    }
}

fn em_row(ui: &mut Ui, p: &Palette, label: &str, value: &mut f32, size: f32, range: [f32; 2]) {
    row(ui, p, label, |ui| {
        em_field(ui, p, value, size, range, label)
    });
}

/// A row of two lengths in ems, each named, the first at the leading side.
fn em_pair(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    (first, second): (&mut f32, &mut f32),
    size: f32,
    range: [f32; 2],
    names: [&str; 2],
) {
    row(ui, p, label, |ui| {
        ui.spacing_mut().item_spacing.x = PAIRED_GAP;
        let width = super::paired_width(ui);
        super::in_reading_order(ui, 2.0 * width + PAIRED_GAP, |ui| {
            em_field_sized(ui, p, first, size, range, (names[0], width));
            em_field_sized(ui, p, second, size, range, (names[1], width));
        });
    });
}

fn color_row(ui: &mut Ui, p: &Palette, label: &str, color: &mut [u8; 4], brand: &[[u8; 4]]) {
    row(ui, p, label, |ui| {
        well::well(ui, p, label, color, true, brand);
    });
}

/// How wide a title's lines may be before they wrap, or off: a number field
/// that reads "Off" at nought, which the kit's field does not say.
fn wrap_field(ui: &mut Ui, p: &Palette, wrap: &mut f32) {
    let surface = ui.painter().add(egui::Shape::Noop);
    let size = vec2(FIELD, metrics::INSPECTOR_CONTROL);
    let enabled = ui.is_enabled();
    let response = ui
        .scope(|ui| {
            let style = ui.style_mut();
            style
                .text_styles
                .insert(style.drag_value_text_style.clone(), theme::regular(12.5));
            style.spacing.interact_size = size;
            style.spacing.button_padding = vec2(8.0, 0.0);
            let visuals = &mut style.visuals;
            if !enabled {
                visuals.override_text_color = Some(p.muted);
            }
            visuals.text_edit_bg_color = Some(egui::Color32::TRANSPARENT);
            visuals.selection.stroke = egui::Stroke::NONE;
            for widget in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.weak_bg_fill = egui::Color32::TRANSPARENT;
                widget.bg_fill = egui::Color32::TRANSPARENT;
                widget.bg_stroke = egui::Stroke::NONE;
                widget.corner_radius = egui::CornerRadius::same(7);
            }
            let drag = egui::DragValue::new(wrap)
                .range(0.0..=16384.0)
                .speed(2.0)
                .custom_formatter(|value, _| {
                    if value <= 0.0 {
                        "Off".to_owned()
                    } else {
                        format!("{value:.0} px")
                    }
                })
                .custom_parser(|text| {
                    let text = text.trim().trim_end_matches("px").trim();
                    if text.eq_ignore_ascii_case("off") {
                        Some(0.0)
                    } else {
                        text.parse().ok()
                    }
                });
            let layout = egui::Layout::centered_and_justified(ui.layout().main_dir())
                .with_main_align(egui::Align::Max);
            ui.allocate_ui_with_layout(size, layout, |ui| ui.add(drag))
                .inner
        })
        .inner;
    response.widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, enabled, "Wrap at"));
    let rect = egui::Rect::from_center_size(response.rect.center(), size);
    let mut shapes = vec![egui::Shape::rect_filled(rect, 7, p.control)];
    if response.has_focus() {
        shapes.push(egui::Shape::rect_stroke(
            rect.expand(2.5),
            9,
            egui::Stroke::new(3.0, theme::tint(p.accent, 0.28)),
            egui::StrokeKind::Inside,
        ));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            7,
            egui::Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        ));
    } else if enabled && response.hovered() {
        shapes.push(egui::Shape::rect_filled(rect, 7, p.hover));
    }
    ui.painter().set(surface, egui::Shape::Vec(shapes));
}

/// What the clip shows kept as one of the person's presets, under a name
/// typed in the popup of its button.
fn keep_card(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    default: &str,
    name: &mut String,
) -> Option<String> {
    let mut kept = None;
    section(ui, p, "Preset");
    card(ui, p, |ui| {
        row(ui, p, label, |ui| {
            let keep = button(ui, p, None, "Keep…", Kind::Plain);
            let opened = keep.clicked();
            if opened {
                default.clone_into(name);
            }
            egui::Popup::menu(&keep)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 240.0);
                    ui.add(egui::Label::new(
                        egui::RichText::new("Name")
                            .font(theme::regular(11.5))
                            .color(p.secondary),
                    ));
                    let id = ui.id().with("preset-name");
                    let field = text_field(ui, p, id, name, "Preset name", ui.available_width());
                    if opened {
                        field.request_focus();
                    }
                    let returned =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(6.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let keep = button(ui, p, None, "Keep", Kind::Primary);
                        if keep.clicked() || returned {
                            kept = Some(name.clone());
                            ui.close();
                        }
                    });
                });
        });
    });
    kept
}

/// The first row of a card whose other rows are there only while it is on:
/// the card is named by its switch.
fn switched<T>(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    value: &mut Option<T>,
    on: impl FnOnce() -> T,
) {
    row(ui, p, label, |ui| {
        if widgets::switch(ui, p, value.is_some(), label).clicked() {
            *value = match value {
                Some(_) => None,
                None => Some(on()),
            };
        }
    });
}

fn note(ui: &mut Ui, p: &Palette, text: &str) {
    super::note(ui, p, text);
}

/// A quiet line of a list: what it holds when it holds no row to choose.
fn menu_line(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add(egui::Label::new(
        egui::RichText::new(text)
            .font(theme::regular(12.0))
            .color(p.secondary),
    ));
}

/// A text the person writes in, across the card: a multi-line field on the
/// card's own material, named by `hint`.
fn words_field(ui: &mut Ui, p: &Palette, text: &mut String, hint: &str, rows: usize) {
    super::rule(ui, p);
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: LEAD as i8,
            right: LEAD as i8,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| writing(ui, p, text, hint, rows));
}

/// A multi-line field `rows` tall, named and hinted by `hint`: edged as
/// every field is, with the accent's outline and halo while it has the
/// keyboard (§3.10). Its words are set 8 in from its edge.
fn writing(ui: &mut Ui, p: &Palette, text: &mut String, hint: &str, rows: usize) {
    let surface = ui.painter().add(egui::Shape::Noop);
    let response = ui.add(
        egui::TextEdit::multiline(text)
            .desired_rows(rows)
            .desired_width(f32::INFINITY)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 5)))
            .font(theme::regular(13.0))
            .hint_text(hint),
    );
    // The hint is shown only while the field is empty: it is also the
    // field's name, which is said whatever it holds.
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, ui.is_enabled(), hint));
    let rect = response.rect;
    let mut shapes = vec![egui::Shape::rect_filled(rect, 7, p.control)];
    if response.has_focus() {
        shapes.push(egui::Shape::rect_stroke(
            rect.expand(2.5),
            9,
            egui::Stroke::new(3.0, theme::tint(p.accent, 0.28)),
            egui::StrokeKind::Inside,
        ));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            7,
            egui::Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        ));
    } else {
        shapes.push(egui::Shape::rect_stroke(
            rect,
            7,
            p.hairline(),
            egui::StrokeKind::Inside,
        ));
    }
    ui.painter().set(surface, egui::Shape::Vec(shapes));
}

/// The families a title may name: the lane's, the project's font files and
/// the bundled face, sorted, once they are known.
fn family_list(families: Option<&[String]>, project: &[String]) -> Option<Vec<String>> {
    let mut all: Vec<String> = families?.to_vec();
    all.extend(project.iter().cloned());
    all.push(title::DEFAULT_FAMILY.to_owned());
    all.sort_by_key(|name| name.to_lowercase());
    all.dedup();
    Some(all)
}

/// The family, weight and slant rows of a face.
pub(super) fn font_rows(
    ui: &mut Ui,
    p: &Palette,
    font: &mut FontFace,
    families: Option<&[String]>,
    project: &[String],
    filter: &mut String,
    salt: &str,
) {
    tall_row(ui, p, "Font", |ui| {
        let width = ui.available_width().min(SELECT);
        // The search stays in view over the families, which scroll.
        let (_, picked) = select_popup(
            ui,
            p,
            (("family", salt), "Font"),
            font.family.as_str(),
            width,
            false,
            |ui| {
                let id = ui.id().with(("font-search", salt));
                controls::search_field_sized(
                    ui,
                    *p,
                    id,
                    filter,
                    "Search fonts",
                    "Search fonts",
                    vec2(ui.available_width(), 28.0),
                );
                ui.add_space(4.0);
                let Some(all) = family_list(families, project) else {
                    menu_line(ui, p, "Looking for fonts…");
                    return None;
                };
                let wanted = filter.trim().to_lowercase();
                let mut picked = None;
                let mut shown = 0;
                egui::ScrollArea::vertical()
                    .max_height(LIST_HEIGHT)
                    .show(ui, |ui| {
                        for name in
                            (all.iter()).filter(|name| name.to_lowercase().contains(&wanted))
                        {
                            if shown == LISTED {
                                menu_line(ui, p, "Search to find more");
                                break;
                            }
                            shown += 1;
                            // Neptune Video: §5.4 shows each family in its
                            // own face here. The families are the title
                            // lane's, which egui has not loaded, and loading
                            // them is file reads the frame may not wait on;
                            // until the lane hands faces over, a row is set
                            // in the menu's face.
                            let on = font.family == *name;
                            if kit::menu_row(ui, p, None, name, "", false, Some(on)).clicked() {
                                picked = Some(name.clone());
                            }
                        }
                        if shown == 0 {
                            menu_line(ui, p, "No font has that name");
                        }
                    });
                picked
            },
        );
        if let Some(name) = picked.flatten() {
            font.family = name;
        }
    });
    if let Some(all) = family_list(families, project)
        && !all.contains(&font.family)
    {
        note(
            ui,
            p,
            &format!(
                "“{}” is not on this computer: {} stands in for it.",
                font.family,
                title::DEFAULT_FAMILY
            ),
        );
    }
    tall_row(ui, p, "Weight", |ui| {
        let width = ui.available_width().min(SELECT);
        // A weight between the named ones reads as the nearest.
        let mut nearest = WEIGHTS
            .iter()
            .min_by_key(|(weight, _)| weight.abs_diff(font.weight))
            .map_or(font.weight, |(weight, _)| *weight);
        let before = nearest;
        select_list(
            ui,
            p,
            ("weight", salt),
            "Weight",
            &mut nearest,
            &WEIGHTS,
            width,
        );
        if nearest != before {
            font.weight = nearest;
        }
    });
    row(ui, p, "Italic", |ui| {
        if widgets::switch(ui, p, font.italic, "Italic").clicked() {
            font.italic = !font.italic;
        }
    });
}

/// The rows of a text's look besides its face, size and colour: layout,
/// outline, shadow and background.
fn style_cards(
    ui: &mut Ui,
    p: &Palette,
    style: &mut TextStyle,
    size: f32,
    salt: &str,
    brand: &[[u8; 4]],
) {
    section(ui, p, "Layout");
    card(ui, p, |ui| {
        super::choice(ui, p, &mut style.align, &ALIGNS);
        row(ui, p, "Line height", |ui| {
            number(
                ui,
                p,
                &mut style.line_height,
                0.5..=4.0,
                0.01,
                "×",
                2,
                FIELD,
                "Line height",
            );
        });
        em_row(ui, p, "Spacing", &mut style.tracking, size, [-0.5, 2.0]);
        row(ui, p, "Wrap at", |ui| wrap_field(ui, p, &mut style.wrap));
        choice(
            ui,
            p,
            "Case",
            &mut style.case,
            &CASES,
            &format!("case-{salt}"),
        );
    });

    card(ui, p, |ui| {
        switched(ui, p, "Outline", &mut style.outline, || TextOutline {
            color: [0, 0, 0, 255],
            width: 0.06,
        });
        if let Some(outline) = &mut style.outline {
            color_row(ui, p, "Colour", &mut outline.color, brand);
            em_row(ui, p, "Width", &mut outline.width, size, [0.005, 0.5]);
        }
    });

    card(ui, p, |ui| {
        switched(ui, p, "Shadow", &mut style.shadow, || TextShadow {
            color: [0, 0, 0, 160],
            offset: [0.04, 0.06],
            blur: 0.12,
        });
        if let Some(shadow) = &mut style.shadow {
            color_row(ui, p, "Colour", &mut shadow.color, brand);
            let [right, down] = &mut shadow.offset;
            em_pair(
                ui,
                p,
                "Offset",
                (right, down),
                size,
                [-2.0, 2.0],
                ["Shadow offset right", "Shadow offset down"],
            );
            em_row(ui, p, "Blur", &mut shadow.blur, size, [0.0, 1.0]);
        }
    });

    card(ui, p, |ui| {
        switched(ui, p, "Background", &mut style.background, || {
            TextBackground {
                color: [0, 0, 0, 160],
                padding: [0.3, 0.15],
                radius: 0.2,
                shape: BackgroundShape::Block,
            }
        });
        if let Some(background) = &mut style.background {
            super::choice(ui, p, &mut background.shape, &SHAPES);
            color_row(ui, p, "Colour", &mut background.color, brand);
            let [sides, above] = &mut background.padding;
            em_pair(
                ui,
                p,
                "Padding",
                (sides, above),
                size,
                [0.0, 4.0],
                ["Padding at the sides", "Padding above and below"],
            );
            em_row(ui, p, "Corners", &mut background.radius, size, [0.0, 4.0]);
        }
    });
}

/// The rows of one edge's animation, which may take at most `room`.
fn animation_card(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    animation: &mut Option<TextAnimation>,
    room: Time,
    rate: FrameRate,
    salt: &str,
) {
    let frame = rate.frame_time(1);
    section(ui, p, label);
    card(ui, p, |ui| {
        let longest = room.min(Time::from_seconds(LONGEST));
        let fits = longest >= frame;
        let none = "None";
        tall_row(ui, p, "Style", |ui| {
            let kind = animation.map(|animation| animation.kind);
            let width = ui.available_width().min(SELECT);
            let mut picked = None;
            ui.add_enabled_ui(fits || animation.is_some(), |ui| {
                // Every style is listed: none is under a scroll.
                let (_, chosen) = select_popup(
                    ui,
                    p,
                    (("animation", salt), label),
                    kind.map_or(none, kind_name),
                    width,
                    false,
                    |ui| {
                        let mut chosen = None;
                        if kit::menu_row(ui, p, None, none, "", false, Some(kind.is_none()))
                            .clicked()
                        {
                            chosen = Some(None);
                        }
                        for option in TextAnimationKind::ALL {
                            let on = kind == Some(option);
                            if kit::menu_row(ui, p, None, kind_name(option), "", false, Some(on))
                                .clicked()
                            {
                                chosen = Some(Some(option));
                            }
                        }
                        chosen
                    },
                );
                picked = chosen.flatten();
            });
            if let Some(kind) = picked
                && kind != animation.map(|animation| animation.kind)
            {
                *animation = kind.map(|kind| match animation {
                    // Another style keeps the timing and takes its usual unit.
                    Some(was) => TextAnimation {
                        kind,
                        unit: kind.usual_unit(),
                        ..*was
                    },
                    None => TextAnimation {
                        kind,
                        unit: kind.usual_unit(),
                        duration: Time::from_seconds(0.5).min(longest).max(frame),
                        delay: Time::ZERO,
                    },
                });
            }
        });
        if !fits && animation.is_none() {
            note(ui, p, "The clip is too short for one here.");
        }
        let Some(animation) = animation else {
            return;
        };
        choice(
            ui,
            p,
            "Order",
            &mut animation.unit,
            &UNITS,
            &format!("unit-{salt}"),
        );
        let seconds = |ui: &mut Ui, value: &mut Time, min: Time, max: Time, name: &str| {
            let mut shown = value.as_seconds() as f32;
            let field = number(
                ui,
                p,
                &mut shown,
                min.as_seconds() as f32..=max.as_seconds().max(min.as_seconds()) as f32,
                0.01,
                " s",
                2,
                FIELD,
                name,
            );
            if field.changed() {
                *value = on_frames(f64::from(shown), rate).clamp(min, max.max(min));
            }
        };
        row(ui, p, "Duration", |ui| {
            let max = longest - animation.delay;
            seconds(ui, &mut animation.duration, frame, max, "Duration");
        });
        row(
            ui,
            p,
            if label == "In" {
                "Delay"
            } else {
                "Ends before"
            },
            |ui| {
                let max = room - animation.duration;
                seconds(ui, &mut animation.delay, Time::ZERO, max, "Delay");
            },
        );
    });
}

/// `seconds` on the canvas's frames.
fn on_frames(seconds: f64, rate: FrameRate) -> Time {
    let per_second = f64::from(rate.numerator) / f64::from(rate.denominator.max(1));
    rate.frame_time((seconds.max(0.0) * per_second).round() as i64)
}

/// The words of a karaoke caption as they are said.
fn karaoke_card(
    ui: &mut Ui,
    p: &Palette,
    highlight: &mut Option<Highlight>,
    timed: bool,
    brand: &[[u8; 4]],
) {
    section(ui, p, "Karaoke");
    card(ui, p, |ui| {
        switched(ui, p, "Highlight words", highlight, || Highlight {
            mode: HighlightMode::Box,
            color: [255, 61, 127, 255],
            scale: 1.1,
            upcoming: 1.0,
        });
        if !timed {
            note(
                ui,
                p,
                "Words light up as they are said in captions with word times.",
            );
        }
        let Some(highlight) = highlight else {
            return;
        };
        super::choice(ui, p, &mut highlight.mode, &HIGHLIGHTS);
        color_row(ui, p, "Colour", &mut highlight.color, brand);
        row(ui, p, "Word size", |ui| {
            let mut percent = highlight.scale * 100.0;
            let field = number(
                ui,
                p,
                &mut percent,
                100.0..=200.0,
                0.5,
                " %",
                0,
                FIELD,
                "Word size",
            );
            if field.changed() {
                highlight.scale = (percent / 100.0).clamp(1.0, 2.0);
            }
        });
        row(ui, p, "Words to come", |ui| {
            // The label is laid out: the slider has the rest of the row.
            let width = ui.available_width().min(120.0);
            widgets::slider(ui, p, &mut highlight.upcoming, 1.0, width, "Words to come");
        });
    });
}

impl Pane<'_> {
    /// What a title says and how it looks.
    pub(super) fn text(&mut self, ui: &mut Ui) {
        let p = self.p;
        let brand = self.brand;
        let project_fonts = self.project_fonts();
        let families = self.families;
        let language = self.track.language.clone();
        self.state.families_wanted = true;
        let filter = &mut self.state.font_filter;
        let Some(ClipContent::Text {
            text,
            font_size,
            color,
            style,
            ..
        }) = &mut self.next.content
        else {
            return;
        };
        section(ui, p, "Text");
        card(ui, p, |ui| {
            words_field(ui, p, text, "Text", 2);
            if let Some(language) = &language {
                fact(ui, p, "Language", language.as_str());
            }
        });
        section(ui, p, "Font");
        card(ui, p, |ui| {
            font_rows(
                ui,
                p,
                &mut style.font,
                families,
                &project_fonts,
                filter,
                "title",
            );
            row(ui, p, "Size", |ui| {
                number(ui, p, font_size, 1.0..=4096.0, 1.0, " px", 0, FIELD, "Size")
            });
            color_row(ui, p, "Colour", color, brand);
        });
        let size = *font_size;
        style_cards(ui, p, style, size, "title", brand);
        let named: String = text.lines().next().unwrap_or("").chars().take(32).collect();
        let named = if named.trim().is_empty() {
            "My title".to_owned()
        } else {
            named
        };
        self.keep(ui, "Keep this look", &named);
    }

    /// The row that keeps what the clip shows as a preset of the person's,
    /// placed where it is.
    fn keep(&mut self, ui: &mut Ui, label: &str, default: &str) {
        let Some(name) = keep_card(ui, self.p, label, default, &mut self.state.keeping) else {
            return;
        };
        if let Some(content) = self.next.content.clone() {
            self.actions.push(Action::Presets(PresetChange::Keep {
                name,
                content: Box::new(content),
                position: Some(self.clip.transform.position),
            }));
        }
    }

    /// How a title or a graphic comes and goes, and how a caption's words
    /// light up.
    pub(super) fn animate(&mut self, ui: &mut Ui) {
        let p = self.p;
        let brand = self.brand;
        let rate = self.project.settings.frame_rate;
        let duration = self.clip.duration;
        match &mut self.next.content {
            Some(ClipContent::Text {
                enter,
                exit,
                style,
                words,
                ..
            }) => {
                let exit_span = exit.map_or(Time::ZERO, |exit| exit.span());
                animation_card(ui, p, "In", enter, duration - exit_span, rate, "in");
                let enter_span = enter.map_or(Time::ZERO, |enter| enter.span());
                animation_card(ui, p, "Out", exit, duration - enter_span, rate, "out");
                karaoke_card(ui, p, &mut style.highlight, !words.is_empty(), brand);
            }
            Some(ClipContent::Graphic { elements, .. }) => {
                graphic_animation(ui, p, elements, duration, rate);
            }
            _ => {}
        }
    }

    /// A graphic's words, colours and font.
    pub(super) fn graphic(&mut self, ui: &mut Ui) {
        let p = self.p;
        let brand = self.brand;
        let project_fonts = self.project_fonts();
        let families = self.families;
        self.state.families_wanted = true;
        let filter = &mut self.state.font_filter;
        let Some(ClipContent::Graphic { elements, .. }) = &mut self.next.content else {
            return;
        };
        let texts = (elements.iter())
            .filter(|element| matches!(element.kind, ElementKind::Text { .. }))
            .count();
        if texts > 0 {
            section(ui, p, "Text");
            card(ui, p, |ui| {
                for element in elements.iter_mut() {
                    let ElementKind::Text { text, .. } = &mut element.kind else {
                        continue;
                    };
                    // The part's name over its words.
                    super::rule(ui, p);
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: LEAD as i8,
                            right: LEAD as i8,
                            top: 8,
                            bottom: 8,
                        })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.add(egui::Label::new(
                                egui::RichText::new(&element.name)
                                    .font(theme::regular(11.5))
                                    .color(p.secondary),
                            ));
                            writing(ui, p, text, element.name.as_str(), 1);
                        });
                }
            });
        }

        let colors = title::graphic_colors(elements);
        if !colors.is_empty() {
            section(ui, p, "Colours");
            card(ui, p, |ui| {
                for from in colors {
                    let name = (elements.iter())
                        .find(|element| element.kind.colors().contains(&from))
                        .map_or("Colour", |element| element.name.as_str())
                        .to_owned();
                    let mut to = from;
                    color_row(ui, p, &name, &mut to, brand);
                    if to != from {
                        for element in elements.iter_mut() {
                            element.kind.recolor(from, to);
                        }
                    }
                }
            });
        }

        // One face for every text in the graphic, read from the first.
        if let Some(mut font) = (elements.iter()).find_map(|element| match &element.kind {
            ElementKind::Text { style, .. } => Some(style.font.clone()),
            _ => None,
        }) {
            section(ui, p, "Font");
            card(ui, p, |ui| {
                font_rows(
                    ui,
                    p,
                    &mut font,
                    families,
                    &project_fonts,
                    filter,
                    "graphic",
                );
            });
            for element in elements.iter_mut() {
                if let ElementKind::Text { style, .. } = &mut element.kind
                    && style.font != font
                {
                    style.font.clone_from(&font);
                }
            }
        }
        let named = self.clip.name.clone();
        self.keep(ui, "Keep as template", &named);
    }

    /// The families of the project's font files.
    fn project_fonts(&self) -> Vec<String> {
        (self.project.assets.values())
            .filter(|asset| asset.kind == editor_core::MediaKind::Font)
            .map(|asset| asset.name.clone())
            .collect()
    }
}

/// One entrance and one exit for every element of a graphic, the elements
/// following each other by a stagger.
fn graphic_animation(
    ui: &mut Ui,
    p: &Palette,
    elements: &mut [Element],
    clip: Time,
    rate: FrameRate,
) {
    let frame = rate.frame_time(1);
    for (label, entering) in [("In", true), ("Out", false)] {
        let edge = |element: &Element| {
            if entering {
                element.enter
            } else {
                element.exit
            }
        };
        let other = |element: &Element| {
            if entering {
                element.exit
            } else {
                element.enter
            }
        };
        // The first element's is shown; the rest follow it.
        let shown = elements.iter().find_map(edge);
        let stagger = stagger_of(elements, entering);
        let room = (elements.iter())
            .map(|element| clip - other(element).map_or(Time::ZERO, |other| other.span()))
            .min()
            .unwrap_or(clip);
        let mut held = shown.map(|animation| TextAnimation {
            delay: Time::ZERO,
            ..animation
        });
        let before = held;
        animation_card(
            ui,
            p,
            label,
            &mut held,
            room,
            rate,
            &format!("graphic-{label}"),
        );
        let mut stagger_held = stagger;
        if held.is_some() && elements.len() > 1 {
            card(ui, p, |ui| {
                row(ui, p, "Between parts", |ui| {
                    let mut shown = stagger_held.as_seconds() as f32;
                    let field = number(
                        ui,
                        p,
                        &mut shown,
                        0.0..=1.0,
                        0.005,
                        " s",
                        2,
                        FIELD,
                        "Between parts",
                    );
                    if field.changed() {
                        stagger_held = on_frames(f64::from(shown), rate);
                    }
                });
            });
        }
        if held == before && stagger_held == stagger {
            continue;
        }
        for (index, element) in elements.iter_mut().enumerate() {
            let room = clip - other(element).map_or(Time::ZERO, |other| other.span());
            // An exit's delay counts back from the clip's end, so the first
            // part, which came first, leaves last.
            let order = index;
            let animation = held.map(|held| {
                let duration = held.duration.min(room).max(frame);
                // The stagger is whole frames, so each delay is too.
                let frames = (stagger_held.0 as f64 / frame.0.max(1) as f64).round() as i64;
                let delay = rate
                    .frame_time(frames.saturating_mul(order as i64))
                    .min(room - duration)
                    .max(Time::ZERO);
                TextAnimation {
                    duration,
                    delay,
                    ..held
                }
            });
            if entering {
                element.enter = animation;
            } else {
                element.exit = animation;
            }
        }
    }
}

/// The time between the elements' animations on one edge, when the first
/// two have one.
fn stagger_of(elements: &[Element], entering: bool) -> Time {
    let delay = |element: &Element| {
        if entering {
            element.enter
        } else {
            element.exit
        }
        .map(|animation| animation.delay)
    };
    match elements {
        [first, second, ..] => match (delay(first), delay(second)) {
            (Some(first), Some(second)) => (second - first).max(Time::ZERO),
            _ => Time::ZERO,
        },
        _ => Time::ZERO,
    }
}
