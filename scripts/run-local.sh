#!/bin/sh
# Launch this checkout's optimized build with the FFmpeg Neptune Video ships.
set -eu

neptune_video_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
neptune_video_binary="$neptune_video_root/target/release/neptune-video"
if [ ! -x "$neptune_video_binary" ]; then
    printf '%s\n' 'Build first: cargo build --release --locked --bins' >&2
    exit 1
fi

# A build in this checkout runs the FFmpeg fetched into .tools/media by itself;
# fetch it once. Explicit NEPTUNE_VIDEO_FFMPEG / NEPTUNE_VIDEO_FFPROBE overrides still win.
if [ -z "${NEPTUNE_VIDEO_FFMPEG:-}" ] || [ -z "${NEPTUNE_VIDEO_FFPROBE:-}" ]; then
    if ! python3 "$neptune_video_root/scripts/fetch-media-tools.py" >/dev/null; then
        printf '%s\n' 'Could not fetch FFmpeg; Neptune Video will look for it on PATH.' >&2
    fi
fi

exec "$neptune_video_binary" "$@"
