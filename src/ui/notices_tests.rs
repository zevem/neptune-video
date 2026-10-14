//! Where notices are shown: over the viewer's top margin, and under an
//! open sheet rather than over it. Drawn in real egui passes with the
//! render queue as the sheet.
use super::*;
use crate::ui::render_queue;
use editor_core::export::{ExportSettings, RenderJobInfo, RenderState};
use eframe::egui::{Pos2, Shape};
use uuid::Uuid;

const NARROW: Vec2 = vec2(760.0, 560.0);

fn done(name: &str) -> RenderJobInfo {
    RenderJobInfo {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        destination: PathBuf::from("/home/someone/Exports").join(name),
        settings: ExportSettings::default(),
        project_id: Uuid::new_v4(),
        revision: 1,
        state: RenderState::Done,
        progress: 1.0,
        frames_done: 10,
        frames_total: 10,
        speed_fps: 0.0,
        elapsed_seconds: 1.0,
        eta_seconds: None,
        encoder: None,
        output_bytes: Some(1000),
        context_id: Uuid::new_v4(),
        message: None,
        attempts: 1,
    }
}

fn notice(text: &str) -> Notice {
    Notice {
        text: text.to_owned(),
        problem: false,
        until: 1e9,
    }
}

/// Where each notice's words were painted, after a few frames of the
/// window: the render queue open over it when `jobs` is given.
fn painted(
    notices: &[Notice],
    jobs: Option<&[RenderJobInfo]>,
    viewer: Rect,
) -> (Vec<(String, Rect)>, Option<Rect>) {
    let ctx = egui::Context::default();
    let palette = Palette::new(Appearance::Dark, Accent::default());
    theme::install_fonts(&ctx);
    theme::apply(&ctx, &palette);
    let mut texts = Vec::new();
    for frame in 0..4 {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, NARROW)),
            time: Some(f64::from(frame) * 0.05),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            if let Some(jobs) = jobs {
                render_queue::show(ui.ctx(), &palette, jobs, &[]);
            }
            super::notices(
                ui.ctx(),
                &palette,
                notices,
                viewer,
                viewer.top() + 10.0,
                jobs.is_some(),
            );
        });
        texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        output.drop_without_applying_deltas();
    }
    let sheet = ctx.memory(|memory| memory.area_rect(Id::new("render-queue-sheet")));
    let words = |notice: &Notice| {
        (texts.iter())
            .find(|(text, _)| *text == notice.text)
            .cloned()
    };
    (notices.iter().filter_map(words).collect(), sheet)
}

fn collect(shape: &Shape, into: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => {
            into.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            ));
        }
        _ => {}
    }
}

fn viewer() -> Rect {
    Rect::from_min_max(pos2(312.0, 46.0), pos2(752.0, 330.0))
}

#[test]
fn notices_sit_at_the_top_of_the_viewer_without_a_sheet() {
    let shown = [notice("Exported a.mp4"), notice("Exported b.mp4")];
    let (placed, _) = painted(&shown, None, viewer());
    assert_eq!(placed.len(), 2, "both are shown: {placed:?}");
    for (text, rect) in &placed {
        assert!(
            viewer().contains_rect(*rect),
            "{text} at {rect:?} is over the viewer"
        );
        assert!(
            rect.top() < viewer().top() + 40.0 + 44.0,
            "{text} at the top: {rect:?}"
        );
    }
    assert!(placed[0].1.top() < placed[1].1.top(), "the newest last");
}

#[test]
fn notices_go_under_an_open_sheet_rather_than_over_it() {
    // The queue as tall as a 760 x 560 window lets it be.
    let jobs: Vec<RenderJobInfo> = (0..8).map(|at| done(&format!("job-{at}.mp4"))).collect();
    let shown = [
        notice("Exported job-6.mp4"),
        notice("Exported job-7.mp4"),
        notice("The file manager could not be opened"),
    ];
    let (placed, sheet) = painted(&shown, Some(&jobs), viewer());
    let sheet = sheet.expect("FIXTURE: the queue is open");
    assert!(
        sheet.top() < viewer().top() + 40.0,
        "FIXTURE: without the fix a notice at the viewer's top is over the sheet: {sheet:?}"
    );
    assert!(
        placed
            .iter()
            .any(|(text, _)| text == "The file manager could not be opened"),
        "the newest is shown: {placed:?}"
    );
    for (text, rect) in &placed {
        assert!(
            !rect.intersects(sheet),
            "{text} at {rect:?} is not over the sheet at {sheet:?}"
        );
        assert!(rect.bottom() <= NARROW.y, "{text} is inside the window");
    }
}
