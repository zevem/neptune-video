# Preview quality and proxy switching

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 04

## Scope

Preview quality (Auto, Full, Half, Quarter) on the viewer's decode boxes and
compositor canvas; proxy requests by mode; per-layer proxy switching with a
consumer reset; in-use reporting; the settings sheet's Preview section and
the library's proxy badge and menu items. Contract review rules 19 to 21.

## Comments

2026-10-14: Wired in [src/proxies.rs](../../../src/proxies.rs),
[src/textures.rs](../../../src/textures.rs), the viewer, the settings sheet
and the library. Tests are declared in `proxies_tests.rs`,
`textures_proxy_tests.rs`, `ui/preview_quality_tests.rs` and
`ui/settings_sheet_tests.rs`; no run result was reported to the docs owner.
Whether proxies or a lower quality make large media play smoothly has not
been measured.
