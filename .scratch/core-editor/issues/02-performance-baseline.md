# Establish release performance and resource baselines

Status: ready-for-agent
Progress: open
Type: task

## Task

Measure the workloads in [performance](../../../docs/performance.md) using the
final optimized native executable and controlled fixtures. Include startup,
idle, timeline interactions at scale, random seek, source-rate preview, audio
sync and save-under-load. Observe subprocess and GPU resources separately from
the desktop's RSS.

## Completion criteria

Retain repeated raw samples and distributions with actual host, profile,
codec/resolution/rate, cache conditions, power mode and competing workload.
Publish regression budgets from evidence. Explain remaining bottlenecks and
unverified cases. Headless reducer throughput remains distinct from native
input-to-display and decode performance.

## Comments

2026-10-12: Neptune's measurements do not transfer to Neptune Video.

2026-10-12: Neptune Video now has a full-timeline 5,000-clip metadata run and one release
software-decoder/worker run of 120 synthetic H.264 frames. The metadata run
averaged 421.812 µs per move/undo and 2.451 µs per active query over 0–499.9
seconds. Optimized H.264 worker preview measured 517.815 frames/s and GIF
interval-reusing worker preview 2,755.856 requests/s; these exclude native
presentation. Two paused X11 samples measured about 0.98%/0.90% of one CPU core
and 226.10/503.54 MiB process-tree RSS. Repeated native GPU/input/display/audio,
idle/minimized, random-seek and large-project distributions remain open; see
[performance](../../../docs/performance.md).
