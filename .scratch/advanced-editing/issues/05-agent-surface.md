# Agent capability list, planner text, context projection, schema cap

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02

## Scope

The new commands on the external protocol and in native planning.

## Result

Every new command is in the capability list and the full command schema, and
available to native plans. The planner instructions (`SYSTEM` in
`crates/editor-agent/src/planner.rs`) carry units, the parameter keys of each
effect kind and the retime, transition, tracker and look rules. The context
projection sends at most 32 keys of an animation track or tracker and a count
beyond that. The command panel gained phrases for effects, looks, blend,
transitions, reverse, freeze, keyframes, chroma key, follow and stop following
(`src/agent.rs`, 21 example rows).

## Evidence

`crates/editor-agent/tests/protocol.rs`, planner tests (provider schema under
24 KiB, instructions under 5 KiB and naming every table key, look and motion
commands planned and committed), `src/agent.rs` tests. Ten phrases were typed
natively in wave 3 and follow/unfollow in wave 4.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- No real Claude or Codex request was made with the new instructions.
- Windows command-line headroom for the schema plus instructions was not
  measured.
- An agent cannot start tracking or a stabilization analysis; it can write
  tracker keys and add a `stabilize` effect.
- "remove transition" takes the selected clip's own transition and reaches the
  next clip's only when "at end" is said.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
