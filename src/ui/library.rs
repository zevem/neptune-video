//! The library: what the project has imported, as cards or rows, and what a
//! clip can be given (effects, transitions and looks), as cards.
//!
//! It is drawn flush on the window's chrome, as Terminal's sidebar is: a
//! strip of tabs, a search row with the tab's own actions, the tab's body,
//! which scrolls, and a footer that imports and opens Preferences.
//!
//! Only rows in view are laid out and only their posters are asked for. An
//! item is dragged to the timeline or added with a double click. The cards
//! of effects, transitions and looks are [`super::browser`]'s.
use super::{
    Action, Selection, arrow_choice,
    browser::{self, BrowserInput, BrowserState},
    captions_tab::{self, CaptionsTabInput, CaptionsTabState},
    controls, icon_of_media, kit, panel, shortcut,
    text_tab::{self, TextTabInput, TextTabState},
    timeline::AssetDrag,
    widgets::{self, icon_button, menu_item, menu_style},
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
    Asset, Command, FolderId, MediaFolder, MediaKind, Project, Time, TrackId,
    captions::SubtitleFormat,
};
use editor_media::{MediaService, ProxyStatus};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, Layout, Painter, Rect, Response, Sense, Stroke,
    StrokeKind, TextureId, Ui, Vec2, WidgetInfo, WidgetType, epaint::RectShape, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, sync::Arc};

/// How far the library's content stands in from the window's edge, and from
/// the viewer: the inspector's insets, mirrored.
pub(super) const LEADING: f32 = 8.0;
pub(super) const TRAILING: f32 = 6.0;
/// The search row under the tab strip, and the kinds row under it.
const SEARCH: f32 = 28.0;
const KINDS: f32 = 22.0;
/// A card's least width with the gap after it: two columns of 147 in the
/// 302 points of a library at its default width, and still two in one of
/// 300; three from 456.
const CARD_PITCH: f32 = 144.0;
pub(super) const CARD_GAP: f32 = 8.0;
/// Between two rows of cards: the hover of one does not meet the next.
pub(super) const CARD_ROW_GAP: f32 = 10.0;
/// What a card of media has under its well: its name, 6 below it, and a
/// line of what it is.
const CARD_TEXT: f32 = 40.0;
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

    /// What stands for the tab where the strip has no room for names.
    fn icon(self) -> Icon {
        match self {
            Self::Media => Icon::Film,
            Self::Text => Icon::Text,
            Self::Captions => Icon::Captions,
            Self::Effects => Icon::Wand,
            Self::Transitions => Icon::Transition,
            Self::Looks => Icon::ColorSwatch,
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

/// The kinds of media the Media tab lists. A lookup table is of none of
/// them and is listed under All only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KindFilter {
    #[default]
    All,
    Video,
    Sound,
    Images,
}

impl KindFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Video, Self::Sound, Self::Images];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Video => "Video",
            Self::Sound => "Sound",
            Self::Images => "Images",
        }
    }

    /// What the control is called where its place does not say it.
    fn name(self) -> &'static str {
        match self {
            Self::All => "Show all media",
            Self::Video => "Show video only",
            Self::Sound => "Show sound only",
            Self::Images => "Show images only",
        }
    }

    /// Whether media of `kind` is listed. A GIF is a picture file and is
    /// found with the images.
    pub fn admits(self, kind: MediaKind) -> bool {
        match self {
            Self::All => true,
            Self::Video => kind == MediaKind::Video,
            Self::Sound => kind == MediaKind::Audio,
            Self::Images => matches!(kind, MediaKind::Image | MediaKind::Gif),
        }
    }

    /// What an empty list is headed with.
    fn none(self) -> &'static str {
        match self {
            Self::All => "Empty folder",
            Self::Video => "No video",
            Self::Sound => "No sound",
            Self::Images => "No images",
        }
    }
}

/// The library's parts on Terminal's strip of tabs: names while they fit,
/// set closer when they nearly do, and icons named on hover when they do
/// not. Left and Right move the choice while a tab has the keyboard.
fn tab_strip(ui: &mut Ui, p: &Palette, strip: Rect, tab: &mut Tab) {
    // The strip's own place, so that its tabs are the same wherever the
    // library is drawn.
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .id(Id::new("library-tab-strip"))
            .max_rect(strip),
    );
    let ui = &mut ui;
    let tabs = Tab::ALL.map(|tab| panel::Tab {
        label: tab.label(),
        icon: tab.icon(),
        waiting: 0,
    });
    let selected = (Tab::ALL.iter())
        .position(|shown| *shown == *tab)
        .unwrap_or(0);
    let cells: Vec<Id> = (tabs.iter())
        .map(|tab| ui.id().with(("panel-tab", tab.label)))
        .collect();
    let arrowed = arrow_choice(ui.ctx(), &cells, selected);
    let clicked = panel::tabs(
        ui,
        strip,
        *p,
        &tabs,
        arrowed.unwrap_or(selected),
        WidgetType::RadioButton,
    );
    if let Some(chosen) = arrowed.or(clicked) {
        *tab = Tab::ALL[chosen];
    }
}

/// The kinds of media listed, as one row of flat tabs of equal width.
/// Left and Right move the choice while one has the keyboard.
fn kind_filter(ui: &mut Ui, p: &Palette, row: Rect, kind: &mut KindFilter) {
    let cells = KindFilter::ALL.map(|option| Id::new(("library-kind", option)));
    let selected = (KindFilter::ALL.iter())
        .position(|option| *option == *kind)
        .unwrap_or(0);
    if let Some(to) = arrow_choice(ui.ctx(), &cells, selected) {
        *kind = KindFilter::ALL[to];
    }
    const GAP: f32 = 2.0;
    let count = KindFilter::ALL.len() as f32;
    let width = (row.width() - GAP * (count - 1.0)) / count;
    for (index, option) in KindFilter::ALL.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(row.left() + index as f32 * (width + GAP), row.top()),
            vec2(width, row.height()),
        );
        let response = ui.interact(cell, cells[index], Sense::click());
        let chosen = *kind == option;
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, true, chosen, option.name())
        });
        let painter = ui.painter();
        if chosen {
            painter.rect_filled(cell, 6, theme::tint(p.fg, 0.08));
        } else if response.hovered() {
            painter.rect_filled(cell, 6, theme::tint(p.fg, 0.045));
        }
        if response.has_focus() {
            controls::focus_ring(painter, cell, 6, *p);
        }
        let (font, ink) = if chosen {
            (theme::medium(11.5), p.fg)
        } else {
            (theme::regular(11.5), p.secondary)
        };
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            option.label(),
            font,
            ink,
        );
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            *kind = option;
        }
    }
}

#[derive(Default)]
pub struct LibraryState {
    /// Which part is in view: the media, or what a clip can be given.
    pub tab: Tab,
    pub browser: BrowserState,
    pub search: String,
    /// The kinds of media listed. The person's for the session: no file and
    /// no setting keeps it.
    pub kind: KindFilter,
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
/// outlined while it is held there.
fn dropped_asset(ui: &Ui, response: &Response, p: &Palette) -> Option<editor_core::AssetId> {
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

/// What a video's card says of its proxy.
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

/// What the card of `asset` shows of its proxy, given what the proxy
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

/// Draws the proxy mark of a card in the top right corner of its picture,
/// with its name and tooltip, and a thin bar along the picture's foot while
/// a proxy is made. The mark is on the picture, so it is drawn in the
/// stage's inks, `well`.
fn proxy_badge(ui: &Ui, well: &Palette, id: Id, picture: Rect, grid: bool, mark: &ProxyMark) {
    let side = if grid { 18.0 } else { 14.0 };
    let badge = Rect::from_min_size(
        pos2(picture.right() - side - 4.0, picture.top() + 4.0),
        Vec2::splat(side),
    );
    let (name, tip) = mark.describe();
    let response = ui.interact(badge, id.with("proxy"), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    response.on_hover_text(tip);
    let painter = ui.painter();
    let (icon, color, fill) = match mark {
        ProxyMark::Ready => (Icon::Layers, Color32::WHITE, Color32::from_black_alpha(140)),
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
        ProxyMark::Failed(_) => (
            Icon::Warning,
            well.legible(well.yellow),
            Color32::from_black_alpha(140),
        ),
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
        painter.rect_filled(fill, CornerRadius::same(1), well.accent);
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

/// What a card or row says an asset is, under its name.
fn meta(asset: &Asset) -> String {
    let kind = kind_name(asset.kind);
    match asset.kind {
        MediaKind::Image => format!("{kind} · {}×{}", asset.width, asset.height),
        MediaKind::Lut => format!("{kind} · {} points", asset.width),
        MediaKind::Font => kind.to_owned(),
        _ => format!("{kind} · {}", timefmt::short(asset.duration)),
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
    // A folder that was undone away is not a place to be.
    if state
        .folder
        .is_some_and(|id| !project.folders.contains_key(&id))
    {
        state.folder = None;
    }
    let left = rect.left() + LEADING;
    let right = rect.right() - TRAILING;

    let strip = Rect::from_min_max(
        pos2(left, rect.top()),
        pos2(right, rect.top() + panel::TABS),
    );
    tab_strip(ui, p, strip, &mut state.tab);
    let tab = state.tab;

    // The search, and after it what this tab alone does.
    let row = Rect::from_min_max(
        pos2(left, strip.bottom() + 4.0),
        pos2(right, strip.bottom() + 4.0 + SEARCH),
    );
    let caption_track = captions_tab::in_view(project, state.captions.track, selection);
    let count = match tab {
        Tab::Media | Tab::Text => 1,
        Tab::Captions => 2 + usize::from(caption_track.is_some()),
        Tab::Effects | Tab::Transitions | Tab::Looks => 0,
    };
    let trailing = if count == 0 {
        0.0
    } else {
        count as f32 * metrics::ICON_BUTTON + (count - 1) as f32 * 2.0 + 4.0
    };
    let field = Rect::from_min_max(row.min, pos2(row.right() - trailing, row.bottom()));
    let sought = if tab == Tab::Media {
        &mut state.search
    } else {
        &mut state.browser.search
    };
    let _ = controls::place(
        ui,
        field,
        Layout::left_to_right(Align::Center),
        "library-search-row",
        |ui| {
            controls::search_field_sized(
                ui,
                *p,
                Id::new(("library-search", tab)),
                sought,
                tab.hint(),
                tab.hint(),
                field.size(),
            )
        },
    );
    if count > 0 {
        let tools = Rect::from_min_max(pos2(field.right() + 4.0, row.top()), row.max);
        controls::place(
            ui,
            tools,
            Layout::right_to_left(Align::Center),
            "library-search-actions",
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                match tab {
                    Tab::Media => view_options(ui, p, &mut grid, &mut sort),
                    Tab::Text => {
                        if icon_button(ui, p, Icon::Import, "Import fonts").clicked() {
                            actions.push(Action::ImportFonts);
                        }
                    }
                    Tab::Captions => {
                        caption_actions(ui, p, caption_track, &mut state.captions, actions);
                    }
                    Tab::Effects | Tab::Transitions | Tab::Looks => {}
                }
            },
        );
    }

    let footer = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - metrics::PANE_FOOTER),
        rect.max,
    );
    let body = Rect::from_min_max(
        pos2(rect.left(), row.bottom() + 6.0),
        pos2(rect.right(), footer.top()),
    );
    match tab {
        Tab::Text => text_tab::show(
            ui,
            body,
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
        ),
        Tab::Captions => captions_tab::show(
            ui,
            body,
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
        ),
        Tab::Effects | Tab::Transitions | Tab::Looks => browser::show(
            ui,
            body,
            &mut state.browser,
            selection,
            &BrowserInput {
                p,
                project,
                playhead: input.playhead,
                tab,
            },
            actions,
        ),
        Tab::Media => {
            // The kinds listed, over whatever folder and order are in view.
            let kinds = Rect::from_min_max(pos2(left, body.top()), pos2(right, body.top() + KINDS));
            kind_filter(ui, p, kinds, &mut state.kind);
            let body = Rect::from_min_max(pos2(body.left(), kinds.bottom() + 6.0), body.max);
            media(
                ui, body, state, selection, textures, input, grid, sort, actions,
            );
        }
    }
    footer_row(ui, p, footer, state, input.importing, actions);
    (grid, sort)
}

/// The Media tab's one control on the search row: how media is laid out
/// and in what order, as one menu.
fn view_options(ui: &mut Ui, p: &Palette, grid: &mut bool, sort: &mut LibrarySort) {
    let view = icon_button(ui, p, Icon::Adjustments, "View options");
    egui::Popup::menu(&view).show(|ui| {
        menu_style(ui, 200.0);
        for (as_grid, text) in [(true, "Show as grid"), (false, "Show as list")] {
            if kit::menu_row(ui, p, None, text, "", false, Some(*grid == as_grid)).clicked() {
                *grid = as_grid;
                ui.close();
            }
        }
        widgets::menu_separator(ui, p);
        kit::menu_heading(ui, p, "Sort media");
        for (by, text) in SortBy::ALL {
            if kit::menu_row(ui, p, None, text, "", false, Some(sort.by == by)).clicked() {
                sort.by = by;
                ui.close();
            }
        }
        widgets::menu_separator(ui, p);
        for (descending, text) in [(false, "Ascending"), (true, "Descending")] {
            let chosen = Some(sort.descending == descending);
            if kit::menu_row(ui, p, None, text, "", false, chosen).clicked() {
                sort.descending = descending;
                ui.close();
            }
        }
    });
}

/// The Captions tab's controls on the search row, made from the trailing
/// edge: what exports the track in view, what imports a subtitle file, and
/// the switch of captions from speech, which stays pressed while its
/// choices are open.
fn caption_actions(
    ui: &mut Ui,
    p: &Palette,
    track: Option<TrackId>,
    state: &mut CaptionsTabState,
    actions: &mut Vec<Action>,
) {
    if let Some(track) = track {
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
    let open = state.speech_open;
    if widgets::icon_toggle(ui, p, Icon::Speech, open, "Captions from speech").clicked() {
        state.speech_open = !open;
        if !open {
            actions.push(Action::LookAtModels);
        }
    }
}

/// The footer, under every tab: a hairline, the import button with its
/// menu and what is being imported, and the way to Preferences.
fn footer_row(
    ui: &mut Ui,
    p: &Palette,
    footer: Rect,
    state: &mut LibraryState,
    importing: usize,
    actions: &mut Vec<Action>,
) {
    let left = footer.left() + LEADING;
    let right = footer.right() - TRAILING;
    let line = footer.top() + 0.5;
    ui.painter()
        .line_segment([pos2(left, line), pos2(right, line)], p.hairline());
    let middle = footer.center().y;
    let import = import_button(ui, p, pos2(left, middle));
    egui::Popup::menu(&import)
        .align(egui::RectAlign::TOP_START)
        .show(|ui| {
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
            widgets::menu_separator(ui, p);
            if menu_item(ui, p, Some(Icon::FolderPlus), "New folder", "").clicked() {
                // A folder is made in the folder of media in view.
                state.tab = Tab::Media;
                state.begin_folder();
                ui.close();
            }
        });
    let gear = Rect::from_center_size(
        pos2(right - metrics::ICON_BUTTON * 0.5, middle),
        Vec2::splat(metrics::ICON_BUTTON),
    );
    if importing > 0 {
        let spinner =
            Rect::from_center_size(pos2(import.rect.right() + 14.0, middle), Vec2::splat(12.0));
        kit::spinner(ui, spinner, p.secondary);
        let room = gear.left() - 8.0 - (spinner.right() + 6.0);
        let words = controls::elided(
            ui.painter(),
            "Importing…",
            theme::regular(11.5),
            p.secondary,
            room,
        );
        let said = controls::galley_at(ui.painter(), pos2(spinner.right() + 6.0, middle), words);
        ui.interact(said, Id::new("library-importing"), Sense::hover())
            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Importing…"));
    }
    let name = format!("Preferences ({})", shortcut("Cmd+,"));
    let preferences = controls::place(
        ui,
        gear,
        Layout::left_to_right(Align::Center),
        "library-preferences",
        |ui| icon_button(ui, p, Icon::Settings, &name),
    );
    if preferences.clicked() {
        actions.push(Action::OpenSettings);
    }
}

/// Terminal's footer button: a plus and "Import…", quiet until the pointer
/// comes, its left edge and middle at `left_center`. Its name says all it
/// opens.
fn import_button(ui: &mut Ui, p: &Palette, left_center: egui::Pos2) -> Response {
    let words = ui
        .painter()
        .layout_no_wrap("Import…".to_owned(), theme::medium(12.5), p.fg);
    let rect = Rect::from_min_size(
        pos2(left_center.x, left_center.y - metrics::CONTROL_HEIGHT * 0.5),
        vec2(31.0 + words.size().x + 12.0, metrics::CONTROL_HEIGHT),
    );
    let response = ui.interact(rect, Id::new("library-import"), Sense::click());
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Button, true, "Import media or add a folder")
    });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let hover = kit::hover_amount(ui, &response);
    let painter = ui.painter();
    if open || response.is_pointer_button_down_on() {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.pressed);
    } else if hover > 0.0 {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.hover.gamma_multiply(hover));
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, metrics::ROW_RADIUS, *p);
    }
    let ink = if open {
        p.fg
    } else {
        theme::mix(p.secondary, p.fg, hover)
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(14.0)),
        Icon::Plus,
        ink,
    );
    painter.galley(
        pos2(rect.left() + 31.0, rect.center().y - words.size().y * 0.5),
        words,
        ink,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Import media or add a folder")
}

/// The Media tab under its kinds, in `body`: where in the folders it is, a
/// folder being named, and the media of the folder in view.
#[expect(clippy::too_many_arguments, reason = "the library's one call")]
fn media(
    ui: &mut Ui,
    body: Rect,
    state: &mut LibraryState,
    selection: &mut Selection,
    textures: &mut Textures,
    input: &LibraryInput,
    grid: bool,
    sort: LibrarySort,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let project = input.project;
    let left = body.left() + LEADING;
    let right = body.right() - TRAILING;
    let mut top = body.top();
    // Where we are, when inside a folder.
    if let Some(folder) = state.folder {
        let row = Rect::from_min_max(pos2(left, top), pos2(right, top + 24.0));
        if let Some(to) = breadcrumb(ui, p, row, project, folder, actions) {
            state.folder = to;
        }
        top = row.bottom() + 6.0;
    }
    // The name of a folder that is being made, here in the folder in view.
    if let Some(name) = &mut state.new_folder {
        let row = Rect::from_min_max(pos2(left, top), pos2(right, top + SEARCH));
        let id = Id::new("library-new-folder");
        controls::field_frame(ui, *p, row, id, true);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(row.left() + 16.0, row.center().y), Vec2::splat(13.0)),
            Icon::FolderPlus,
            p.muted,
        );
        let inner = Rect::from_min_max(
            pos2(row.left() + 30.0, row.top()),
            pos2(row.right() - 10.0, row.bottom()),
        );
        let response = controls::place(
            ui,
            inner,
            Layout::left_to_right(Align::Center),
            "library-new-folder-field",
            |ui| controls::bare_text_edit(ui, id, name, "Folder name", 13.0, inner.width()),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, "Folder name"));
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
    let list = Rect::from_min_max(pos2(body.left(), top), body.max);

    // What this view lists.
    let query = state.search.trim().to_lowercase();
    let items = listed(project, state.folder, &query, state.kind, sort);

    if items.is_empty() {
        if let Some(problem) = input.problem.filter(|_| project.assets.is_empty()) {
            kit::empty_state(
                ui,
                p,
                list,
                Icon::Warning,
                "Media is unavailable",
                problem,
                None,
            );
        } else if project.assets.is_empty() && query.is_empty() {
            if kit::empty_state(
                ui,
                p,
                list,
                Icon::Film,
                "No media yet",
                "Import video, sound and images, or drop them here.",
                Some("Import…"),
            ) {
                actions.push(Action::ImportFiles);
            }
        } else {
            let (icon, title, detail) = nothing_listed(state.kind, &query, state.folder.is_some());
            kit::empty_state(ui, p, list, icon, title, detail, None);
        }
        return;
    }

    let mut area = ui.new_child(egui::UiBuilder::new().max_rect(list));
    area.set_clip_rect(list.intersect(ui.clip_rect()));
    area.spacing_mut().item_spacing = Vec2::ZERO;
    let width = list.width() - LEADING - TRAILING;
    let (columns, cell_width) = if grid {
        card_columns(width)
    } else {
        (1, width)
    };
    let well = cell_width * 9.0 / 16.0;
    // A card stands 6 below its row's top, so that its rings are not cut by
    // the top of the list; a row, 4.
    let (pitch, cell_height, inset) = if grid {
        (well + CARD_TEXT + CARD_ROW_GAP, well + CARD_TEXT, 6.0)
    } else {
        (ROW_HEIGHT, ROW_HEIGHT - 2.0, 4.0)
    };
    let rows = items.len().div_ceil(columns);
    let output = egui::ScrollArea::vertical()
        .id_salt("library")
        .auto_shrink([false, false])
        .show_rows(&mut area, pitch, rows, |ui, range| {
            for row in range {
                let (row_rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), pitch), Sense::hover());
                for column in 0..columns {
                    let Some(item) = items.get(row * columns + column) else {
                        break;
                    };
                    let cell = Rect::from_min_size(
                        pos2(
                            row_rect.left() + LEADING + column as f32 * (cell_width + CARD_GAP),
                            row_rect.top() + inset,
                        ),
                        vec2(cell_width, cell_height),
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
    fade_when_clipped(ui, p, &output);
}

/// The columns of cards in `width`, and each card's width: as many of at
/// least 136 as fit with the gaps between them.
pub(super) fn card_columns(width: f32) -> (usize, f32) {
    let columns = ((width + CARD_GAP) / CARD_PITCH).floor().max(1.0) as usize;
    let cell = (width - CARD_GAP * (columns as f32 - 1.0)) / columns as f32;
    (columns, cell)
}

/// Says that a list runs on below what is shown: the scroll fade over its
/// foot, while something is cut off there.
pub(super) fn fade_when_clipped<R>(
    ui: &Ui,
    p: &Palette,
    output: &egui::scroll_area::ScrollAreaOutput<R>,
) {
    let below = output.content_size.y - output.state.offset.y - output.inner_rect.height();
    if below > 0.5 {
        kit::scroll_fade(ui.painter(), output.inner_rect, kit::Edge::Bottom, p.chrome);
    }
}

/// A heading over a group of cards: the look of a section label, read out
/// as the text it is.
pub(super) fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let words = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::medium(11.5), p.secondary);
    let said = controls::galley_at(
        ui.painter(),
        pos2(rect.left() + 4.0, rect.center().y),
        words,
    );
    ui.interact(said, ui.id().with(("heading", text)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
}

/// What a card shows of its state, under its well: `hover` behind it under
/// the pointer, `pressed` behind it and an accent ring round its well while
/// it is chosen, and the focus ring round the whole card while it has the
/// keyboard.
pub(super) fn card_states(
    ui: &Ui,
    p: &Palette,
    cell: Rect,
    well: Rect,
    response: &Response,
    chosen: bool,
) {
    let painter = ui.painter();
    let back = cell.expand(4.0);
    // Lit at once, as Terminal's rows are: a list at rest does not repaint.
    if chosen {
        painter.rect_filled(back, 10, p.pressed);
        painter.rect_stroke(
            well.expand(2.0),
            10,
            Stroke::new(2.0, p.accent),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(back, 10, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, back, 10, *p);
    }
}

/// A card's name, 6 below its well, and what it is under that.
pub(super) fn card_name(ui: &Ui, p: &Palette, well: Rect, name: &str, about: Option<&str>) {
    let painter = ui.painter();
    let name = controls::elided(painter, name, theme::medium(12.0), p.fg, well.width());
    let below = well.bottom() + 6.0 + name.size().y;
    painter.galley(pos2(well.left(), well.bottom() + 6.0), name, p.fg);
    if let Some(about) = about {
        let about = controls::elided(
            painter,
            about,
            theme::regular(11.0),
            p.secondary,
            well.width(),
        );
        painter.galley(pos2(well.left(), below + 2.0), about, p.secondary);
    }
}

/// A row's state: `hover` and `pressed` fills, and the focus ring.
fn row_states(ui: &Ui, p: &Palette, rect: Rect, response: &Response, chosen: bool) {
    let painter = ui.painter();
    if chosen {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.pressed);
    } else if response.hovered() {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, metrics::ROW_RADIUS, *p);
    }
}

/// A poster filling `well`, cut to its shape rather than letterboxed.
fn cover(painter: &Painter, well: Rect, radius: u8, texture: TextureId, size: Vec2) {
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (well.width() / size.x).max(well.height() / size.y);
    let shown = vec2(
        well.width() / (size.x * scale),
        well.height() / (size.y * scale),
    );
    let uv = Rect::from_center_size(pos2(0.5, 0.5), shown);
    painter.add(RectShape::filled(well, radius, Color32::WHITE).with_texture(texture, uv));
}

/// Where in the folders the Media tab is: "Media", then each folder down
/// to the one in view, each but the last a quiet button that goes there and
/// takes media let go on it. Gives the place one asked to go to.
fn breadcrumb(
    ui: &mut Ui,
    p: &Palette,
    row: Rect,
    project: &Project,
    folder: FolderId,
    actions: &mut Vec<Action>,
) -> Option<Option<FolderId>> {
    let mut chain: Vec<&MediaFolder> = Vec::new();
    let mut at = project.folders.get(&folder);
    // A folder is never its own parent, but a damaged file is not trusted
    // to say so.
    while let Some(found) = at.filter(|_| chain.len() <= project.folders.len()) {
        chain.push(found);
        at = found.parent_id.and_then(|id| project.folders.get(&id));
    }
    chain.reverse();
    let segments: Vec<(Option<FolderId>, &str)> = std::iter::once((None, "Media"))
        .chain(
            chain
                .iter()
                .map(|folder| (Some(folder.id), folder.name.as_str())),
        )
        .collect();
    let last = segments.len() - 1;
    let mut x = row.left();
    let mut went = None;
    for (index, (target, name)) in segments.into_iter().enumerate() {
        if index > 0 {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(x + 6.0, row.center().y), Vec2::splat(10.0)),
                Icon::ChevronRight,
                p.muted,
            );
            x += 12.0;
        }
        let room = row.right() - x - 12.0;
        if room < 12.0 {
            break;
        }
        let here = index == last;
        let words = controls::elided(ui.painter(), name, theme::regular(12.0), p.fg, room);
        let cell = Rect::from_min_size(
            pos2(x, row.top() + 1.0),
            vec2(words.size().x + 12.0, row.height() - 2.0),
        );
        x = cell.right() + 2.0;
        let sense = if here { Sense::hover() } else { Sense::click() };
        let response = ui.interact(cell, Id::new(("library-crumb", index)), sense);
        let role = if here {
            WidgetType::Label
        } else {
            WidgetType::Button
        };
        response.widget_info(|| WidgetInfo::labeled(role, true, name));
        let mut ink = p.fg;
        if !here {
            let painter = ui.painter();
            if response.is_pointer_button_down_on() {
                painter.rect_filled(cell, 6, p.pressed);
            } else if response.hovered() {
                painter.rect_filled(cell, 6, p.hover);
            }
            if response.has_focus() {
                controls::focus_ring(painter, cell, 6, *p);
            }
            if !response.hovered() {
                ink = p.secondary;
            }
            // Media let go on the way back goes there.
            if let Some(asset_id) = dropped_asset(ui, &response, p) {
                actions.push(Action::Edit(Command::MoveAssetToFolder {
                    asset_id,
                    folder_id: target,
                }));
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                went = Some(target);
            }
        }
        ui.painter().galley(
            pos2(cell.left() + 6.0, cell.center().y - words.size().y * 0.5),
            words,
            ink,
        );
    }
    went
}

/// What the Media tab lists: the folders of the folder in view, by name, then
/// its media of the kinds asked for, in the order asked for. A search looks
/// through every folder for a name and lists no folders. `query` is trimmed
/// and in lower case.
fn listed<'a>(
    project: &'a Project,
    folder: Option<FolderId>,
    query: &str,
    kind: KindFilter,
    sort: LibrarySort,
) -> Vec<Item<'a>> {
    let mut items: Vec<Item> = Vec::new();
    if query.is_empty() {
        let mut folders: Vec<&MediaFolder> = project
            .folders
            .values()
            .filter(|found| found.parent_id == folder)
            .collect();
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        items.extend(folders.into_iter().map(Item::Folder));
    }
    let mut assets: Vec<&Asset> = project
        .assets
        .values()
        .filter(|asset| kind.admits(asset.kind))
        .filter(|asset| {
            if query.is_empty() {
                asset.folder_id == folder
            } else {
                asset.name.to_lowercase().contains(query)
            }
        })
        .collect();
    sort_assets(&mut assets, sort);
    items.extend(assets.into_iter().map(Item::Asset));
    items
}

/// What stands where the list would be when a library with media in it has
/// nothing to list here: the icon, the heading and what is meant by it.
fn nothing_listed(
    kind: KindFilter,
    query: &str,
    in_folder: bool,
) -> (Icon, &'static str, &'static str) {
    if !query.is_empty() {
        let detail = match kind {
            KindFilter::All => "Nothing in this project has that name.",
            KindFilter::Video => "No video in this project has that name.",
            KindFilter::Sound => "No sound in this project has that name.",
            KindFilter::Images => "No image in this project has that name.",
        };
        return (Icon::Search, "No matches", detail);
    }
    let icon = icon_of_media(match kind {
        KindFilter::All => return (Icon::Folder, "Empty folder", ""),
        KindFilter::Video => MediaKind::Video,
        KindFilter::Sound => MediaKind::Audio,
        KindFilter::Images => MediaKind::Image,
    });
    let detail = if in_folder {
        "Nothing of that kind is in this folder."
    } else {
        "Nothing of that kind is in this project."
    };
    (icon, kind.none(), detail)
}

/// The picture of a row: inset 4 at its leading edge, 16:9.
fn row_picture(rect: Rect) -> Rect {
    Rect::from_min_size(
        pos2(rect.left() + 4.0, rect.top() + 4.0),
        vec2((rect.height() - 8.0) * 16.0 / 9.0, rect.height() - 8.0),
    )
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
        WidgetInfo::labeled(WidgetType::Button, true, format!("Folder {}", folder.name))
    });
    let count = project
        .assets
        .values()
        .filter(|asset| asset.folder_id == Some(folder.id))
        .count();
    let detail = match count {
        1 => "1 item".to_owned(),
        n => format!("{n} items"),
    };
    if grid {
        let well = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0));
        card_states(ui, p, rect, well, &response, false);
        let painter = ui.painter();
        painter.rect_filled(well, metrics::CONTROL_RADIUS, p.control);
        icons::paint(
            painter,
            Rect::from_center_size(well.center(), Vec2::splat(26.0)),
            Icon::Folder,
            p.secondary,
        );
        card_name(ui, p, well, &folder.name, Some(&detail));
    } else {
        row_states(ui, p, rect, &response, false);
        let picture = row_picture(rect);
        let painter = ui.painter();
        icons::paint(
            painter,
            Rect::from_center_size(picture.center(), Vec2::splat(18.0)),
            Icon::Folder,
            p.secondary,
        );
        let detail = painter.layout_no_wrap(detail, theme::regular(11.0), p.secondary);
        let name_left = picture.right() + 10.0;
        let name = controls::elided(
            painter,
            &folder.name,
            theme::medium(13.0),
            p.fg,
            rect.right() - 16.0 - detail.size().x - name_left,
        );
        controls::galley_at(painter, pos2(name_left, rect.center().y), name);
        let detail_left = rect.right() - 8.0 - detail.size().x;
        controls::galley_at(painter, pos2(detail_left, rect.center().y), detail);
    }
    // Media let go on a folder moves into it.
    if let Some(asset_id) = dropped_asset(ui, &response, p) {
        actions.push(Action::Edit(Command::MoveAssetToFolder {
            asset_id,
            folder_id: Some(folder.id),
        }));
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
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
    // What is drawn on a picture is drawn in the stage's inks, which is dark
    // in either appearance.
    let well = Palette::stage_palette(Default::default());
    let id = Id::new(("library-asset", asset.id));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let selected = selection.asset == Some(asset.id);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &asset.name));
    let picture = if grid {
        Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0))
    } else {
        row_picture(rect)
    };
    let radius = if grid { metrics::CONTROL_RADIUS } else { 5 };
    if grid {
        card_states(ui, p, rect, picture, &response, selected);
    } else {
        row_states(ui, p, rect, &response, selected);
    }
    let painter = ui.painter();
    painter.rect_filled(picture, radius, p.stage());
    let poster = textures.poster(asset.id);
    match poster {
        Some(texture) => cover(painter, picture, radius, texture.id(), texture.size_vec2()),
        // Still being looked at, or nothing to show: what it is, quietly.
        None => icons::paint(
            painter,
            Rect::from_center_size(picture.center(), Vec2::splat(16.0)),
            icon_of_media(asset.kind),
            well.muted,
        ),
    }

    // The asset in the source monitor says so on its picture.
    if input.previewing == Some(asset.id) {
        let side = if grid { 18.0 } else { 14.0 };
        let mark = Rect::from_min_size(picture.min + vec2(4.0, 4.0), Vec2::splat(side));
        painter.rect_filled(mark, CornerRadius::same(5), Color32::from_black_alpha(140));
        icons::paint(
            painter,
            mark.shrink(side * 0.24),
            Icon::Play,
            Color32::WHITE,
        );
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
        proxy_badge(ui, &well, id, picture, grid, &mark);
    }
    // A lookup table is neither played nor placed on a track.
    let placeable = crate::rules::placeable(asset);
    // Its length, over the picture's bottom right corner.
    if grid && asset.kind != MediaKind::Image && placeable {
        let length = timefmt::short(asset.duration);
        let width = (ui.painter())
            .layout_no_wrap(length.clone(), theme::mono(11.0), Color32::WHITE)
            .size()
            .x
            + 12.0;
        kit::badge(
            ui.painter(),
            p,
            pos2(picture.right() - 4.0 - width, picture.bottom() - 4.0 - 8.0),
            &length,
            kit::BadgeOn::Picture,
        );
    }
    // Under the pointer, the picture offers to play its source. A secondary
    // click on the offer is one on the card: it opens the card's menu.
    let mut menu_of = response.clone();
    if placeable && ui.rect_contains_pointer(rect) && !response.dragged() {
        let side = if grid { 30.0 } else { 22.0 };
        let badge = Rect::from_center_size(picture.center(), Vec2::splat(side));
        let preview = ui
            .interact(badge, id.with("preview"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Preview (Enter)");
        preview.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Preview"));
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
        if preview.secondary_clicked() {
            menu_of = response.union(preview);
        }
    }

    let about = meta(asset);
    if grid {
        card_name(ui, p, picture, &asset.name, Some(&about));
    } else {
        let left = picture.right() + 10.0;
        let painter = ui.painter();
        let name = controls::elided(
            painter,
            &asset.name,
            theme::medium(13.0),
            p.fg,
            rect.right() - left - 8.0,
        );
        painter.galley(pos2(left, rect.top() + 6.0), name, p.fg);
        let about = controls::elided(
            painter,
            &about,
            theme::regular(11.0),
            p.secondary,
            rect.right() - left - 8.0,
        );
        painter.galley(pos2(left, rect.top() + 24.0), about, p.secondary);
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
    // What is being carried follows the pointer: its poster, lifted.
    if placeable
        && response.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("library-carry"));
        let mut carry = ui.ctx().layer_painter(layer);
        let lifted = Rect::from_min_size(pos + vec2(12.0, 10.0), vec2(64.0, 36.0));
        carry.add(p.popup_shadow().as_shape(lifted, 6));
        carry.set_opacity(0.85);
        carry.rect_filled(lifted, 6, p.stage());
        match poster {
            Some(texture) => cover(&carry, lifted, 6, texture.id(), texture.size_vec2()),
            None => icons::paint(
                &carry,
                Rect::from_center_size(lifted.center(), Vec2::splat(14.0)),
                icon_of_media(asset.kind),
                well.muted,
            ),
        }
    }
    menu_of.context_menu(|ui| {
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
        // Offered of a sound file; one that cannot be laid under the project
        // as it is shows the row without being able to choose it, and says
        // why.
        if asset.kind == MediaKind::Audio {
            let refused = crate::rules::bed_refusal(input.project, asset.id);
            ui.add_enabled_ui(refused.is_none(), |ui| {
                let row = menu_item(ui, p, Some(Icon::Music), "Add as background music", "");
                if row.clicked() {
                    actions.push(Action::AddBackgroundMusic(asset.id));
                    ui.close();
                }
                if let Some(reason) = refused {
                    row.on_disabled_hover_text(reason);
                }
            });
        }
        // Offered of media with sound; media whose sound has nothing to
        // save shows the row without being able to choose it, and says why.
        if asset.has_audio || asset.kind == MediaKind::Audio {
            let source = crate::extract::Source::Asset(asset.id);
            let refused = crate::extract::refusal(input.project, source);
            ui.add_enabled_ui(refused.is_none(), |ui| {
                let row = menu_item(ui, p, Some(Icon::Save), crate::extract::LABEL, "");
                if row.clicked() {
                    actions.push(Action::SaveAudio(source));
                    ui.close();
                }
                if let Some(reason) = refused {
                    row.on_disabled_hover_text(reason);
                }
            });
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
            widgets::menu_separator(ui, p);
            kit::menu_heading(ui, p, "Move to");
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
        widgets::menu_separator(ui, p);
        if widgets::menu_item_destructive(ui, p, Some(Icon::Trash), "Remove from project", "")
            .clicked()
        {
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

    /// Draws the media tab once and hands back the focus rings that were
    /// painted, as the rects they outline.
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
                    // The ring of `controls::focus_ring` (spec §7.4): 2 points of
                    // the accent at .55.
                    egui::Shape::Rect(rect)
                        if rect.stroke.color == theme::tint(palette.accent, 0.55)
                            && rect.stroke.width == 2.0 =>
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
        sort: LibrarySort,
        size: Vec2,
        seen: Vec<(String, Rect)>,
        /// The words painted in the last frame, where, and at what size.
        words: Vec<(String, Rect, f32)>,
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
                sort: LibrarySort::default(),
                size: vec2(320.0, 600.0),
                seen: Vec::new(),
                words: Vec::new(),
                actions: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let palette = Palette::new(crate::theme::Appearance::Dark, Default::default());
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), self.size)),
                events,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let output = ctx.run_ui(input, |ui| {
                let rect = ui.max_rect();
                (self.grid, self.sort) = show(
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
                        sort: self.sort,
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
            self.words = (output.shapes.iter())
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        Rect::from_min_size(text.pos, text.galley.size()),
                        (text.galley.job.sections.first())
                            .map_or(0.0, |section| section.format.font_id.size),
                    )),
                    _ => None,
                })
                .collect();
            output.drop_without_applying_deltas();
        }

        fn find(&self, name: &str) -> Option<Rect> {
            (self.seen.iter())
                .find(|(seen, _)| seen == name)
                .map(|(_, rect)| *rect)
        }

        /// Where `text` was painted, and at what size.
        fn painted(&self, text: &str) -> Option<(Rect, f32)> {
            (self.words.iter())
                .find(|(painted, ..)| painted == text)
                .map(|(_, rect, size)| (*rect, *size))
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

    /// The names of what the Media tab lists, folders in brackets.
    fn listing(
        project: &Project,
        folder: Option<FolderId>,
        query: &str,
        kind: KindFilter,
    ) -> Vec<String> {
        listed(project, folder, query, kind, LibrarySort::default())
            .into_iter()
            .map(|item| match item {
                Item::Folder(folder) => format!("[{}]", folder.name),
                Item::Asset(asset) => asset.name.clone(),
            })
            .collect()
    }

    /// One of each kind at the top, and a folder holding a second sound.
    fn mixed_library() -> (Project, FolderId) {
        let mut project = Project::new("Library");
        let folder = MediaFolder {
            id: uuid::Uuid::new_v4(),
            name: "Beds".to_owned(),
            parent_id: None,
        };
        let folder_id = folder.id;
        let mut inside = asset("drone.wav", MediaKind::Audio, 60.0, 70);
        inside.folder_id = Some(folder_id);
        let assets = std::sync::Arc::make_mut(&mut project.assets);
        for held in [
            asset("beach.mp4", MediaKind::Video, 30.0, 900),
            asset("Alarm.wav", MediaKind::Audio, 4.0, 50),
            asset("cover.png", MediaKind::Image, 5.0, 300),
            asset("loop.gif", MediaKind::Gif, 2.0, 40),
            asset("film.cube", MediaKind::Lut, 1.0, 20),
            inside,
        ] {
            assets.insert(held.id, held);
        }
        std::sync::Arc::make_mut(&mut project.folders).insert(folder_id, folder);
        (project, folder_id)
    }

    #[test]
    fn the_kind_filter_lists_media_by_its_kind_in_the_folder_in_view() {
        let (project, folder) = mixed_library();
        let all = listing(&project, None, "", KindFilter::All);
        assert_eq!(
            all,
            [
                "[Beds]",
                "Alarm.wav",
                "beach.mp4",
                "cover.png",
                "film.cube",
                "loop.gif"
            ]
        );
        // The folders stay, to be walked into; the media is of one kind.
        assert_eq!(
            listing(&project, None, "", KindFilter::Video),
            ["[Beds]", "beach.mp4"]
        );
        assert_eq!(
            listing(&project, None, "", KindFilter::Sound),
            ["[Beds]", "Alarm.wav"]
        );
        assert_eq!(
            listing(&project, None, "", KindFilter::Images),
            ["[Beds]", "cover.png", "loop.gif"]
        );
        // Inside the folder it is that folder's media that is filtered.
        assert_eq!(
            listing(&project, Some(folder), "", KindFilter::Sound),
            ["drone.wav"]
        );
        assert!(listing(&project, Some(folder), "", KindFilter::Video).is_empty());
        // A search looks through every folder, within the kind.
        assert_eq!(
            listing(&project, None, "a", KindFilter::Sound),
            ["Alarm.wav", "drone.wav"]
        );
        assert_eq!(
            listing(&project, Some(folder), "a", KindFilter::Video),
            ["beach.mp4"]
        );
        assert!(listing(&project, None, "beach", KindFilter::Sound).is_empty());
        // Every kind of media is under exactly one filter besides All, but a
        // lookup table, which is under none.
        for kind in [
            MediaKind::Video,
            MediaKind::Audio,
            MediaKind::Image,
            MediaKind::Gif,
        ] {
            let under = KindFilter::ALL[1..]
                .iter()
                .filter(|filter| filter.admits(kind));
            assert_eq!(under.count(), 1, "{kind:?}");
        }
        assert!(
            !KindFilter::ALL[1..]
                .iter()
                .any(|filter| filter.admits(MediaKind::Lut))
        );

        // The sort is the one asked for, over what the filter leaves.
        let by_length = LibrarySort {
            by: SortBy::Duration,
            descending: true,
        };
        let sounds: Vec<&str> = listed(&project, None, "wav", KindFilter::Sound, by_length)
            .into_iter()
            .map(|item| match item {
                Item::Asset(asset) => asset.name.as_str(),
                Item::Folder(_) => panic!("a search lists no folder"),
            })
            .collect();
        assert_eq!(sounds, ["drone.wav", "Alarm.wav"]);
    }

    #[test]
    fn nothing_of_a_kind_says_so_and_where() {
        let sound = icon_of_media(MediaKind::Audio);
        assert_eq!(
            nothing_listed(KindFilter::Sound, "", false),
            (
                sound,
                "No sound",
                "Nothing of that kind is in this project."
            )
        );
        assert_eq!(
            nothing_listed(KindFilter::Sound, "", true),
            (sound, "No sound", "Nothing of that kind is in this folder.")
        );
        assert_eq!(nothing_listed(KindFilter::Video, "", true).1, "No video");
        assert_eq!(nothing_listed(KindFilter::Images, "", true).1, "No images");
        assert_eq!(
            nothing_listed(KindFilter::All, "", true),
            (Icon::Folder, "Empty folder", "")
        );
        assert_eq!(
            nothing_listed(KindFilter::Images, "beach", false),
            (
                Icon::Search,
                "No matches",
                "No image in this project has that name."
            )
        );
        assert_eq!(
            nothing_listed(KindFilter::All, "beach", false).2,
            "Nothing in this project has that name."
        );
    }

    /// The Media tab on a 320 by 600 point panel, drawn frame after frame
    /// with what the pointer does in each.
    struct Panel {
        ctx: egui::Context,
        state: LibraryState,
        selection: Selection,
        actions: Vec<Action>,
    }

    impl Panel {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            Self {
                ctx,
                state: LibraryState::default(),
                selection: Selection::default(),
                actions: Vec::new(),
            }
        }

        /// One frame; the words drawn in it, each with where it is.
        fn frame(&mut self, project: &Project, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
            let palette = Palette::new(crate::theme::Appearance::Dark, Default::default());
            let mut textures = Textures::default();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut titles = crate::titles::Titles::default();
            let presets = std::sync::Arc::<crate::presets::Presets>::default();
            let (state, selection, actions) =
                (&mut self.state, &mut self.selection, &mut self.actions);
            let mut output = self.ctx.run_ui(input, |ui| {
                let rect = ui.max_rect();
                show(
                    ui,
                    rect,
                    state,
                    selection,
                    &mut textures,
                    &mut titles,
                    &LibraryInput {
                        p: &palette,
                        project,
                        media: None,
                        grid: false,
                        sort: LibrarySort::default(),
                        importing: 0,
                        problem: None,
                        previewing: None,
                        playhead: Time::ZERO,
                        presets: &presets,
                        speech: captions_tab::SpeechInput {
                            view: &crate::speech::SpeechView::Idle,
                            settings: &crate::speech::SpeechSettings::default(),
                            inventory: None,
                        },
                    },
                    actions,
                );
            });
            output.textures_delta.clear();
            (output.shapes.iter())
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        Rect::from_min_size(text.pos, text.galley.size()),
                    )),
                    _ => None,
                })
                .collect()
        }

        /// A press and a release of `button` at `at`, and the frames that
        /// take them; the words of the last.
        fn click(
            &mut self,
            project: &Project,
            at: egui::Pos2,
            button: egui::PointerButton,
        ) -> Vec<(String, Rect)> {
            let pressed = |pressed| egui::Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(project, vec![egui::Event::PointerMoved(at)]);
            self.frame(project, vec![pressed(true)]);
            self.frame(project, vec![pressed(false)]);
            self.frame(project, Vec::new())
        }
    }

    fn place(words: &[(String, Rect)], wanted: &str) -> Option<Rect> {
        (words.iter())
            .find(|(text, _)| text == wanted)
            .map(|(_, rect)| *rect)
    }

    #[test]
    fn a_click_on_a_kind_hides_the_rest_and_an_empty_result_says_so() {
        let mut project = Project::new("Library");
        let assets = std::sync::Arc::make_mut(&mut project.assets);
        for held in [
            asset("beach.mp4", MediaKind::Video, 30.0, 900),
            asset("Alarm.wav", MediaKind::Audio, 4.0, 50),
        ] {
            assets.insert(held.id, held);
        }
        let mut panel = Panel::new();
        let words = panel.frame(&project, Vec::new());
        // The four kinds stand on one row, inside the panel, All chosen.
        let cells = KindFilter::ALL.map(|kind| place(&words, kind.label()).unwrap());
        for pair in cells.windows(2) {
            assert!(pair[0].right() < pair[1].left());
            assert!((pair[0].center().y - pair[1].center().y).abs() < 1.0);
        }
        assert!(cells[0].left() >= 10.0 && cells[3].right() <= 310.0);
        assert_eq!(panel.state.kind, KindFilter::All);
        assert!(place(&words, "beach.mp4").is_some() && place(&words, "Alarm.wav").is_some());

        let words = panel.click(&project, cells[2].center(), egui::PointerButton::Primary);
        assert_eq!(panel.state.kind, KindFilter::Sound);
        assert!(place(&words, "beach.mp4").is_none() && place(&words, "Alarm.wav").is_some());

        let words = panel.click(&project, cells[1].center(), egui::PointerButton::Primary);
        assert_eq!(panel.state.kind, KindFilter::Video);
        assert!(place(&words, "beach.mp4").is_some() && place(&words, "Alarm.wav").is_none());

        // Nothing of the kind: the list says so, and the row is still there
        // to choose another from.
        let words = panel.click(&project, cells[3].center(), egui::PointerButton::Primary);
        assert_eq!(panel.state.kind, KindFilter::Images);
        assert!(place(&words, "No images").is_some(), "{words:?}");
        assert!(place(&words, "Nothing of that kind is in this project.").is_some());
        assert!(place(&words, "beach.mp4").is_none() && place(&words, "Alarm.wav").is_none());
        let words = panel.click(&project, cells[0].center(), egui::PointerButton::Primary);
        assert_eq!(panel.state.kind, KindFilter::All);
        assert!(place(&words, "beach.mp4").is_some() && place(&words, "Alarm.wav").is_some());
        // Choosing a kind edits nothing.
        assert!(panel.actions.is_empty());
    }

    #[test]
    fn a_sounds_menu_offers_background_music_and_a_pictures_does_not() {
        let mut editor = editor_core::Editor::default();
        let song = asset("song.wav", MediaKind::Audio, 4.0, 50);
        let shot = asset("beach.mp4", MediaKind::Video, 30.0, 900);
        let song_id = song.id;
        editor
            .dispatch(Command::AddAssets {
                assets: vec![song, shot.clone()],
            })
            .unwrap();
        const ROW: &str = "Add as background music";

        // Nothing is on the timeline: the row is shown and cannot be chosen.
        let empty = editor.project().clone();
        let mut panel = Panel::new();
        let words = panel.frame(&empty, Vec::new());
        let song_at = place(&words, "song.wav").unwrap().center();
        let words = panel.click(&empty, song_at, egui::PointerButton::Secondary);
        let row = place(&words, ROW).expect("the row is offered of a sound");
        // Resting on it says why, once the pointer has been still a while.
        const REASON: &str =
            "Put a clip on the timeline first: background music is as long as the project.";
        assert!(place(&words, REASON).is_none());
        panel.frame(&empty, vec![egui::Event::PointerMoved(row.center())]);
        let mut said = false;
        for _ in 0..90 {
            let words = panel.frame(&empty, Vec::new());
            said |= (words.iter()).any(|(text, _)| text.replace('\n', " ") == REASON);
        }
        assert!(said, "the disabled row gives its reason");
        panel.click(&empty, row.center(), egui::PointerButton::Primary);
        assert!(panel.actions.is_empty(), "{:?}", panel.actions);

        // With a clip to lie under, choosing it asks for the bed of that sound.
        let track_id = editor.project().tracks[0].id;
        editor
            .dispatch(Command::AddClip {
                track_id,
                clip: editor_core::Clip::from_asset(&shot, Time::ZERO),
            })
            .unwrap();
        let project = editor.project().clone();
        let mut panel = Panel::new();
        let words = panel.frame(&project, Vec::new());
        let song_at = place(&words, "song.wav").unwrap().center();
        let shot_at = place(&words, "beach.mp4").unwrap().center();
        let words = panel.click(&project, song_at, egui::PointerButton::Secondary);
        let row = place(&words, ROW).expect("the row is offered of a sound");
        panel.click(&project, row.center(), egui::PointerButton::Primary);
        assert!(
            matches!(panel.actions.as_slice(), [Action::AddBackgroundMusic(asset)] if *asset == song_id),
            "{:?}",
            panel.actions
        );
        // What it asks for is the one step the rules make.
        editor
            .dispatch(crate::rules::background_music(editor.project(), song_id).unwrap())
            .unwrap();
        assert_eq!(editor.undo_label(), Some("Add background music"));

        // A video's menu has no such row.
        let mut panel = Panel::new();
        panel.frame(&project, Vec::new());
        let words = panel.click(&project, shot_at, egui::PointerButton::Secondary);
        assert!(place(&words, "Add to timeline").is_some(), "{words:?}");
        assert!(place(&words, ROW).is_none());
    }

    #[test]
    fn media_with_sound_offers_to_save_it_and_a_silent_picture_does_not() {
        let song = asset("song.wav", MediaKind::Audio, 4.0, 50);
        let mut talk = asset("talk.mp4", MediaKind::Video, 30.0, 900);
        talk.has_audio = true;
        let still = asset("cover.png", MediaKind::Image, 5.0, 300);
        let mut project = Project::new("Library");
        let assets = std::sync::Arc::make_mut(&mut project.assets);
        for held in [song.clone(), talk.clone(), still] {
            assets.insert(held.id, held);
        }
        const ROW: &str = crate::extract::LABEL;
        // Chosen of a sound and of a video with sound, it asks for that
        // media's sound by its id; nothing is edited.
        for (name, id) in [("song.wav", song.id), ("talk.mp4", talk.id)] {
            let mut panel = Panel::new();
            let words = panel.frame(&project, Vec::new());
            let at = place(&words, name).unwrap().center();
            let words = panel.click(&project, at, egui::PointerButton::Secondary);
            let row = place(&words, ROW).unwrap_or_else(|| panic!("{name}: {words:?}"));
            panel.click(&project, row.center(), egui::PointerButton::Primary);
            assert!(
                matches!(
                    panel.actions.as_slice(),
                    [Action::SaveAudio(crate::extract::Source::Asset(asked))] if *asked == id
                ),
                "{name}: {:?}",
                panel.actions
            );
        }
        // A still has no such row.
        let mut panel = Panel::new();
        let words = panel.frame(&project, Vec::new());
        let at = place(&words, "cover.png").unwrap().center();
        let words = panel.click(&project, at, egui::PointerButton::Secondary);
        assert!(place(&words, "Copy path").is_some(), "{words:?}");
        assert!(place(&words, ROW).is_none());
    }

    #[test]
    fn a_secondary_click_on_the_play_offer_opens_the_tiles_menu_and_a_click_plays() {
        let song = asset("song.wav", MediaKind::Audio, 4.0, 50);
        let song_id = song.id;
        let mut project = Project::new("Library");
        std::sync::Arc::make_mut(&mut project.assets).insert(song.id, song);
        // Where the offer to play is on a listed tile: the middle of its
        // picture, which stands left of the name.
        let offer = |words: &[(String, Rect)]| {
            let name = place(words, "song.wav").expect("the sound is listed");
            let side = ROW_HEIGHT - 2.0 - 8.0;
            pos2(
                name.left() - 10.0 - side * 16.0 / 9.0 * 0.5,
                name.top() - 6.0 + 4.0 + side * 0.5,
            )
        };

        // A click on it plays the sound and opens no menu.
        let mut panel = Panel::new();
        let words = panel.frame(&project, Vec::new());
        let at = offer(&words);
        let words = panel.click(&project, at, egui::PointerButton::Primary);
        assert!(
            matches!(panel.actions.as_slice(), [Action::PreviewAsset(asked)] if *asked == song_id),
            "{:?}",
            panel.actions
        );
        assert!(place(&words, "Add to timeline").is_none(), "{words:?}");

        // A secondary click on it opens the tile's own menu, as one elsewhere
        // on the tile does, and plays nothing.
        let mut panel = Panel::new();
        let words = panel.frame(&project, Vec::new());
        let elsewhere = place(&words, "song.wav").unwrap().center();
        let words = panel.click(&project, at, egui::PointerButton::Secondary);
        assert!(panel.actions.is_empty(), "{:?}", panel.actions);
        const ROWS: [&str; 4] = [
            "Preview",
            "Add to timeline",
            "Add as background music",
            "Copy path",
        ];
        let on_offer: Vec<&str> = (words.iter())
            .map(|(text, _)| text.as_str())
            .filter(|text| ROWS.contains(text))
            .collect();
        assert_eq!(on_offer, ROWS, "{words:?}");
        let mut panel = Panel::new();
        panel.frame(&project, Vec::new());
        let words = panel.click(&project, elsewhere, egui::PointerButton::Secondary);
        let on_tile: Vec<&str> = (words.iter())
            .map(|(text, _)| text.as_str())
            .filter(|text| on_offer.contains(text))
            .collect();
        assert_eq!(on_tile, on_offer);
        // Chosen from the menu opened on the offer, a row does what it says.
        let mut panel = Panel::new();
        panel.frame(&project, Vec::new());
        let words = panel.click(&project, at, egui::PointerButton::Secondary);
        let row = place(&words, "Add to timeline").expect("the menu is open");
        panel.click(&project, row.center(), egui::PointerButton::Primary);
        assert!(
            matches!(
                panel.actions.as_slice(),
                [Action::InsertAsset { asset, place: None }] if *asset == song_id
            ),
            "{:?}",
            panel.actions
        );
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

    #[test]
    fn the_six_tabs_are_named_at_the_default_width_and_are_icons_when_narrow() {
        // §5.2: TIGHT names at the default 316, ROOMY from 383, ICONS at 236.
        for (width, size) in [(316.0, 11.5), (383.0, 12.0)] {
            let mut shelf = Shelf::new(Project::new("Library"), true);
            shelf.size = vec2(width, 600.0);
            shelf.frame(Vec::new());
            shelf.frame(Vec::new());
            for tab in Tab::ALL {
                let (painted, set) = (shelf.painted(tab.label()))
                    .unwrap_or_else(|| panic!("{} is named at {width}", tab.label()));
                assert_eq!(set, size, "{} at {width}", tab.label());
                assert!(
                    painted.left() >= LEADING && painted.right() <= width - TRAILING,
                    "{} at {width}: {painted:?}",
                    tab.label()
                );
                let tab_rect = shelf.find(tab.label()).expect("each tab is named");
                assert!(tab_rect.contains_rect(painted), "{}", tab.label());
            }
        }
        let mut shelf = Shelf::new(Project::new("Library"), true);
        shelf.size = vec2(236.0, 600.0);
        shelf.frame(Vec::new());
        shelf.frame(Vec::new());
        let mut cells = Vec::new();
        for tab in Tab::ALL {
            assert!(
                shelf.painted(tab.label()).is_none(),
                "{} is its icon",
                tab.label()
            );
            let cell = shelf.find(tab.label()).expect("an icon keeps its name");
            assert!(cell.width() >= 24.0 && cell.right() <= 236.0, "{cell:?}");
            cells.push(cell);
        }
        for pair in cells.windows(2) {
            assert!(pair[0].right() <= pair[1].left() + 0.5, "{pair:?}");
        }
    }

    #[test]
    fn the_footer_stays_in_place_while_the_media_scrolls() {
        let mut project = Project::new("Library");
        for index in 0..40 {
            let clip = asset(&format!("clip {index:02}.mp4"), MediaKind::Video, 4.0, 10);
            Arc::make_mut(&mut project.assets).insert(clip.id, clip);
        }
        let mut shelf = Shelf::new(project, false);
        shelf.frame(Vec::new());
        shelf.frame(Vec::new());
        let gear_name = format!("Preferences ({})", shortcut("Cmd+,"));
        let import = shelf
            .find("Import media or add a folder")
            .expect("the footer imports");
        let gear = shelf
            .find(&gear_name)
            .expect("the footer opens Preferences");
        for control in [import, gear] {
            assert!(
                control.top() >= 600.0 - metrics::PANE_FOOTER && control.bottom() <= 600.0,
                "{control:?}"
            );
        }
        let (first, _) = shelf.painted("clip 00.mp4").expect("the first is listed");
        shelf.frame(vec![egui::Event::PointerMoved(pos2(160.0, 300.0))]);
        shelf.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }]);
        for _ in 0..20 {
            shelf.frame(Vec::new());
        }
        let moved = shelf
            .painted("clip 00.mp4")
            .is_none_or(|(now, _)| now.top() < first.top() - 100.0);
        assert!(moved, "the list scrolled");
        assert_eq!(shelf.find("Import media or add a folder"), Some(import));
        assert_eq!(shelf.find(&gear_name), Some(gear));

        // The gear asks for Preferences; the import button opens its menu.
        shelf.click(gear.center(), egui::PointerButton::Primary);
        assert!(
            matches!(shelf.actions.as_slice(), [Action::OpenSettings]),
            "{:?}",
            shelf.actions
        );
        shelf.click(import.center(), egui::PointerButton::Primary);
        for row in ["Import files…", "Import folder…", "New folder"] {
            assert!(shelf.find(row).is_some(), "the menu offers {row}");
        }
        let row = shelf.find("Import files…").unwrap();
        shelf.click(row.center(), egui::PointerButton::Primary);
        assert!(matches!(shelf.actions.last(), Some(Action::ImportFiles)));
    }

    #[test]
    fn view_options_carry_the_layout_and_the_order() {
        let mut project = Project::new("Library");
        for held in [
            asset("beach.mp4", MediaKind::Video, 30.0, 900),
            asset("Alarm.wav", MediaKind::Audio, 4.0, 50),
        ] {
            Arc::make_mut(&mut project.assets).insert(held.id, held);
        }
        let mut shelf = Shelf::new(project, false);
        shelf.frame(Vec::new());
        shelf.frame(Vec::new());
        // One control on the search row, where three were.
        for gone in ["Sort media", "Show as grid", "Show as list"] {
            assert!(shelf.find(gone).is_none(), "{gone} is in the menu now");
        }
        let view = shelf
            .find("View options")
            .expect("the Media tab has its options");
        let search = shelf.words.iter().any(|(text, ..)| text == "Search media");
        assert!(search, "the field says what it searches");
        shelf.click(view.center(), egui::PointerButton::Primary);
        for row in [
            "Show as grid",
            "Show as list",
            "Name",
            "Duration",
            "Kind",
            "File size",
            "Ascending",
            "Descending",
        ] {
            assert!(shelf.find(row).is_some(), "the menu offers {row}");
        }
        let row = shelf.find("Show as grid").unwrap();
        shelf.click(row.center(), egui::PointerButton::Primary);
        assert!(shelf.grid, "the media is laid out as cards");
        for (row, wanted) in [("Duration", SortBy::Duration), ("File size", SortBy::Size)] {
            shelf.click(view.center(), egui::PointerButton::Primary);
            let at = shelf.find(row).unwrap();
            shelf.click(at.center(), egui::PointerButton::Primary);
            assert_eq!(shelf.sort.by, wanted);
        }
        shelf.click(view.center(), egui::PointerButton::Primary);
        let at = shelf.find("Descending").unwrap();
        shelf.click(at.center(), egui::PointerButton::Primary);
        assert!(shelf.sort.descending);
        assert!(shelf.actions.is_empty(), "the view edits nothing");
    }
}
