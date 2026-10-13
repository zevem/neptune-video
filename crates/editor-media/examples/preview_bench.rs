//! Release-only local decode/worker benchmark. This does not measure GPU upload,
//! display presentation, audio sync or native input-to-screen latency.
use editor_core::{Asset, Time};
use editor_media::{MediaEvent, MediaService};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn event(service: &MediaService) -> Result<MediaEvent, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(event) = service.poll() {
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for the media worker".into());
        }
        thread::sleep(Duration::from_micros(200));
    }
}

fn run() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Usage: preview_bench MEDIA_PATH [FRAME_COUNT] [REQUEST_FPS]")?;
    let count: usize = std::env::args()
        .nth(2)
        .map(|count| count.parse())
        .transpose()
        .map_err(|_| "Invalid frame count")?
        .unwrap_or(120);
    let service = MediaService::new(|| {}).map_err(|error| error.to_string())?;
    service
        .import(vec![path])
        .map_err(|error| error.to_string())?;
    let asset: Asset = match event(&service)? {
        MediaEvent::Imported {
            mut assets, errors, ..
        } => {
            if !errors.is_empty() {
                return Err(errors[0].message.clone());
            }
            assets.pop().ok_or("No media was imported")?
        }
        _ => return Err("Expected an import result".into()),
    };
    let rate = if let Some(request_fps) = std::env::args().nth(3) {
        let numerator = request_fps
            .parse::<u32>()
            .map_err(|_| "Invalid request rate")?;
        let rate = editor_core::FrameRate {
            numerator,
            denominator: 1,
        };
        if !rate.validate() {
            return Err("Request rate must be between 1 and 240 fps".into());
        }
        rate
    } else {
        asset.frame_rate.unwrap_or_default()
    };
    let count = count.min(rate.frame_at(asset.duration).saturating_sub(1).max(0) as usize);
    if count == 0 {
        return Err("Media is too short for a sequential preview benchmark".into());
    }
    service
        .request_frame(&asset, Time::ZERO, [1280, 720])
        .map_err(|error| error.to_string())?;
    if let MediaEvent::Error { message, .. } = event(&service)? {
        return Err(message);
    }
    let mut timings = Vec::with_capacity(count);
    let mut bytes = 0_u64;
    let started = Instant::now();
    for index in 1..=count {
        let time = rate.frame_time(index as i64);
        let frame_start = Instant::now();
        service
            .request_frame(&asset, time, [1280, 720])
            .map_err(|error| error.to_string())?;
        match event(&service)? {
            MediaEvent::FrameReady { frame, .. } => {
                bytes += frame.byte_len() as u64;
            }
            MediaEvent::Error { message, .. } => return Err(message),
            _ => return Err("Unexpected import result during preview".into()),
        }
        timings.push(frame_start.elapsed().as_secs_f64() * 1000.0);
    }
    let elapsed = started.elapsed().as_secs_f64();
    timings.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| {
        timings
            .get(((timings.len().saturating_sub(1)) as f64 * fraction) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "{}",
        serde_json::json!({
            "scope":"software_decode_and_worker_roundtrip_excludes_gpu_presentation",
            "platform":std::env::consts::OS, "architecture":std::env::consts::ARCH,
            "optimized":!cfg!(debug_assertions), "requested_size":[1280,720],
            "request_fps":rate.fps(), "frames":count, "elapsed_seconds":elapsed, "frames_per_second":count as f64 / elapsed,
            "rgba_mebibytes_per_second":bytes as f64 / (1024.0*1024.0) / elapsed,
            "p50_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99)
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
