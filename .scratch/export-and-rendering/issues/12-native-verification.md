# Native verification and interface captures on Linux

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 09

## Scope

Drive the final native build on the Linux host: the export dialog and the
render queue at 1440x900 and 760x560, in progress, paused, failed and done;
the settings sheet's Preview section; an end-to-end export of each format in
the spec's acceptance table, including NVENC; a proxied asset exporting
original pixels; quitting with a job running. Record the host, build and what
was not tried in `docs/verification.md#export-and-rendering`, and fill the
measurements in `docs/performance.md#export-and-proxy-budgets` if any are
taken.

## Comments

2026-10-14: Driven on the Linux host (i9-13900HX, RTX 4060 Laptop, driver
595.99.02, Ubuntu 26.04.1, FFmpeg 8.0.1) with the release build, `aster`
SHA-256 `3452f930…f2e572`, on private Xvfb displays at 1440x900 and 760x560.
Record: `docs/verification.md#export-and-rendering`; measurements:
`docs/performance.md#release-build-export-measurements`; raw files:
`artifacts/export-phase7/verify-20261014-093405/` (local, not tracked).

One defect fixed in the run: `Control::hold` reported a wait when not
paused, so speed and time left were never shown; a regression test covers
it. Module order in `crates/editor-core/src/lib.rs` was reformatted.

## Answer

Verified on Linux: every format in the spec's end-to-end row (MP4 H.264 and
H.265, MOV ProRes 422 HQ with PCM, WebM VP9 with Opus, M4A, WAV, MP3, FLAC,
AV1 software over a range), 720p to 4K, a custom size, 25 fps from 29.97, a
range, a portrait preset on a 16:9 canvas and a portrait canvas, caption
sidecars, embedding and burn-in, NVENC H.264 and HEVC on the RTX 4060, and
an export started from the sheet's button. Frame counts and durations
matched; sound-only lengths were sample-exact; the click landed within
0.03 ms of its frame in every video export with sound (WebM within Opus's
7 ms start). A proxied 4K asset exported original pixels (41.4 to 41.9 dB
PSNR against 33.6 dB for the proxy). Cancel, pause, resume, retry of a
cancelled job, a job failing on a removed folder, every refusal code and
quitting with a job running behaved as specified. Captures at both sizes:
the sheet, the queue rendering and done, the settings' Preview section, the
proxy badges, the toolbar ring and the quit and save questions.

Not done here and not in tickets 13 or 14: captures of a paused and a failed
row; Add to queue, Start all and the pause and resume buttons through the
window (driven through the agent); retrying a failed job; the Fill fit;
96 kHz and mono sound; libaom AV1; picture parity with the viewer beyond
looking. Those and defects 4 to 7 of the record are listed in ticket 15.
Other platforms, other hardware encoders and the automatic software
fallback on hardware are ticket 13; long renders, GPU memory, the window's
frame time during a job, the export FFmpeg thread cap and proxy benefit are
ticket 14.
