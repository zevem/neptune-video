# Independent mix loudness review

Reviewed the new `AudioRender::measure_loudness` pipeline, MCP mix job,
clip measurement routing and normalize rejection against the existing export
PCM mixer and `editor_core::sound` admission rules.

## Corrected findings

1. The generic picture/media root check refused mix analysis for fonts,
   picture-only media, muted or soloed-out sources and media outside the
   requested range. `check_mix_readable` now uses `Sounding::of`, `sounds`,
   `audible_run` and the requested sample span to check only contributing
   audio source paths. Track selection keeps every track and clip, preserving
   detached-audio ownership and the original solo flags.
2. `OfflineMix::next` always rendered 1024 sample frames, and callers truncated
   afterward. A final short chunk could open sources beyond the requested
   range. Added `next_up_to(remaining)`, used by both the export writer and
   loudness meter. A pure control requests one silent sample while 17 clips
   start later in that chunk and verifies no decoder process starts.
3. Per-track results now say each selected track is measured alone through its
   own master limiter with existing mute/solo/ownership retained; LUFS and peak
   values are not additive. Range sample counts reuse export::sample_count.
4. Mix measurements check cancellation before starting each meter and after
   its result, preventing further per-track work after a known cancellation.

## Retained behavior verified from code

- Measurement and export use the same `OfflineMix::with_layout`, including
  clip/track gains, fades, steady retimes, audio effects, ownership and the
  master limiter. They share the same bounded final-chunk path.
- A measurement's meter and decoders are registered in its task-owned
  `Processes`. The job's cancellation watcher triggers `AudioRender`'s
  canceller, which kills those registered children. Process drop kills/reaps
  remaining task-owned children. PCM chunks and stderr tail remain bounded.
- Analysis takes an immutable authority snapshot. It never writes project
  state. Result metadata carries project/context/revision and explicit sample
  rate/channel count/stage.
- Clip normalization refuses a mix result; original clip measurements and
  sound-key stale-data checks remain the normalization input.

## Coverage and limits

Added pure root/selection controls in `mcp::tools::mix::tests` and the bounded
sample test in `editor_media::audio::tests`. The root agent owns compilation,
media preflight and integration execution and records actual test results.
The meter reports post-limiter PCM at the stated sample rate and layout;
encoded output may differ after codec encoding. A range starts a fresh mixer,
as rendering that range does. No native audio-device acceptance is claimed.
