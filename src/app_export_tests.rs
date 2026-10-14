//! The export chords, read from real egui input.
use super::*;

/// The action `chord` gives with `overlay` open, and whether the chord was
/// taken from the input.
fn pressed(modifiers: Modifiers, key: Key, overlay: &Overlay) -> (Option<Action>, bool) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    let mut answer = (None, false);
    let output = ctx.run_ui(raw, |ui| {
        let action = ui.ctx().input_mut(|i| export_chords(i, overlay));
        let left = ui.ctx().input(|i| {
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::Key { key: k, .. } if *k == key))
        });
        answer = (action, !left);
    });
    output.drop_without_applying_deltas();
    answer
}

#[test]
fn cmd_e_opens_the_export_sheet_and_shift_cmd_e_the_queue() {
    let (action, taken) = pressed(Modifiers::COMMAND, Key::E, &Overlay::None);
    assert!(matches!(action, Some(Action::OpenExport)), "{action:?}");
    assert!(taken);
    let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
    let (action, taken) = pressed(shifted, Key::E, &Overlay::None);
    assert!(
        matches!(action, Some(Action::OpenRenderQueue)),
        "{action:?}"
    );
    assert!(taken, "the shifted chord is not also read as Cmd+E");
    let (action, _) = pressed(Modifiers::NONE, Key::E, &Overlay::None);
    assert!(action.is_none(), "E alone is not a chord");
}

#[test]
fn the_chords_neither_reopen_nor_stack_over_a_sheet() {
    let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
    for overlay in [
        Overlay::Export,
        Overlay::RenderQueue,
        Overlay::Settings,
        Overlay::StopRendering,
        Overlay::ReplaceExport("Film.mp4".into()),
        Overlay::Unsaved(Pending::Quit),
    ] {
        for modifiers in [Modifiers::COMMAND, shifted] {
            let (action, taken) = pressed(modifiers, Key::E, &overlay);
            // Over the export sheet the queue's chord, and over the queue
            // the sheet's, switch between the two.
            let switches = (overlay == Overlay::Export && modifiers == shifted)
                || (overlay == Overlay::RenderQueue && modifiers == Modifiers::COMMAND);
            assert_eq!(action.is_some(), switches, "{overlay:?}: {action:?}");
            assert!(taken, "{overlay:?}: the chord is still taken");
        }
    }
}

#[test]
fn the_chords_switch_between_the_export_sheet_and_the_queue() {
    let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
    let (action, _) = pressed(shifted, Key::E, &Overlay::Export);
    assert!(
        matches!(action, Some(Action::OpenRenderQueue)),
        "Shift+Cmd+E over the export sheet shows the queue: {action:?}"
    );
    let (action, _) = pressed(Modifiers::COMMAND, Key::E, &Overlay::RenderQueue);
    assert!(
        matches!(action, Some(Action::OpenExport)),
        "Cmd+E over the queue shows the export sheet: {action:?}"
    );
}
