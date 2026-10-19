# Export and rendering

Status: ready-for-agent
Progress: claimed
Type: task

Phase 7: a person configures an export of the timeline, queues it, watches
it render in the background, pauses, cancels or retries it, and gets a file
whose picture is the preview's composition drawn from the original media at
the chosen size, with synchronized sound. Preview gains a quality setting
and proxies for large media; export never reads a proxy.

This document is the binding design. Code comments, tests and the later ADR
0009 follow it; a change to a decision is a dated comment below.

## Decisions

1. **Export is not an edit.** It reads an immutable `Arc<Project>` snapshot
   captured (after committing drafts) when the job is queued, with the
   project id, `Editor::instance_id` and revision recorded in the job. Later
   edits never change a queued job. No `Command` variant exists for it; the
   agent protocol gains non-mutating `AgentOperation`s that answer at once
   with a job id.
2. **One picture path for export: the GPU compositor, from the plan.** Every
   output frame is `editor_core::resolve(render_project, frame_time(n), overrides)`
   drawn by `src/compose` at the output size. Plain layers, composite runs
   and titles are all drawn by the compositor in linear light, preserving the
   viewer's run semantics: a composite run after a title starts on a
   transparent canvas and is laid over what is below; plain layers and titles
   are laid over with Normal blending. The viewer's mesh path blends plain
   layers in gamma space; that difference (half-transparent plain layers)
   is the known limit recorded in architecture.md and is not reproduced. No
   FFmpeg filter graph composes anything (ADR 0006); FFmpeg encodes RGBA
   frames from stdin and converts them to Y'CbCr only.
3. **Originals only.** The export reads `asset.path` from the snapshot. The
   export modules have no access to the proxy index; a test proves a project
   whose asset has a proxy exports pixels of the original.
4. **Same source frames as preview.** `editor_core::render::source_frame`
   (moved out of the viewer) chooses the source frame for both. Export
   decodes with the preview's seek and frame-grid arguments (`-ss` input
   seek, `fps=num/den` on the asset's rate) and a better scaler
   (`bicubic+accurate_rnd+full_chroma_int`), at the size the clip needs on
   the output at its largest, never upscaled. Colour matrix defaults match
   the preview decode so RGB matches.
5. **Exact timing.** Output frame `n` is `rate.frame_time(first + n)`
   (`FrameSpan`); the range is snapped outwards to whole output frames.
   Audio covers exactly `[span.start, span.end)`:
   `sample_count(start, end, rate)` sample frames, rendered by the playback
   mixer offline (`OfflineMix` with a channel count) to a task-owned raw
   `f32le` file, then muxed. Both streams start at pts 0; no `-shortest`.
6. **Encoding is external FFmpeg, probed per host.** `Capabilities` lists
   only encoders that encoded a test picture here (hardware ones included),
   and containers whose muxer exists. The dialog offers nothing else.
   `EncoderChoice::Auto` takes the best working hardware encoder that fits
   the size and otherwise software. A hardware encode that fails under
   `Auto` is retried once, automatically, from the start with the software
   encoder; the job says so.
7. **Never damage a file.** Output is written to a hidden part file in the
   destination directory (`.<name>.<uuid>.part`) and renamed over the
   destination only after FFmpeg exited successfully and an `ffprobe` check
   found the expected streams and duration. Cancel or failure removes the
   part file and job temp files. Sidecar subtitles are written the same way.
8. **One render at a time, many queued.** The render queue runs jobs in
   order on one `neptune-video-render` thread. Pause stops feeding frames (FFmpeg
   blocks on its pipe; no signals); resume continues the same encode.
   Cancel kills the job's own processes. Retry re-runs a failed or cancelled
   job from its snapshot. Finished jobs stay listed until removed. Quitting
   with an active or queued job asks first.
9. **Settings live outside the project.** Built-in presets are code
   (`editor_core::export::builtin_presets`). Saved presets live in
   `presets.json` (`exports`, tolerant). The last used export settings and
   folder, and preview settings, live in `settings.json`, stored as raw JSON
   values parsed leniently so an unknown value can never reset the person's
   settings.
10. **Proxies are derived data.** A proxy is an H.264 (or MJPEG when libx264
    is missing) short-GOP, constant-rate, audio-less copy at most 1280 on the
    long side, with the original's aspect, rate and frame count, kept in a
    bounded LRU cache directory keyed by source identity and profile
    version. The mapping lives in app/worker state, never in `Project`.
    The viewer reads a proxy only when one exists, matches the asset's
    `SourceIdentity`, and supplies at least the pixels the preview needs.
11. **Preview quality is a person's setting.** `Auto`, `Full`, `Half`,
    `Quarter` cap the viewer's decode box and compositor canvas. `Auto` is
    `Full` while paused and `Half` while playing when the canvas's long side
    is above 1920.

## Module map and contracts

| Module | Owns |
| --- | --- |
| `crates/editor-core/src/export.rs` | Settings, formats, `Capabilities` data, `KnownVideoEncoder` table, validation into `ExportPlan`, `FrameSpan`, `sample_count`, `Geometry`, `range_cues`, presets, `RenderJobInfo` |
| `crates/editor-core/src/render.rs` | `source_frame`, `source_frame_time`, `backwards_until`, `SOURCE_SLACK` (shared with the viewer) |
| `crates/editor-media/src/export/` | `probe_capabilities`, FFmpeg argument building (`EncodeArgs`), `Encoder` (stdin-piped process), `render_audio`, `SourceReader` (sequential full-quality decode), `verify_output` |
| `crates/editor-media/src/proxy.rs` | `ProxyService`, the proxy cache |
| `src/compose/*` | Production readback, run stacking for export, shared job assembly used by the viewer and export |
| `src/export/frame.rs` | `FrameRenderer`: plan → output-size opaque RGBA |
| `src/export/queue.rs`, `engine.rs` | `RenderQueue` service and the job runner |
| `src/ui/export_sheet.rs` | The export dialog |
| `src/ui/render_queue.rs` | The queue panel |
| `crates/editor-agent/src/lib.rs` | Render operations on the wire |

### editor-core (`editor_core::export`)

Implemented as declared in `export.rs`. Rules for the functions left open:

- `ExportSettings::plan(project, caps)`:
  - `span` gives `(start, end)`; empty timeline or empty range → `Empty`.
  - Container must be in `caps.containers` else `Unsupported("<label> files
    cannot be written by this FFmpeg")`.
  - `video: None` requires an audio container and `audio: Some`; `video:
    Some` requires a video container. Audio containers with `video: Some`
    → `Invalid`.
  - Video: codec must be in `container.video_codecs()`; output size =
    `resolution.size(canvas)`; frame rate = settings or project, must
    `validate()`; bitrate within `MIN/MAX_VIDEO_KBPS`; custom sizes within
    `MIN/MAX_OUTPUT_SIDE` and at most 134,217,728 pixels; encoder =
    `caps.video_encoder(codec, choice, size)` else `Unsupported` naming the
    codec (and the family for a hardware choice, and the size when only
    smaller encoders exist). `FrameSpan::covering(start, end, rate)`;
    `Geometry::new(canvas, size, fit)`.
  - Audio: codec in `container.audio_codecs()`, encoder present, channels 1
    or 2, `accepts_rate`, kbps within limits when `has_bitrate`. When video
    is present the sample span is `[frames.start(), frames.end())`;
    otherwise `[start, end)`. `frames = sample_count(..)`.
  - Captions: a sidecar or embed needs a caption track (`subtitles.track`
    or the first of `caption_tracks`) else `Invalid("There is no caption
    track to write")`; embed needs `container.embedded_subtitles()` and a
    video stream. `cues = range_cues(track, plan.start, plan.end)`; empty →
    `Empty("The caption track has no captions in the range")`.
  - `plan.start/end` are the snapped video span when video is present.
- `render_project`: clone; when `video.frame_rate` is `Some(r)` set
  `settings.frame_rate = r` (titles and noise then step at the output rate);
  when `!subtitles.burn_in` hide every caption track.
- `estimate_bytes`: bitrate mode exact `kbps*1000/8*seconds`; quality mode
  `suggested_kbps(codec, output, fps, level)`; plus audio `kbps` or PCM
  `rate*channels*bytes`, FLAC ≈ 60 % of PCM 16; plus 1 % container.
- `range_cues`: `captions::track_cues`, keep cues overlapping `[start, end)`,
  clamp to it, subtract `start` from cue and word times, drop empty cues and
  words outside, keep order (`settle`).
- `builtin_presets`: ids and settings —

| id | Name | Destination | Settings |
| --- | --- | --- | --- |
| `recommended` | Recommended | General | MP4 H.264 project size, Recommended quality, AAC 256 |
| `high-quality` | High quality | General | MP4 H.264 project size, Higher, AAC 320 |
| `small-file` | Small file | General | MP4 H.265 project size, Recommended, Smaller speed, AAC 160 |
| `youtube-1080` | YouTube 1080p | YouTube | MP4 H.264 1080p, Higher, AAC 384 48 kHz |
| `youtube-4k` | YouTube 4K | YouTube | MP4 H.265 4K, Higher, AAC 384 |
| `youtube-shorts` | YouTube Shorts | YouTube | MP4 H.264 Custom 1080x1920, Higher, AAC 256 |
| `tiktok` | TikTok | TikTok | MP4 H.264 Custom 1080x1920, Higher, AAC 256 |
| `instagram-reels` | Instagram Reels | Instagram | MP4 H.264 Custom 1080x1920, Recommended, AAC 256 |
| `instagram-feed` | Instagram feed (4:5) | Instagram | MP4 H.264 Custom 1080x1350, Recommended, AAC 256 |
| `facebook` | Facebook | Facebook | MP4 H.264 1080p, Recommended, AAC 256 |
| `x` | X | X | MP4 H.264 1080p, Bitrate 12,000, AAC 256 |
| `vimeo` | Vimeo | Vimeo | MP4 H.264 1080p, Higher, AAC 320 |
| `prores-master` | ProRes 422 HQ master | Master | MOV ProRes project size, Higher (HQ), PCM 24-bit |
| `webm` | Web (VP9) | General | WebM VP9 project size, Recommended, Opus 160 |
| `audio-aac` | Audio (AAC) | Audio | M4A AAC 256 |
| `audio-wav` | Audio (WAV) | Audio | WAV PCM 24-bit 48 kHz |
| `audio-mp3` | Audio (MP3) | Audio | MP3 320 44.1 kHz |
| `audio-flac` | Audio (FLAC) | Audio | FLAC 48 kHz |

  Notes name size, codec and rate, e.g. "1080 × 1920 · H.264 · Higher".
  All presets keep the project frame rate (`None`), `Auto` encoder,
  `Letterbox` fit, burn-in on, no sidecar.

### editor-media (`editor_media::export`, `editor_media::proxy`)

```rust
// Capabilities: run on a worker; at most ~15 s on a slow host.
pub fn probe_capabilities() -> Result<Capabilities, MediaError>;
// - `ffmpeg -hide_banner -version` (first line), `-encoders`, `-muxers`.
// - Every KnownVideoEncoder / AUDIO_ENCODERS name FFmpeg lists is test-encoded
//   (2 frames of lavfi `color=black:s=256x144:r=30` → `-f null -`) through
//   the same argument builder used for export (VA-API gets its device and
//   hwupload); success lists it, failure goes to `unavailable` with the
//   last stderr line. Probes run concurrently (≤4) with 10 s deadlines.

pub struct EncodeInputs<'a> {
    pub video: Option<VideoInput>,        // rawvideo rgba WxH at rate from pipe:0
    pub audio: Option<AudioInput<'a>>,    // f32le file, rate, channels
    pub subtitles: Option<SubtitleInput<'a>>, // srt/vtt file + language
    pub title: &'a str,                   // container title metadata
}
pub(crate) fn encode_args(plan: &ExportPlan, inputs: &EncodeInputs, output: &Path) -> Vec<OsString>; // pure, crate-private

pub struct Encoder { /* owns its Processes and a stdin-piped Process */ }
impl Encoder {
    pub fn start(plan: &ExportPlan, inputs: &EncodeInputs, output: &Path) -> Result<Self, MediaError>;
    pub fn write_frame(&mut self, rgba: &[u8], deadline: Duration) -> Result<(), MediaError>;
    pub fn finish(self, deadline: Duration) -> Result<(), MediaError>; // close stdin, wait, status
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static;     // kill, from any thread
}

/// Owns its Processes; a canceller is made before the call by the caller
/// through `AudioRender::new(..).canceller()` (or an equivalent shape).
pub fn render_audio(project: Arc<Project>, span: SampleSpan, out: &Path,
                    cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Result<(), MediaError>;

pub struct SourceReader { .. }   // owns its Processes
impl SourceReader {
    pub fn new() -> Self;
    /// `lane` is the clip asking; pipes are keyed by (lane, asset, size) so
    /// a clip's difference-matte plate does not restart its picture's pipe.
    /// `time` is a `source_frame` result; `size` is the decode size
    /// (≤ native, ≤ 8192 a side, ≤ 256 MiB; the preview's 4096/64 MiB
    /// `frame_bytes` limit does not apply).
    pub fn picture(&mut self, lane: Uuid, asset: &Asset, time: Time, size: [u32; 2])
        -> Result<Frame, MediaError>;
    pub fn retain(&mut self, live: &dyn Fn(Uuid) -> bool); // drop pipes of lanes gone
    pub fn canceller(&self) -> impl Fn() + Send + Sync;    // kill_all, from any thread
}
/// ffprobe the finished part file: the stream kinds expected are present
/// (counted by `codec_type`), and `format=duration` is within tolerance of
/// `duration`: one output frame + one codec frame and its delay (AAC 2048,
/// Opus 1272, MP3 2257 samples) + 10 ms. Stream durations and start times
/// are not compared (Matroska has none; Opus/MP3 start off zero). Stderr
/// noise with exit status 0 is not failure.
pub struct Expectation { pub video: bool, pub audio: bool, pub subtitles: bool, pub duration: Time,
                         pub frame: Time, pub audio_codec: Option<AudioCodec>, pub sample_rate: u32 }
pub fn verify_output(path: &Path, expect: &Expectation) -> Result<(), MediaError>;

pub struct ProxyService { .. }
impl ProxyService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static, dir: PathBuf, budget: u64) -> Result<Self, MediaError>;
    pub fn request(&self, asset: &Asset) -> Result<(), MediaError>;  // queue ≤ 64, dedup
    pub fn cancel(&self, asset: AssetId);
    pub fn cancel_all(&self);
    pub fn remove(&self, asset: &Asset);
    pub fn clear(&self);
    pub fn set_budget(&self, bytes: u64);
    pub fn lookup(&self, asset: &Asset) -> ProxyStatus; // non-blocking; in-memory index
    pub fn poll(&self) -> Option<ProxyEvent>;
    pub fn usage(&self) -> (u64, usize);
}
pub enum ProxyStatus { None, Queued, Generating(f32), Ready(Proxy), Failed(String) }
pub struct Proxy { pub path: PathBuf, pub width: u32, pub height: u32, pub source: SourceIdentity }
pub enum ProxyEvent { Progress { asset: AssetId, source: SourceIdentity, done: f32 },
                      Ready { asset: AssetId, proxy: Proxy },
                      Failed { asset: AssetId, source: SourceIdentity, message: String } }
pub fn wants_proxy(asset: &Asset) -> bool; // Auto: video, long side > 1920 or > 50 Mbit/s
```

Encoder argument rules (`encode_args`):

- Prefix `-hide_banner -nostdin -loglevel error -y` is wrong: stdin is the
  frame pipe, so omit `-nostdin`; use `-hide_banner -loglevel error -y`.
- Video input `-f rawvideo -pix_fmt rgba -s WxH -framerate N/D -i pipe:0`;
  audio input `-f f32le -ar R -ac C -i <file>`; subtitles `-i <file>`.
- Explicit `-map`s, `-f <container.muxer()>` (the part file has no usable
  extension), `-metadata title=…`, MP4/MOV/M4A `-movflags +faststart`.
- Colour: `-vf scale=out_color_matrix=bt709:out_range=tv:flags=bicubic+accurate_rnd+full_chroma_int,format=<pix>`
  with `<pix>` `yuv420p` (software, NVENC, AMF, VideoToolbox, MF) or `nv12`
  (QSV) or `nv12,hwupload` (VA-API, with `-vaapi_device` before inputs), and
  ProRes `yuv422p10le` (4444: `yuv444p10le`); tags `-colorspace bt709
  -color_primaries bt709 -color_trc bt709 -color_range tv`.
- GOP `-g round(2*fps)` except ProRes. H.265 in MP4/MOV `-tag:v hvc1`.
- Rate control per family: x264/x265 `-crf` or `-b:v -maxrate 1.5x -bufsize 2x`,
  `-preset veryfast|medium|slow`; NVENC `-preset p3|p5|p7 -tune hq -rc vbr`
  with `-cq N -b:v 0` or `-b:v`; AMF `-quality speed|balanced|quality`,
  `-rc cqp -qp_i/-qp_p/-qp_b` or `-rc vbr_peak -b:v -maxrate`; QSV
  `-preset veryfast|medium|veryslow`, `-global_quality N` or `-b:v`;
  VideoToolbox `-q:v` 45/60/75/90 or `-b:v`, `-allow_sw 0`; VA-API
  `-rc_mode CQP -qp N` or `-rc_mode VBR -b:v`; MF `-hw_encoding 1
  -rate_control u_vbr -b:v` (quality mode uses `suggested_kbps`); VP9
  `-crf N -b:v 0 -row-mt 1 -deadline good -cpu-used 4|2|1`; SVT-AV1
  `-crf N -preset 10|8|5`; libaom `-crf N -b:v 0 -cpu-used 6|4|3 -row-mt 1`;
  ProRes `-profile:v <prores_profile> -vendor apl0`.
- Audio `-c:a <encoder> [-b:a Kk] -ar R -ac C`; subtitles `-c:s mov_text|webvtt`
  with a three-letter `language` tag when the track's language maps to one.

### desktop

```rust
// src/export/frame.rs
pub trait Pictures {
    fn picture(&mut self, lane: ClipId, asset: &Asset, time: Time, size: [u32; 2])
        -> Result<editor_media::Frame, String>;
}
pub struct FrameRenderer { .. }
impl FrameRenderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, geometry: Geometry,
               project: &Project, steady: Option<PathBuf>) -> Result<Self, String>;
    /// The output picture at timeline time `at`: opaque RGBA, output size,
    /// bars black when letterboxed. Warnings (an unstabilized clip) are
    /// collected in `warnings()`.
    pub fn render(&mut self, project: &Project, at: Time, pictures: &mut dyn Pictures)
        -> Result<&[u8], String>;
    pub fn warnings(&self) -> &[String];
}

// src/export/queue.rs
pub struct RenderQueue { .. }
impl RenderQueue {
    pub fn new(wake: impl Fn() + Send + Sync + 'static, gpu: Option<(wgpu::Device, wgpu::Queue)>,
               steady: Option<PathBuf>) -> Self;
    pub fn capabilities(&self) -> CapabilityState; // Probing | Ready(Arc<Capabilities>) | Failed(String)
    pub fn reprobe(&self);
    pub fn enqueue(&self, job: NewJob) -> Result<Uuid, String>; // validates with plan()
    pub fn pause(&self, id: Uuid) -> bool;  pub fn resume(&self, id: Uuid) -> bool;
    pub fn cancel(&self, id: Uuid) -> bool; pub fn retry(&self, id: Uuid) -> bool;
    pub fn remove(&self, id: Uuid) -> bool; pub fn move_by(&self, id: Uuid, by: i32) -> bool;
    pub fn clear_finished(&self);
    pub fn jobs(&self) -> Vec<RenderJobInfo>;   // cheap snapshot
    pub fn summary(&self) -> QueueSummary;      // active count, overall progress
    pub fn poll(&self) -> Option<RenderEvent>;  // Finished { id, result }
    pub fn shutdown(&self);                     // kill and clean, joins nothing
}
pub struct NewJob { pub project: Arc<Project>, pub context: Uuid, pub revision: u64,
                    pub settings: ExportSettings, pub destination: PathBuf, pub start_paused: bool }
```

Queue limits: at most 64 jobs listed; wakes at most every 100 ms while
rendering; frame pipeline holds at most 3 output frames in flight.

### Interface

- **Toolbar**: an `Export` primary button (icon Export, tooltip "Export
  (⌘E)") as the rightmost trailing control, and beside it a queue toggle
  (icon List) that shows a small progress ring and count while jobs run.
  Name room widens accordingly so 720 pt windows never overlap.
- **Project menu**: "Export…" (Cmd+E) and "Render queue" after Checkpoints.
  Shortcut table and help list updated.
- **Export sheet** (`Overlay::Export`): 720 pt wide (narrow: one column).
  Left column: a live poster of the playhead frame (from the viewer's
  composited texture when available, otherwise none), the name field and
  folder chooser, the range, the summary (output size, rate, duration,
  estimated size, encoder in use incl. "NVIDIA NVENC"/"Software"), and
  warnings. Right column, scrolled: preset chips grouped by destination
  (General, YouTube, TikTok, Instagram, …, Saved), then sections Video
  (switch), Format, Resolution (segmented 720p/1080p/1440p/4K/Project/Custom
  with W×H fields), Frame rate, Codec, Quality (segmented
  Lower/Recommended/Higher/Maximum/Bitrate with kbps field), Encoder
  (Auto (name)/Software/each detected family), Speed, Fit when aspects
  differ; Audio (switch), codec, bitrate, sample rate, channels; Captions:
  burn in, sidecar SRT/VTT, embed stream, which track. Options a host cannot
  encode are absent or disabled with the reason on hover. Footer: Save
  preset…, Cancel, "Add to queue", primary "Export".
- **Render queue panel**: a sheet (`Overlay::RenderQueue`) listing jobs with
  name, destination folder, state, progress bar, frames, speed, elapsed and
  ETA, encoder, and actions Pause/Resume, Cancel, Retry, Show in folder,
  Remove; "Clear finished". Empty state explains nothing is rendering.
  Finishing posts a brief notice; failures stay in the list.
- **Settings sheet**: a Preview section (quality, proxies Off/Auto/Always,
  proxy cache size and Clear) and the library shows a proxy badge and a
  per-asset "Create proxy" / "Remove proxy" menu item, with progress.

### Agent protocol

New `AgentOperation`s (snake_case): `export_capabilities`, `queue_export
{ settings, destination, overwrite }`, `render_jobs`, `cancel_render { job }`,
`pause_render { job }`, `resume_render { job }`, `retry_render { job }`,
`remove_render { job }`. Results: `export_capabilities { capabilities,
presets, settings_schema }`, `render_jobs { jobs }`, `render_job { job }`.
`handle` (headless) answers `unsupported`; the desktop answers them from
its queue without blocking, after the usual project/context guards.
`destination` must be absolute; an existing file needs `overwrite: true`.

## Contract review rules (binding, 2026-10-14)

Found by reviewers against FFmpeg 8.0.1, wgpu 30 and this code; each is a
requirement.

Encoding (`editor-media`):

1. Colour tags go in the filter chain, not as output options: the chain is
   `scale=out_color_matrix=bt709:out_range=tv:flags=bicubic+accurate_rnd+full_chroma_int,format=<pix>,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv`
   (then `,hwupload` for VA-API). Verified: every encoder then reports
   bt709/bt709/bt709.
2. MP4/MOV/M4A: `-movie_timescale <audio rate>` when there is audio,
   otherwise `<frame rate numerator>`; the default 1000 cuts AAC to whole
   milliseconds.
3. `Process` gains a stdin-piped spawn (`spawn_writer` or a parameter)
   whose deadline monitor is armed around each write; `spawn` keeps
   `Stdio::null()` for every existing caller.
4. Audio encoders are probed with `-f lavfi -i anullsrc=r=48000:cl=stereo
   -t 0.1 -map 0:a -c:a <enc> -f null -`; a video-only probe exits 0 for any
   audio encoder name.
5. VA-API: the probe tries each `/dev/dri/renderD*` and records the node
   that encoded in `VideoEncoder::device`; `encode_args` passes
   `-vaapi_device <device>`. `av1_vaapi`/`vp9_vaapi` have no `-qp`: use
   `-rc_mode CQP -global_quality <crf*4>`; h264/hevc keep `-qp`.
6. Bitrate mode for VP9 `-b:v Kk -row-mt 1 -deadline good -cpu-used N`,
   SVT-AV1 `-b:v Kk -preset N`, libaom `-b:v Kk -cpu-used N -row-mt 1`.
   ProRes with `RateControl::Bitrate` is `Invalid("ProRes quality is set by
   its profile")` in `plan`.
7. Audio bitrates are checked against `AudioCodec::max_kbps(channels)`.
8. WAV `-rf64 auto`; FLAC `-sample_fmt s16`; MOV embedded subtitles
   `-tag:s tx3g`; libx265 `-x265-params log-level=error`; SVT-AV1 runs with
   the environment `SVT_LOG=1`.
9. A failed probe's reason is the first stderr line matching
   `[<encoder> @`, `[enc:`, `Device creation failed` or `Failed to
   initialise`, otherwise the first non-empty line.
10. `Encoder` and `render_audio` own their `Processes` (it is crate-private);
    `encode_args` is crate-private. Update `editor-media/src/lib.rs`'s
    invariant: the export APIs are blocking calls for a desktop-owned
    worker thread.
11. The export reader has its own bounds (≤ 8192 a side, ≤ 256 MiB a
    frame) and a checked constructor; it does not use `frame_bytes` or
    `fitted_dimensions`.

Rendering (desktop):

12. `FrameRenderer` owns its own `compose::gpu::Gpu` and sizes its targets
    from the job, never through `Composer`/`Ledger`: no step-down, no
    `MAX_CANVASES`; runs are drawn one after another onto one accumulator.
    A turn that cannot hold `geometry.drawn` fails the job with a reason;
    `plan` refuses `drawn` above `MAX_OUTPUT_SIDE` ("Fill would need a
    picture larger than the compositor can draw; use Letterbox").
13. Every allocation and submit of the render thread runs inside
    `Gpu::checked` on that thread; the job runner wraps each job in
    `catch_unwind`, marks it Failed and removes part and temp files. Queue
    locks are never held across job work.
14. Readback never blocks on the shared device: `map_async` then a loop of
    `device.poll(PollType::Poll)` with ≤ 2 ms sleeps that checks cancel and
    a deadline. `PollType::wait_indefinitely` is test-only.
15. `render_project` is what `FrameRenderer::new` and `render` receive, so
    `resolve` and `source_frame` use the output rate.

Queue and protocol:

16. `enqueue` refuses a destination that canonically equals any asset path
    of the snapshot, the project file or its sidecars, or whose extension is
    not `container.extension()`; errors are typed (`EnqueueError`).
17. Agent calls: `AgentCall::claim(self) -> Option<Claimed>` performs the
    QUEUED→EXECUTING exchange for desktop-answered operations;
    `check_target(editor, request)` is extracted from `handle` and reused.
    `queue_export` requires `expected_revision`, `expected_project_id` and
    `expected_context_id`; job operations need none. Error codes:
    `capabilities_pending`, `invalid_export`, `unsupported_export`,
    `empty_export`, `queue_full`, `destination_exists`, `unknown_job`,
    `unsupported` (headless). `AgentResult::Capabilities` gains
    `operations: Vec<String>` (`#[serde(default)]`).
18. Export presets live in their own `export-presets.json`
    (`{version, presets: [raw JSON]}`); each preset parses on its own and
    an unreadable one is kept and written back unchanged. Saved presets and
    last-used settings drop `range` and `subtitles.track`.

Proxies and preview:

19. A viewer layer records which source it reads (original, or proxy path);
    a change resets `requested`/`admitted`/`failed` as `grant()` does and
    frames are accepted only from the current source. A proxy that fails
    to decode is marked failed and the original is read. Proxies are put
    only into picture requests, never into a `Project` given to audio,
    analysis, speech or titles.
20. Eviction and Clear skip proxies that are open in a decoder; `.part`
    files older than an hour are swept.
21. Auto preview quality steps down only after measured late frames during
    playback and keeps the stepped box until playback stops, so starting
    playback does not restart every pipe.

## Acceptance

| Area | Proof |
| --- | --- |
| Core math and validation | `crates/editor-core/tests/export.rs` |
| Encoder arguments and capabilities | `editor-media` export tests with real FFmpeg, skips reported |
| Audio length and sync | sample-exact length; a click at a known frame lands on the same frame in video and audio |
| Picture parity | the same plan through the viewer's compositor path and the export path gives equal pixels (±2) |
| Originals only | a proxied asset exports original pixels |
| Queue | pause, resume, cancel mid-frame, retry, failure, hardware→software fallback, part-file cleanup |
| End to end | MP4 H.264/H.265, MOV ProRes, WebM VP9, audio-only, range, 720p–4K, custom, NVENC on the RTX 4060 host |
| Interface | native captures at 1440×900 and 760×560 of the sheet, queue in progress, settings |

## Not claimed

HDR, 10-bit H.264/H.265 output, alpha-channel export, variable frame rate
output, audio crossfades on transitions, pitch-preserving speed, surround
sound, smart rendering (stream copy), distributed or parallel jobs, export
from the headless `neptune-video-agent` binary, and native acceptance on macOS or
Windows (VideoToolbox, AMF, QSV, MF are probed and driven by the same code but
were not run here).

## Tickets

| Id | Title | Depends |
| --- | --- | --- |
| 01 | Core export data, validation and presets | – |
| 02 | Encoding: capabilities, FFmpeg arguments, encoder, sound render, output check | 01 |
| 03 | Export source reader | 01 |
| 04 | Proxy service and cache | – |
| 05 | Frame renderer and shared layer assembly | 01, 03 |
| 06 | Render queue and job runner | 02, 03, 05 |
| 07 | Export dialog and render queue panel | 01 |
| 08 | Agent render operations | 01 |
| 09 | Window integration | 06, 07, 08, 10 |
| 10 | Preview quality and proxy switching | 04 |
| 11 | ADR 0009, glossary, architecture, performance, usage and installation | 01–08, 10 |
| 12 | Native verification and interface captures on Linux | 09 |
| 13 | Other platforms and hardware encoders | 12 |
| 14 | Export throughput, memory and proxy benefit | 12 |
| 15 | Follow-ups found during implementation | – |

## Comments

2026-10-14: Spec written from the subsystem map; implementation fans out by
module.

2026-10-14: Where what was built differs from the text above, the code and
these notes are what holds:

- Decision 9: saved export presets live in `export-presets.json` (rule 18),
  not in `presets.json`.
- Decision 11: Auto preview quality is Full until 4 of the last 16 frame
  advances of playing video layers were late, then Half until playback stops
  (rule 21); there is no 1920-pixel threshold.
- `range_cues` clamps word times into the cut cue instead of dropping words
  outside it. `plan` also refuses MP3 and Opus rates outside `SAMPLE_RATES`.
- Encoder arguments add `-nostats`; a sound-only encode has `-nostdin` and no
  input pipe; bitrate mode adds `-maxrate` (and `-bufsize` for x264, x265
  and NVENC).
- "Three output frames in flight" is the frame being drawn plus two handed to
  the writer (`IN_FLIGHT`). `shutdown` waits at most 750 ms. `attempts`
  counts starts. Only Done and Failed send a finished event. Two jobs may
  name the same destination; the last to finish wins.
- The export sheet is `min(760, screen − 64)` wide with two columns from
  640 pt. "Add to queue" queues the job paused.
- Proxies: the `fps` filter runs before `scale`; a proxy whose source cannot
  be read at all stays usable; `set_in_use` takes proxy paths.

Docs written (ticket 11); verification is ticket 12.

2026-10-14: Review changes. The fix lanes after review changed these rules;
the code and these notes hold over the text above:

- Estimate (`estimate_bytes`): quality mode is `suggested_kbps × factor`,
  the factor 0.85 for libx264 at Faster and 1.0 otherwise. Bits per pixel at
  Recommended are 0.14 for H.264 (was 0.10) and 0.11 for H.265 (was 0.065),
  so bitrate-mode suggestions and Media Foundation's quality target rise by
  40 % and 69 % (1080p30 H.264 Recommended: 8,709 kbit/s). The dialog shows
  the estimate with "≈".
- Rate control: NVENC quality is `-cq` at the x264/x265 value +5 (H.264) or
  +3 (HEVC), AV1 unchanged, with `-b:v 0 -maxrate 4× -bufsize 8×` the
  suggested rate. AMF and VA-API constant QP take +2 (H.264) and +1 (HEVC),
  NVENC's match standing in for theirs, unmeasured. QSV and AV1/VP9
  mappings are unchanged.
- Sound: the decoder filter starts `aresample=R:async=1:first_pts=0`, so
  sound keeps the file's time as the picture is read: a late audio stream is
  preceded by silence, Matroska seeks stay in place, and Opus can move a few
  milliseconds to agree with the picture. Playback shares it.
- Proxies (rule 19, decision 10): the frame grid starts at the file's start
  (`fps=N/D:start_time=0`, `PROXY_PROFILE` 2). A source with an alpha pixel
  format gets no proxy; the job fails with "it has transparency, which a
  proxy cannot keep, so the original is read". Budget eviction sends
  `ProxyEvent::Removed`; the window sets that asset's status from the lane,
  and the mode does not remake it until the library, project or mode
  changes. `ProxyService::shutdown` is called on exit and waits at most
  750 ms. A proxy file that fails to decode is not read again until it is
  removed, made again or replaced, not for the rest of the session. A layer
  chooses a proxy by the box it is granted (after the viewer budget, and
  while playing no larger than the box already granted), not the box asked
  for.
- Preview settings (decision 9): an unknown `quality` or `proxies`, or a
  non-numeric `proxy_budget_gib`, reads as the default and is written back
  unchanged until the person chooses that setting.
- Working folder: `.neptune-video-render-<id>`, hidden beside the destination; the
  system's temporary folder only when that one cannot be made. The part
  name is cut so that it stays within 255 bytes.
- Sweep (rule 20, extended to exports): at each job start, part files and
  working folders in the destination folder unchanged for an hour
  (`STALE_AFTER`) and of no listed job are removed; at queue start, the same
  for `neptune-video-render-<id>` in the temporary folder. Symbolic links are
  skipped.
- Finish watchdog: `Encoder::finish` runs on `neptune-video-render-finish`; FFmpeg
  gets at least `FINISH_DEADLINE` (600 s) plus the part file's size at
  `FINISH_PACE` (8 MiB/s), then as long as the file changes, and
  `FINISH_STALL` (120 s) without a change fails the job.
- Fallback exclusions (decision 6): a missing folder, a stalled finish, and
  an FFmpeg reason naming the disk or the file (no space, quota, read-only,
  name too long, I/O error, file too large on any line; permission denied or
  no such file only on a line about the part file or "Error opening
  output") fail at once with no software retry. When the software retry
  fails too, both reasons are kept.
- Queue destinations (rule 16): `enqueue` reads no file and compares names
  only (absolute, extension, sources, project files), for the destination
  and its caption file, case-insensitively on every platform. The render
  thread checks the file system as the job starts (folder, missing folder,
  links, `..`, and device and inode on Unix) and fails the job with the same
  text. Two unfinished jobs may no longer write the same file, export or
  caption file: `DestinationQueued` ("Another export in the queue already
  writes <name>; choose another name", `invalid_export`), and `retry` is
  refused while another unfinished job writes it. This replaces "Two jobs
  may name the same destination; the last to finish wins."
- Replacing: an existing caption file needs the same consent as the file.
  The agent gets `destination_exists` for it; the person is asked "Replace
  “X” and “Y”?".
- Held jobs: a job queued paused, or paused before its turn, is held.
  `resume_all` starts every held job in list order, `waiting()` lists the
  jobs `move_by` moves among (held ones included), `QueueSummary.held`
  counts them apart from `queued`, and the close guard counts active,
  queued and held jobs. The panel says "Held" and "Held until you start
  it", and offers "Start all".
- Interface: the poster is drawn from the originals by its own
  `neptune-video-poster` thread (`POSTER_SIDE` 640, `SETTLE` 150 ms), not from the
  viewer's texture. Shift+Cmd/Ctrl+E opens the render queue, and each chord
  opens its sheet over the other. Closing the sheet unqueued keeps its state
  for the same project; `export_preset` in the settings remembers the
  preset chosen. Presets are one dropdown grouped by destination, saved ones
  last, with one note line and a row of quick chips.

2026-10-14: Changes after the native verification (ticket 12); the code and
these notes hold over the text above:

- Export FFmpeg threads (resource policy): libx264 gets `-threads:v` x264's
  own count, 1.5 × the logical processors, capped at 24
  (`X264_MAX_THREADS`); libsvtav1 gets `-svtav1-params lp=4`
  (`SVT_AV1_PARALLELISM`); each source reader pipe decodes with `-threads 4`
  (`DECODE_THREADS`) beside its 2 filter threads and 1 output thread. Other
  encoders, hardware included, get no thread option. A 4K libx264 encode
  still peaks near 4 GiB; a lower lookahead would cut it but changes the
  output, and is left to a product decision (ticket 14).
- Decision 8, quitting: Stop and quit cancels every job at once, waiting
  ones first so none starts, before unsaved changes are asked about; if the
  person then stays, the jobs stay cancelled and can be retried.
- Render queue panel: the job under way is listed first, then waiting and
  held jobs in render order, then finished jobs newest first; a job that
  starts is scrolled into sight once. "Finishing posts a brief notice" holds
  only while the panel is closed: with it open, its row tells the result.
  While any sheet is open, notices sit at the bottom of the window, under
  it.
- The export sheet's folder button names the folder as the queue rows do
  (`short_folder`), the full path on hover.
