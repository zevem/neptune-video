---
status: accepted
---

# Recovery ownership uses an OS lock

Keep active recovery ownership in an exclusive OS lock on the adjacent
`.aster.lock` file, held through pending recovery writes; a second editor probes
the lock instead of trusting a PID or file contents. Leave the sidecar in place
after release so another process cannot acquire a replacement inode while an
old handle still owns the first one. The handle's lifetime releases ownership
after exit/crash without a stale-owner timeout, while the lease deliberately
covers recovery rather than concurrent manual-save conflict resolution.
