# Proxy service and cache

Status: ready-for-agent
Progress: resolved
Type: task

## Scope

`ProxyService` on an `aster-proxy` thread: queue, dedup, cancel, the H.264 or
MJPEG recipe, manifests, keys, LRU budget, in-use protection, sweeping.

## Comments

2026-10-14: Implemented in [proxy.rs](../../../crates/editor-media/src/proxy.rs).
16 tests passed with FFmpeg 8.0.1 in a debug build, none skipped: a 2560x1440
source gives a 1280x720 H.264 proxy with no sound and the original's frames;
forced MJPEG; a rotated source gives an upright proxy; cancel stops a paced
encode and leaves no part file; restart reloads manifests and drops stale or
damaged ones; eviction skips proxies in use.

Not verified: macOS and Windows; MJPEG on an FFmpeg really without libx264;
HDR and 10-bit sources. Two sessions sharing one cache can evict each other's
proxies.
