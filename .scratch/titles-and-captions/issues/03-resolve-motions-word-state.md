# Resolve: motions and word state

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02

## Scope

`LayerSource::Text` and `LayerSource::Graphic` carry style, enter/exit motion progress and the karaoke word state at the frame-snapped clip time; `title::unit_progress` spreads a progress over units.

## Comments

Done in fa1c1bc: `LayerSource::Text` carries style, enter/exit `Motion` and `WordState`; `LayerSource::Graphic` carries elements with motions; progress at the frame-snapped clip time.
