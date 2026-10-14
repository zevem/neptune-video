# Inspector Text, Graphic and Animate sections

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 06

## Scope

Text: content, font family/weight/italic, size, fill, outline, shadow, background, alignment, spacing, wrap, case. Animate: enter/exit preset, unit, duration; karaoke highlight. Graphic: element texts, shared colours and font. One entry a gesture.

## Comments

- A title offers Video, Text, Animate and Motion; a graphic offers Video, Graphic, Animate and Motion. A media clip is offered neither.
- Text: a multiline field (one step when the field is left), and a Language fact on a caption track. Font: family, weight and italic, then size and colour. Layout: alignment, line height, spacing, wrap width and case. Outline, Shadow and Background are switch cards, each switched look one step with its rows. Lengths kept in ems are shown and written as pixels at the title's size.
- Families are asked of the lane only when a Text section first opens, so the system's faces are not walked at start. The list joins the lane's families, the project's font assets and Geist. A family that is not on this computer is named, and Geist is said to stand in for it.
- Animate: In and Out cards, each with style, order, duration and delay (Out: "Ends before"). A new style takes its usual order. Durations and delays are whole frames at the project rate and fit the room the clip leaves; a clip too short for one says so. Karaoke can be switched on for any title and says that words light up only on captions with word times.
- Graphic: one field per Text element, labelled with its name. Shared colours are listed by the first element using each, and a change recolours every element using it. One font is applied to every Text element. Animate gives every part the same entrance and exit, with a "Between parts" stagger in whole frames; the first part in leaves last.
- Every edit is a draft until the pointer is released or the field is left, then one history step (SetText, SetTextStyle, SetTextAnimation or SetGraphic as each part differs).
- Tests: eight inspector tests in `src/ui/inspector/text_tests.rs`, and the section offer test is updated. Desktop: 617 passed. Clippy `-D warnings` and the architecture check pass.
- Native captures of the debug build under Xvfb + lavapipe: caption selected with text-box handles; Text, Animate and Graphic sections at 1440×900; the In style list and Typewriter chosen; and the Text section at 760×560. Sound reports no device on this host.
