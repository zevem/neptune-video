//! Aster: a desktop video editor.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod agent;
mod ai;
mod app;
mod audio;
mod clipboard;
mod compose;
mod duck;
mod export;
mod extract;
mod file_drop;
mod icons;
mod languages;
mod normalize;
mod pacing;
mod peaks;
mod presets;
mod proxies;
mod record;
mod rules;
mod settings;
mod speech;
mod speed;
mod steady;
mod stems;
mod textures;
mod theme;
mod timefmt;
mod titles;
mod ui;

use app::{App, Launch};
use eframe::egui;
use std::path::PathBuf;

const USAGE: &str = "\
Usage: aster [OPTIONS] [PROJECT] [MEDIA]...

Arguments:
  [PROJECT]                A .aster project to open
  [MEDIA]...               Files or folders to import

Options:
      --agent-session PATH Publish the agent session descriptor at PATH
      --settings PATH      Read and write preferences at PATH
      --size WxH           Open the window at this size, in points
      --screenshot PATH    Save a picture of the window to PATH and exit
      --screenshot-delay S Seconds to wait before that picture (default 2.5)
  -h, --help               Print this help
  -V, --version            Print the version";

fn parse(mut args: impl Iterator<Item = String>) -> Result<(Launch, [f32; 2]), String> {
    let mut launch = Launch::default();
    let mut size = [1440.0, 900.0];
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("aster {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--agent-session" => launch.agent_session = Some(value("--agent-session")?.into()),
            "--settings" => launch.settings = Some(value("--settings")?.into()),
            "--screenshot" => launch.screenshot = Some(value("--screenshot")?.into()),
            "--screenshot-delay" => {
                let text = value("--screenshot-delay")?;
                launch.screenshot_delay = text
                    .parse::<f64>()
                    .ok()
                    .filter(|seconds| (0.0..=600.0).contains(seconds))
                    .ok_or_else(|| format!("--screenshot-delay expects seconds, not “{text}”"))?;
            }
            "--size" => {
                let text = value("--size")?;
                let parsed = text.split_once(['x', 'X']).and_then(|(width, height)| {
                    Some([width.parse::<f32>().ok()?, height.parse::<f32>().ok()?])
                });
                size = parsed
                    .filter(|size| size.iter().all(|side| (200.0..=16000.0).contains(side)))
                    .ok_or_else(|| format!("--size expects WIDTHxHEIGHT, not “{text}”"))?;
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("Unknown option {other}\n\n{USAGE}"));
            }
            other => {
                // Recent projects and recovery copies outlive this working
                // directory, so paths are made absolute once, here.
                let path = PathBuf::from(other);
                let path = std::path::absolute(&path).unwrap_or(path);
                let project = path
                    .extension()
                    .is_some_and(|extension| extension == "aster");
                if project && launch.project.is_none() {
                    launch.project = Some(path);
                } else {
                    launch.import.push(path);
                }
            }
        }
    }
    Ok((launch, size))
}

/// The window draws its own titlebar and frame, so the system adds none.
fn native_options(size: [f32; 2]) -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aster")
            .with_app_id("aster")
            .with_inner_size(size)
            .with_min_inner_size(settings::LEAST_WINDOW)
            .with_drag_and_drop(true)
            // Windows rounds and outlines the window itself; elsewhere the
            // window paints its corners over a clear surface.
            .with_transparent(!cfg!(target_os = "windows"))
            .with_decorations(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    // A blocking FIFO acquire can stall the interface thread for seconds on
    // Linux hybrid graphics after the window was covered. The automatic mode
    // takes Immediate or Mailbox where they exist and keeps FIFO as fallback.
    options.wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(
        eframe::egui_wgpu::SurfaceConfig {
            present_mode: if cfg!(target_os = "linux") {
                eframe::wgpu::PresentMode::AutoNoVsync
            } else {
                eframe::wgpu::PresentMode::AutoVsync
            },
            ..eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
        },
    );
    options
}

fn main() -> std::process::ExitCode {
    let (launch, size) = match parse(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(problem) => {
            eprintln!("aster: {problem}");
            return std::process::ExitCode::from(2);
        }
    };

    let options = native_options(size);
    let run = eframe::run_native(
        "Aster",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, launch)))),
    );
    match run {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aster: the window could not be opened: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::native_options;

    #[test]
    fn the_window_draws_its_own_frame_over_clear_corners() {
        let options = native_options([1440.0, 900.0]);
        assert_eq!(options.viewport.decorations, Some(false));
        assert_eq!(
            options.viewport.transparent,
            Some(!cfg!(target_os = "windows"))
        );
    }
}
