# Render queue and job runner

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02, 03, 05

## Scope

`RenderQueue` on `aster-render`: probe, enqueue checks, order, pause,
resume, cancel, retry, remove, reorder, events, shutdown; the job runner:
re-plan, sound, captions, encoder, writer thread, verify, rename, cleanup,
hardware-to-software fallback.

## Comments

2026-10-14: Implemented in [queue.rs](../../../src/export/queue.rs) and
[engine.rs](../../../src/export/engine.rs). On the RTX 4060 host with FFmpeg
8.0.1, 17 queue tests (fake runner) and 12 engine tests (real FFmpeg and
GPU) passed, also on llvmpipe, three runs in a row; without FFmpeg the
engine tests skip. Proven: MP4 H.264 + AAC, sync of a white frame and a tone
onset within one frame, a range, exact WAV length, WebM VP9/Opus and MOV
ProRes/PCM 24, burn-in on and off, cancel mid-render leaving no part file,
temp folder or FFmpeg process, forced NVENC failure finishing in software,
pause and resume, sidecar and embedded captions, letterboxed scaling.
Measured whole jobs: 320x180 NVENC, 60 frames in 0.42 s; 1920x1080 NVENC, 60
frames in 0.60 s (1.17 s on llvmpipe); build profile not recorded.

Not run: fallback from AMF, Quick Sync, VideoToolbox, VA-API or Media
Foundation; long or 4K renders. A proxied asset exporting original pixels is
not among these tests (the export modules have no access to proxies).
