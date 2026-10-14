# Text renderer and text lane

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 03

## Scope

`editor-media::text` on vello_cpu, skrifa and harfrust: font index (font assets, bundled Geist, system fonts), shaping with fallback, layout, background, shadow, outline, fill, colour emoji, per-unit animation, karaoke and graphic elements; a bounded lane with stale-safe results and a picture budget.

## Comments

Done: `editor-media::text` (font book over bundled, project and system faces with lazy reads and a 192 MiB read-face budget; harfrust shaping with per-character fallback; greedy wrap; scene of background, blurred shadow, outline and fill; per-unit motion; karaoke word, box and fill; graphic elements) and the `aster-titles` lane (newest request per title, 64 waiting titles, 64 MiB of undelivered pictures). Synthetic bold is a stroke of the fill's colour, since glifo's emboldening is in font units for fills. Font files import as `Font` assets named for their family. Tests in `crates/editor-media/src/text/tests.rs`; the emoji test reports SKIP without a colour emoji face.
