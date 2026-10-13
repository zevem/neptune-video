# Repository verification scripts

`check-architecture.py` validates Cargo dependency ownership and direct forbidden
I/O references in the pure domain core. It reads source only and launches no
desktop or media processes. Run it after dependency or ownership changes;
use `--self-test` when changing the checker itself.

Native tools introduced later must use task-owned media/project/settings roots,
retain exact child-process handles and clean up only their own processes. Avoid
shared desktop input concurrently with another agent. Record native captures
after the final build, actual host/backend and unverified platform behavior.
