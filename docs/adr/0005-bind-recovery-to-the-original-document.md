---
status: accepted
---

# Bind recovery to the original document

Named recovery archives capture the original document session's canonical
destination and baseline, recording expected absence or exact byte count and
hash when the snapshot is submitted, then restore that authority without
rebasing to today's ordinary file so another writer's intervening save remains
a conflict. Legacy and private recovery restores dirty unsaved work and
requires a create-only Save As to acquire a destination. Keeping the archived
binding immutable can conservatively conflict after this editor's own in-flight
save, preserving work instead of silently adopting a newer baseline.

The persistent `.write-lock` advisory lock covers the byte count/SHA-256 recheck
and atomic replacement for cooperating writers; the recovery lease owns only
recovery. This is not a global filesystem compare-and-swap: external programs
that ignore the lock can race the final check and replacement.

The [headless CLI](../../src/bin/aster-agent.rs) and
[desktop](../../src/app.rs) implement this recovery Save As restriction.
[Protocol verification](../agent-protocol.md) and the
[db30 native GTK recovery run](../../artifacts/native/ai-round8/report.json)
cover refusal of existing destinations and adoption of a fresh file. The
desktop keeps an unbound restored origin until successful adoption, then permits
deliberate ordinary Save As to replace an explicitly selected valid destination.
This local Linux evidence does not establish atomic replacement on other OSes.
