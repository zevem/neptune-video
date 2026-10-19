# Repository verification scripts

`check-architecture.py` validates Cargo dependency ownership and direct forbidden
I/O references in the pure domain core. It reads source only and launches no
desktop or media processes. Run it after dependency or ownership changes;
use `--self-test` when changing the checker itself.

`fetch-media-tools.py` downloads the pinned FFmpeg build for one platform,
checking each download's SHA-256 and trying again after a timeout or dropped
connection; `test-fetch-media-tools.py` checks those retries with a stand-in
network and runs in CI.

`export-icons.py` encodes the native icons from the supplied artwork in
`assets/branding`, which it never writes; `test-export-icons.py` checks that
artwork and that the committed icons are its export. Both need Pillow.

Native tools introduced later must use task-owned media/project/settings roots,
retain exact child-process handles and clean up only their own processes. Avoid
shared desktop input concurrently with another agent. Record native captures
after the final build, actual host/backend and unverified platform behavior.
