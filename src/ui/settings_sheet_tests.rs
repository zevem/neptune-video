//! Preferences' Preview pane, driven as a person drives it: a control is
//! found by the name accessibility is given for it and pressed with the
//! pointer.
use super::*;
use crate::theme::{Accent, Appearance};
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, accesskit::Role};

struct Seen {
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
    selected: Option<bool>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    settings: Settings,
    cache: Cache,
    seen: Vec<Seen>,
    painted: Vec<String>,
    /// The rectangles painted this frame, filled or outlined.
    rects: Vec<egui::epaint::RectShape>,
    actions: Vec<Action>,
    /// Whether the sheet asked to close this frame.
    closed: bool,
}

impl Bench {
    fn new(settings: Settings, cache: Cache) -> Self {
        let mut bench = Self::opened(settings, cache);
        // §5.9.2: the Preview settings are a pane of their own, chosen from
        // the source list.
        let at = bench.get("Preview").rect.center();
        bench.click(at);
        bench
    }

    /// Preferences just opened, as Ctrl+, opens it, with nothing done in it.
    fn opened(settings: Settings, cache: Cache) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut bench = Self {
            ctx,
            palette,
            settings,
            cache,
            seen: Vec::new(),
            painted: Vec::new(),
            rects: Vec::new(),
            actions: Vec::new(),
            closed: false,
        };
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        bench
    }

    fn frame(&mut self, events: Vec<Event>) {
        let raw = egui::RawInput {
            // Tall enough that the whole sheet is in sight.
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 3200.0))),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let output = ctx.run_ui(raw, |ui| {
            self.closed = settings(
                ui.ctx(),
                &self.palette,
                &self.settings,
                self.cache,
                &mut self.actions,
            );
        });
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        disabled: node.is_disabled(),
                        selected: (node.toggled())
                            .map(|toggled| toggled == egui::accesskit::Toggled::True),
                    })
                })
                .collect();
        }
        self.painted.clear();
        self.rects.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.painted, &mut self.rects);
        }
        output.drop_without_applying_deltas();
    }

    /// The controls named `name`, top to bottom.
    fn all(&self, name: &str) -> Vec<&Seen> {
        let mut found: Vec<&Seen> = (self.seen.iter())
            .filter(|seen| seen.name == name && !matches!(seen.role, Role::Label | Role::TextRun))
            .collect();
        found.sort_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
        found
    }

    fn get(&self, name: &str) -> &Seen {
        self.all(name)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("FIXTURE: no control is named \"{name}\""))
    }

    fn says(&self, text: &str) -> bool {
        self.painted.iter().any(|painted| painted.contains(text))
            || self.seen.iter().any(|seen| seen.name.contains(text))
    }

    fn click(&mut self, at: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
    }
}

fn collect(shape: &egui::Shape, into: &mut Vec<String>, rects: &mut Vec<egui::epaint::RectShape>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, into, rects)),
        egui::Shape::Text(text) => into.push(text.galley.text().to_owned()),
        egui::Shape::Rect(rect) => rects.push(rect.clone()),
        _ => {}
    }
}

fn cache(bytes: u64, proxies: usize) -> Cache {
    Cache {
        available: true,
        bytes,
        proxies,
    }
}

#[test]
fn the_preview_section_sets_the_quality_and_the_proxy_mode() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    for (name, nth, wanted) in [
        ("Half", 0, "quality Half"),
        ("Quarter", 0, "quality Quarter"),
        ("Full", 0, "quality Full"),
        ("Always", 0, "proxies Always"),
        ("Off", 0, "proxies Off"),
    ] {
        let at = bench.all(name)[nth].rect.center();
        bench.actions.clear();
        bench.click(at);
        let got = match bench.actions.as_slice() {
            [Action::SetPreviewQuality(quality)] => format!("quality {}", quality.name()),
            [Action::SetProxyMode(mode)] => format!("proxies {}", mode.name()),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(got, wanted);
    }
    // The second Auto is the proxies'.
    bench.settings.preview.quality = PreviewQuality::Full;
    bench.settings.preview.proxies = ProxyMode::Off;
    bench.frame(Vec::new());
    let autos = bench.all("Auto");
    assert_eq!(autos.len(), 2, "one Auto for each");
    let (quality, proxies) = (autos[0].rect.center(), autos[1].rect.center());
    bench.actions.clear();
    bench.click(quality);
    assert!(matches!(
        bench.actions.as_slice(),
        [Action::SetPreviewQuality(PreviewQuality::Auto)]
    ));
    bench.actions.clear();
    bench.click(proxies);
    assert!(matches!(
        bench.actions.as_slice(),
        [Action::SetProxyMode(ProxyMode::Auto)]
    ));
}

#[test]
fn the_preview_section_says_what_proxies_are_and_what_the_cache_holds() {
    let mut settings = Settings::default();
    settings.preview.quality = PreviewQuality::Half;
    let bench = Bench::new(settings, cache(3_435_973_837, 4));
    assert!(bench.says("Proxies are smaller copies used only for preview"));
    assert!(bench.says("exports always use the"));
    assert!(bench.says(quality_note(PreviewQuality::Half)));
    assert!(bench.says("3.2 GB of 20 GB"), "{:?}", bench.painted);
    assert!(!bench.get("Clear proxies").disabled);
    assert_eq!(bench.get("Proxy cache size").role, Role::ComboBox);
}

#[test]
fn clear_proxies_asks_to_clear_and_waits_for_something_to_clear() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    assert!(bench.get("Clear proxies").disabled, "nothing to clear");
    assert!(bench.says("Nothing of 20 GB"));
    let at = bench.get("Clear proxies").rect.center();
    bench.click(at);
    assert!(bench.actions.is_empty(), "{:?}", bench.actions);

    let mut bench = Bench::new(Settings::default(), cache(10 << 20, 1));
    let at = bench.get("Clear proxies").rect.center();
    bench.click(at);
    assert!(matches!(bench.actions.as_slice(), [Action::ClearProxies]));
}

#[test]
fn clear_proxies_says_they_are_not_made_again_unasked() {
    // Clearing declines every video's proxy until the person asks again or
    // the project is opened again; the tooltip says what happens.
    let mut bench = Bench::new(Settings::default(), cache(10 << 20, 1));
    let at = bench.get("Clear proxies").rect.center();
    bench.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..90 {
        bench.frame(Vec::new());
    }
    assert!(
        bench.says("None is made again until you ask for one or reopen the project."),
        "{:?}",
        bench.painted
    );
    assert!(!bench.says("made again."), "{:?}", bench.painted);
}

#[test]
fn a_cache_size_is_chosen_from_the_list() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let at = bench.get("Proxy cache size").rect.center();
    bench.click(at);
    let at = (bench.all("50 GB").first())
        .expect("the sizes are listed")
        .rect
        .center();
    bench.click(at);
    assert!(
        matches!(bench.actions.as_slice(), [Action::SetProxyBudget(50)]),
        "{:?}",
        bench.actions
    );
}

#[test]
fn without_proxies_the_section_says_so_and_offers_no_cache() {
    let bench = Bench::new(Settings::default(), Cache::default());
    assert!(bench.says("Proxies are unavailable"));
    assert!(bench.all("Clear proxies").is_empty());
    assert!(bench.all("Proxy cache size").is_empty());
}

#[test]
fn cache_usage_reads_in_the_budget_s_gigabytes() {
    assert_eq!(cache_usage(0, 20), "Nothing of 20 GB");
    assert_eq!(cache_usage(1, 20), "1 MB of 20 GB");
    assert_eq!(cache_usage(50 << 20, 20), "50 MB of 20 GB");
    assert_eq!(cache_usage(3_435_973_837, 20), "3.2 GB of 20 GB");
    assert_eq!(cache_usage(20 << 30, 20), "20.0 GB of 20 GB");
}

#[test]
fn preferences_opens_on_the_pane_it_is_asked_for() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    assert_eq!(bench.get("Preview").selected, Some(true));
    assert_eq!(bench.get("Appearance").selected, Some(false));
    // As Ctrl+, and the agent's "Model…" leave it for the sheet.
    PreferencesPane::Keyboard.show(&bench.ctx);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.get("Keyboard").selected, Some(true));
    assert!(bench.says("Play or pause"), "{:?}", bench.painted);
    assert!(bench.says("Show or hide the library"));
    assert!(bench.all("Auto").is_empty(), "one pane at a time");
    // Choosing a pane is kept for the next time the sheet is shown.
    let at = bench.get("Agent").rect.center();
    bench.click(at);
    assert_eq!(PreferencesPane::shown(&bench.ctx), PreferencesPane::Agent);
    assert_eq!(bench.get("Claude").role, Role::RadioButton);
}

#[test]
fn a_search_finds_settings_across_panes_and_says_when_nothing_is() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let at = bench.get("Search settings").rect.center();
    bench.click(at);
    bench.frame(vec![Event::Text("proxy".to_owned())]);
    bench.frame(Vec::new());
    assert!(bench.says("Results"));
    assert!(bench.says("Preview · Proxy media"), "{:?}", bench.painted);
    assert!(!bench.all("Proxy cache size").is_empty());
    // A pane with nothing found says so in its ink, not by going away.
    assert!(!bench.all("Keyboard").is_empty());

    bench.frame(vec![Event::Text("zzqx".to_owned())]);
    bench.frame(Vec::new());
    assert!(bench.says("No settings found"));
    assert!(bench.says("Try another word for it."));
}

#[test]
fn appearance_offers_the_system_s_nine_accents_and_reduce_motion() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let at = bench.get("Appearance").rect.center();
    bench.click(at);
    for name in ["Light", "Dark", "System"] {
        assert_eq!(bench.get(name).role, Role::RadioButton, "{name}");
    }
    let accents: Vec<_> = Accent::ALL
        .iter()
        .map(|(_, name)| bench.get(name).rect.left())
        .collect();
    assert!(
        accents.windows(2).all(|pair| pair[0] < pair[1]),
        "Blue leads: {accents:?}"
    );
    assert_eq!(bench.get("Blue").selected, Some(true));
    for (name, wanted) in [
        ("System", "appearance System"),
        ("Yellow", "accent Yellow"),
        ("Reduce motion", "reduce motion true"),
        ("Reset layout", "reset layout"),
    ] {
        let at = bench.get(name).rect.center();
        bench.actions.clear();
        bench.click(at);
        let got = match bench.actions.as_slice() {
            [Action::SetAppearance(appearance)] => format!("appearance {appearance:?}"),
            [Action::SetAccent(accent)] => format!("accent {accent:?}"),
            [Action::SetReduceMotion(on)] => format!("reduce motion {on}"),
            [Action::ResetLayout] => "reset layout".to_owned(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(got, wanted);
    }
    // At 100% the size goes nowhere by its own cell, and each step is named
    // with its keys.
    assert!(bench.get("100%").disabled);
    assert!(!bench.all(&shortcut("Larger (Cmd+=)")).is_empty());
    assert!(!bench.all(&shortcut("Smaller (Cmd+-)")).is_empty());
}

impl Bench {
    /// The labels said, as accessibility reads them: line by line, and
    /// leading to trailing along a line.
    fn labels(&self) -> Vec<&str> {
        let mut labels: Vec<&Seen> = (self.seen.iter())
            .filter(|seen| seen.role == Role::Label)
            .collect();
        let line = |seen: &Seen| (seen.rect.center().y.round(), seen.rect.left());
        labels.sort_by(|a, b| line(a).partial_cmp(&line(b)).expect("no NaN"));
        labels.iter().map(|seen| seen.name.as_str()).collect()
    }

    fn escape(&mut self) {
        self.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(Vec::new());
    }
}

#[test]
fn every_card_is_titled_and_never_as_its_pane() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    for (pane, card) in [
        ("Appearance", "Window"),
        ("Editing", "While editing"),
        ("Preview", "Quality"),
        ("Agent", "Requests"),
        ("Keyboard", "Playback"),
    ] {
        let at = bench.get(pane).rect.center();
        bench.click(at);
        // The content's header, then the first card's title (§5.9.2: a
        // section label above each card, as in Terminal), which is not the
        // pane's name again.
        let content: Vec<&str> = (bench.labels().into_iter())
            .filter(|&label| label != "Preferences")
            .collect();
        assert_eq!(content.get(..2), Some(&[pane, card][..]), "{content:?}");
    }
    // The proxies' card is not titled as its first row.
    let at = bench.get("Preview").rect.center();
    bench.click(at);
    assert!(bench.labels().contains(&"Proxy media"));
    assert_eq!(
        (bench.labels().iter())
            .filter(|&&label| label == "Proxies")
            .count(),
        1
    );
}

#[test]
fn a_system_appearance_says_what_the_system_chose() {
    let settings = Settings {
        appearance: Appearance::System,
        ..Settings::default()
    };
    let mut bench = Bench::new(settings, cache(0, 0));
    let at = bench.get("Appearance").rect.center();
    bench.click(at);
    // A window that has heard nothing from the system is dark.
    assert!(
        bench.says("Your system has not reported light or dark mode, so Neptune Video is dark."),
        "{:?}",
        bench.painted
    );
    bench.settings.appearance = Appearance::Dark;
    bench.frame(Vec::new());
    assert!(!bench.says("Your system"));
}

#[test]
fn the_keyboard_pane_says_each_action_and_its_keys_apart() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let at = bench.get("Keyboard").rect.center();
    bench.click(at);
    // §7.1: the names the rows always had, the action and then its keys.
    let labels = bench.labels();
    for (what, keys) in [
        ("Play or pause", "Space".to_owned()),
        ("Back or forward one second", shortcut("Shift+← →")),
    ] {
        let at = (labels.iter())
            .position(|&label| label == what)
            .unwrap_or_else(|| panic!("{what}: {labels:?}"));
        assert_eq!(labels.get(at + 1), Some(&keys.as_str()), "{labels:?}");
    }
    assert!(
        !labels.iter().any(|label| label.contains(": ")),
        "{labels:?}"
    );
}

// §3.10 and §9.12: a pane in the source list lies on `hover` under the
// pointer and on `pressed` while held or chosen, and rings with the
// keyboard.
#[test]
fn a_source_row_lies_on_hover_and_pressed_and_rings_with_the_keyboard() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let p = bench.palette;
    let row = bench.get("Keyboard").rect;
    let near = |a: Rect, b: Rect| (a.min - b.min).length() + (a.max - b.max).length() < 0.1;
    let lies = |bench: &Bench, fill: Color32| {
        (bench.rects.iter()).any(|shape| near(shape.rect, row) && shape.fill == fill)
    };
    let ringed = |bench: &Bench| {
        let ring = theme::tint(p.accent, 0.55);
        (bench.rects.iter()).any(|shape| near(shape.rect, row) && shape.stroke.color == ring)
    };
    assert!(
        !lies(&bench, p.hover) && !lies(&bench, p.pressed),
        "at rest"
    );
    assert!(!ringed(&bench));
    bench.frame(vec![Event::PointerMoved(row.center())]);
    assert!(lies(&bench, p.hover), "under the pointer");
    let button = |at, pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    bench.frame(vec![button(row.center(), true)]);
    assert!(lies(&bench, p.pressed), "held");
    // Let go beside it, so the pane stays as it was.
    let beside = pos2(row.right() + 200.0, row.center().y);
    bench.frame(vec![Event::PointerMoved(beside)]);
    bench.frame(vec![button(beside, false), Event::PointerGone]);
    assert_eq!(bench.get("Keyboard").selected, Some(false));
    assert!(!lies(&bench, p.hover) && !lies(&bench, p.pressed));
    let id = Id::new(("preferences-pane", "Keyboard"));
    bench.ctx.memory_mut(|memory| memory.request_focus(id));
    bench.frame(Vec::new());
    assert!(ringed(&bench), "with the keyboard");
    // The chosen pane stays pressed.
    let chosen = bench.get("Preview").rect;
    assert!((bench.rects.iter()).any(|shape| near(shape.rect, chosen) && shape.fill == p.pressed));
}

#[test]
fn two_folders_of_one_name_are_each_removed_by_their_own_remove() {
    let mut settings = Settings::default();
    settings.ai.folders = vec![
        PathBuf::from("/Volumes/A/Footage"),
        PathBuf::from("/home/someone/B/Footage"),
    ];
    for (row, wanted) in [(1, 1), (0, 0)] {
        let mut bench = Bench::new(settings.clone(), cache(0, 0));
        let at = bench.get("Agent").rect.center();
        bench.click(at);
        let removes: Vec<Pos2> = (bench.all("Remove").iter())
            .map(|seen| seen.rect.center())
            .collect();
        assert_eq!(removes.len(), 2, "each folder has its own Remove");
        bench.actions.clear();
        bench.click(removes[row]);
        assert!(
            matches!(bench.actions.as_slice(), [Action::RemoveAgentFolder(index)] if *index == wanted),
            "the Remove of row {row}: {:?}",
            bench.actions
        );
    }
}

#[test]
fn escape_closes_an_open_list_before_it_leaves_a_search() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    let at = bench.get("Search settings").rect.center();
    bench.click(at);
    bench.frame(vec![Event::Text("cache".to_owned())]);
    bench.frame(Vec::new());
    assert!(bench.says("Results"));
    let at = bench.get("Proxy cache size").rect.center();
    bench.click(at);
    assert!(!bench.all("50 GB").is_empty(), "FIXTURE: the list is open");
    // §7.3: one Escape closes the innermost thing, once.
    bench.escape();
    assert!(bench.all("50 GB").is_empty(), "the list is closed");
    assert!(bench.says("Results"), "the search is still there");
    bench.escape();
    assert!(!bench.says("Results"), "then the search is left");
}

#[test]
fn preferences_opens_ready_to_search_and_one_escape_closes_it() {
    let mut bench = Bench::opened(Settings::default(), cache(0, 0));
    let search = controls::viewport_id(&bench.ctx, Id::new("preferences-search"));
    // §5.9.2, as in Terminal: the search has the keyboard as the sheet
    // appears, so what is typed straight away searches.
    assert_eq!(bench.ctx.memory(|memory| memory.focused()), Some(search));
    bench.frame(vec![Event::Text("proxy".to_owned())]);
    bench.frame(Vec::new());
    assert!(bench.says("Results"));
    assert!(!bench.all("Proxy cache size").is_empty());
    // The first Escape leaves the search and keeps the sheet.
    bench.escape();
    assert!(!bench.says("Results"));
    assert!(!bench.closed);

    let mut bench = Bench::opened(Settings::default(), cache(0, 0));
    assert_eq!(bench.ctx.memory(|memory| memory.focused()), Some(search));
    // With nothing searched, leaving the field is not an Escape of its own:
    // one Escape closes Preferences.
    bench.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(bench.closed, "one Escape closes the sheet");
}

#[test]
fn the_editing_and_agent_panes_name_each_control() {
    let mut bench = Bench::new(Settings::default(), cache(0, 0));
    // The names the controls always had, each with its role, so a reader
    // finds them as before the panes were split.
    for (pane, controls) in [
        (
            "Editing",
            &[
                ("Snap while dragging", Role::CheckBox),
                ("Keep a recovery copy", Role::CheckBox),
            ][..],
        ),
        (
            "Agent",
            &[
                ("Claude", Role::RadioButton),
                ("Codex", Role::RadioButton),
                ("Default", Role::RadioButton),
                ("Low", Role::RadioButton),
                ("Medium", Role::RadioButton),
                ("High", Role::RadioButton),
                ("As it works", Role::RadioButton),
                ("For review", Role::RadioButton),
                ("Add folder…", Role::Button),
            ][..],
        ),
    ] {
        let at = bench.get(pane).rect.center();
        bench.click(at);
        assert_eq!(bench.get(pane).selected, Some(true), "{pane}");
        for &(name, role) in controls {
            assert_eq!(bench.get(name).role, role, "{pane}: {name}");
        }
    }
    // The agent's model is a field named for what it holds.
    assert!(!bench.all("Model").is_empty());
}
