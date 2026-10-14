//! The Text, Graphic and Animate sections: what a title says and how it
//! looks, a graphic's words, colours and font, and how either comes and goes.
//!
//! Every row writes the draft's content, as the rest of the inspector writes
//! its values, and the application commits it as one undo step when the
//! gesture ends: a word typed, a colour picked, a value dragged. Sizes the
//! project keeps in ems (outline, shadow, padding, spacing) are shown in
//! canvas pixels at the title's size, which is how they look.
use super::{card, clip::Pane, fact};
use crate::{
    theme::{self, Palette},
    ui::{
        Action, PresetChange,
        widgets::{self, fitted_row, row, section},
    },
};
use editor_core::{
    BackgroundShape, ClipContent, Element, ElementKind, FontFace, FrameRate, Highlight,
    HighlightMode, TextAlign, TextAnimation, TextAnimationKind, TextBackground, TextCase,
    TextOutline, TextShadow, TextStyle, TextUnit, Time, title,
};
use eframe::egui::{self, Ui, WidgetInfo, WidgetType, ecolor::Hsva, widgets::color_picker};

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

fn name_of<T: PartialEq + Copy>(options: &[(T, &'static str)], value: T) -> &'static str {
    (options.iter())
        .find(|(option, _)| *option == value)
        .map_or("", |(_, name)| name)
}

/// A menu of named values in a row.
fn choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    value: &mut T,
    options: &[(T, &'static str)],
    salt: &str,
) {
    fitted_row(ui, p, label, |ui| {
        egui::ComboBox::from_id_salt(salt)
            .selected_text(name_of(options, *value))
            .width(128.0)
            .height(options.len() as f32 * 40.0)
            .show_ui(ui, |ui| {
                for (option, name) in options {
                    ui.selectable_value(value, *option, *name);
                }
            })
            .response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, label));
    });
}

/// A length the project keeps in ems, shown in canvas pixels at `size`.
fn em_field(ui: &mut Ui, value: &mut f32, size: f32, range: [f32; 2], name: &str) {
    let mut shown = *value * size;
    let field = widgets::named_number(
        ui,
        &mut shown,
        range[0] * size..=range[1] * size,
        0.25,
        " px",
        1,
        super::clip::FIELD,
        name,
    );
    if field.changed() {
        *value = (shown / size.max(1.0)).clamp(range[0], range[1]);
    }
}

fn em_row(ui: &mut Ui, p: &Palette, label: &str, value: &mut f32, size: f32, range: [f32; 2]) {
    row(ui, p, label, |ui| em_field(ui, value, size, range, label));
}

fn color_row(ui: &mut Ui, p: &Palette, label: &str, color: &mut [u8; 4], brand: &[[u8; 4]]) {
    row(ui, p, label, |ui| {
        color_button(ui, p, label, color, brand);
    });
}

/// A colour's button and its picker, with the person's brand colours over
/// the picker. Changed when a colour is picked or dragged to.
fn color_button(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    color: &mut [u8; 4],
    brand: &[[u8; 4]],
) -> egui::Response {
    let id = ui.auto_id_with("colour");
    let popup = id.with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let size = ui.spacing().interact_size;
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ColorButton, true, label));
    let shown = |color: [u8; 4]| {
        let [r, g, b, a] = color;
        egui::Color32::from_rgba_unmultiplied(r, g, b, a)
    };
    if ui.is_rect_visible(rect) {
        let visuals = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&response)
        };
        let rect = rect.expand(visuals.expansion);
        color_picker::show_color_at(ui.painter(), shown(*color), rect.shrink(1.0));
        ui.painter().rect_stroke(
            rect,
            visuals.corner_radius.at_most(2),
            egui::Stroke::new(1.0, visuals.bg_fill),
            egui::StrokeKind::Inside,
        );
    }
    // The hue is kept while the colour is the one picked, so a grey keeps
    // the hue it was dragged from.
    let kept: Option<([u8; 4], Hsva)> = ui.data(|data| data.get_temp(id));
    let mut hsva = match kept {
        Some((was, hsva)) if was == *color => hsva,
        _ => Hsva::from_srgba_unmultiplied(*color),
    };
    let mut changed = false;
    egui::Popup::menu(&response)
        .id(popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = 275.0;
            if !brand.is_empty() {
                ui.label(
                    egui::RichText::new("Brand colours")
                        .font(theme::regular(11.5))
                        .color(p.muted),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for swatch in brand {
                        let (rect, picked) =
                            ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::click());
                        let name = format!("Brand colour {}", crate::ui::text_tab::hex(*swatch));
                        picked.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
                        color_picker::show_color_at(ui.painter(), shown(*swatch), rect.shrink(1.0));
                        let ring = if picked.hovered() || *swatch == *color {
                            egui::Stroke::new(1.5, p.accent)
                        } else {
                            egui::Stroke::new(1.0, p.border)
                        };
                        ui.painter().rect_stroke(
                            rect,
                            egui::CornerRadius::same(4),
                            ring,
                            egui::StrokeKind::Inside,
                        );
                        if picked.clicked() {
                            *color = *swatch;
                            hsva = Hsva::from_srgba_unmultiplied(*swatch);
                            changed = true;
                        }
                    }
                });
                ui.add_space(6.0);
            }
            if color_picker::color_picker_hsva_2d(ui, &mut hsva, color_picker::Alpha::OnlyBlend) {
                *color = hsva.to_srgba_unmultiplied();
                changed = true;
            }
        });
    if changed {
        response.mark_changed();
        ui.data_mut(|data| data.insert_temp(id, (*color, hsva)));
    }
    response
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
            let button = widgets::button(ui, p, None, "Keep…", widgets::Kind::Plain);
            let opened = button.clicked();
            if opened {
                default.clone_into(name);
            }
            egui::Popup::menu(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    widgets::menu_style(ui, 240.0);
                    ui.label(
                        egui::RichText::new("Name")
                            .font(theme::regular(11.5))
                            .color(p.muted),
                    );
                    let field = ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(f32::INFINITY)
                            .font(theme::regular(12.5)),
                    );
                    field.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::TextEdit, true, "Preset name")
                    });
                    if opened {
                        field.request_focus();
                    }
                    let returned =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(6.0);
                    let keep = widgets::button(ui, p, None, "Keep", widgets::Kind::Primary);
                    if keep.clicked() || returned {
                        kept = Some(name.clone());
                        ui.close();
                    }
                });
        });
    });
    kept
}

/// A heading of a card whose rows are there only while it is on.
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
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(11.5))
                .color(p.muted),
        )
        .wrap(),
    );
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
    fitted_row(ui, p, "Font", |ui| {
        egui::ComboBox::from_id_salt(("family", salt))
            .selected_text(font.family.as_str())
            .width(150.0)
            .height(320.0)
            .show_ui(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(filter)
                        .hint_text("Search fonts")
                        .desired_width(f32::INFINITY),
                );
                let Some(all) = family_list(families, project) else {
                    ui.label("Looking for fonts…");
                    return;
                };
                let wanted = filter.trim().to_lowercase();
                let mut shown = 0;
                for name in all
                    .iter()
                    .filter(|name| name.to_lowercase().contains(&wanted))
                {
                    if shown == LISTED {
                        ui.label("Search to find more");
                        break;
                    }
                    shown += 1;
                    if ui.selectable_label(font.family == *name, name).clicked() {
                        font.family.clone_from(name);
                    }
                }
                if shown == 0 {
                    ui.label("No font has that name");
                }
            })
            .response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Font"));
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
    fitted_row(ui, p, "Weight", |ui| {
        let current = WEIGHTS
            .iter()
            .min_by_key(|(weight, _)| weight.abs_diff(font.weight))
            .map_or("", |(_, name)| name);
        egui::ComboBox::from_id_salt(("weight", salt))
            .selected_text(current)
            .width(128.0)
            .height(WEIGHTS.len() as f32 * 40.0)
            .show_ui(ui, |ui| {
                for (weight, name) in WEIGHTS {
                    ui.selectable_value(&mut font.weight, weight, name);
                }
            })
            .response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Weight"));
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
        let width = ui.available_width();
        widgets::segmented(ui, p, &mut style.align, &ALIGNS, width);
        row(ui, p, "Line height", |ui| {
            widgets::named_number(
                ui,
                &mut style.line_height,
                0.5..=4.0,
                0.01,
                "×",
                2,
                super::clip::FIELD,
                "Line height",
            );
        });
        em_row(ui, p, "Spacing", &mut style.tracking, size, [-0.5, 2.0]);
        row(ui, p, "Wrap at", |ui| {
            ui.add_sized(
                [super::clip::FIELD, 24.0],
                egui::DragValue::new(&mut style.wrap)
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
                    }),
            )
            .widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, true, "Wrap at"));
        });
        choice(
            ui,
            p,
            "Case",
            &mut style.case,
            &CASES,
            &format!("case-{salt}"),
        );
    });

    section(ui, p, "Outline");
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

    section(ui, p, "Shadow");
    card(ui, p, |ui| {
        switched(ui, p, "Shadow", &mut style.shadow, || TextShadow {
            color: [0, 0, 0, 160],
            offset: [0.04, 0.06],
            blur: 0.12,
        });
        if let Some(shadow) = &mut style.shadow {
            color_row(ui, p, "Colour", &mut shadow.color, brand);
            row(ui, p, "Offset", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                em_field(
                    ui,
                    &mut shadow.offset[1],
                    size,
                    [-2.0, 2.0],
                    "Shadow offset down",
                );
                em_field(
                    ui,
                    &mut shadow.offset[0],
                    size,
                    [-2.0, 2.0],
                    "Shadow offset right",
                );
            });
            em_row(ui, p, "Blur", &mut shadow.blur, size, [0.0, 1.0]);
        }
    });

    section(ui, p, "Background");
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
            let width = ui.available_width();
            widgets::segmented(ui, p, &mut background.shape, &SHAPES, width);
            color_row(ui, p, "Colour", &mut background.color, brand);
            row(ui, p, "Padding", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                em_field(
                    ui,
                    &mut background.padding[1],
                    size,
                    [0.0, 4.0],
                    "Padding above and below",
                );
                em_field(
                    ui,
                    &mut background.padding[0],
                    size,
                    [0.0, 4.0],
                    "Padding at the sides",
                );
            });
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
        fitted_row(ui, p, "Style", |ui| {
            let mut kind = animation.map(|animation| animation.kind);
            ui.add_enabled_ui(fits || animation.is_some(), |ui| {
                egui::ComboBox::from_id_salt(("animation", salt))
                    .selected_text(kind.map_or(none, kind_name))
                    .width(128.0)
                    .height(10.0 * 40.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut kind, None, none);
                        for option in TextAnimationKind::ALL {
                            ui.selectable_value(&mut kind, Some(option), kind_name(option));
                        }
                    })
                    .response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, label));
            });
            if kind != animation.map(|animation| animation.kind) {
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
            let field = widgets::named_number(
                ui,
                &mut shown,
                min.as_seconds() as f32..=max.as_seconds().max(min.as_seconds()) as f32,
                0.01,
                " s",
                2,
                super::clip::FIELD,
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
        let width = ui.available_width();
        widgets::segmented(ui, p, &mut highlight.mode, &HIGHLIGHTS, width);
        color_row(ui, p, "Colour", &mut highlight.color, brand);
        row(ui, p, "Word size", |ui| {
            let mut percent = highlight.scale * 100.0;
            let field = widgets::named_number(
                ui,
                &mut percent,
                100.0..=200.0,
                0.5,
                " %",
                0,
                super::clip::FIELD,
                "Word size",
            );
            if field.changed() {
                highlight.scale = (percent / 100.0).clamp(1.0, 2.0);
            }
        });
        row(ui, p, "Words to come", |ui| {
            widgets::slider(ui, p, &mut highlight.upcoming, 1.0, 120.0, "Words to come");
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
            ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE)
                    .font(theme::regular(13.0))
                    .hint_text("Text"),
            );
            if let Some(language) = &language {
                widgets::hairline(ui, p);
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
                widgets::named_number(
                    ui,
                    font_size,
                    1.0..=4096.0,
                    1.0,
                    " px",
                    0,
                    super::clip::FIELD,
                    "Size",
                )
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
                for (index, element) in elements.iter_mut().enumerate() {
                    let ElementKind::Text { text, .. } = &mut element.kind else {
                        continue;
                    };
                    if index > 0 {
                        ui.add_space(2.0);
                    }
                    ui.label(
                        egui::RichText::new(&element.name)
                            .font(theme::regular(11.5))
                            .color(p.muted),
                    );
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY)
                            .frame(egui::Frame::NONE)
                            .font(theme::regular(13.0))
                            .hint_text(element.name.as_str()),
                    );
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
                    let field = widgets::named_number(
                        ui,
                        &mut shown,
                        0.0..=1.0,
                        0.005,
                        " s",
                        2,
                        super::clip::FIELD,
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
