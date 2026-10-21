//! A clip's masks: the shapes that say how much of its picture is kept.
//!
//! The rows of a mask are made from `look::mask_spec`. Every change is the
//! whole new list, held in the draft and committed as one `SetMasks`. The
//! mask whose rows are open is the one the viewer outlines.
use super::{
    button, button_row, card, card_head, choice, clip::Pane, fitted_row, head_button, keyed_card,
    note, section,
};
use crate::{
    icons::Icon,
    ui::{
        ClipDraft,
        widgets::{self, Kind},
    },
};
use editor_core::{AnimTarget, Mask, MaskOp, MaskShape, ParamKey, look};
use eframe::egui::{self, Ui};

const SHAPES: [(MaskShape, &str); 2] = [
    (MaskShape::Rectangle, "Rectangle"),
    (MaskShape::Ellipse, "Ellipse"),
];

const OPS: [(MaskOp, &str); 3] = [
    (MaskOp::Add, "Add"),
    (MaskOp::Subtract, "Subtract"),
    (MaskOp::Intersect, "Intersect"),
];

fn shape_name(shape: MaskShape) -> &'static str {
    SHAPES
        .iter()
        .find(|(listed, _)| *listed == shape)
        .map_or("Mask", |(_, name)| name)
}

fn set_param(mask: &mut Mask, key: ParamKey, value: f32) {
    match key {
        ParamKey::CenterX => mask.center[0] = value,
        ParamKey::CenterY => mask.center[1] = value,
        ParamKey::Width => mask.size[0] = value,
        ParamKey::Height => mask.size[1] = value,
        ParamKey::Rotation => mask.rotation = value,
        ParamKey::Feather => mask.feather = value,
        ParamKey::Roundness => mask.roundness = value,
        _ => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    Remove(usize),
    Add(MaskShape),
}

impl Pane<'_> {
    pub(super) fn masks(&mut self, ui: &mut Ui) {
        let p = self.p;
        let clip = self.clip;
        // The list is read where it is and copied only when it changes.
        let held = self.next.masks.take();
        let list: &[Mask] = held.as_deref().unwrap_or(&clip.masks);
        let mut edited: Vec<(usize, Mask)> = Vec::new();
        let mut change: Option<Change> = None;

        section(ui, p, "Masks");
        for (index, mask) in list.iter().enumerate() {
            ui.push_id(mask.id, |ui| {
                keyed_card(ui, p, |ui| {
                    if let Some(mask) = self.mask_rows(ui, mask, index, &mut change) {
                        edited.push((index, mask));
                    }
                });
            });
        }
        if list.is_empty() {
            card(ui, p, |ui| note(ui, p, "No masks on this clip."));
        }
        button_row(ui, p, |ui| {
            ui.add_enabled_ui(list.len() < look::MAX_MASKS, |ui| {
                let add = button(ui, p, Some(Icon::Plus), "Add mask", Kind::Plain)
                    .on_disabled_hover_text("A clip holds eight masks at most");
                egui::Popup::menu(&add).show(|ui| {
                    widgets::menu_style(ui, 160.0);
                    for (shape, name) in SHAPES {
                        if widgets::menu_item(ui, p, None, name, "").clicked() {
                            change = Some(Change::Add(shape));
                        }
                    }
                });
            });
        });

        self.next.masks = if edited.is_empty() && change.is_none() {
            held
        } else {
            let mut masks = list.to_vec();
            for (index, mask) in edited {
                masks[index] = mask;
            }
            match change {
                Some(Change::Remove(index)) => {
                    let removed = masks.remove(index).id;
                    self.next.keys.retain(|key| key.target != Some(removed));
                }
                Some(Change::Add(shape)) => {
                    let mask = Mask::new(shape);
                    self.state.mask = Some(mask.id);
                    masks.push(mask);
                }
                None => {}
            }
            (masks != clip.masks).then_some(masks)
        };
    }

    /// One mask: its heading, and its rows while it is open. Hands back the
    /// mask as edited, when it was.
    fn mask_rows(
        &mut self,
        ui: &mut Ui,
        mask: &Mask,
        index: usize,
        change: &mut Option<Change>,
    ) -> Option<Mask> {
        let p = self.p;
        let name = format!("{} {}", shape_name(mask.shape), index + 1);
        let open = self.state.mask == Some(mask.id);
        let mut edited: Option<Mask> = None;
        let mut edit = |change: &dyn Fn(&mut Mask)| {
            change(edited.get_or_insert_with(|| mask.clone()));
        };

        let header = card_head(
            ui,
            p,
            &name,
            Some(open),
            false,
            (1, |ui: &mut Ui| {
                if head_button(ui, p, Icon::Trash, true, &format!("Remove {name}")) {
                    *change = Some(Change::Remove(index));
                }
            }),
        );
        if header.is_some_and(|header| header.clicked()) {
            self.state.mask = (!open).then_some(mask.id);
        }
        if !open {
            return edited;
        }

        let mut shape = mask.shape;
        if choice(ui, p, &mut shape, &SHAPES) {
            edit(&|mask| mask.shape = shape);
        }
        let mut op = mask.op;
        if choice(ui, p, &mut op, &OPS) {
            edit(&|mask| mask.op = op);
        }
        fitted_row(ui, p, "Invert", |ui| {
            if widgets::switch(ui, p, mask.invert, &format!("Invert {name}")).clicked() {
                edit(&|mask| mask.invert = !mask.invert);
            }
        });
        let stored_of = |_: &ClipDraft, key: ParamKey| mask.param(key).unwrap_or(0.0);
        for spec in look::mask_spec() {
            // Only a rectangle has corners to round.
            if spec.key == ParamKey::Roundness && mask.shape != MaskShape::Rectangle {
                continue;
            }
            let stored = mask.param(spec.key).unwrap_or(spec.default);
            if let Some(value) =
                self.param_row(ui, mask.id, AnimTarget::Mask, spec, stored, &stored_of)
            {
                edit(&|mask| set_param(mask, spec.key, value));
            }
        }
        if index == 0 && mask.op != MaskOp::Add {
            note(ui, p, "The first mask takes from the whole picture.");
        }
        edited
    }
}
