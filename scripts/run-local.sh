#!/bin/sh
# Launch this checkout's optimized build with optional task-local media tools.
set -eu

aster_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
aster_binary="$aster_root/target/release/aster"
if [ ! -x "$aster_binary" ]; then
    printf '%s\n' 'Build first: cargo build --release --locked --bins' >&2
    exit 1
fi

if [ -z "${ASTER_FFMPEG:-}" ] && [ -x "$aster_root/.tools/ffmpeg/usr/bin/ffmpeg" ]; then
    ASTER_FFMPEG="$aster_root/.tools/ffmpeg/usr/bin/ffmpeg"
    export ASTER_FFMPEG
fi
if [ -z "${ASTER_FFPROBE:-}" ] && [ -x "$aster_root/.tools/ffmpeg/usr/bin/ffprobe" ]; then
    ASTER_FFPROBE="$aster_root/.tools/ffmpeg/usr/bin/ffprobe"
    export ASTER_FFPROBE
fi

exec "$aster_binary" "$@"
