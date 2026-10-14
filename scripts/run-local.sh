#!/bin/sh
# Launch this checkout's optimized build with the FFmpeg Aster ships.
set -eu

aster_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
aster_binary="$aster_root/target/release/aster"
if [ ! -x "$aster_binary" ]; then
    printf '%s\n' 'Build first: cargo build --release --locked --bins' >&2
    exit 1
fi

# A build in this checkout runs the FFmpeg fetched into .tools/media by itself;
# fetch it once. Explicit ASTER_FFMPEG / ASTER_FFPROBE overrides still win.
if [ -z "${ASTER_FFMPEG:-}" ] || [ -z "${ASTER_FFPROBE:-}" ]; then
    if ! python3 "$aster_root/scripts/fetch-media-tools.py" >/dev/null; then
        printf '%s\n' 'Could not fetch FFmpeg; Aster will look for it on PATH.' >&2
    fi
fi

exec "$aster_binary" "$@"
