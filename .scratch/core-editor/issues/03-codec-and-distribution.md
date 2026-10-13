# Specify and verify codec deployment and distribution

Status: needs-triage
Progress: claimed
Type: task

## Task

Define target OS/architecture packages, codec availability, third-party notices,
license policy, signing/notarization and update behavior. Verify clean-machine
launch without developer-shell assumptions. Bound malformed-media processing
and record the supported-format matrix and hardware-decoding policy.

## Completion criteria

The release spec identifies deployed runtimes, formats, signing policy and
update/recovery behavior. Test installers/bundles on each declared platform and
retain launch/media/input/storage evidence. Source builds and configured CI jobs
remain separate from accepted packages.

## Comments

2026-10-12: Packaging and release policy require a separate scoped deliverable;
this initial core build does not claim signed installers or production codec
deployment.

2026-10-13: The user selected private/proprietary source. The root LICENSE and
third-party notices record that choice while preserving inherited skill/font
licenses. The [portable packager](../../../scripts/package-release.py) verifies
native source/binary identity and available notices without executing tools or
bundling codecs. Thirteen packaging guards pass. The current Linux local trial
archive is reproducible across two actual builds, and thirteen native editing
checks pass through its extracted launcher from a Unicode/space path. Desktop
template validation passes; no menu integration is installed. FFmpeg remains an
external runtime dependency. Minimum-host/platform installer acceptance, missing
dependency notice texts, signing and public distribution remain open.
