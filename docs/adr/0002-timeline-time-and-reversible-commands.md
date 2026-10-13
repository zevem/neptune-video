---
status: accepted
---

# Integer timeline time and reversible targeted commands

Timeline endpoints use integer microseconds with a rational project frame rate;
frame boundaries derive from their frame index rather than repeated rounded
additions. Persisted edits use typed serializable commands with stable targets
and a reversible history boundary, shared by the UI and agent callers.
Floating-point timestamps and direct widget mutations are simpler initially
but make long fractional-frame timelines, reproducible agent edits and
save/recovery semantics depend on accumulated rounding or current UI selection.
