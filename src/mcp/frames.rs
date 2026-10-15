//! Pictures of the project for an agent to look at: the timeline at given
//! times, or one media file's frames, drawn by the export's own renderer on
//! a device of the server's own. One thread owns the device, the renderer
//! and the decoders; requests wait in a short queue, so concurrent calls
//! cannot each open a device.
use super::errors::{ToolError, codes};
use crate::export::frame::{FrameRenderer, Pictures, headless_device};
use editor_core::{
    Asset, ClipId, Project, Time,
    export::{ExportSettings, Fit, Geometry},
};
use editor_media::{Frame, export::SourceReader};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::Duration,
};

/// The longest side of a picture handed to an agent.
pub const MOST_SIDE: u32 = 2048;
/// The most pictures one request draws.
pub const MOST_FRAMES: usize = 48;
/// The most pixels one request draws, all frames together: 256 MB of RGBA.
const MOST_PIXELS: u64 = 64 * 1024 * 1024;
/// The gap between the cells of a contact sheet.
const GAP: u32 = 4;
/// One frame may take the device this long.
const FRAME_DEADLINE: Duration = Duration::from_secs(30);

/// One drawn picture: opaque RGBA rows.
pub struct Drawn {
    pub at: Time,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Request {
    project: Arc<Project>,
    times: Vec<Time>,
    longest: u32,
    /// Raised when the caller stopped waiting: no more frames are drawn.
    cancel: Arc<AtomicBool>,
    answer: tokio::sync::oneshot::Sender<Result<Vec<Drawn>, ToolError>>,
}

#[derive(Clone)]
pub struct Frames {
    requests: SyncSender<Request>,
}

/// The project as frames are drawn of it: captions burned in, as an export
/// with the default settings draws them.
fn drawn_project(project: &Project) -> Project {
    ExportSettings::default().render_project(project)
}

struct Drawer {
    device: Option<(eframe::wgpu::Device, eframe::wgpu::Queue)>,
    failed: Option<String>,
    renderer: Option<(Geometry, [u32; 2], FrameRenderer)>,
    reader: SourceReader,
    stop: Arc<AtomicBool>,
}

struct Sources<'a>(&'a mut SourceReader);

impl Pictures for Sources<'_> {
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        self.0
            .picture(lane, asset, time, size)
            .map_err(|error| format!("{} could not be read: {error}", asset.name))
    }
}

impl Drawer {
    fn device(&mut self) -> Result<(eframe::wgpu::Device, eframe::wgpu::Queue), ToolError> {
        if let Some(why) = &self.failed {
            return Err(ToolError::unavailable(why.clone()));
        }
        if self.device.is_none() {
            match headless_device() {
                Ok(headless) => self.device = Some((headless.device, headless.queue)),
                Err(why) => {
                    self.failed = Some(why.clone());
                    return Err(ToolError::unavailable(why));
                }
            }
        }
        Ok(self.device.clone().expect("opened above"))
    }

    fn draw(
        &mut self,
        project: &Project,
        times: &[Time],
        longest: u32,
        cancel: &AtomicBool,
    ) -> Result<Vec<Drawn>, ToolError> {
        let render = drawn_project(project);
        let canvas = [render.settings.width, render.settings.height];
        let output = output_size(&render, longest);
        let geometry = Geometry::new(canvas, output, Fit::Letterbox);
        let fresh = !matches!(
            &self.renderer,
            Some((made, size, _)) if *made == geometry && *size == canvas
        );
        if fresh {
            self.renderer = None;
            let (device, queue) = self.device()?;
            let mut renderer = FrameRenderer::new(device, queue, geometry, &render, None)
                .map_err(ToolError::unavailable)?;
            renderer.set_stop(self.stop.clone());
            renderer.set_deadline(FRAME_DEADLINE);
            self.renderer = Some((geometry, canvas, renderer));
        }
        let (_, _, renderer) = self.renderer.as_mut().expect("made above");
        // Fonts the project brought are the renderer's to load again.
        let mut drawn = Vec::with_capacity(times.len());
        for &at in times {
            if cancel.load(Ordering::Acquire) {
                return Err(ToolError::new(codes::JOB_FAILED, "Drawing was cancelled"));
            }
            let rgba = renderer
                .render(&render, at, &mut Sources(&mut self.reader))
                .map_err(|why| ToolError::new(codes::JOB_FAILED, why))?
                .to_vec();
            drawn.push(Drawn {
                at,
                width: geometry.output[0],
                height: geometry.output[1],
                rgba,
            });
        }
        // Decoders of other times are not kept.
        self.reader.retain(&|_| false);
        Ok(drawn)
    }
}

impl Frames {
    pub fn start() -> Self {
        let (requests, inbox) = sync_channel::<Request>(2);
        thread::Builder::new()
            .name("aster-mcp-frames".into())
            .spawn(move || {
                let mut drawer = Drawer {
                    device: None,
                    failed: None,
                    renderer: None,
                    reader: SourceReader::new(),
                    stop: Arc::new(AtomicBool::new(false)),
                };
                for request in inbox {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        drawer.draw(
                            &request.project,
                            &request.times,
                            request.longest,
                            &request.cancel,
                        )
                    }))
                    .unwrap_or_else(|_| {
                        drawer.renderer = None;
                        Err(ToolError::new(
                            codes::JOB_FAILED,
                            "Drawing the frame failed",
                        ))
                    });
                    let _ = request.answer.send(result);
                }
            })
            .expect("the frames thread starts");
        Self { requests }
    }

    /// Draws `project` at each of `times`, the longest side at most
    /// `longest` pixels.
    pub async fn draw(
        &self,
        project: Arc<Project>,
        times: Vec<Time>,
        longest: u32,
    ) -> Result<Vec<Drawn>, ToolError> {
        if times.is_empty() || times.len() > MOST_FRAMES {
            return Err(ToolError::invalid(format!(
                "Ask for 1 to {MOST_FRAMES} frames at a time"
            )));
        }
        let longest = longest.clamp(16, MOST_SIDE);
        let pixels = output_size(&project, longest)
            .into_iter()
            .map(u64::from)
            .product::<u64>()
            * times.len() as u64;
        if pixels > MOST_PIXELS {
            return Err(ToolError::new(
                codes::TOO_LARGE,
                format!(
                    "{} frames of that size are {} million pixels; one call draws at most {}. \
                     Ask for fewer frames or a smaller size.",
                    times.len(),
                    pixels / 1_000_000,
                    MOST_PIXELS / 1_000_000
                ),
            ));
        }
        let (answer, answered) = tokio::sync::oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        // A call the client cancels drops this future; the frames thread
        // then stops at the next frame.
        struct StopOnDrop(Arc<AtomicBool>);
        impl Drop for StopOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = StopOnDrop(cancel.clone());
        let request = Request {
            project,
            times,
            longest,
            cancel,
            answer,
        };
        match self.requests.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(ToolError::new(
                    codes::BUSY,
                    "Other frames are being drawn; retry in a moment",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(ToolError::unavailable("The frame renderer stopped"));
            }
        }
        answered
            .await
            .map_err(|_| ToolError::unavailable("The frame renderer stopped"))?
    }
}

/// The size of the pictures drawn of `project` with the longest side at
/// most `longest`, as the frames thread draws them.
fn output_size(project: &Project, longest: u32) -> [u32; 2] {
    let canvas = [project.settings.width, project.settings.height];
    let scale = (f64::from(longest) / f64::from(canvas[0].max(canvas[1]).max(1))).min(1.0);
    canvas.map(|side| (((f64::from(side) * scale).round() as u32).max(2)) & !1)
}

/// The largest cell (longest side) with which `count` cells in `columns`
/// columns make a sheet no side of which is over [`MOST_SIDE`].
pub fn sheet_cell(columns: u32, count: u32) -> u32 {
    let columns = columns.max(1);
    let rows = count.max(1).div_ceil(columns);
    let fit = |cells: u32| MOST_SIDE.saturating_sub((cells + 1) * GAP) / cells;
    fit(columns).min(fit(rows)).max(16)
}

/// The picture as a PNG.
pub fn png(drawn: &Drawn) -> Result<Vec<u8>, ToolError> {
    encode(
        drawn.width,
        drawn.height,
        &drawn.rgba,
        image::ImageFormat::Png,
    )
}

/// The picture as a JPEG of good quality, for contact sheets.
pub fn jpeg(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ToolError> {
    encode(width, height, rgba, image::ImageFormat::Jpeg)
}

fn encode(
    width: u32,
    height: u32,
    rgba: &[u8],
    format: image::ImageFormat,
) -> Result<Vec<u8>, ToolError> {
    let picture = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| ToolError::unavailable("The picture has the wrong size"))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    let result = if format == image::ImageFormat::Jpeg {
        image::DynamicImage::ImageRgba8(picture)
            .to_rgb8()
            .write_to(&mut bytes, format)
    } else {
        picture.write_to(&mut bytes, format)
    };
    result
        .map_err(|error| ToolError::unavailable(format!("The picture was not encoded: {error}")))?;
    Ok(bytes.into_inner())
}

/// Tiles `cells` (all one size) into a sheet `columns` wide, with a gap.
pub fn sheet(cells: &[Drawn], columns: u32) -> (u32, u32, Vec<u8>) {
    let columns = columns.clamp(1, cells.len().max(1) as u32);
    let rows = (cells.len() as u32).div_ceil(columns);
    let (cell_w, cell_h) = cells
        .first()
        .map_or((1, 1), |cell| (cell.width, cell.height));
    let width = columns * cell_w + (columns + 1) * GAP;
    let height = rows * cell_h + (rows + 1) * GAP;
    let mut rgba = [24u8, 24, 27, 255].repeat((width * height) as usize);
    for (index, cell) in cells.iter().enumerate() {
        let (column, row) = (index as u32 % columns, index as u32 / columns);
        let (x0, y0) = (GAP + column * (cell_w + GAP), GAP + row * (cell_h + GAP));
        for y in 0..cell.height.min(cell_h) {
            let from = (y * cell.width * 4) as usize;
            let to = (((y0 + y) * width + x0) * 4) as usize;
            let len = (cell.width.min(cell_w) * 4) as usize;
            rgba[to..to + len].copy_from_slice(&cell.rgba[from..from + len]);
        }
    }
    (width, height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_places_every_cell_in_reading_order() {
        let cell = |shade: u8| Drawn {
            at: Time::ZERO,
            width: 2,
            height: 2,
            rgba: [shade, shade, shade, 255].repeat(4),
        };
        let (width, height, rgba) = sheet(&[cell(10), cell(20), cell(30)], 2);
        assert_eq!((width, height), (2 * 2 + 3 * 4, 2 * 2 + 3 * 4));
        let at = |x: u32, y: u32| rgba[((y * width + x) * 4) as usize];
        assert_eq!(at(4, 4), 10);
        assert_eq!(at(10, 4), 20);
        assert_eq!(at(4, 10), 30);
        assert_eq!(at(10, 10), 24, "the empty cell is the background");
        assert_eq!(png(&cell(5)).unwrap()[..4], [0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn a_sheet_of_any_shape_fits_the_longest_side() {
        for (columns, count) in [(4, 12), (48, 48), (1, 48), (4, 1), (7, 48)] {
            let cell = sheet_cell(columns, count).min(640);
            let rows = count.div_ceil(columns);
            assert!(
                columns * cell + (columns + 1) * GAP <= MOST_SIDE,
                "{columns}x{count}"
            );
            assert!(
                rows * cell + (rows + 1) * GAP <= MOST_SIDE,
                "{columns}x{count}"
            );
        }
    }
}
