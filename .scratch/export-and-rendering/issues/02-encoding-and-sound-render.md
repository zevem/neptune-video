# Encoding: capabilities, FFmpeg arguments, encoder, sound render, output check

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

`editor_media::export`: `probe_capabilities`, the crate-private `encode_args`,
the stdin-fed `Encoder`, `AudioRender` / `render_audio` through
`OfflineMix::with_layout`, `verify_output`, and `Process::spawn_writer` with a
write deadline. Contract review rules 1 to 11.

## Comments

2026-10-14: Implemented in
[editor-media/src/export](../../../crates/editor-media/src/export/mod.rs).
With FFmpeg 8.0.1 on the RTX 4060 Linux host, 41 export tests passed with
none skipped (17 argument tests, 3 probe and verify unit tests, 12
integration tests, 9 reader tests); without FFmpeg the 18 that need it print
a skip. Encoded and read back: x264 + AAC in MP4 (BT.709 tags, `mov_text`,
audio time base 1/48000), NVENC H.264, H.265 and AV1, x265 tagged `hvc1`,
ProRes 422 + PCM 24 in MOV, VP9 + Opus with WebVTT in WebM, M4A, WAV 16 and
24-bit, FLAC, MP3. The probe took about 0.9 s and offered the software
encoders and NVENC; Quick Sync and VA-API failed to initialise here.

Not run: AMF, Quick Sync, VideoToolbox, Media Foundation and VA-API encodes
(arguments only); an MOV with an embedded `tx3g` stream; ProRes 4444;
export-length libsvtav1 and libaom encodes.
