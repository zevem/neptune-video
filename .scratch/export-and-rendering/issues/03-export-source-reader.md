# Export source reader

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

`SourceReader`: sequential full-quality decode of originals with the
preview's seek and frame-grid arguments and a better scaler, one pipe per
lane, asset and size, its own bounds (8192 a side, 256 MiB a frame), stills
and GIFs, cancel from any thread.

## Comments

2026-10-14: Implemented in
[reader.rs](../../../crates/editor-media/src/export/reader.rs); its 9 tests
are among the 41 of ticket 02. Long-GOP H.264 matched a decode from zero
byte for byte at native and half size through in-order reads, gaps, repeats
and backward and far jumps; a lossless clip matched the preview's
`DecoderPool` byte for byte. 4K and 8K read throughput was not measured.
