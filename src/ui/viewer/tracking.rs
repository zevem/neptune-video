//! Motion tracking on the canvas: the box being chosen for a tracker, and
//! the paths of the trackers the selected clip has to do with.
//!
//! Both are drawn on the picture of the clip that shows the tracked media,
//! through its place, turn, crop and flips, and through the reading of a
//! picture drawn steadied, in fractions of the source frame: a box drawn
//! over something on the canvas is the place of the frame that is drawn
//! there.
use super::{Placed, ViewerState, frame, prompt};
use crate::{
    compose,
    theme::{self, Palette},
    ui::{Selection, TrackRegion},
};
use editor_core::{ClipId, Project, Reading, RenderPlan, Time, Transform, render};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, vec2,
};

/// How a region is held by the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RegionHold {
    /// A new box from this corner, in fractions of the source frame.
    Draw { anchor: [f32; 2] },
    /// The box by a place this far from its centre.
    Move { grab: [f32; 2] },
}

/// A clip's picture on screen: where a place in its source frame is drawn,
/// and which place of the source is under a point of the screen.
#[derive(Clone, Copy)]
pub(super) struct Picture<'a> {
    place: &'a Placed,
    /// The picture's uv at the left top and right bottom of the box.
    uv: [f32; 4],
    /// How the box reads the picture, when not as it is: a steadied one.
    reading: Option<Reading>,
}

/// The frame being drawn: its plan, where each layer was put on screen, and
/// the reading of each layer that was drawn steadied.
#[derive(Clone, Copy)]
pub(super) struct Drawn<'a> {
    pub plan: &'a RenderPlan,
    pub placed: &'a [Placed],
    pub steadied: &'a [(ClipId, Reading)],
}

impl<'a> Picture<'a> {
    pub fn new(place: &'a Placed, transform: &Transform, reading: Option<Reading>) -> Self {
        Self {
            place,
            uv: compose::pass::corners(transform),
            reading,
        }
    }

    /// The picture of `clip` in the frame being drawn.
    pub fn of(drawn: &Drawn<'a>, clip: ClipId) -> Option<Self> {
        let place = drawn.placed.iter().rev().find(|place| place.clip == clip)?;
        let layer = (drawn.plan.layers.iter().rev()).find(|layer| layer.clip == clip)?;
        let reading = (drawn.steadied.iter())
            .find(|(steadied, _)| *steadied == clip)
            .map(|(_, reading)| *reading);
        Some(Self::new(place, &layer.transform, reading))
    }

    pub fn to_screen(self, source: [f32; 2]) -> Pos2 {
        let source = (self.reading).map_or(source, |reading| reading.shown(source));
        let [u0, v0, u1, v1] = self.uv;
        let span = |from: f32, to: f32| {
            if (to - from).abs() < f32::EPSILON {
                f32::EPSILON
            } else {
                to - from
            }
        };
        let across = (source[0] - u0) / span(u0, u1);
        let down = (source[1] - v0) / span(v0, v1);
        self.place.corner(across * 2.0 - 1.0, down * 2.0 - 1.0)
    }

    pub fn to_source(self, pos: Pos2) -> [f32; 2] {
        let place = self.place;
        let (sin, cos) = (-place.angle).sin_cos();
        let d = pos - place.center;
        let local = vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
        let across = (local.x / place.half.x.max(f32::EPSILON) + 1.0) * 0.5;
        let down = (local.y / place.half.y.max(f32::EPSILON) + 1.0) * 0.5;
        let [u0, v0, u1, v1] = self.uv;
        let shown = [u0 + (u1 - u0) * across, v0 + (v1 - v0) * down];
        (self.reading).map_or(shown, |reading| reading.read(shown))
    }
}

/// The region as a drag leaves it: a new box between the anchor and the
/// pointer, or the same box moved. Both in fractions of the source frame.
pub(super) fn dragged(region: TrackRegion, hold: RegionHold, pointer: [f32; 2]) -> TrackRegion {
    let mut next = region;
    match hold {
        RegionHold::Draw { anchor } => {
            let to = pointer.map(|value| value.clamp(0.0, 1.0));
            next.center = [0, 1].map(|axis| (anchor[axis] + to[axis]) * 0.5);
            next.size = [0, 1].map(|axis| (to[axis] - anchor[axis]).abs());
        }
        RegionHold::Move { grab } => {
            next.center = [0, 1].map(|axis| pointer[axis] - grab[axis]);
        }
    }
    next.held()
}

/// Whether a place of the source frame is inside the region.
fn inside(region: &TrackRegion, at: [f32; 2]) -> bool {
    (0..2).all(|axis| (at[axis] - region.center[axis]).abs() <= region.size[axis] * 0.5)
}

/// The stage while a region is chosen on it: the box is drawn on its clip's
/// picture, a drag inside it moves it and a drag anywhere else on the
/// picture draws a new one. Nothing is selected or moved meanwhile. Returns
/// where the stage says what to do.
pub(super) fn choose_region(
    ui: &mut Ui,
    stage: Rect,
    picture: Option<Picture>,
    p: &Palette,
    state: &mut ViewerState,
) -> Option<Rect> {
    const NAME: &str = "Drag on the picture to draw the box to track";
    let region = state.region?;
    let response = ui.interact(stage, Id::new("viewer-stage"), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, NAME));
    let said = prompt(
        ui,
        stage,
        p,
        if picture.is_some() {
            NAME
        } else {
            "The clip is not on screen at the playhead"
        },
    );
    let painter = ui.painter_at(stage);
    let Some(picture) = picture else {
        state.region_hold = None;
        return Some(said);
    };
    let pointer = ui.input(|i| i.pointer.latest_pos());
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && picture.place.contains(pos)
    {
        let at = picture.to_source(pos);
        state.region_hold = Some(if inside(&region, at) {
            RegionHold::Move {
                grab: [at[0] - region.center[0], at[1] - region.center[1]],
            }
        } else {
            RegionHold::Draw {
                anchor: at.map(|value| value.clamp(0.0, 1.0)),
            }
        });
    }
    let mut shown = region;
    if let (Some(hold), Some(pos)) = (state.region_hold, pointer)
        && response.dragged()
    {
        shown = dragged(region, hold, picture.to_source(pos));
        if shown != region {
            state.region = Some(shown);
            state.region_changed = true;
        }
    }
    if !response.dragged() {
        state.region_hold = None;
    }
    if let Some(pos) = response.hover_pos() {
        ui.ctx().set_cursor_icon(if !picture.place.contains(pos) {
            CursorIcon::Default
        } else if inside(&shown, picture.to_source(pos)) {
            CursorIcon::Move
        } else {
            CursorIcon::Crosshair
        });
    }
    let [x, y] = shown.center;
    let [w, h] = [shown.size[0] * 0.5, shown.size[1] * 0.5];
    let corners = [
        [x - w, y - h],
        [x + w, y - h],
        [x + w, y + h],
        [x - w, y + h],
    ];
    frame(
        &painter,
        corners.map(|corner| picture.to_screen(corner)),
        p.accent,
        p,
    );
    let centre = picture.to_screen(shown.center);
    painter.circle_filled(centre, 3.5, p.frame_halo());
    painter.circle_filled(centre, 2.5, p.accent);
    Some(said)
}

/// The paths of the trackers the selected clip has to do with, on the
/// picture they were read from: those of the media it shows, and the one it
/// follows. Each with its point at the playhead and its name.
pub(super) fn paths(
    painter: &egui::Painter,
    p: &Palette,
    project: &Project,
    drawn: &Drawn,
    selection: &Selection,
    playhead: Time,
) {
    let Some((_, clip)) = selection.single().and_then(|id| project.clip(id)) else {
        return;
    };
    for tracker in project.trackers.values() {
        let followed = (clip.follow.as_ref()).is_some_and(|follow| follow.tracker_id == tracker.id);
        // The clip the path lies on: the selected one when it shows the
        // tracked media, else the one its follower reads the point from.
        let on = if clip.asset_id() == Some(tracker.asset_id) {
            Some(clip)
        } else if followed {
            render::tracked_clip(project, playhead, clip.id, tracker)
        } else {
            None
        };
        let Some(on) = on else { continue };
        let Some(picture) = Picture::of(drawn, on.id) else {
            continue;
        };
        let color = if followed { p.accent } else { p.yellow };
        let points: Vec<Pos2> = (tracker.keys.iter())
            .map(|key| picture.to_screen(key.center))
            .collect();
        if points.len() > 1 {
            painter.add(Shape::line(
                points.clone(),
                Stroke::new(3.0, p.frame_halo()),
            ));
            painter.add(Shape::line(points, Stroke::new(1.5, color)));
        }
        let Some(now) = tracker.at(on.source_time(playhead)) else {
            continue;
        };
        let at = picture.to_screen(now.center);
        let (fill, _) = p.handle();
        painter.circle_filled(at, 5.5, p.frame_halo());
        painter.circle_filled(at, 4.0, color);
        painter.circle_stroke(at, 4.0, Stroke::new(1.0, fill));
        let label = at + vec2(9.0, -9.0);
        painter.text(
            label + vec2(1.0, 1.0),
            Align2::LEFT_BOTTOM,
            &tracker.name,
            theme::medium(11.0),
            Color32::from_black_alpha(160),
        );
        painter.text(
            label,
            Align2::LEFT_BOTTOM,
            &tracker.name,
            theme::medium(11.0),
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::ClipId;
    use eframe::egui::pos2;

    fn placed(angle: f32) -> Placed {
        Placed {
            clip: ClipId::from_u128(1),
            center: pos2(400.0, 300.0),
            half: vec2(200.0, 100.0),
            angle,
            locked: false,
        }
    }

    fn region() -> TrackRegion {
        TrackRegion::new(ClipId::from_u128(1), ClipId::from_u128(2))
    }

    #[test]
    fn a_place_of_the_source_and_its_point_on_screen_are_each_others() {
        let plain = Transform::default();
        let place = placed(0.0);
        let picture = Picture::new(&place, &plain, None);
        assert_eq!(picture.to_screen([0.0, 0.0]), pos2(200.0, 200.0));
        assert_eq!(picture.to_screen([0.5, 0.5]), pos2(400.0, 300.0));
        assert_eq!(picture.to_screen([1.0, 1.0]), pos2(600.0, 400.0));
        // Through a turn, a crop and a flip the two still undo each other.
        let turned = placed(0.6);
        let cropped = Transform {
            crop: [0.1, 0.2, 0.3, 0.0],
            flip_x: true,
            ..Transform::default()
        };
        let picture = Picture::new(&turned, &cropped, None);
        // The left edge of the box shows the right edge of what the crop
        // leaves, since it is flipped.
        let left = picture.to_source(turned.corner(-1.0, 0.0));
        assert!((left[0] - 0.7).abs() < 1e-5, "{left:?}");
        for source in [[0.2, 0.3], [0.65, 0.9], [0.4, 0.25]] {
            let back = picture.to_source(picture.to_screen(source));
            assert!(
                (back[0] - source[0]).abs() < 1e-4 && (back[1] - source[1]).abs() < 1e-4,
                "{source:?} came back as {back:?}"
            );
        }
    }

    #[test]
    fn a_box_on_a_steadied_picture_is_the_place_of_the_frame_drawn_there() {
        // Enlarged 1.2 times and moved against a shake, as the place pass
        // reads it: `0.5 + matrix * (shown - 0.5) + offset`.
        let reading = Reading {
            matrix: [1.0 / 1.2, 0.0, 0.0, 1.0 / 1.2],
            offset: [0.01, -0.02],
        };
        let place = placed(0.0);
        let plain = Transform::default();
        let picture = Picture::new(&place, &plain, Some(reading));
        // What is drawn at 0.9 across the box is not 0.9 across the frame.
        let across = pos2(200.0 + 0.9 * 400.0, 300.0);
        let source = picture.to_source(across);
        assert!(
            (source[0] - (0.5 + 0.4 / 1.2 + 0.01)).abs() < 1e-5,
            "{source:?}"
        );
        assert!((source[1] - 0.48).abs() < 1e-5, "{source:?}");
        let unsteadied = Picture::new(&place, &plain, None).to_source(across);
        assert!((unsteadied[0] - 0.9).abs() < 1e-5);
        // And a place of the frame is drawn where the reading shows it,
        // through a turn, a crop and a flip too.
        let back = picture.to_screen(source);
        assert!((back - across).length() < 1e-2, "{back:?}");
        let turned = placed(0.6);
        let cropped = Transform {
            crop: [0.1, 0.2, 0.3, 0.0],
            flip_x: true,
            ..Transform::default()
        };
        let picture = Picture::new(&turned, &cropped, Some(reading));
        for source in [[0.2, 0.3], [0.65, 0.9], [0.4, 0.25]] {
            let back = picture.to_source(picture.to_screen(source));
            assert!(
                (back[0] - source[0]).abs() < 1e-4 && (back[1] - source[1]).abs() < 1e-4,
                "{source:?} came back as {back:?}"
            );
        }
    }

    #[test]
    fn a_drag_draws_a_box_or_moves_the_one_there_and_keeps_it_in_the_frame() {
        let drawn = dragged(
            region(),
            RegionHold::Draw { anchor: [0.2, 0.3] },
            [0.5, 0.7],
        );
        assert!((drawn.center[0] - 0.35).abs() < 1e-6 && (drawn.center[1] - 0.5).abs() < 1e-6);
        assert!((drawn.size[0] - 0.3).abs() < 1e-6 && (drawn.size[1] - 0.4).abs() < 1e-6);
        // Dragged back past its anchor it is the same box the other way.
        let back = dragged(
            region(),
            RegionHold::Draw { anchor: [0.5, 0.7] },
            [0.2, 0.3],
        );
        assert!((back.center[0] - drawn.center[0]).abs() < 1e-6);
        assert!((back.size[1] - drawn.size[1]).abs() < 1e-6);
        // Past the picture's edge it ends at the edge; a press that has not
        // moved is the smallest box there is.
        let edge = dragged(
            region(),
            RegionHold::Draw { anchor: [0.8, 0.8] },
            [1.7, 0.9],
        );
        assert!((edge.center[0] - 0.9).abs() < 1e-6 && (edge.size[0] - 0.2).abs() < 1e-6);
        let speck = dragged(
            region(),
            RegionHold::Draw { anchor: [0.4, 0.4] },
            [0.4, 0.4],
        );
        assert_eq!(speck.size, [TrackRegion::MIN_SIDE; 2]);
        // A move keeps the size and stops at the frame.
        let moved = dragged(
            region(),
            RegionHold::Move { grab: [0.05, 0.0] },
            [0.35, 0.6],
        );
        assert!((moved.center[0] - 0.3).abs() < 1e-6 && (moved.center[1] - 0.6).abs() < 1e-6);
        assert_eq!(moved.size, region().size);
        let stopped = dragged(region(), RegionHold::Move { grab: [0.0, 0.0] }, [2.0, -1.0]);
        assert!((stopped.center[0] - 0.9).abs() < 1e-6 && (stopped.center[1] - 0.1).abs() < 1e-6);
        assert!(inside(&region(), [0.45, 0.58]));
        assert!(!inside(&region(), [0.45, 0.61]));
    }
}
