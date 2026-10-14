//! The library: what the project has imported, as posters or rows, and what
//! a clip can be given (effects, transitions and looks), as tiles.
//!
//! Only rows in view are laid out and only their posters are asked for. An
//! item is dragged to the timeline or added with a double click. The tiles
//! are [`super::browser`]'s.
use super::{
    Action, Selection,
    browser::{self, BrowserInput, BrowserState},
    captions_tab::{self, CaptionsTabInput, CaptionsTabState},
    icon_of_media, shortcut,
    text_tab::{self, TextTabInput, TextTabState},
    timeline::AssetDrag,
    tone_of_media,
    widgets::{self, Kind, icon_button, menu_item, menu_style},
};
use crate::{
    icons::{self, Icon},
    presets::Presets,
    textures::Textures,
    theme::{self, Palette, metrics},
    timefmt,
    titles::Titles,
};
use editor_core::{
    Asset, Command, FolderId, MediaFolder, MediaKind, Project, Time, captions::SubtitleFormat,
};
use editor_media::{MediaService, ProxyStatus};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    epaint::RectShape, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, sync::Arc};

const TILE_MIN: f32 = 112.0;
const TILE_GAP: f32 = 10.0;
const NAME_HEIGHT: f32 = 34.0;
const ROW_HEIGHT: f32 = 44.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Name,
    Duration,
    Kind,
    Size,
}

impl SortBy {
    const ALL: [(Self, &str); 4] = [
        (Self::Name, "Name"),
        (Self::Duration, "Duration"),
        (Self::Kind, "Kind"),
        (Self::Size, "File size"),
    ];
}

/// The order the library lists media in. Folders stay first, by name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibrarySort {
    pub by: SortBy,
    pub descending: bool,
}

fn kind_rank(kind: MediaKind) -> u8 {
    match kind {
        MediaKind::Video => 0,
        MediaKind::Audio => 1,
        MediaKind::Image => 2,
        MediaKind::Gif => 3,
        MediaKind::Lut => 4,
        MediaKind::Font => 5,
    }
}

/// Orders `assets`. Equal values fall back to the name, and equal names to
/// the identifier, so the order never depends on where an asset was stored.
pub fn sort_assets(assets: &mut Vec<&Asset>, sort: LibrarySort) {
    let mut keyed: Vec<(String, &Asset)> = assets
        .drain(..)
        .map(|asset| (asset.name.to_lowercase(), asset))
        .collect();
    let directed = |order: Ordering| {
        if sort.descending {
            order.reverse()
        } else {
            order
        }
    };
    keyed.sort_by(|(a_name, a), (b_name, b)| {
        let by_name = || a_name.cmp(b_name).then_with(|| a.id.cmp(&b.id));
        match sort.by {
            SortBy::Name => directed(by_name()),
            SortBy::Duration => directed(a.duration.cmp(&b.duration)).then_with(by_name),
            SortBy::Kind => directed(kind_rank(a.kind).cmp(&kind_rank(b.kind))).then_with(by_name),
            SortBy::Size => directed(a.file_size.cmp(&b.file_size)).then_with(by_name),
        }
    });
    assets.extend(keyed.into_iter().map(|(_, asset)| asset));
}

/// The part of the library in view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Media,
    Text,
    Captions,
    Effects,
    Transitions,
    Looks,
}

impl Tab {
    pub const ALL: [Self; 6] = [
        Self::Media,
        Self::Text,
        Self::Captions,
        Self::Effects,
        Self::Transitions,
        Self::Looks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Text => "Text",
            Self::Captions => "Captions",
            Self::Effects => "Effects",
            Self::Transitions => "Transitions",
            Self::Looks => "Looks",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Media => Icon::Film,
            Self::Text => Icon::Text,
            Self::Captions => Icon::Captions,
            Self::Effects => Icon::Sparkles,
            Self::Transitions => Icon::Transition,
            Self::Looks => Icon::Image,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Media => "Search media",
            Self::Text => "Search titles, stickers and emoji",
            Self::Captions => "Search captions",
            Self::Effects => "Search effects",
            Self::Transitions => "Search transitions",
            Self::Looks => "Search looks",
        }
    }
}

/// The library's parts on one track, each as wide as its name needs and a
/// share of what is left over. When the names do not fit even small, each
/// part is its icon, named on hover.
fn tabs(ui: &mut Ui, p: &Palette, rect: Rect, tab: &mut Tab) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(metrics::CONTROL_RADIUS), p.control);
    let inner = rect.shrink(2.0);
    let names = |font: egui::FontId| {
        Tab::ALL.map(|tab| {
            ui.painter()
                .layout_no_wrap(tab.label().to_owned(), font.clone(), p.fg)
        })
    };
    let needed = |galleys: &[std::sync::Arc<egui::Galley>]| {
        galleys.iter().map(|galley| galley.size().x).sum::<f32>()
    };
    let mut galleys = names(theme::medium(12.0));
    if needed(&galleys) + 8.0 * Tab::ALL.len() as f32 > inner.width() {
        galleys = names(theme::medium(11.0));
    }
    // Names that would touch give way to icons.
    let iconic = needed(&galleys) + 6.0 * Tab::ALL.len() as f32 > inner.width();
    let widths = (galleys.each_ref()).map(|galley| if iconic { 0.0 } else { galley.size().x });
    let spare = ((inner.width() - widths.iter().sum::<f32>()) / Tab::ALL.len() as f32).max(0.0);
    let mut left = inner.left();
    for ((option, galley), width) in Tab::ALL.into_iter().zip(galleys).zip(widths) {
        let cell =
            Rect::from_min_size(pos2(left, inner.top()), vec2(width + spare, inner.height()))
                .intersect(inner);
        left = cell.right();
        let response = ui.interact(cell, Id::new(("library-tab", option)), Sense::click());
        let selected = *tab == option;
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                true,
                selected,
                option.label(),
            )
        });
        if selected {
            ui.painter().rect_filled(
                cell,
                CornerRadius::same(metrics::CONTROL_RADIUS - 2),
                if p.dark { p.pressed } else { p.elevated },
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                cell,
                CornerRadius::same(metrics::CONTROL_RADIUS - 2),
                Stroke::new(1.5, p.accent),
                StrokeKind::Inside,
            );
        }
        let ink = if selected || response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        if iconic {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(cell.center(), Vec2::splat(14.0)),
                option.icon(),
                ink,
            );
        } else {
            ui.painter().galley_with_override_text_color(
                cell.center() - galley.size() * 0.5,
                galley,
                ink,
            );
        }
        let response = if iconic {
            response.on_hover_text(option.label())
        } else {
            response
        };
        if response.clicked() {
            *tab = option;
        }
    }
}

#[derive(Default)]
pub struct LibraryState {
    /// Which part is in view: the media, or what a clip can be given.
    pub tab: Tab,
    pub browser: BrowserState,
    pub search: String,
    /// The folder in view; `None` is the top of the library.
    pub folder: Option<FolderId>,
    /// The name being typed for a folder that is about to be made.
    pub new_folder: Option<String>,
    focus_new_folder: bool,
    pub text: TextTabState,
    pub captions: CaptionsTabState,
}

impl LibraryState {
    fn begin_folder(&mut self) {
        self.new_folder = Some(String::new());
        self.focus_new_folder = true;
    }
}

fn move_to(asset: &Asset, folder: Option<FolderId>) -> Action {
    Action::Edit(Command::MoveAssetToFolder {
        asset_id: asset.id,
        folder_id: folder,
    })
}

/// The asset being carried, when it is let go over `response`. The target is
/// tinted while it is held there.
fn dropped_asset(ui: &Ui, response: &egui::Response, p: &Palette) -> Option<editor_core::AssetId> {
    if response.dnd_hover_payload::<AssetDrag>().is_some() {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    response
        .dnd_release_payload::<AssetDrag>()
        .map(|carried| carried.0)
}

pub struct LibraryInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub media: Option<&'a MediaService>,
    pub grid: bool,
    pub sort: LibrarySort,
    /// Import batches the media worker has not reported yet.
    pub importing: usize,
    /// Why media cannot be imported or decoded, when it cannot.
    pub problem: Option<&'a str>,
    /// The asset the source monitor is showing.
    pub previewing: Option<editor_core::AssetId>,
    /// Where a transition given from here goes: the nearer edge of a clip.
    pub playhead: Time,
    /// The person's kept looks and brand colours.
    pub presets: &'a Arc<Presets>,
    /// Captions from speech: their choices and the worker's progress.
    pub speech: captions_tab::SpeechInput<'a>,
}

enum Item<'a> {
    Folder(&'a MediaFolder),
    Asset(&'a Asset),
}

/// What a video's tile says of its proxy.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProxyMark {
    /// The preview can read a smaller copy.
    Ready,
    /// One is made, but Preview › Proxies is Off: the preview reads the
    /// original.
    Unused,
    /// One waits to be made, or is being made: how much of it, when known.
    Making(Option<f32>),
    /// None could be made, or the one made could not be read, and why.
    Failed(String),
}

impl ProxyMark {
    /// Its name for accessibility and its tooltip.
    pub(crate) fn describe(&self) -> (&'static str, String) {
        match self {
            Self::Ready => (
                "Proxy ready",
                "Proxy ready: the preview reads a smaller copy. Exports use the original."
                    .to_owned(),
            ),
            Self::Unused => (
                "Proxy not used",
                "Proxy made; Preview › Proxies is Off, so the original is shown".to_owned(),
            ),
            Self::Making(None) => ("Proxy waiting", "Waiting to make a proxy".to_owned()),
            Self::Making(Some(done)) => (
                "Making proxy",
                format!("Making a proxy: {:.0}%", done.clamp(0.0, 1.0) * 100.0),
            ),
            Self::Failed(why) => ("Proxy failed", format!("No proxy: {why}")),
        }
    }
}

/// What the tile of `asset` shows of its proxy, given what the proxy
/// service says of it, whether the preview could not read the one made and
/// whether the preview reads proxies at all (`read`, Proxies is not Off).
pub(crate) fn proxy_mark(
    asset: &Asset,
    status: &ProxyStatus,
    unreadable: bool,
    read: bool,
) -> Option<ProxyMark> {
    if asset.kind != MediaKind::Video {
        return None;
    }
    match status {
        ProxyStatus::None => None,
        ProxyStatus::Queued => Some(ProxyMark::Making(None)),
        ProxyStatus::Generating(done) => Some(ProxyMark::Making(Some(*done))),
        ProxyStatus::Ready(_) if !read => Some(ProxyMark::Unused),
        ProxyStatus::Ready(_) if unreadable => Some(ProxyMark::Failed(
            "the proxy could not be read, so the original is shown".to_owned(),
        )),
        ProxyStatus::Ready(_) => Some(ProxyMark::Ready),
        ProxyStatus::Failed(why) => Some(ProxyMark::Failed(why.clone())),
    }
}

/// The proxy item of a video's menu, and what it asks: to make one when it
/// has none, or none could be made, and to remove the one it has or is
/// making. Only a video with a picture has one. While the preview reads no
/// proxies (`read` is false) the item says when the one made is used.
pub(crate) fn proxy_item(
    asset: &Asset,
    status: &ProxyStatus,
    read: bool,
) -> Option<(&'static str, Action)> {
    if asset.kind != MediaKind::Video || asset.width == 0 || asset.height == 0 {
        return None;
    }
    Some(match status {
        ProxyStatus::None | ProxyStatus::Failed(_) if !read => (
            "Create proxy (used when Proxies is on)",
            Action::CreateProxy(asset.id),
        ),
        ProxyStatus::None | ProxyStatus::Failed(_) => {
            ("Create proxy", Action::CreateProxy(asset.id))
        }
        ProxyStatus::Queued | ProxyStatus::Generating(_) | ProxyStatus::Ready(_) => {
            ("Remove proxy", Action::RemoveProxy(asset.id))
        }
    })
}

/// Draws the proxy mark of a tile in the top right corner of its picture,
/// with its name and tooltip, and a thin bar along the picture's foot while
/// a proxy is made.
fn proxy_badge(ui: &Ui, p: &Palette, id: Id, picture: Rect, grid: bool, mark: &ProxyMark) {
    let side = if grid { 18.0 } else { 14.0 };
    let badge = Rect::from_min_size(
        pos2(picture.right() - side - 5.0, picture.top() + 5.0),
        Vec2::splat(side),
    );
    let (name, tip) = mark.describe();
    let response = ui.interact(badge, id.with("proxy"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name));
    response.on_hover_text(tip);
    let painter = ui.painter();
    let (icon, color, fill) = match mark {
        ProxyMark::Ready => (Icon::Layers, Color32::WHITE, Color32::from_black_alpha(150)),
        // Dimmed: made, but not what the preview reads.
        ProxyMark::Unused => (
            Icon::Layers,
            Color32::from_white_alpha(110),
            Color32::from_black_alpha(90),
        ),
        ProxyMark::Making(_) => (
            Icon::Layers,
            Color32::from_white_alpha(150),
            Color32::from_black_alpha(110),
        ),
        ProxyMark::Failed(_) => (Icon::Warning, p.yellow, Color32::from_black_alpha(150)),
    };
    painter.rect_filled(badge, CornerRadius::same(5), fill);
    icons::paint(painter, badge.shrink(side * 0.2), icon, color);
    if let ProxyMark::Making(Some(done)) = mark {
        let track = Rect::from_min_max(
            pos2(picture.left() + 4.0, picture.bottom() - 5.0),
            pos2(picture.right() - 4.0, picture.bottom() - 3.0),
        );
        painter.rect_filled(track, CornerRadius::same(1), Color32::from_black_alpha(120));
        let fill = Rect::from_min_max(
            track.min,
            pos2(
                track.left() + track.width() * done.clamp(0.0, 1.0),
                track.bottom(),
            ),
        );
        painter.rect_filled(fill, CornerRadius::same(1), p.accent);
    }
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "Video",
        MediaKind::Audio => "Audio",
        MediaKind::Image => "Image",
        MediaKind::Gif => "GIF",
        MediaKind::Lut => "LUT",
        MediaKind::Font => "Font",
    }
}

#[expect(clippy::too_many_arguments, reason = "the application's one call")]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut LibraryState,
    selection: &mut Selection,
    textures: &mut Textures,
    titles: &mut Titles,
    input: &LibraryInput,
    actions: &mut Vec<Action>,
) -> (bool, LibrarySort) {
    let p = input.p;
    let project = input.project;
    let mut grid = input.grid;
    let mut sort = input.sort;
    widgets::panel(ui, rect, p);
    // A folder that was undone away is not a place to be.
    if state
        .folder
        .is_some_and(|id| !project.folders.contains_key(&id))
    {
        state.folder = None;
    }

    let header = Rect::from_min_size(rect.min, vec2(rect.width(), metrics::PANEL_HEADER));
    let mut head = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    head.add_space(4.0);
    head.label(
        egui::RichText::new("Library")
            .font(theme::medium(13.0))
            .color(p.fg),
    );
    if input.importing > 0 {
        head.add_space(4.0);
        head.add(egui::Spinner::new().size(12.0).color(p.muted));
        head.label(
            egui::RichText::new("Importing…")
                .font(theme::regular(11.5))
                .color(p.muted),
        );
    }
    let media = state.tab == Tab::Media;
    let text = state.tab == Tab::Text;
    let captions = state.tab == Tab::Captions;
    let caption_track = captions_tab::in_view(project, state.captions.track, selection);
    // What imports, sorts and lays out media is the media's.
    head.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if text {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, p, Icon::Import, "Import fonts").clicked() {
                actions.push(Action::ImportFonts);
            }
            return;
        }
        if captions {
            ui.spacing_mut().item_spacing.x = 2.0;
            if let Some(track) = caption_track {
                let export = icon_button(ui, p, Icon::Export, "Export captions");
                egui::Popup::menu(&export).show(|ui| {
                    menu_style(ui, 200.0);
                    for (format, name) in [
                        (SubtitleFormat::Srt, "SubRip (.srt)…"),
                        (SubtitleFormat::Vtt, "WebVTT (.vtt)…"),
                    ] {
                        if menu_item(ui, p, None, name, "").clicked() {
                            actions.push(Action::ExportCaptions { track, format });
                            ui.close();
                        }
                    }
                });
            }
            if icon_button(ui, p, Icon::Import, "Import SRT or VTT").clicked() {
                actions.push(Action::ImportCaptions);
            }
            let open = state.captions.speech_open;
            let speech = widgets::icon_button_tinted(
                ui,
                p,
                Icon::Speech,
                "Captions from speech",
                open.then_some(p.accent),
            );
            if speech.clicked() {
                state.captions.speech_open = !open;
                if !open {
                    actions.push(Action::LookAtModels);
                }
            }
            return;
        }
        if !media {
            return;
        }
        ui.spacing_mut().item_spacing.x = 2.0;
        let import = icon_button(ui, p, Icon::Import, "Import media or add a folder");
        egui::Popup::menu(&import).show(|ui| {
            menu_style(ui, 220.0);
            if menu_item(ui, p, Some(Icon::File), "Import files…", &shortcut("Cmd+I")).clicked() {
                actions.push(Action::ImportFiles);
                ui.close();
            }
            if menu_item(
                ui,
                p,
                Some(Icon::Folder),
                "Import folder…",
                &shortcut("Shift+Cmd+I"),
            )
            .clicked()
            {
                actions.push(Action::ImportFolder);
                ui.close();
            }
            ui.separator();
            if menu_item(ui, p, Some(Icon::FolderPlus), "New folder", "").clicked() {
                state.begin_folder();
                ui.close();
            }
        });
        let (icon, label) = if grid {
            (Icon::List, "Show as list")
        } else {
            (Icon::Grid, "Show as grid")
        };
        if icon_button(ui, p, icon, label).clicked() {
            grid = !grid;
        }
        let order = icon_button(ui, p, Icon::Sort, "Sort media");
        egui::Popup::menu(&order).show(|ui| {
            menu_style(ui, 180.0);
            for (by, text) in SortBy::ALL {
                let chosen = (sort.by == by).then_some(Icon::Check);
                if menu_item(ui, p, chosen, text, "").clicked() {
                    sort.by = by;
                    ui.close();
                }
            }
            ui.separator();
            for (descending, text) in [(false, "Ascending"), (true, "Descending")] {
                let chosen = (sort.descending == descending).then_some(Icon::Check);
                if menu_item(ui, p, chosen, text, "").clicked() {
                    sort.descending = descending;
                    ui.close();
                }
            }
        });
    });

    let switch = Rect::from_min_size(
        pos2(rect.left() + 10.0, header.bottom()),
        vec2(rect.width() - 20.0, metrics::CONTROL_HEIGHT),
    );
    tabs(ui, p, switch, &mut state.tab);
    let media = state.tab == Tab::Media;

    // Search.
    let search = Rect::from_min_size(
        pos2(rect.left() + 10.0, switch.bottom() + 6.0),
        vec2(rect.width() - 20.0, metrics::CONTROL_HEIGHT),
    );
    ui.painter().rect_filled(
        search,
        CornerRadius::same(metrics::CONTROL_RADIUS),
        p.control,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(search.left() + 14.0, search.center().y),
            Vec2::splat(13.0),
        ),
        Icon::Search,
        p.muted,
    );
    let mut field = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
        pos2(search.left() + 26.0, search.top() + 4.0),
        pos2(search.right() - 6.0, search.bottom() - 4.0),
    )));
    let sought = if media {
        &mut state.search
    } else {
        &mut state.browser.search
    };
    field.add(
        egui::TextEdit::singleline(sought)
            .hint_text(state.tab.hint())
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .font(theme::regular(12.5)),
    );

    let mut top = search.bottom() + 8.0;
    if text {
        text_tab::show(
            ui,
            Rect::from_min_max(pos2(rect.left(), top), rect.max),
            &state.browser.search,
            &mut state.text,
            selection,
            titles,
            &TextTabInput {
                p,
                project,
                presets: input.presets,
            },
            actions,
        );
        return (grid, sort);
    }
    if captions {
        captions_tab::show(
            ui,
            Rect::from_min_max(pos2(rect.left(), top), rect.max),
            &state.browser.search,
            &mut state.captions,
            selection,
            &CaptionsTabInput {
                p,
                project,
                playhead: input.playhead,
                presets: input.presets,
                speech: input.speech,
            },
            actions,
        );
        return (grid, sort);
    }
    if !media {
        browser::show(
            ui,
            Rect::from_min_max(pos2(rect.left(), top), rect.max),
            &mut state.browser,
            selection,
            &BrowserInput {
                p,
                project,
                playhead: input.playhead,
                tab: state.tab,
            },
            actions,
        );
        return (grid, sort);
    }
    // Where we are, when inside a folder.
    if let Some(folder) = state.folder.and_then(|id| project.folders.get(&id)) {
        let crumb = Rect::from_min_size(
            pos2(rect.left() + 10.0, top),
            vec2(rect.width() - 20.0, 24.0),
        );
        let mut crumbs = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(crumb)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        crumbs.spacing_mut().item_spacing.x = 4.0;
        let back = crumbs.add(
            egui::Label::new(
                egui::RichText::new("All media")
                    .font(theme::regular(12.0))
                    .color(p.secondary),
            )
            .sense(Sense::click()),
        );
        // Media let go on the way back leaves the folder.
        if let Some(asset_id) = dropped_asset(ui, &back, p) {
            actions.push(Action::Edit(Command::MoveAssetToFolder {
                asset_id,
                folder_id: None,
            }));
        }
        if back.has_focus() {
            focus_ring(ui, p, back.rect.expand2(vec2(4.0, 2.0)));
        }
        if back
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            state.folder = None;
        }
        let (chevron, _) = crumbs.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        icons::paint(crumbs.painter(), chevron, Icon::ChevronRight, p.muted);
        crumbs.add(
            egui::Label::new(
                egui::RichText::new(&folder.name)
                    .font(theme::medium(12.0))
                    .color(p.fg),
            )
            .truncate(),
        );
        top = crumb.bottom() + 6.0;
    }
    // The name of a folder that is being made, here in the folder in view.
    if let Some(name) = &mut state.new_folder {
        let row = Rect::from_min_size(
            pos2(rect.left() + 10.0, top),
            vec2(rect.width() - 20.0, metrics::CONTROL_HEIGHT),
        );
        ui.painter()
            .rect_filled(row, CornerRadius::same(metrics::CONTROL_RADIUS), p.control);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(row.left() + 14.0, row.center().y), Vec2::splat(13.0)),
            Icon::FolderPlus,
            p.muted,
        );
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
            pos2(row.left() + 26.0, row.top() + 4.0),
            pos2(row.right() - 6.0, row.bottom() - 4.0),
        )));
        let response = field.add(
            egui::TextEdit::singleline(name)
                .hint_text("Folder name")
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .font(theme::regular(12.5)),
        );
        if std::mem::take(&mut state.focus_new_folder) {
            response.request_focus();
        }
        // Return makes the folder; leaving the field any other way does not.
        if response.lost_focus() {
            let name = name.trim().to_owned();
            if !name.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(Action::Edit(Command::AddFolder {
                    folder: MediaFolder {
                        id: FolderId::new_v4(),
                        name,
                        parent_id: state.folder,
                    },
                }));
            }
            state.new_folder = None;
        }
        top = row.bottom() + 8.0;
    }
    let list = Rect::from_min_max(pos2(rect.left(), top), rect.max);

    // What this view lists.
    let query = state.search.trim().to_lowercase();
    let mut items: Vec<Item> = Vec::new();
    if query.is_empty() {
        let mut folders: Vec<&MediaFolder> = project
            .folders
            .values()
            .filter(|folder| folder.parent_id == state.folder)
            .collect();
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        items.extend(folders.into_iter().map(Item::Folder));
    }
    let mut assets: Vec<&Asset> = project
        .assets
        .values()
        .filter(|asset| {
            if query.is_empty() {
                asset.folder_id == state.folder
            } else {
                asset.name.to_lowercase().contains(&query)
            }
        })
        .collect();
    sort_assets(&mut assets, sort);
    items.extend(assets.into_iter().map(Item::Asset));

    if items.is_empty() {
        if let Some(problem) = input.problem.filter(|_| project.assets.is_empty()) {
            widgets::empty_state(ui, p, list, Icon::Warning, "Media is unavailable", problem);
        } else if !query.is_empty() {
            widgets::empty_state(
                ui,
                p,
                list,
                Icon::Search,
                "No matches",
                "Nothing in this project has that name.",
            );
        } else if project.assets.is_empty() {
            widgets::empty_state(
                ui,
                p,
                list,
                Icon::Import,
                "No media yet",
                "Drop files or folders here, or import them.",
            );
            let button = Rect::from_center_size(list.center() + vec2(0.0, 62.0), vec2(120.0, 28.0));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(button)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if widgets::button(&mut child, p, Some(Icon::Import), "Import…", Kind::Primary)
                .clicked()
            {
                actions.push(Action::ImportFiles);
            }
        } else {
            widgets::empty_state(ui, p, list, Icon::Folder, "Empty folder", "");
        }
        return (grid, sort);
    }

    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink2(vec2(10.0, 0.0))));
    body.set_clip_rect(list);
    let width = body.available_width();
    let (columns, tile_width, row_height) = if grid {
        let columns = ((width + TILE_GAP) / (TILE_MIN + TILE_GAP))
            .floor()
            .max(1.0) as usize;
        let tile = (width - TILE_GAP * (columns as f32 - 1.0)) / columns as f32;
        (columns, tile, tile * 9.0 / 16.0 + NAME_HEIGHT + TILE_GAP)
    } else {
        (1, width, ROW_HEIGHT)
    };
    let rows = items.len().div_ceil(columns);
    body.spacing_mut().item_spacing = Vec2::ZERO;
    egui::ScrollArea::vertical()
        .id_salt("library")
        .auto_shrink([false, false])
        .show_rows(&mut body, row_height, rows, |ui, range| {
            for row in range {
                let (row_rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                for column in 0..columns {
                    let Some(item) = items.get(row * columns + column) else {
                        break;
                    };
                    let cell = Rect::from_min_size(
                        pos2(
                            row_rect.left() + column as f32 * (tile_width + TILE_GAP),
                            row_rect.top(),
                        ),
                        vec2(tile_width, row_height - if grid { TILE_GAP } else { 2.0 }),
                    );
                    match item {
                        Item::Folder(folder) => {
                            if folder_cell(ui, cell, folder, project, grid, p, actions) {
                                state.folder = Some(folder.id);
                                state.search.clear();
                            }
                        }
                        Item::Asset(asset) => {
                            if let Some(media) = input.media {
                                textures.want_poster(media, asset);
                            }
                            asset_cell(ui, cell, asset, selection, textures, grid, input, actions);
                        }
                    }
                }
            }
        });
    (grid, sort)
}

fn folder_cell(
    ui: &mut Ui,
    rect: Rect,
    folder: &MediaFolder,
    project: &Project,
    grid: bool,
    p: &Palette,
    actions: &mut Vec<Action>,
) -> bool {
    let response = ui.interact(rect, Id::new(("library-folder", folder.id)), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Folder {}", folder.name),
        )
    });
    // Media let go on a folder moves into it.
    if let Some(asset_id) = dropped_asset(ui, &response, p) {
        actions.push(Action::Edit(Command::MoveAssetToFolder {
            asset_id,
            folder_id: Some(folder.id),
        }));
    }
    let count = project
        .assets
        .values()
        .filter(|asset| asset.folder_id == Some(folder.id))
        .count();
    let detail = match count {
        1 => "1 item".to_owned(),
        n => format!("{n} items"),
    };
    let painter = ui.painter();
    if grid {
        let picture = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0));
        painter.rect_filled(
            picture,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            if response.hovered() {
                p.pressed
            } else {
                p.control
            },
        );
        icons::paint(
            painter,
            Rect::from_center_size(picture.center(), Vec2::splat(26.0)),
            Icon::Folder,
            p.secondary,
        );
        let name = widgets::truncated(ui, &folder.name, theme::medium(12.0), p.fg, rect.width());
        ui.painter()
            .galley(pos2(rect.left() + 1.0, picture.bottom() + 5.0), name, p.fg);
        ui.painter().text(
            pos2(rect.left() + 1.0, picture.bottom() + 21.0),
            Align2::LEFT_TOP,
            detail,
            theme::regular(11.0),
            p.muted,
        );
    } else {
        if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(metrics::CONTROL_RADIUS), p.hover);
        }
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(18.0)),
            Icon::Folder,
            p.secondary,
        );
        let name = widgets::truncated(
            ui,
            &folder.name,
            theme::medium(12.5),
            p.fg,
            rect.width() - 130.0,
        );
        ui.painter().galley(
            pos2(rect.left() + 60.0, rect.center().y - name.size().y * 0.5),
            name,
            p.fg,
        );
        ui.painter().text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            detail,
            theme::regular(11.5),
            p.muted,
        );
    }
    if response.has_focus() {
        let shape = if grid {
            Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0))
        } else {
            rect
        };
        focus_ring(ui, p, shape);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The ring of what has the keyboard, as an effect's tile has it.
fn focus_ring(ui: &Ui, p: &Palette, rect: Rect) {
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(metrics::CONTROL_RADIUS),
        Stroke::new(1.5, p.accent),
        StrokeKind::Inside,
    );
}

#[expect(clippy::too_many_arguments, reason = "one call site per cell")]
fn asset_cell(
    ui: &mut Ui,
    rect: Rect,
    asset: &Asset,
    selection: &mut Selection,
    textures: &Textures,
    grid: bool,
    input: &LibraryInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let id = Id::new(("library-asset", asset.id));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let selected = selection.asset == Some(asset.id);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &asset.name)
    });
    let tone = p.tone(tone_of_media(asset.kind));
    let picture = if grid {
        Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0))
    } else {
        Rect::from_min_size(
            pos2(rect.left() + 4.0, rect.top() + 4.0),
            vec2((rect.height() - 8.0) * 16.0 / 9.0, rect.height() - 8.0),
        )
    };
    let radius = CornerRadius::same(if grid { metrics::CONTROL_RADIUS } else { 5 });
    let painter = ui.painter();
    if !grid && (selected || response.hovered()) {
        painter.rect_filled(
            rect,
            CornerRadius::same(metrics::CONTROL_RADIUS),
            if selected {
                theme::tint(p.accent, 0.2)
            } else {
                p.hover
            },
        );
    }
    painter.rect_filled(picture, radius, p.sunken);
    match textures.poster(asset.id) {
        Some(texture) => {
            // The poster keeps its shape inside the 16:9 frame.
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y);
            let fitted = Rect::from_center_size(picture.center(), size * scale);
            let full = fitted.width() >= picture.width() - 0.5
                && fitted.height() >= picture.height() - 0.5;
            painter.add(
                RectShape::filled(
                    fitted,
                    if full { radius } else { CornerRadius::ZERO },
                    Color32::WHITE,
                )
                .with_texture(
                    texture.id(),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                ),
            );
        }
        None => {
            icons::paint(
                painter,
                Rect::from_center_size(
                    picture.center(),
                    Vec2::splat(if grid { 24.0 } else { 16.0 }),
                ),
                icon_of_media(asset.kind),
                tone,
            );
        }
    }
    // Pictures get a one-point outline in pure white or black, never a tint.
    painter.rect_stroke(
        picture,
        radius,
        Stroke::new(
            1.0,
            if p.dark {
                Color32::from_white_alpha(26)
            } else {
                Color32::from_black_alpha(26)
            },
        ),
        StrokeKind::Inside,
    );
    if grid && selected {
        painter.rect_stroke(
            picture.expand(2.0),
            CornerRadius::same(metrics::CONTROL_RADIUS + 2),
            Stroke::new(2.0, p.accent),
            StrokeKind::Inside,
        );
    }
    // The keyboard's place is shown apart from the selection: a tile can be
    // walked past without being chosen.
    if response.has_focus() {
        focus_ring(ui, p, if grid { picture } else { rect });
    }

    // The asset in the source monitor says so on its picture.
    if input.previewing == Some(asset.id) {
        let side = if grid { 18.0 } else { 14.0 };
        let mark = Rect::from_min_size(picture.min + vec2(5.0, 5.0), Vec2::splat(side));
        painter.rect_filled(mark, CornerRadius::same(5), p.accent);
        icons::paint(painter, mark.shrink(side * 0.22), Icon::Play, p.on_accent);
    }
    // A video with a proxy, or one on its way, says so on its picture.
    let proxy_status = textures.proxy_status(asset.id);
    let read = textures.reads_proxies();
    if let Some(mark) = proxy_mark(
        asset,
        proxy_status,
        textures.proxy_unreadable(asset.id),
        read,
    ) {
        proxy_badge(ui, p, id, picture, grid, &mark);
    }
    // A lookup table is neither played nor placed on a track.
    let placeable = crate::rules::placeable(asset);
    // Under the pointer, the picture offers to play its source.
    if placeable && ui.rect_contains_pointer(rect) && !response.dragged() {
        let side = if grid { 30.0 } else { 22.0 };
        let badge = Rect::from_center_size(picture.center(), Vec2::splat(side));
        let preview = ui
            .interact(badge, id.with("preview"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Preview (Enter)");
        preview
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Preview"));
        let painter = ui.painter();
        painter.circle_filled(
            badge.center(),
            side * 0.5,
            Color32::from_black_alpha(if preview.hovered() { 200 } else { 150 }),
        );
        icons::paint(
            painter,
            badge.shrink(side * 0.27),
            Icon::Play,
            Color32::WHITE,
        );
        if preview.clicked() {
            actions.push(Action::PreviewAsset(asset.id));
        }
    }
    let painter = ui.painter();

    let length = timefmt::short(asset.duration);
    if grid {
        if asset.kind != MediaKind::Image && placeable {
            let badge = painter.layout_no_wrap(length, theme::medium(10.5), Color32::WHITE);
            let pill = Rect::from_min_size(
                pos2(
                    picture.right() - badge.size().x - 13.0,
                    picture.bottom() - badge.size().y - 9.0,
                ),
                badge.size() + vec2(8.0, 4.0),
            );
            painter.rect_filled(pill, CornerRadius::same(5), Color32::from_black_alpha(150));
            painter.galley(pill.min + vec2(4.0, 2.0), badge, Color32::WHITE);
        }
        let name = widgets::truncated(ui, &asset.name, theme::medium(12.0), p.fg, rect.width());
        ui.painter()
            .galley(pos2(rect.left() + 1.0, picture.bottom() + 5.0), name, p.fg);
        icons::paint(
            ui.painter(),
            Rect::from_min_size(
                pos2(rect.left() + 1.0, picture.bottom() + 22.0),
                Vec2::splat(11.0),
            ),
            icon_of_media(asset.kind),
            tone,
        );
        ui.painter().text(
            pos2(rect.left() + 16.0, picture.bottom() + 21.0),
            Align2::LEFT_TOP,
            kind_name(asset.kind),
            theme::regular(11.0),
            p.muted,
        );
    } else {
        let left = picture.right() + 10.0;
        let name = widgets::truncated(
            ui,
            &asset.name,
            theme::medium(12.5),
            p.fg,
            rect.right() - left - 8.0,
        );
        ui.painter()
            .galley(pos2(left, rect.top() + 6.0), name, p.fg);
        let detail = if asset.kind == MediaKind::Image {
            format!(
                "{} · {}×{}",
                kind_name(asset.kind),
                asset.width,
                asset.height
            )
        } else if asset.kind == MediaKind::Lut {
            format!("{} · {} points", kind_name(asset.kind), asset.width)
        } else {
            format!("{} · {length}", kind_name(asset.kind))
        };
        ui.painter().text(
            pos2(left, rect.top() + 24.0),
            Align2::LEFT_TOP,
            detail,
            theme::regular(11.0),
            p.muted,
        );
    }

    if response.clicked() || response.drag_started() {
        selection.clips.clear();
        selection.asset = Some(asset.id);
    }
    if placeable && response.double_clicked() {
        actions.push(Action::InsertAsset {
            asset: asset.id,
            place: None,
        });
    }
    if placeable && response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), AssetDrag(asset.id));
    }
    // What is being carried follows the pointer.
    if placeable
        && response.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("library-carry"));
        let carry = ui.ctx().layer_painter(layer);
        let label = carry.layout_no_wrap(asset.name.clone(), theme::medium(12.0), p.fg);
        let chip = Rect::from_min_size(
            pos + vec2(12.0, 10.0),
            vec2(label.size().x.min(180.0) + 34.0, 26.0),
        );
        carry.rect_filled(chip, CornerRadius::same(8), p.elevated);
        carry.rect_stroke(
            chip,
            CornerRadius::same(8),
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
        icons::paint(
            &carry,
            Rect::from_center_size(pos2(chip.left() + 14.0, chip.center().y), Vec2::splat(13.0)),
            icon_of_media(asset.kind),
            tone,
        );
        carry.with_clip_rect(chip.shrink2(vec2(6.0, 0.0))).galley(
            pos2(chip.left() + 26.0, chip.center().y - label.size().y * 0.5),
            label,
            p.fg,
        );
    }
    response.context_menu(|ui| {
        menu_style(ui, 220.0);
        if placeable && menu_item(ui, p, Some(Icon::Play), "Preview", "Enter").clicked() {
            actions.push(Action::PreviewAsset(asset.id));
            ui.close();
        }
        if placeable && menu_item(ui, p, Some(Icon::Plus), "Add to timeline", "").clicked() {
            actions.push(Action::InsertAsset {
                asset: asset.id,
                place: None,
            });
            ui.close();
        }
        if menu_item(ui, p, Some(Icon::Copy), "Copy path", "").clicked() {
            ui.ctx().copy_text(asset.path.display().to_string());
            ui.close();
        }
        if let Some((label, action)) = proxy_item(asset, proxy_status, textures.reads_proxies())
            && menu_item(ui, p, Some(Icon::Layers), label, "").clicked()
        {
            actions.push(action);
            ui.close();
        }
        let project = input.project;
        if asset.folder_id.is_some() || !project.folders.is_empty() {
            ui.separator();
            ui.label(
                egui::RichText::new("Move to")
                    .font(theme::medium(11.0))
                    .color(p.muted),
            );
            if asset.folder_id.is_some()
                && menu_item(ui, p, Some(Icon::Layers), "All media", "").clicked()
            {
                actions.push(move_to(asset, None));
                ui.close();
            }
            let mut folders: Vec<&MediaFolder> = project
                .folders
                .values()
                .filter(|folder| Some(folder.id) != asset.folder_id)
                .collect();
            folders.sort_by_key(|folder| folder.name.to_lowercase());
            egui::ScrollArea::vertical()
                .max_height(156.0)
                .show(ui, |ui| {
                    for folder in folders {
                        if menu_item(ui, p, Some(Icon::Folder), &folder.name, "").clicked() {
                            actions.push(move_to(asset, Some(folder.id)));
                            ui.close();
                        }
                    }
                });
        }
        ui.separator();
        if menu_item(ui, p, Some(Icon::Trash), "Remove from project", "").clicked() {
            actions.push(Action::RemoveAsset(asset.id));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::Time;

    fn asset(name: &str, kind: MediaKind, seconds: f64, size: u64) -> Asset {
        let mut asset = Asset::new(
            format!("/media/{name}").into(),
            kind,
            Time::from_seconds(seconds),
        );
        asset.name = name.to_owned();
        asset.file_size = size;
        asset
    }

    fn names(assets: &[Asset], by: SortBy, descending: bool) -> Vec<&str> {
        let mut listed: Vec<&Asset> = assets.iter().collect();
        sort_assets(&mut listed, LibrarySort { by, descending });
        listed
            .into_iter()
            .map(|asset| asset.name.as_str())
            .collect()
    }

    /// Draws the media tab once and hands back the accent strokes of a
    /// focus ring that were painted, as the rects they outline.
    fn rings(ctx: &egui::Context, project: &Project, grid: bool, focus: Option<Id>) -> Vec<Rect> {
        let palette = Palette::new(crate::theme::Appearance::Dark, Default::default());
        let mut state = LibraryState::default();
        let mut selection = Selection::default();
        let mut textures = Textures::default();
        let mut found = Vec::new();
        for _ in 0..2 {
            if let Some(id) = focus {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 600.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let rect = ui.max_rect();
                show(
                    ui,
                    rect,
                    &mut state,
                    &mut selection,
                    &mut textures,
                    &mut Titles::default(),
                    &LibraryInput {
                        p: &palette,
                        project,
                        media: None,
                        grid,
                        sort: LibrarySort::default(),
                        importing: 0,
                        problem: None,
                        previewing: None,
                        playhead: Time::ZERO,
                        presets: &Arc::default(),
                        speech: captions_tab::SpeechInput {
                            view: &crate::speech::SpeechView::Idle,
                            settings: &crate::speech::SpeechSettings::default(),
                            inventory: None,
                        },
                    },
                    &mut Vec::new(),
                );
            });
            // No renderer takes the textures.
            output.textures_delta.clear();
            found = (output.shapes.iter())
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(rect)
                        if rect.stroke.color == palette.accent && rect.stroke.width == 1.5 =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
        }
        found
    }

    #[test]
    fn a_media_tile_and_a_folder_with_the_keyboard_show_the_focus_ring() {
        let mut project = Project::new("Library");
        let clip = asset("clip.mp4", MediaKind::Video, 4.0, 10);
        let folder = MediaFolder {
            id: uuid::Uuid::new_v4(),
            name: "Shots".to_owned(),
            parent_id: None,
        };
        let (asset_id, folder_id) = (clip.id, folder.id);
        std::sync::Arc::make_mut(&mut project.assets).insert(clip.id, clip);
        std::sync::Arc::make_mut(&mut project.folders).insert(folder.id, folder);
        for grid in [true, false] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            assert!(
                rings(&ctx, &project, grid, None).is_empty(),
                "nothing has the keyboard: no ring"
            );
            for id in [
                Id::new(("library-asset", asset_id)),
                Id::new(("library-folder", folder_id)),
            ] {
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                let rings = rings(&ctx, &project, grid, Some(id));
                assert_eq!(
                    rings.len(),
                    1,
                    "grid {grid}: one ring, on what has the keyboard"
                );
            }
        }
    }

    fn proxy(asset: &Asset) -> editor_media::Proxy {
        editor_media::Proxy {
            path: format!("/cache/{}.mp4", asset.id).into(),
            width: 1280,
            height: 720,
            source: editor_media::SourceIdentity::of(asset),
        }
    }

    fn wide(name: &str) -> Asset {
        let mut asset = asset(name, MediaKind::Video, 8.0, 500_000_000);
        asset.width = 3840;
        asset.height = 2160;
        asset
    }

    #[test]
    fn a_video_tile_marks_its_proxy_and_nothing_else_does() {
        let video = wide("uhd.mov");
        let ready = ProxyStatus::Ready(proxy(&video));
        assert_eq!(proxy_mark(&video, &ProxyStatus::None, false, true), None);
        assert_eq!(
            proxy_mark(&video, &ProxyStatus::Queued, false, true),
            Some(ProxyMark::Making(None))
        );
        assert_eq!(
            proxy_mark(&video, &ProxyStatus::Generating(0.4), false, true),
            Some(ProxyMark::Making(Some(0.4)))
        );
        assert_eq!(
            proxy_mark(&video, &ready, false, true),
            Some(ProxyMark::Ready)
        );
        assert_eq!(
            proxy_mark(
                &video,
                &ProxyStatus::Failed("disk full".into()),
                false,
                true
            ),
            Some(ProxyMark::Failed("disk full".into()))
        );
        // One the viewer could not read is a warning, with the reason.
        let Some(ProxyMark::Failed(why)) = proxy_mark(&video, &ready, true, true) else {
            panic!("an unreadable proxy is a failure");
        };
        assert!(why.contains("original"), "{why}");
        let still = asset("still.png", MediaKind::Image, 5.0, 10);
        assert_eq!(proxy_mark(&still, &ready, false, true), None);
        // Each says what it is.
        assert_eq!(ProxyMark::Ready.describe().0, "Proxy ready");
        assert_eq!(
            ProxyMark::Making(Some(0.42)).describe().1,
            "Making a proxy: 42%"
        );
        assert_eq!(
            ProxyMark::Failed("disk full".into()).describe().1,
            "No proxy: disk full"
        );
    }

    #[test]
    fn a_video_menu_creates_or_removes_its_proxy() {
        let video = wide("uhd.mov");
        let id = video.id;
        let item = |status: &ProxyStatus| proxy_item(&video, status, true);
        assert!(matches!(
            item(&ProxyStatus::None),
            Some(("Create proxy", Action::CreateProxy(asset))) if asset == id
        ));
        assert!(matches!(
            item(&ProxyStatus::Failed("no".into())),
            Some(("Create proxy", Action::CreateProxy(asset))) if asset == id
        ));
        for status in [
            ProxyStatus::Queued,
            ProxyStatus::Generating(0.5),
            ProxyStatus::Ready(proxy(&video)),
        ] {
            assert!(matches!(
                item(&status),
                Some(("Remove proxy", Action::RemoveProxy(asset))) if asset == id
            ));
        }
        let still = asset("still.png", MediaKind::Image, 5.0, 10);
        assert!(proxy_item(&still, &ProxyStatus::None, true).is_none());
        let unknown = asset("unknown.mkv", MediaKind::Video, 5.0, 10);
        assert!(proxy_item(&unknown, &ProxyStatus::None, true).is_none());
    }

    #[test]
    fn with_proxies_off_a_made_proxy_is_not_said_to_be_read() {
        // Preview › Proxies is Off: the preview reads originals, so a proxy
        // that is ready says it is not used rather than that it is read.
        let video = wide("uhd.mov");
        let ready = ProxyStatus::Ready(proxy(&video));
        assert_eq!(
            proxy_mark(&video, &ready, false, false),
            Some(ProxyMark::Unused)
        );
        assert_eq!(
            proxy_mark(&video, &ready, true, false),
            Some(ProxyMark::Unused),
            "an unread proxy is not a failure either"
        );
        let (name, tip) = ProxyMark::Unused.describe();
        assert_eq!(name, "Proxy not used");
        assert_eq!(
            tip,
            "Proxy made; Preview › Proxies is Off, so the original is shown"
        );
        assert!(!tip.contains("reads a smaller copy"));
        // One being made still says so; the menu says when it is used.
        assert_eq!(
            proxy_mark(&video, &ProxyStatus::Generating(0.5), false, false),
            Some(ProxyMark::Making(Some(0.5)))
        );
        assert!(matches!(
            proxy_item(&video, &ProxyStatus::None, false),
            Some((
                "Create proxy (used when Proxies is on)",
                Action::CreateProxy(_)
            ))
        ));
        assert!(matches!(
            proxy_item(&video, &ready, false),
            Some(("Remove proxy", Action::RemoveProxy(_)))
        ));
    }

    /// The media tab of `project` in a window of its own, driven by the
    /// pointer and read through what accessibility is told.
    struct Shelf {
        ctx: egui::Context,
        project: Project,
        textures: Textures,
        state: LibraryState,
        selection: Selection,
        grid: bool,
        seen: Vec<(String, Rect)>,
        actions: Vec<Action>,
    }

    impl Shelf {
        fn new(project: Project, grid: bool) -> Self {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx.enable_accesskit();
            // Preview › Proxies at Auto, as a new window starts.
            let mut textures = Textures::default();
            textures.read_proxies(true);
            Self {
                ctx,
                project,
                textures,
                state: LibraryState::default(),
                selection: Selection::default(),
                grid,
                seen: Vec::new(),
                actions: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let palette = Palette::new(crate::theme::Appearance::Dark, Default::default());
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 600.0))),
                events,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let output = ctx.run_ui(input, |ui| {
                let rect = ui.max_rect();
                show(
                    ui,
                    rect,
                    &mut self.state,
                    &mut self.selection,
                    &mut self.textures,
                    &mut Titles::default(),
                    &LibraryInput {
                        p: &palette,
                        project: &self.project,
                        media: None,
                        grid: self.grid,
                        sort: LibrarySort::default(),
                        importing: 0,
                        problem: None,
                        previewing: None,
                        playhead: Time::ZERO,
                        presets: &Arc::default(),
                        speech: captions_tab::SpeechInput {
                            view: &crate::speech::SpeechView::Idle,
                            settings: &crate::speech::SpeechSettings::default(),
                            inventory: None,
                        },
                    },
                    &mut self.actions,
                );
            });
            if let Some(update) = &output.platform_output.accesskit_update {
                self.seen = (update.nodes.iter())
                    .filter_map(|(_, node)| {
                        let bounds = node.bounds()?;
                        Some((
                            node.label().or_else(|| node.value())?.to_owned(),
                            Rect::from_min_max(
                                pos2(bounds.x0 as f32, bounds.y0 as f32),
                                pos2(bounds.x1 as f32, bounds.y1 as f32),
                            ),
                        ))
                    })
                    .collect();
            }
            output.drop_without_applying_deltas();
        }

        fn find(&self, name: &str) -> Option<Rect> {
            (self.seen.iter())
                .find(|(seen, _)| seen == name)
                .map(|(_, rect)| *rect)
        }

        fn click(&mut self, at: egui::Pos2, button: egui::PointerButton) {
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![press(true)]);
            self.frame(vec![press(false)]);
            self.frame(Vec::new());
        }
    }

    #[test]
    fn a_tile_says_what_there_is_of_its_proxy() {
        let video = wide("uhd.mov");
        let id = video.id;
        let mut project = Project::new("Library");
        Arc::make_mut(&mut project.assets).insert(video.id, video.clone());
        for grid in [true, false] {
            let mut shelf = Shelf::new(project.clone(), grid);
            shelf.frame(Vec::new());
            shelf.frame(Vec::new());
            assert!(shelf.find("Proxy ready").is_none(), "none yet");
            for (status, name) in [
                (ProxyStatus::Queued, "Proxy waiting"),
                (ProxyStatus::Generating(0.3), "Making proxy"),
                (ProxyStatus::Ready(proxy(&video)), "Proxy ready"),
                (ProxyStatus::Failed("disk full".into()), "Proxy failed"),
            ] {
                shelf.textures.set_proxy(id, status);
                shelf.frame(Vec::new());
                shelf.frame(Vec::new());
                let tile = shelf.find("uhd.mov").expect("the tile is named");
                let mark = (shelf.find(name)).unwrap_or_else(|| panic!("grid {grid}: {name}"));
                assert!(
                    tile.contains_rect(mark),
                    "grid {grid}: {name} is on its tile"
                );
            }
            // With Proxies off the one made is not said to be read.
            shelf
                .textures
                .set_proxy(id, ProxyStatus::Ready(proxy(&video)));
            shelf.textures.read_proxies(false);
            shelf.frame(Vec::new());
            shelf.frame(Vec::new());
            assert!(shelf.find("Proxy ready").is_none(), "grid {grid}");
            assert!(shelf.find("Proxy not used").is_some(), "grid {grid}");
            assert!(shelf.actions.is_empty(), "a mark asks for nothing");
        }
    }

    #[test]
    fn the_tile_menu_asks_to_create_or_remove_the_proxy_of_that_video() {
        let video = wide("uhd.mov");
        let id = video.id;
        let mut project = Project::new("Library");
        Arc::make_mut(&mut project.assets).insert(video.id, video.clone());
        for (status, item) in [
            (ProxyStatus::None, "Create proxy"),
            (ProxyStatus::Ready(proxy(&video)), "Remove proxy"),
        ] {
            let mut shelf = Shelf::new(project.clone(), true);
            shelf.textures.set_proxy(id, status);
            shelf.frame(Vec::new());
            shelf.frame(Vec::new());
            let tile = shelf.find("uhd.mov").expect("the tile is named");
            // Away from the play badge in its middle.
            let at = tile.left_top() + vec2(8.0, 8.0);
            shelf.click(at, egui::PointerButton::Secondary);
            let row = shelf
                .find(item)
                .unwrap_or_else(|| panic!("the menu offers {item}"));
            shelf.click(row.center(), egui::PointerButton::Primary);
            let asked: Vec<_> = (shelf.actions.iter())
                .filter(|action| matches!(action, Action::CreateProxy(_) | Action::RemoveProxy(_)))
                .collect();
            assert_eq!(asked.len(), 1, "{item}: {:?}", shelf.actions);
            match (item, asked[0]) {
                ("Create proxy", Action::CreateProxy(asset))
                | ("Remove proxy", Action::RemoveProxy(asset)) => assert_eq!(*asset, id),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn media_sorts_by_each_field_in_both_directions() {
        let assets = [
            asset("beach.mp4", MediaKind::Video, 30.0, 900),
            asset("Alarm.wav", MediaKind::Audio, 4.0, 50),
            asset("cover.png", MediaKind::Image, 5.0, 300),
        ];
        assert_eq!(
            names(&assets, SortBy::Name, false),
            ["Alarm.wav", "beach.mp4", "cover.png"]
        );
        assert_eq!(
            names(&assets, SortBy::Name, true),
            ["cover.png", "beach.mp4", "Alarm.wav"]
        );
        assert_eq!(
            names(&assets, SortBy::Duration, true),
            ["beach.mp4", "cover.png", "Alarm.wav"]
        );
        assert_eq!(
            names(&assets, SortBy::Kind, false),
            ["beach.mp4", "Alarm.wav", "cover.png"]
        );
        assert_eq!(
            names(&assets, SortBy::Size, false),
            ["Alarm.wav", "cover.png", "beach.mp4"]
        );
    }

    #[test]
    fn equal_values_fall_back_to_the_name() {
        let assets = [
            asset("b.png", MediaKind::Image, 5.0, 10),
            asset("a.png", MediaKind::Image, 5.0, 10),
        ];
        for by in [SortBy::Duration, SortBy::Kind, SortBy::Size] {
            assert_eq!(names(&assets, by, true), ["a.png", "b.png"]);
        }
    }
}
