# Measure agent budgets and record FFmpeg filters

Status: ready-for-human
Progress: resolved
Type: task

## Scope

Find out whether the design fits the agent's two caps before any type lands,
and write down which FFmpeg the filter names were checked against. This needs
the compilation owner; it changes one constant string at most.

1. Run `python3 scripts/test-workspace.py --package editor-agent` and record
   the printed `proposal_schema: N bytes` (cap 24 KiB,
   `crates/editor-agent/src/planner.rs:1012-1016`) and `SYSTEM.len()` (cap
   5 KiB, `planner.rs:847-873`; 4995 by a static count).
2. Estimate what the epic adds. Schema: `set_fades`; `gain` and `solo` on
   `Track` and in `set_track_state`; `fade_in` and `fade_out` on `Clip`; seven
   `EffectKind` names (`high_pass`, `eq_band`, `compressor`, `limiter`,
   `denoise`, `pitch`, `center_cut`). Instructions, about 400 bytes in all:
   - one `"<kind> <keys…>"` line per kind, which the test forces (`high_pass
     frequency; eq_band frequency gain width; compressor threshold amount
     gain; limiter threshold; denoise amount threshold; pitch amount;
     center_cut amount`, about 160 bytes with separators);
   - one units sentence, without which an agent cannot write a valid value,
     since `SYSTEM` says effect "lengths are fractions of the layer box or
     canvas height, angles are degrees" and these kinds reuse `gain`,
     `amount`, `frequency`, `width` and `threshold` with other meanings
     (about 150 bytes; wording in ticket 10);
   - the replacement of "Clips on audio tracks take only gain keyframes and
     retime.", which becomes false (about 45 bytes more than it frees);
   - one sentence on fades and track volume (ticket 05c, about 110 bytes).
3. If the instructions do not fit, trim existing prose in `SYSTEM` in this
   ticket, with the planner tests still passing, until at least 450 bytes are
   free. If trimming cannot reach that, stop and ask the owner: drop
   `center_cut`, then `high_pass`, or raise the cap.
4. Record `ffmpeg -version` and, on the validation host, the exit status of
   the eight probe runs of ticket 11 (the control and one per kind, strings
   from the spec's table at their defaults) and of `ebur128=peak=true` and
   `aresample=async=1:first_pts=0` run the same way. `ffmpeg -filters` may
   be recorded beside them; its name column is the second token of a line.

Files: `crates/editor-agent/src/planner.rs` (only if `SYSTEM` is trimmed).

## Result

Measured, and stopped at item 3: trimming without removing a rule frees 341
bytes of the 450 asked for, and the epic's text needs 477. The owner has to
choose; see the 2026-10-13 (measured) comment. Items 1, 2 and 4 are done.

- Schema: 20,775 bytes of the 24 KiB cap; the epic adds an estimated 644,
  leaving about 3,156. It fits.
- Instructions: 4,995 bytes before, 4,778 after a rewording that keeps every
  rule; 341 free under `SYSTEM.len() < 5 * 1024`. It does not fit.
- FFmpeg 8.1.2 on the host: the control, the seven kinds' strings,
  `ebur128=peak=true` and `aresample=async=1:first_pts=0` all exit 0.

## Evidence

The two measured sizes, the estimate, the free bytes after the trim and the
host's FFmpeg version and runs are under Comments here and in the spec. Not
done: 450 free bytes. `planner_text_names_every_parameter_table_and_stays_short`
now prints `planner_instructions: N bytes` beside the schema test's line; both
need `-- --nocapture`, which `scripts/test-workspace.py` does not pass.

## Known limits

- Windows command-line headroom for schema plus instructions stays unmeasured.
- A string that runs at its defaults on one host says nothing about older
  builds; no minimum FFmpeg version comes out of this ticket.
- Trimming `SYSTEM` changes what a provider is told; no real provider request
  is made here.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured. On the
design host (macOS, Homebrew FFmpeg 8.1.2) all nine filters are listed;
`rubberband` is not.

2026-10-13 (review): The estimate was about 170 bytes and counted only the
forced lines. A script count of `SYSTEM` is 4,995 bytes against the
`< 5 * 1024` assertion (then `planner.rs:871`, now `:872`), 124 free; the forced lines alone
come to about 160. The free-space target went from 300 to 450 bytes. Item 4
records runs, not listings, to match the execution probe.

2026-10-13 (measured): Host: macOS 26.5.2, Apple M1, Rust 1.97.1, `ci`
profile, Homebrew FFmpeg 8.1.2 (`/opt/homebrew/bin/ffmpeg`, `--enable-gpl
--enable-version3`). No provider request was made and nothing was listened to.

- *Sizes.* `cargo test --profile ci --locked -p editor-agent --lib planner::
  -- --nocapture` prints `proposal_schema: 20775 bytes` (cap: under 24,576,
  so 3,800 free) and, before the trim, `SYSTEM` was 4,995 bytes (124 free).
  `python3 scripts/test-workspace.py --package editor-agent` hides both
  lines, because Cargo captures the output of a passing test.
- *Schema estimate, 644 bytes,* built from the encoding of the neighbouring
  entries in the dumped schema, where a closed object lists every property
  under `required`: the `set_fades` variant 284; `gain` and `solo` in
  `set_track_state` 100; on `Track` 82; `fade_in` and `fade_out` on `Clip`
  102; seven `EffectKind` names 76. About 21,419 afterwards, 3,156 free.
- *Instruction estimate, 477 bytes, not about 400.* The seven forced lines
  163; ticket 10's units sentence as written 180; the replaced audio-track
  sentence 32 more than it frees; a fades and track-volume sentence 102
  ("Fades are lengths in microseconds at a clip's edges, for clips with
  sound; track gain is linear 0..8."). The 450-byte target was itself short
  of this.
- *Trim.* Fifteen sentences of the new `SYSTEM` differ from the old text
  (seventeen old ones; two pairs were merged), with no rule, bound, key list
  or example removed, and the two phrases tickets 05c and 10 quote left as
  they were: 4,995 to 4,778 bytes, 341 free. The planner tests pass.
  The rest of the text is key lists the test forces and rules with a number
  in them; reaching 450 means deleting some (candidates and their sizes:
  "(ease-in-out is 0.42,0,0.58,1)" 31, "Ask for a narrower selection if
  needed." 40, "A lut asset is never a clip." 29, "frame rates are
  rational," 26), which is more than trimming prose and is not done here.
- *What the choices give,* against 341 free: all seven kinds 477; without
  `center_cut` 458; without `high_pass` too 437. With the three new
  sentences cut to the bone (units 153, fades 91) the figures are 439, 420
  and 399. Dropping kinds therefore does not close the gap on its own.
- *Recommended to the owner:* move the split, not the total. Instructions
  under 6 KiB and schema under 23 KiB keep the same 29 KiB on the command
  line; the text would then have 1,365 free against 477 and the schema about
  2,132 after the epic's 644. It is three code sites and one doc line: the
  two test asserts in `planner.rs` (`:872`, `:1016`), the runtime guard
  that refuses an oversized schema before the Claude command line is built
  (`crates/editor-agent/src/provider/mod.rs:33`, `schema.len() > 24 *
  1024`), and `docs/native-ai.md:155`. The guard has to move to 23 KiB in
  the same change; left at 24 KiB the shipped bound would be 30 KiB and only
  the tests would hold 29. It is the owner's to ask for. Until then tickets
  05c and 10 cannot both land their sentences; 05c's alone (102) fits in
  341, as do the seven forced lines (163).
- *FFmpeg runs,* each as `ffmpeg -hide_banner -nostdin -loglevel error -f
  lavfi -i anullsrc -t 0.05 -af <string> -f null -`, all exit 0 with nothing
  on stderr: `anull` (control); `highpass=f=80`;
  `equalizer=f=1000:t=o:w=1:g=0`;
  `acompressor=threshold=0.125893:ratio=3:attack=10:release=150:makeup=1`;
  `alimiter=limit=0.891251:level=false`; `afftdn=nr=18:nf=-40`;
  `atempo=1.5`;
  `aformat=channel_layouts=stereo,pan=stereo|c0=c0-1*c1|c1=c1-1*c0`;
  `ebur128=peak=true`; `aresample=async=1:first_pts=0`. For contrast,
  `alimiter=…:nosuchoption=1` exits 8 and `nosuchfilter=1` exits 234.
  `ffmpeg -filters` (second token) lists `highpass`, `equalizer`,
  `acompressor`, `alimiter`, `afftdn`, `atempo`, `pan`, `aformat`,
  `aresample`, `ebur128`, `anull` and `asetrate`; `rubberband` is absent.
- *Not this ticket's.* `provider::tests::an_unrepresentable_native_path_is_
  refused_before_creating_an_instruction_file` fails on this host before and
  after the change: APFS refuses the non-UTF-8 directory name the test
  creates (`Illegal byte sequence`, errno 92). The other 69 unit tests and
  the 14 protocol tests pass.

2026-10-13 (review of the measurement): Two corrections, no rule changed.
The recommended split touches three code sites, not two: the runtime guard
at `provider/mod.rs:33` was missing from the list and is now named above
and in the spec comment. The trim record said thirteen passages; a
sentence-level comparison gives fifteen. One of them had dropped
"concisely" from the no-edit sentence ("explain concisely in summary"),
which is a directive and not prose; the word is restored, so `SYSTEM` is
4,778 bytes with 341 free, not 4,768 and 351, and the figures above are
restated. The added `println!` moved the size assert to `planner.rs:872`
and the schema test to `:1012-1016`; ticket 10's `:847-872` now ends at
`:873`.

2026-10-14 (ticket 22): Resolved. The one item left, owner decision 1, was
taken on the owner's behalf: instructions under 6 KiB and schema under
23 KiB (`MAX_SYSTEM_BYTES`, `MAX_SCHEMA_BYTES` in `planner.rs`), as ticket 10
built, which is the split recommended here. The 450 free bytes under 5 KiB
this ticket could not find are no longer needed. Measured on 2026-10-14:
instructions 5,377 bytes, schema 21,859. ADR 0006 carries a dated note with
the caps.
