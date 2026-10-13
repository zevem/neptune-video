# Verification and release readiness

Aster's requested core features now have Rust implementations and native Linux
editing evidence. Production readiness, CapCut/Palmier Pro parity and extreme
performance remain goals requiring the gates below.
Neptune was inspected read-only as the stack and engineering reference.

## Current performance-only checkpoint

The [performance acceptance handover](../artifacts/performance-acceptance/round1/REPORT.md)
records the current Linux source `b09ea17e` (96 source/build files), optimized
`aster` `9b9ff3f0` and `aster-agent` `cace9ba5`. The
[exact build record](../artifacts/performance-acceptance/round1/final-build.json)
passes **310 focused tests** (57 core, 253 desktop), zero failures/ignored,
formatting, architecture, denied-warning core/desktop all-target Clippy,
explicit FFmpeg/ffprobe 8.0.1 preflight and locked release linking.
The prior 610-test whole-workspace result belongs to its preceding source.

[Declared budgets](../artifacts/performance-acceptance/round1/PLAN.md) and
[raw-result audit](../artifacts/performance-acceptance/round1/acceptance.json)
cover 5,000/100,000-clip metadata, large native timeline gestures and exact
undo, two 120-second five-independent-source 1080p30 loops, 138 exact seek
targets, held pointer bursts, sampled process/GPU memory and decoder counts,
and visible/native minimized CPU with restore/input controls. The native final
build has normal, narrow and changed-state captures; the handover identifies
the viewed captures and final owned-process cleanup.

Metadata/query, history, idle and resource budgets pass. Playback controls and
the large-input control use task-owned affinity to logical CPUs 0–15 on this
i9-13900HX/NVIDIA Linux host. **Unpinned shared-host playback remains below
budget**, including a slower seek/loop trial; one unpinned large zoom tail is
211.809 ms. Controlled passing evidence is qualified by that affinity and does
not establish default-host acceptance. No affinity feature ships in Aster.

This performance task does not update platform packs or portable archives.
Framebuffer sampling does not prove physical presentation or zero dropped
frames; audio/A-V, macOS/Windows, other formats/resolutions, concurrent storage
or agent pressure and hours-long memory behavior retain their separate gates.

## Preceding playback source/build checkpoint

The [playback performance build record](../artifacts/production-readiness/playback-performance-round1/final-build-attempt-03/build-record.json)
is the preceding Linux checkpoint (2026-10-13 UTC): **610 workspace tests passed,
zero failures or ignored**, including 252 desktop and 147 media tests.
Source is `ebc9da4c…6375` (93 files); optimized `aster` is
`4b3128d8…70b0`, and `aster-agent` remains `00906bba…fc76a`.
Formatting, architecture validation, warning-denied workspace/all-target
Clippy, explicit FFmpeg/ffprobe 8.0.1 preflight and locked release linking pass.

The [measured findings and handover](../artifacts/production-readiness/playback-performance-round1/FINDINGS.md)
retain three matched exact-pixel 1080p30 worker pairs, normal/narrow final native
input checks, and repeated full-resolution native framebuffer observations on
an owned Weston GL/NVIDIA RTX 4060/Xwayland session. Every owner progresses at
approximately 30 pictures/s in the accelerated eight-second runs. Five actual
1920×1080 decoder commands are observed before and during playback. Exact
one-second seeks, the final cached scrub target, unchanged project/history and
owned cleanup pass. The finite observations include occasional missing source
keys and do not establish physical display timing or hours of looping.
Full-resolution software Xvfb presentation remains approximately 15 fps.
The [40-second loop audit](../artifacts/production-readiness/playback-performance-round1/loop-audit.md)
adds three valid restarts per owner, 29.70–29.83 observed changes/s, exact held
frames and steady resource ranges. Sampled PSS is 1,195–1,224 MiB and falls in
the final ten-second bucket; peak threads are 64. Loop reentry takes 100–151 ms
and the finite capture retains omission candidates. This is not gapless looping,
a physical presentation guarantee or an hours-long memory soak.

The [local launcher](../artifacts/production-readiness/playback-performance-round1/run-aster.sh)
selects this working release binary and the repository's external FFmpeg tools.
The older portable archive and prepared macOS/Windows pack retain their own
R17 pins; they were not repackaged or re-pinned by this playback-only task.
Audio/A/V sync, physical display/GPU-memory accounting, codec diversity and
native macOS/Windows acceptance remain unverified here.

### Earlier published-first read-ahead checkpoint

The [published-first read-ahead build record](../artifacts/production-readiness/publish-first-read-ahead-round1/build-record.json)
is the preceding R17 Linux checkpoint (2026-10-13 UTC): **549 workspace tests passed,
zero failures or ignored**, including 206 desktop and 132 media tests.
Source is `89aa6908…1673` (87 files); optimized `aster` is
`61dc6abd…09c5`, and `aster-agent` remains `00906bba…fc76a`.
Formatting, warning-denied all-target media Clippy and release linking pass.
Only five backend source files changed; UI source is unchanged from R16.
The [final root verification](../artifacts/production-readiness/publish-first-read-ahead-round1/root-final-verification.json)
records the current source/binaries, exact-pixel trials, native cleanup, eight
viewed captures and prepared-pack checks after the build record was created.

Playback can read ahead after publishing its requested picture, under one
four-millisecond speculative deadline and the existing cache/process budgets.
[Three paired independent-video trials](../artifacts/production-readiness/publish-first-read-ahead-round1/FINDINGS.md)
show increased source progression with improved overflow-owner tails and a
small protected-owner latency tradeoff. The
[current native five-layer check](../artifacts/native/playing-multilayer-round2/publish-first-r17-attempt-01/playing.json)
retains all-five advancement across forty reads; all reach frame 119 after
pause. The [thirteen native editing checks](../artifacts/native/publish-first-editing-r17-attempt-02/root-verification.json)
also pass. Root viewed four playing and four editing captures. These Xvfb
observations establish neither improved presentation rate nor hardware/audio
acceptance. High-resolution sustained throughput remains open.

The [prepared native platform pack](native-platform-tests.md) is re-pinned to
this source. All 106 offline guards, validation, three target plans and dry runs,
and three Linux wrong-host refusals pass. All 64 macOS/Windows cases remain
`not_run`; their actual execution is deferred as requested.

The [larger-preview feedback](../artifacts/production-readiness/high-resolution-preview-round1/FINDINGS.md)
adds twelve matched 1280×720/1920×1080 cases with exact pixels and cleanup.
It finds only modest gains and does not establish real-time five-video
full-resolution playback. That remains an explicit release blocker.

The R17 [proprietary Linux portable trial](../artifacts/packaging/linux-attempt-02/Aster-0.1.0-x86_64-unknown-linux-gnu-local-trial.tar.gz)
packages these exact binaries. Thirteen packaging guards pass, two actual builds
produce identical archives, extracted launcher paths/symlinks pass, and all
thirteen native editing checks pass through the extracted `AppRun`. Root viewed
the selection and playing captures and the icon at 256/16 px. The desktop
template validates but is not installed. FFmpeg remains external; minimum-host
acceptance, missing dependency notice texts and public release remain unfinished.

### Earlier playback-admission checkpoint

The [playback-admission build record](../artifacts/production-readiness/playback-admission-round1/build-record.json)
is the earlier R16 Linux checkpoint (2026-10-13 UTC): **538 workspace tests passed,
zero failures or ignored**, including 206 desktop and 121 media tests.
Source is `b7fc3308…f87d` (85 files); optimized `aster` is
`160efa02…1da5`, and `aster-agent` remains `00906bba…fc76a`.
Affected-package/all-target Clippy with warnings denied and the optimized
Linux build passed. These checks do not refresh the older cross-target checks.

The correction prevents advancing playback clocks from continually cancelling
unfinished owner requests. Explicit seeks, edits, cuts and bounds changes still
supersede old work. The four-decoder and other resource budgets are unchanged.
[The public real-video comparison](../artifacts/production-readiness/playback-admission-round1/FINDINGS.md)
delivered advancing frames to every owner while demand continued, with exact
pixel validation. The [native five-tile rerun](../artifacts/native/playing-multilayer-round2/root-attempt-01/observation.json)
showed all five advancing at forty screen reads; the previous build froze two.
All five reached frame 119 after pause, with unchanged project state.
The [thirteen native editing checks](../artifacts/native/playback-admission-editing/root-verification.json)
also pass on this build. These Xvfb observations exclude hardware GPU,
physical audio, frames between reads and sustained real-time performance.

At R16 the prepared native platform pack was re-pinned to that source.
Its 106 offline guards and validation passed; all 64 native cases
remain `not_run` on macOS and Windows. Earlier fixture openings and broader
Linux observations below retain their original build identities.

### Earlier round-fifteen checkpoint

On 2026-10-12, the immutable
[round-fifteen build record](../artifacts/production-readiness/final/round15-build-record.json)
passed **521 tests, zero failures and zero ignored**: 198 desktop logic tests,
12 headless CLI tests and 311 library tests (52 core, 81 agent, 112 media,
66 storage). Source is `dfbb53e9…af6c2` (83 files), and its optimized Linux `aster` binary is
`362ba180…3f364` (27,290,168 bytes); `aster-agent` is `00906bba…fc76a`.
Real FFmpeg/ffprobe 8.0.1 availability is recorded in
[the media preflight](../artifacts/production-readiness/final/round15-media-tools.log).
[Architecture](../artifacts/production-readiness/final/round15-architecture.log),
[formatting](../artifacts/production-readiness/final/round15-fmt.log),
[tests](../artifacts/production-readiness/final/round15-tests.log) and
[release build](../artifacts/production-readiness/final/round15-release.log)
passed on Linux. [Full-workspace/all-target Clippy](../artifacts/production-readiness/final/round15-clippy.log)
with warnings denied passed on Linux and by cross-target typechecking for macOS
ARM64, Windows MSVC and Windows GNU. Cross-target checks did not link or run
those platforms. The regression suite covers scheduled/stale PCM publication,
monotonic audio-clock handoff and bounded voice planning; it does not establish
audible output quality or display synchronization.
[Independent backend verification](../artifacts/production-readiness/final/round15-backend-root-verification.json)
checked the 83-file inventory, binaries and all nine sequential passing stages;
the [release graph](../artifacts/production-readiness/final/round15-release-libraries.json)
attributes optimized libraries to the exact Cargo-built desktop binary.
R15 includes consumer routing, bounded dispatch/viewer reservations, decoder
retention and failure/admission corrections. Its thirteen-check native editing
subset now passes; wake/cache/multi-owner acceptance remains unrun. Cache,
overlap and broader narrow workflows retain the
[accepted R14 build record](../artifacts/production-readiness/final/round14-native-editing-build-record.json)
scope below. [R14 binaries and three release libraries](../artifacts/production-readiness/final/round14-retained-release/record.json)
were preserved before replacement. Formatting changed only two test files after
headless GREEN; [pre-format copies/diffs](../artifacts/production-readiness/final/round15-preformat/result.json)
retain that attribution. The completed bridge probe closes the nonempty hint
call and restart-reset evidence below; other lifecycle/native behavior remains
open. Platform-pack pins/offline checks pass, and independent review confirms
local preparation readiness without native target acceptance.
The separately verified 81-case worker comparison does not establish native
native wake/cache acceptance or frontend hint effects.

Root viewed and [verified three R15 native startup captures](../artifacts/native/round15-window-captures/root-visual-verification.json)
at 1440×900, 800×600 and 760×560. Video/text composition, posters, tracks and
inspector are visible; at 800×600 both panels remain visible and Play/Speed are
separate. At 760×560 the library collapses while preview, inspector and timeline
remain visible. The deliberately unavailable audio route shows a notice over
the upper inspector. Source, binary and owned project bytes stayed unchanged;
all fifteen sampled process identities were gone. These captures establish
startup presentation and compact visuals, without input, transitions, wake
cadence, multi-owner/cache/GPU bounds or audio acceptance.

The [unchanged-behavior extraction](../artifacts/native/cache-culling-round1/phase-a-root-verification.json)
reproduced seven failures with thirteen passing controls. The
[correction](../artifacts/native/cache-culling-round1/phase-b-correction.json)
then passed [32 cache/timeline regressions](../artifacts/native/cache-culling-round1/phase-b-executed-tests.json),
and the [frozen Opus review](../artifacts/native/cache-culling-round1/phase-b-review-response.md)
found no blocking defect. Posters now have a 256-item/64 MiB logical RGBA budget;
shared visible clip/row/height predicates admit timeline demand alongside
visible library, selected and source-monitor demand. The unit policy and native
observed picture counts below are separate from measured GPU-memory bounds.

The [paused-preview delivery correction](../.scratch/core-editor/issues/08-paused-preview-delivery.md)
reproduced [eight failures with three passing controls](../artifacts/production-readiness/preview-delivery/before-fix.log),
then passed [all eleven regressions](../artifacts/production-readiness/preview-delivery/after-fix.log).
The [correction review](../artifacts/production-readiness/preview-delivery-review-response.md)
confirms bounded pending/in-flight identity and cancellable result backpressure.
Shared pixel storage proves logical mailbox accounting, not five distinct
decode allocations; some import/late-cleanup interleavings were inspected
rather than executed. Two [public-probe attempts](../artifacts/production-readiness/final/round13-public-preview-root-verification.json)
then delivered five distinct 1920-square PNG payloads each, with distinct live
pixel-storage addresses and exact full-pixel checks. Root verified source,
binaries, four CI-library fingerprints and all 18 recorded process/thread
identities gone. Missing a fifth wake during a one-second consumer pause proves
neither completed decoding nor a parked publisher; the private tests establish
the actual wait seam. These opt-level-zero probes are not performance measurements.

The earlier [R14 native editing report](../artifacts/native/cache-culling-acceptance-round1/report.json)
passed 13 smoke checks, exact whole-project held-move undo/redo, nonmutating
source preview, narrow selection, corrected overlap drawing and bounded-poster
workflows on `f74d6560` / `8b446cca`.
[Independent root verification](../artifacts/production-readiness/final/round14-native-editing-root-verification.json)
checked 75 source files, both binaries, 144 capture hashes and all 75 recorded
process identities gone across nineteen attempts. Audio was unavailable in that UI run.
Finite five-layer native/public delivery remains scoped to R13. Healthy physical
zero-PCM and private silent-stall recovery proof remain scoped to round twelve's
exact components. Native fatal-error callbacks remain unexercised.
Measured native performance remains the earlier `b29ccb69` / `14435a4a`,
333-test checkpoint; it was not repeated on round twelve, thirteen or fourteen.
The separate optimized R14 distinct-video worker diagnostic below now records
actual multi-layer round trips; it is not native presentation evidence. New
dispatch/consumer work now has the headless R15 checkpoint above, with the
original [liveness RED](../artifacts/production-readiness/preview-request-liveness-round1/phase-a/root-red-verification.json)
and [21-focused/198-desktop correction GREEN](../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
preserved. Native R15 wake/idle and continuous presentation remain unverified;
its startup captures have the narrow scope above. The public-worker
comparison below has its separate optimized scope. Real-provider AI and
GTK-dialog acceptance were not repeated either:
their latest evidence remains source `28259254…1edf1`, binary
`2c596cb0…60b52`, preserved in
[the verified-native record](../artifacts/production-readiness/final/verified-native-build-record.json).
Its [native round-nine acceptance](../artifacts/native/ai-round9/report.json)
passed with source/binary hashes unchanged before and after. The preserved
[db30/9f checkpoint](../artifacts/production-readiness/final/db30-native-round5-build-record.json)
records the preceding native recovery/provider run. The original
[169-test record](../artifacts/verification/build-record.json)
and its logs remain historical evidence.

```sh
cargo fmt --all -- --check
python3 scripts/test-workspace.py --profile ci
cargo clippy --profile ci --workspace --all-targets --locked -- -D warnings
cargo build --release --locked --bins
```

The original optimized binaries passed 13 native X11 smoke checks, a native
exit/recovery-history check and a fresh Weston Wayland file-drop check. Two
paused-window CPU/RSS observations are recorded in [performance](performance.md).
Their exact scope and the earlier refinement rounds are recorded below.

The architecture checker/self-tests, Markdown link audit and CI YAML parsing
passed. The local launcher passed shell syntax, ShellCheck and `--help` checks;
the architecture guard also covers agent boundaries. `actionlint` was
unavailable. Linux/macOS/Windows CI jobs are configured
but no hosted CI run is recorded. Headless tests and source configuration do not
establish native platform acceptance.

## Implemented feature matrix

| Requested category | Current implementation | Evidence and limits |
| --- | --- | --- |
| Projects | Create/open/save, autosave recovery, duplicate, project/canvas settings, dirty state, protected ordinary saves | Native X11 save/duplicate/settings and actual GTK dialogs; independent-writer conflicts preserve bytes/history. db30 native legacy/private unbound recovery refused existing destinations, adopted a fresh file and saved it again |
| Media import | Video, still images, GIF, audio, native file-drop handlers and recursive folder import | Real FFmpeg/backend fixtures, native Weston URI drops and actual private GTK file/folder imports with spaces/Unicode/percent names; external file-manager and GNOME/KDE/portal-only transfer acceptance remains open. 10,000 import entries per batch, 4,096 dropped paths |
| Media library | Cached thumbnails, virtual folders, search, grid/list, name/duration/kind/size sorting and source monitor | XTest folder creation/move/drag, persisted sorting and nonmutating source-monitor input; final optimized X11 smoke passed |
| Timeline | Video/audio/text tracks without fixed model count, visible-row rendering, snapping, zoom and markers | Core timing/track tests, native marker/split/frame stepping and scoped 5,000-clip/20-track framebuffer feedback measurements; larger scale and physical-display latency remain unmeasured |
| Clip editing | Trim/split/crop/move, OS clipboard copy/paste, duplicate and delete | Core boundary/reversal tests and native X11 clipboard/duplicate/split/move/right-trim; held move/trim commits once and undoes exactly. Remaining OS/input variants need platform coverage |
| Advanced clip editing | Ripple trim/delete, linked clips, groups, lock/hide/mute | Backend invariants cover markers, relationships, locks and source preservation; X11 track lock rejects a held move. Multi-device/platform acceptance remains open |
| Playback | Real software media decoding/composition, scrubbing, rational frame steps, speed/loop controls and optional audio-device clock | Native transport/decode evidence, scheduled-PCM regressions and current physical default-output silence clock; audible quality, underrun-free output, A/V drift and display latency unverified |
| Transform | Position/scale/rotation/opacity, horizontal/vertical reflection, source crop | Core validation/reversal, real decoded composition and X11 canvas-position gesture with one exact undo; complete inspector input acceptance on each OS remains open |
| Canvas | 16:9, 9:16, 1:1, custom dimensions, rational frame rate and background | Typed settings/save tests and native project settings; clean-machine/DPI/driver behavior remains open |
| History | Undo/redo, named checkpoints, reversible checkpoint restore, crash recovery and recovery leases | XTest checkpoint restore/undo/redo, crash recovery offer and optimized exit/relaunch retaining the full undo/redo chain; store shared-root/corruption/lease tests. Ordinary `.aster` open starts fresh history; recovery sidecar retains it |
| Looks, animation and motion | Keyframes, transitions, effects, looks, speed, masks, blend, colour, keying, tracking, stabilization as preview | See [advanced editing](#advanced-editing): Linux headless tests, device readback on two adapters and native captures on virtual displays; no playback rate, export or other platform |
| Native AI | Claude/Codex CLI planning, captured/focused metadata, review/apply/discard/cancel and exact prepared-state commits | Exact reviewed split IDs verified through the public runtime; `2c596cb0` both real-provider split/undo/redo and owned no-op/stale fixtures passed. Prior db30 cancel/reopen evidence stays scoped. Metadata only; no audiovisual semantic analysis |

The [user guide](usage.md) explains controls and current limits. Local agent
commands and the authenticated live/headless protocol use the same reducer;
mutation requests require project, live-context and revision identities.
[Native AI](native-ai.md) uses separately installed and signed-in CLIs;
credentials are not bundled or stored in Aster project/preferences files.

## Advanced editing

Keyframes, transitions, effects, looks, speed, masks, blend modes, colour
grading, background removal, tracking, stabilization and picture-in-picture
from [the advanced-editing design](../.scratch/advanced-editing/spec.md) are
implemented as preview features. This section states what evidence exists for
each. It records no platform acceptance, no playback frame rate and no export:
there is no export path, so everything is preview at preview resolution.

### Where the evidence comes from

| Kind | What it is | Host and build |
| --- | --- | --- |
| Headless tests | Core, store, media, agent and desktop unit tests at the owning seam. Desktop tests drive the inspector, timeline and viewer through a test bench that mirrors `App`; `App` itself cannot be built in a test | One Linux host (i9-13900HX, Ubuntu 26.04.1), unoptimized `ci` profile. The last whole-workspace run before the final wave reported 545 `aster-editor` tests passing; the final wave's tasks report their packages and workspace Clippy passing. This documentation task ran no build |
| Device readback | `src/compose/*_tests.rs` render through a real wgpu device and read pixels back. A test without an adapter prints `SKIP:` and passes, so a green run on a host without one proves nothing | Ran on NVIDIA GeForce RTX 4060 Laptop (Vulkan) and, with `ASTER_TEST_FALLBACK_ADAPTER=1`, on llvmpipe (Vulkan, CPU). Not run on Metal, Direct3D, Intel or AMD. The GL backend was not reached by the test harness |
| [Wave 2 captures](../artifacts/advanced-editing/wave2/README.md) | Paused compositor output of a project built over the agent protocol, parked by key presses | Xvfb + lavapipe at 1440x900 and 760x560; headless Weston + Xwayland on the RTX 4060 (Vulkan) at both sizes; wgpu GL on llvmpipe at 1440x900. Canvas differs by at most 4/255 between lavapipe and NVIDIA |
| [Wave 3 captures](../artifacts/advanced-editing/wave3/README.md) | The editing interface driven by XTest pointer and keyboard, each gesture read back over the agent protocol and undone | Xvfb + lavapipe only, no window manager, 1440x900; at 760x560 mostly states made at the larger size |
| Wave 4 captures | Defect fixes, tracking and stabilization driven the same way | Xvfb + lavapipe, both sizes. Kept in the session's scratch directory, not under `artifacts/`; several were taken one build before the final binary, as each task reported |

### Feature matrix

"Readback" names the adapters above. "Native" is a capture of the running
build that was looked at. "Measured" is a number with its conditions in
[performance](performance.md#advanced-editing-budgets); everything else in
that column is unmeasured.

| Feature | Headless tests | Readback | Native capture | Measured | Known limits and unverified |
| --- | --- | --- | --- | --- | --- |
| Model, bounds, schema v2 | Bounds, NaN, reversal, atomic failure, v1 decode ([advanced_model](../crates/editor-core/tests/advanced_model.rs), [advanced_commands](../crates/editor-core/tests/advanced_commands.rs)); v1 files load and the next save writes v2, v3 is refused ([schema](../crates/editor-store/tests/schema.rs)) | n/a | n/a | No | No build that knows only v1 was run against a v2 file; the refusal is shown for v3 on this build through the same path |
| `resolve` and the render plan | Order, keyframe values, transition pairing, follow, seed and time determinism ([render](../crates/editor-core/tests/render.rs)) | n/a | n/a | No | Cost of the per-frame scan on a large project |
| Keyframe toggles and keyed rows | Inspector bench: one entry a gesture, Escape, locked track, paired keys | n/a | 1440 and 760 (wave 3, wave 4) | No | Screen reader; inspector scroll-into-view of a keyboard-focused row not captured |
| Keyframe lane and graph editor | Move, delete, paste, easing, handles, snapping, selection scope ([keyframes_tests](../src/ui/keyframes_tests.rs)) | n/a | 1440: move, delete, value drag, easing, handle. 760: states only | No | Marquee, shift-add, cut, U, value nudges, Shift axis lock and paste refusals are headless only |
| Effects (glow, shake, wave, motion blur, grain, sharpen, shadow, blur, vignette, glitch) | Stack, mix, reorder, sixteen-effect bound | Every kind, mix and stack order | Glow and blur on three adapters (wave 2); glow from the inspector (wave 3) | No | Shake, wave, motion blur, grain, sharpen, shadow, vignette and glitch were never looked at in the running app; wide blurs read back at 64x64 only |
| Looks | Replace in place, strength as one entry, derived look | Presets numerically on a grey ramp | Teal & orange on three adapters; inspector look and strength | No | Looks tab of the library not exercised natively; not judged on real footage |
| Colour grade, hue bands, curves | Section rules, removal at identity, curve keys | Grade, band and curve passes; curve held to the shader | 1440: each grade row, a hue band, a curve point (lavapipe) | No | 760 gestures; hue-band menu and canvas colour picker not captured |
| LUT (`.cube`) | Parser bounds, import, round trip, stale table | Lookup pass on NVIDIA and llvmpipe | Red/blue swap on three adapters; LUT row at 1440 | No | Import button opens a file dialog and was not driven; GL downlevel not reached; table upload time |
| Chroma key and eyedropper | Section, pick, Escape | Key pass | Key over a backdrop on three adapters; eyedropper pick at 1440 | No | A one-pixel fringe stays at defaults on 4:2:0 hard edges; spill darkens a saturated shape when the key colour is white |
| Clean plate (difference matte) | Plate request, staleness, no repaint loop (returns early without an adapter) | Matte pass with a supplied plate | 1440 on lavapipe with a matching fixture | No | 760 capture missed; plate must be a still image; not steadied with a stabilized layer |
| Background helper | One batch, nearest free track or a new one | n/a | 1440: adds "Video 3" under the clip as one revision | No | Video backgrounds, locked tracks natively |
| Masks | Add, shape, op, invert, eight-mask bound, rotation drag speed | Coverage, feather, rotation direction | Feathered ellipse on three adapters; ellipse outline at 1440 | No | Rectangle, subtract, intersect and invert not captured; no on-canvas handles |
| Blend modes | Mode list, title refusal | Nine modes against the W3C formulas in linear light | Screen on three adapters; list of nine at 1440 (wave 4) | No | Blend of a transition pair has no readback; a blend above a title blends only with media between them |
| Picture-in-picture | Layout presets, refusal while keyed | Through transform, mask and blend | Half-size masked inset on three adapters; "Inset, top right" at 1440 | No | Other presets natively |
| Transitions | Add, replace, default, handle drag, marking hit area, one entry ([transitions_tests](../src/ui/transitions_tests.rs)); six video consumers pixel-exact at the pool | Every kind | Wipe left and dissolve on three adapters; badge, handle drag, kind change and remove at 1440; Ctrl+T at 760 | Decoder starts with six consumers on 64x32 fixtures | Twelve kinds never looked at in the app; `out` edge; titles always fade; incoming clip not pre-rolled; no preview while a handle is held; "Save as default" across a restart |
| Speed, reverse, freeze, ramp | Map bounds, split and trim rebasing, key following, speed model, audio plan | n/a | Forward, freeze and two reverse moments on three adapters; 0.5x, reverse, freeze, ramp preset and curve at 1440; point drag at both sizes | Reverse steps at `DecoderPool`: 3 starts and 6.4 to 6.8 ms a frame against 30 and 55.5 to 60.8. Reversed 1920x1080 30 fps H.264 at 1x, release build, RTX 4060 under headless Weston, 60 Hz samples over three 6 s windows: 180 distinct pictures each and no hold over 50 ms, against 144 to 147 with holds of 133 to 167 ms every 0.533 s before the look-behind ([figures](../artifacts/advanced-editing/final-fixes/reverse-pacing.txt)) | Reverse and ramp key-following are core tests only; the reverse rate is one fixture (keyframe every 15 frames), one adapter, a plain clip; distant keyframes, effects on a reversed clip and 1080p-size preview frames are unmeasured; sound silent on freeze, reverse and ramp; trim stop-short not dragged natively |
| Motion tracking | Lane queue, cancel, stale drop, lost, synthetic accuracy ([analysis tests](../crates/editor-media/src/analysis/tests.rs)); `tracker_command` identity checks | n/a | Both sizes: track, progress, cancel, lost, rename, remove, undo | 125 keys within 1.8 source px on a generated fixture; about 220 to 240 % of one core | Translation only, forwards only (Track is disabled where a clip plays backwards), video only; a loss is said once in a notice and not kept; not tried on real footage; stale drop not driven through a running app that replaces its project. Wave 4 fixes, session captures on lavapipe: a box drawn on a steadied clip cropped 1.2 times read as the source place the fixture has it at (82.3 %, 27.5 %), the first key on the playhead's frame, and a path that came after an undo waited for Add tracker at 1440 and 760 |
| Follow | Core follow, offset, inert off screen; agent phrases | n/a | Follow, centre, offset drag as one entry, stop, agent follow | No | Position only in the interface. Placement through the tracked clip's box, crop, flips, scale, turn and keyframes is a core test ([render](../crates/editor-core/tests/render.rs)); natively only a steadied, cropped clip that fills the canvas was driven (a title stayed on a marker over four frames at 1440), not a scaled or rotated one. `follow NAME` and "stay where it is" use the reading of the last frame drawn |
| Stabilization | Estimator, smoothing, crop, cache eviction and staleness ([steady_tests](../crates/editor-media/src/analysis/steady_tests.rs)); window table | Steadied sampling in the place pass (NVIDIA) | 1440 and 760 on lavapipe: card, analyse, progress, steadied picture | 7.52 to 0.10 px RMS on a 320x240 fixture; 15 to 16 ms a frame | No camera footage; no rolling shutter or parallax; turn and size estimation tested synthetically only; no app-level test from analysis to canvas; cache-write failure notice not exercised |
| Compositor budget and fallback | Ledger: budget, steps, refusal, skip, release ([compose tests](../src/compose/tests.rs)); device-refusal path with a validation error | Resize update path | Paused picture identical after 8 s idle and a pointer wake (wave 2) | Paused CPU, single samples | "Effects not shown" was never seen in a running app: the window always has a device; internal and out-of-memory device errors not reproduced; GPU memory |
| Keyboard routes and focus | Reach, focus order, diamond menu | n/a | C and Alt arrows, diamond menu by keyboard, one side panel at 760 (wave 4) | No | Alt arrows left to a focused curve point by a unit test of the key rule only, not driven natively; track-head focus ring and edge-grip click by unit test only; clip tiles are not focusable; Option-key labels on macOS |
| Command-panel phrases | Every listed example parses ([agent.rs](../src/agent.rs)) | n/a | Ten phrases at 1440 (wave 3); follow and unfollow (wave 4) | No | Tracking and analysis have no phrase |
| Native AI planning | Provider schema under 24 KiB, planner text names every parameter key, look and motion commands plan and commit | n/a | No | No | No real Claude or Codex request with the new instructions; Windows command-line headroom |

### Unverified for every row

- **macOS and Windows.** Nothing in this section was linked or run there:
  Metal and Direct3D backends, shader translation, the stabilization cache
  location, shortcut labels and HiDPI are all untested.
- **A hardware GPU in an ordinary windowed session.** NVIDIA evidence is
  readback tests and paused captures under headless Weston; the interface was
  driven on software Vulkan only. Intel and AMD adapters were not used.
- **A window manager, a physical display and HiDPI.** Every capture is a
  private virtual display at scale 1 with no window manager.
- **Playback frame rates.** No effect stack, transition, ramped, tracked or
  stabilized clip has a measured presentation rate. A plain reversed clip has
  one: the release-build figure in the Speed row above, for one fixture on one
  adapter under headless Weston. A played stabilized clip was looked at; that
  is not a rate.
- **Optimized builds.** Every figure and capture is from the unoptimized `ci`
  profile, except the reversed-playback rate and the release rows of the
  [final acceptance](../artifacts/advanced-editing/final/README.md).
- **Idle behaviour in the running app.** Paused CPU was sampled; wakeups,
  repaint counts and the three-second compositor release were not observed
  natively. Headless idle tests cover each new surface.
- **Real media.** Fixtures are generated by FFmpeg `lavfi` or synthesized.
  Camera footage, video with alpha, long files and variable frame rate were
  not tried.
- **The light theme, text and sound clips, and locked tracks** in the new
  panels were not captured natively; locked-track refusal is headless only.

Wave 3 found nine defects (D1 to D9) and wave 4 reports them fixed, most
confirmed natively; the remaining gaps are the "unit test only" entries above.
During wave 3 one editor session inherited the person's D-Bus address and may
have raised a Save dialog on their desktop; later sessions isolate D-Bus and
`XDG_RUNTIME_DIR`. Automatic subject segmentation is not implemented; see
[its research ticket](../.scratch/advanced-editing/issues/16-automatic-subject-segmentation.md).

## Native Linux evidence

The unchanged smoke harness passed [all thirteen R15 native editing/input controls](../artifacts/native/round15-editing-smoke/run/report.json),
with [independent root verification and five viewed captures](../artifacts/native/round15-editing-smoke/root-verification.json).
On `dfbb53e9` / `362ba180`, real pointer/keyboard input covers duplicate/undo/redo,
clipboard, clip/trim/canvas gestures committing once and reversing exactly,
track locking, rational frame/marker/split, command-panel rename, durable save,
stale-plan refusal and transport/settings. Source media, original fixture,
harness, source and binary stayed unchanged; eleven sampled process identities
were gone. The five 1440×900 captures show selection/inspector, command panel,
saved settings and playing video with an advanced playhead. Private Xvfb lacked
DRI3 and output audio was deliberately unavailable. This is a tested editing
subset, without complete gesture/platform coverage, multi-owner/cache/wake
cadence, hardware GPU, audio, display-latency or fps proof.

The earlier [R14 acceptance report](../artifacts/native/cache-culling-acceptance-round1/report.json)
passed the unchanged thirteen-check smoke and editing run on their first attempts.
A held move made one history entry with exact whole-project undo/redo/undo;
source preview left the document/history unchanged, including narrow 760×560
clip selection. Root verified 144 captures, nineteen attempts and all 75 owned
process identities gone, and viewed four actual normal/narrow/cache captures.
This software-rendered X11 run exercised no current provider, GTK chooser,
physical audio, hardware-GPU, Wayland or performance acceptance.

The overlapping long clip filled the 49.97–59.83-second viewport (fill 1.0)
at normal and narrow sizes; the R13 baseline fill was 0.0016. It stayed
selectable in both versions, and scrolling/selection preserved the document.
The fitted normal timeline showed 256 of 480 own posters; all 480 were readable
across pages and revisit. At 760×560, 320 posters on readable lanes were checked;
the clipped third lane and narrow fitted centre pixels do not provide an exact count.
One Ctrl+wheel notch admitted 43 newly visible pictures normally and 21 narrowly
without further input. That proves progress after room freed, not an exact
repaint count. After the startup notice dismissed itself, unchanged posters with
no input used 1–3 CPU ticks over five seconds; this diagnostic is not a repeated
native performance measurement. Earlier 25–28-tick samples included notice dismissal.

After browsing 464/424 other posters with the inspector hidden, Still 007's
inspector went from empty to its poster without more input. Immediate reopen
and continuously shown-inspector controls retained the picture. Source-monitor
poster production is indirect evidence: the solid still cannot distinguish
the viewer's poster and layer routes. A validated video-with-sound clip on a
44-point audio row showed its poster at both widths and one undo restored the
full project; no audible output was tested. Five harness failures (c1, c2,
c4-narrow, c5-narrow, e1) remain recorded. The
[800×600 library capture](../artifacts/native/cache-culling-acceptance-round1/shots/e2-library-narrow-800x600.png)
also shows the existing Play/Speed overlap outside this cache/culling correction;
the scoped acceptance does not imply a globally defect-free interface.

The earlier R13
[post-preview-delivery editing report](../artifacts/native/post-preview-delivery-round1/report.json)
used source `5cc20680` / release `6496d42f`: 13/13 smoke checks passed on the
first attempt. A held move committed once; undo, redo and second undo restored
the exact full project. Source scrub/play/step preserved project, revision,
dirty state and redo. Captures include an actual
[narrow clip selection](../artifacts/native/post-preview-delivery-round1/shots/a1-edit-narrow-selected.png)
and [five-layer paused composition](../artifacts/native/post-preview-delivery-round1/shots/b3-layers3-open-stable.png),
both viewed by root. Root checked all 35 capture hashes, unchanged source/build
identities and all 31 recorded process entries stopped. Editing was one bounded
Xvfb pass with no output device, compositor/DRI3, physical display, real provider
request or GTK dialog. It establishes no performance or native failure callback.

Three fresh launches showed all five paused layers at the first observation,
about 2.5 seconds after session readiness, without editing input. Seven
one-second reads stayed identical; paused seek showed set B and Home restored
set A. Revision stayed zero, clean, with no undo/redo and unchanged project bytes.
The 1468-pixel canvas/source size implies 1920-square requests from code; actual
request dimensions and publisher parking were not observed. Missing NumPy
prevented the first harness launch; a cropped region/white-interface-pixel check
caused the second harness failure. Both attempts remain recorded, with no
product failure attributed. This is finite composition acceptance, not latency,
throughput, atomic presentation or general scheduling fairness.

The [round-twelve post-liveness report](../artifacts/native/post-liveness-round1/report.json)
and [its independent verification](../artifacts/production-readiness/final/round12-native-editing-root-verification.json)
retain their separate `2b43797d` / `ec99d455` scope.
The [round-ten post-audio report](../artifacts/native/post-audio-round1/report.json)
and [its independent verification](../artifacts/production-readiness/final/round10-native-editing-root-verification.json)
retain their separate `08647cfc` / `4a75ae1e` identity and scope.

The earlier accepted [editing/performance round one](../artifacts/native/performance-round1/report.json)
identifies source `b29ccb69` / release `14435a4a`. The 13-check smoke passed.
Held movement committed once and exact undo/redo restored the full project;
video source preview left project, revision, dirty state and redo unchanged.
Normal and narrow owned-window captures are preserved, including
[exact undo](../artifacts/native/performance-round1/shots/edit-normal-undone.png)
and [narrow source preview](../artifacts/native/performance-round1/shots/edit-narrow-source.png).

Three paused trials per workload/rig and three X11 minimize trials were sampled;
the 5,000-clip timeline, seeks and 30-second playback observations are summarized
in [performance](performance.md#native-paused-window-observations). Xvfb and
headless Weston/Xwayland were task-owned; audio was disabled and no provider
requests ran. Forced-loader comparison, per-PID GPU memory and driver threads
strongly support NVIDIA Vulkan rendering, but Aster emitted no adapter log.
There was no physical output/vblank observation or A/V measurement.

The report retains a missing creation-time digest for generated media, a narrow
"selected" capture that actually shows a ruler seek, absent post-restore click
acceptance, observer overhead and approximately 115 paused NVIDIA-path voluntary
context switches/s requiring attribution. Root verified source/binary identity
and all 48 recorded process entries stopped. These limits remain part of the
accepted scope, rather than evidence of full production readiness.

The historical [AI/dialog round-nine report](../artifacts/native/ai-round9/report.json)
passed all four required gates and an additional import-dialog check, using
the exact `2c596cb0` binary/source `28259254`. A labeled owned provider fixture
went through the actual Ask/runtime/Apply flow with rename-away-and-back
commands. The "No changes" card retained the complete project, revision,
clean state and redo branch, including after a repeat click; Redo still restored
the earlier marker edit. This proves native state and feedback; pointer identity
remains a core/unit-test assertion. Feedback stays in the proposal card rather
than adding an Apply toast.

Both real default CLIs produced a correct selected text split on their first
request, with no retries or bad plans in this round. Observed models were
`claude-haiku-5-5` and `gpt-6.1-sol`. Each applied once, left other content
unchanged and restored exact full project/new clip IDs through undo/redo. A
fixture proposal became stale after a human edit and refused Apply. Normal
1440×900 and narrow 760×560 pending/proposal/no-op/applied/stale captures had
no clipping or overflow.

Round-nine GTK legacy-unbound recovery again refused original/other existing
destinations after GTK Replace, retained bytes/history/dirty/no file, adopted
a fresh destination, and saved a subsequent edit to that same file. Ordinary
replacement after adoption and duplicate-existing refusal also passed. Native
file/folder import cancellation left state unchanged; choosing a space-named
file and a folder with Unicode/percent-named sources imported correct metadata
without timeline or source-byte changes. The 13-check native smoke passed,
and all recorded helper/editor/portal processes stopped; root independently
checked their recorded PIDs. Real cancel/reopen and private-unsaved restore
were not repeated on this binary and retain the prior db30 scope below.

The preceding [native round-eight report](../artifacts/native/ai-round8/report.json)
identifies the preserved db30 source / `9f721c8b` release. Through actual private
GTK Save dialogs, both a simulated legacy unbound archive and a second window's
private recovery restored dirty work with full history. Choosing the original
or another existing project and answering GTK Replace was refused without
changing owned bytes or history. A fresh destination saved successfully, and a
later edit/Ctrl+S wrote that same file. Ordinary deliberate replacement worked
after adoption; duplication onto an existing file remained refused. The legacy
fixture was a current archive with its binding removed, not an older build's
archive.

On that same build, Claude produced two proposals from three requests, with one
cancelled request producing no late edit during a 30-second observation. A
marker made one proposal stale; a correct text split applied once and undid/
redid exactly. Codex produced three proposals: reopening invalidated one,
one described a split but proposed an unchanged transform and was discarded,
and one retry applied the exact split with one undo/redo. Observed CLI defaults
were `claude-haiku-5-5` and `gpt-6.1-sol`; the Claude default differed from the
backend example's `claude-opus-5-5`. Native smoke passed 13/13 again and source
fixture hashes stayed unchanged. All recorded task-owned processes stopped.

The round-nine release differed from db30 only in three desktop source files
correcting the no-op result/status. Its 309 tests prove unchanged proposals retain clean state,
revision, root and redo, and display "No changes" without an undo promise.
The later native acceptance above verifies this feedback and retained redo.
The db30 report keeps its own binary identity and does not substitute for it.

The earlier records below establish their named historical builds and scopes.

The final optimized [X11 smoke report](../artifacts/native/final-smoke/report.json)
passed **13 of 13 checks** through real OS input and the live agent protocol.
It covers project open/inspection; pointer duplicate/undo/redo; OS clipboard;
clip movement, right-edge trim and canvas-position drags with one commit and
exact undo; track-lock refusal of a held move; rational frame stepping, marker
and split; the local command panel; durable save; stale agent-plan refusal;
playback/settings; and unchanged source hashes. Its scope excludes audible
output and display latency. Captures include
[selected clips](../artifacts/native/final-smoke/selected.png),
[agent commands](../artifacts/native/final-smoke/agent.png) and
[settings](../artifacts/native/final-smoke/settings.png).

The optimized [shutdown/history report](../artifacts/native/final-shutdown/report.json)
passed a native screenshot-approved exit before the periodic autosave interval.
The latest dirty rename survived in recovery, the ordinary project bytes stayed
unchanged, and reopening recovery through the actual agent CLI restored complete
undo/redo availability. The [last edit](../artifacts/native/final-shutdown/last-edit.png)
and [test helper](../artifacts/native/check-shutdown.py) identify this flow.
Forced interruption and stalled/full filesystems were outside that check.

The final optimized [Wayland report](../artifacts/native/final-wayland/report.json)
passed actual GTK3 OS drag under a fresh task-owned Weston 14.0.2/pixman
compositor on Xvfb, with Aster using lavapipe. Three local PNG/MP4 fixtures
containing space, `é` and literal `%` names imported; remote URIs were refused
and a repeated URI deduplicated. Library import left the timeline unchanged.
Its [before](../artifacts/native/final-wayland/before.png) and
[after](../artifacts/native/final-wayland/after.png) captures preserve the result.
This covers ordinary URI lists on the first seat/root surface; GNOME/KDE,
sandboxed portal transfers and actual file-manager/chooser acceptance remain open.

The final release build also launched on the actual GNOME Wayland desktop with
owned fixtures/settings, composed decoded video and text, and exited normally
after its [native capture](../artifacts/native/final-desktop/native.png).
The [log](../artifacts/native/final-desktop/native.log) records that run. It is
GNOME launch/render/teardown evidence; OS input, file drop and chooser were not
exercised in that run.

Earlier [X11 smoke](../artifacts/native/smoke3/report.json) passed nine checks:
native project open/inspection; pointer selection and duplicate/undo/redo;
OS clipboard; rational frame stepping, marker and split; local command panel;
durable save; stale live-agent plan refusal; playback/settings; and unchanged
source-media hashes. Its scope explicitly excludes audible output and display
latency. Captures include [selected clips](../artifacts/native/smoke3/selected.png)
and [settings](../artifacts/native/smoke3/settings.png).

The expanded [gesture smoke report](../artifacts/native/gesture-smoke/report.json)
passed 13 checks against the round-three binary. Four additional real OS drag
checks cover clip movement, right-edge trim, canvas position and lock refusal;
each successful gesture commits exactly one edit and undo restores exact source
state. These expand rather than replace the historical nine-check reports;
the final optimized rerun above also passed all 13.

The [second UI round](../artifacts/native/ui-round2/) contains 21 native XTest
captures. It exercised cross-window clipboard and ignoring unrelated plaintext,
checkpoint create/restore/undo/redo in the same project/path/context, duplication
with fresh identity and unchanged original, persisted sorting, virtual folder
create/move/drag, and recovery ownership across windows. A live lease suppressed
the second recovery offer; after SIGKILL released the owner, the offer returned.
It includes [checkpoint restoration](../artifacts/native/ui-round2/05-restored.png),
[cross-window paste](../artifacts/native/ui-round2/18-pasted-in-b.png),
[crash recovery offer](../artifacts/native/ui-round2/20-offer-after-crash.png) and
[narrow checkpoints](../artifacts/native/ui-round2/21-narrow-checkpoints.png).
These precede the final source-monitor round.

The [third UI round](../artifacts/native/ui-round3/) added real-input source
preview for video/GIF/WAV/PNG, source-only scrub/frame/transport/loop behavior,
returning to the unchanged timeline, still-source transport bounds and source
metadata invalidation through checkpoint restore/undo/redo. Representative
captures are [video playback](../artifacts/native/ui-round3/05-source-video-playing-b.png),
[GIF playback](../artifacts/native/ui-round3/14-source-gif-playing-b.png),
[audio source controls](../artifacts/native/ui-round3/18-source-audio-playing.png)
and [same-asset changed source](../artifacts/native/ui-round3/28-after-restore-same-id-new-path.png).
The [round-three smoke report](../artifacts/native/ui-round3/smoke/report.json)
also passed nine checks. Actual sound-device output remains outside those checks,
and these captures precede the final cadence/drop/shutdown follow-ups.

The [fourth round](../artifacts/native/ui-round4/) added actual native Wayland
file drop under Weston 14, running nested on the owned Xvfb display with pixman
and Aster/lavapipe. A GTK3 source offered six URIs; three distinct local files
(including spaces, `é` and `%`) imported, duplicate paths were deduplicated,
and remote/HTTPS entries were rejected with a visible notice. The protocol
trace records accept/action/receive/finish/destroy. A stalled sender timed out
without importing; closing during its transfer released the native reader and
offers in 0.09 seconds without a panic. Shift+Arrow used each event's modifiers
and advanced exactly once in native X11 input. Rebuilt native GIF preview at
2.000 seconds showed its own frame-20 stamp and 10 fps header.

This Wayland acceptance covers the first seat/root surface and ordinary local
URI lists under Weston. GNOME/KDE and sandboxed portal-only transfers remain
open. Dirty-project exit persistence was covered by deterministic unit tests
at that round; the final optimized native exit/history result is recorded above.

Earlier preserved native Wayland captures
[shot2](../artifacts/native/wayland-initial/shot2.png) and
[shot4](../artifacts/native/wayland-initial/shot4.png) showed real still/GIF
decoding and transformed composition with isolated settings. Native Wayland OS
input was not exercised in those earlier runs; later Weston drop checks above
provide actual OS-input evidence within their stated scope.

Earlier Xvfb rounds lacked a usable file-picker portal. The later
[native AI round-seven report](../artifacts/native/ai-round7/report.json) records
actual rfd→private DBus→GTK FileChooser v4 Open/Save As/Duplicate interaction:
14 steps, including cancellation, durable fresh saves, deliberate ordinary
replacement, stale-destination refusal and independent duplicate identity.
It also records crash→independent writer→restored history, conflict visibility
before writing, legacy/private unsaved restoration and exit flush.

That round used binary `bca32395…6f043b`, predating the final prepared-edit and
provider-isolation changes. Both real providers ultimately applied and undid/
redid native edits exactly. Claude succeeded twice; Codex succeeded twice out
of five requests, with three safe failures diagnosed as schema/`oneOf` rejection.
The native schema now uses disjoint `anyOf`/`enum` variants, and tests preserve
exact split/link/group IDs from review. The later db30 native split/recovery
run above passed, followed by round-nine no-op/provider and import-chooser checks.
External file-manager, portal-only drop
and physical audio-device acceptance are outside the recorded GTK dialog scope.

## Performance evidence

The [accepted round-ten virtual reproduction](../artifacts/production-readiness/final/round10-virtual-stall-root-verification-v2.json)
showed raw callbacks frozen at 51,200 frames with zero errors, while public audio
and Playback stayed at 1,045,333 µs for over 14 seconds after private server loss.
The absolute-route missing-server control refused startup without fallback;
earlier failed setups/controls are retained. This prompted callback liveness.

Round twelve's [virtual-stall report](../artifacts/production-readiness/audio-device-probe/virtual-failure-round6/report.json)
and [root verification](../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
show raw CPAL still emitting no error, but public `AudioService` losing its clock,
Playback continuing monotonically and one coalesced notice. The decoder
disappeared naturally around 1.81 seconds from session start, approximately
0.60 seconds after observed server loss, before explicit stop around 17.1 seconds.
Worker detection also succeeded with five position reads omitted. Root checked
73 source files, exact components and all 15 tracked process identities gone,
with zero forced cleanup, only zero PCM and no physical/foreign sound access.
This is one owned virtual experiment, not native fatal-callback, hardware
unplug, acoustic, A/V or performance acceptance.

The nine liveness regressions include a real publication-seam race caught after
the preliminary [353-test round-eleven build](../artifacts/production-readiness/final/round11-build-record.json).
That build has no native acceptance. The
[initial review](../artifacts/production-readiness/audio-device-probe/callback-liveness-source-review.md),
[failing race test](../artifacts/production-readiness/audio-device-probe/liveness-period-race-before-fix.log)
and [corrected nine-test result](../artifacts/production-readiness/audio-device-probe/liveness-period-race-after-fix.log)
remain preserved with the [review response](../artifacts/production-readiness/audio-device-probe/callback-liveness-review-response.md).
The active-clock policy keeps four largest observed callback periods with a
500 ms floor; paused clocks are excluded, and an already-observed expiry may
win an overlapping reset. Its callback cost is unmeasured.

The [round-twelve healthy physical-output clock probe](../artifacts/production-readiness/audio-device-probe/round12/report.json)
used only verified zero PCM on the inherited Linux ALSA/PipeWire default route,
with the owned stream connected to an AD107 HDMI hardware sink. Four phases
sampled callback-clock slopes near 1×, 1× after seek, 0.5× and 2× for about two
seconds each; sync/pause/stop invalidated position. The
[source/component record](../artifacts/production-readiness/audio-device-probe/round12/build.json)
matches `2b43797d` and the exact CI-profile libraries throughout the run;
[independent root verification](../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
checked zero PCM, four 21-sample phases and exact owned PID cleanup. The
diagnostic and observed decoders stopped without changing the default route.
This healthy direct-rustc probe establishes neither native output-error callback
behavior, acoustic quality, underrun-free PCM, hardware presentation timestamps
nor A/V synchronization. The
[before-zero-gain report](../artifacts/production-readiness/audio-device-probe/before-zero-gain/report.json)
remains separate historical evidence at source `28259254`.

An actual FFmpeg [red regression](../artifacts/production-readiness/audio-device-probe/zero-gain-before-fix.log)
showed zero-gain offline clips opening nonexistent sources and consuming audible
voice capacity. Skipping zero-gain clips before decoder allocation passed the
[15-test media suite](../artifacts/production-readiness/audio-device-probe/zero-gain-after-fix-media-tests.log)
at round eight. Round nine's [34 media tests](../artifacts/production-readiness/final/round9-tests.log)
also cover stale/late PCM publication, callback scheduling and bounded voice
admission. Round ten's [46 media tests](../artifacts/production-readiness/final/round10-tests.log)
include 12 device-free fatal/recoverable policy, clock, wake and coalesced-notice
regressions. Round twelve adds the nine liveness regressions and virtual silent-stall
proof above; native fatal callbacks and sustained PCM/A-V acceptance remain open.

[Performance](performance.md) retains the historical
[5,000-clip metadata benchmark](../artifacts/performance/timeline.json) and the
[software preview run](../artifacts/performance/preview-final.json).
The latter decoded 120 synthetic 1080p H.264 frames to 1280×720 at 517.815 fps,
with 1.753 ms p50 / 3.241 ms p95 / 3.803 ms p99 worker roundtrip latency. It
excludes GPU/audio/native input/display. The metadata benchmark sampled the full
0–499.9-second timeline: 421.812 µs average move/undo and 2.451 µs active-query
time. The [round-nine five-trial metadata sample](../artifacts/production-readiness/final/round9-timeline-repeated.json)
recorded medians of per-trial means of 363.872 µs move/undo and 2.258 µs active
query after owned Cargo/UI/device work had stopped. The shared host was not
CPU-isolated; these are not individual-operation tail latencies. These results do not establish native
editing latency or parity with another editor. Two
[optimized native idle observations](../artifacts/performance/native/idle.json)
measured 0.980% of one CPU core / 226.10 MiB process-tree RSS for an empty paused
window, and 0.901% / 503.54 MiB for a loaded paused 1080p fixture. Each sampled
about 12 seconds after six seconds of warmup at 1440×900. They exclude GPU memory
and presentation latency and do not establish repeated-run distributions.

The completed [optimized R14 distinct-video diagnostic](../artifacts/production-readiness/multilayer-preview-round1/FINDINGS.md)
passed all 27 cases, with [independent root verification](../artifacts/production-readiness/multilayer-preview-round1/root-verification-attempt-01.json)
and a separate [raw-data audit](../artifacts/production-readiness/multilayer-preview-round1/audit-attempt-01.md).
Median-of-three-trial complete-batch p50s for sequential 1/4/5-video demand were
0.257/0.764/163.942 ms. All 96 measured five-video sequential batches exceeded
33.333 ms: an approximately 214.6× four-to-five limitation in this worker workload.
All 2,370 completions passed recorded full-pixel/identity checks across 711
batches (693 measured, 18 warmups). Root checked exact source/binary/release
libraries, 206 fixture files and distributions, with no live recorded PID/TID
identities. Successful returned buffers were transient, not independently
replayed. Polling, inter-batch prefetch, unflushed OS caches and uncontrolled
host/power conditions limit interpretation; GPU, native fps, audio and target
runtime remain outside this diagnostic. [Performance](performance.md#optimized-distinct-video-worker-diagnostic)
records cold/random/cached distributions and further limits.

## Platform matrix

The [R15 optimized three-mode run](../artifacts/production-readiness/multilayer-preview-round3/root-measurement-verification.json)
passed all 81 cases. Five-video sequential batch p50 medians were
185.490/179.190/67.395 ms for legacy/explicit/protected modes (legacy/protected
ratio 2.7523×); every measured five-video sequential batch in each mode exceeded
33.333 ms. All 7,110 requests/2,133 batches, transient full-RGBA checks, 324
distribution fields and matching fixtures were root-verified. Recorded sampled
PID/TID identities were gone; they are not exact decoder-start telemetry.
Shared-host/prefetch/polling limits and exclusions are in
[performance](performance.md#optimized-r15-legacyexplicitprotected-comparison).
The [completed independent audit](../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
agrees with the arithmetic and exact build attribution and checks all 3,040
observed process/thread identities gone, including setup identities. This is
not exact decoder-start telemetry, native fps, audio/GPU or G1 frontend
setter/reset proof.

| Platform | Recorded proof | Remaining native acceptance |
| --- | --- | --- |
| Linux X11 | R15 `dfbb53e9`/`362ba180`: 521 headless tests, source checks, optimized linking, 81-case worker comparison, canonical hint/restart bridge, three startup captures and thirteen native editing/input checks with five captures; cache/overlap workflows remain R14; finite five-layer delivery R13, audio R12, native performance R9, real-provider AI/GTK `28259254`/`2c596cb0` | Complete R15 input, wake/cache/multi-owner acceptance and remaining frontend lifecycle/source/cut/playing retention proof; slow/in-flight video, native wake/idle, GPU/upload bounds, poster competition and physical A/V/display |
| Linux Wayland | Final GNOME launch/render/normal exit; native Weston URI drop/timeout/close and fresh optimized URI-drop result | GNOME/KDE input/drop, sandboxed portal transfer, chooser/file-manager, complete Wayland keyboard/IME/clipboard, fractional scale, idle and sound/GPU timing |
| macOS | R15 full workspace/all-target ARM64 typecheck and denied-warning Clippy on Linux; configured ARM64 and Intel native CI; pinned R15 64-case pack and 106 offline guards | Native execution deferred by user: link/run and portable fault fixtures, Metal, CoreAudio, Retina/IME, dialogs/clipboard and notarized package |
| Windows | R15 full workspace/all-target MSVC and GNU typecheck and denied-warning Clippy on Linux; configured native MSVC CI; pinned R15 64-case pack and 106 offline guards | Native execution deferred by user: link/run and portable fault fixtures, Direct3D/GPU fallback, WASAPI, Unicode paths, DPI, atomic replacement, dialogs/clipboard and signed package |

The [native acceptance pack](native-platform-tests.md) now contains 64 cases
(62 acceptance, two diagnostic; 63 per target), all `not_run`.
[Corrected pinned-pack verification](../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json)
passed 106 offline guards, three 63-case plans/dry runs and three wrong-host
refusals. There are zero pending pins or validation problems. Earlier unpinned
checks and the first audit's case-ID regex error remain preserved.
[Fixture-byte verification](../artifacts/native/platform-regressions-integration-round1/root-fixture-attempt-01/root-byte-verification.json)
checked 494 generated files, including 480 PNGs, two 640×360 H.264 videos and
ten projects. [Linux native opening verification](../artifacts/native/platform-regressions-integration-round1/fixture-opener-round1/root-attempt-01/root-verification.json)
opened all ten on exact R15: live project JSON matched each fixture, contexts
were fresh, revisions were zero with clean state and no undo/redo, ten 1440×900 captures were viewed
and all 46 sampled process identities were gone. One paused startup capture
shows all forty numbered still tiles; six video counters and reused/alternate
source patterns also appear. The missing-media fixture retains its absent
poster/error, separate from expected unavailable audio. These are startup
observations, not catalog passes or sustained/wake/audio/GPU proof. Source and
fixture pins are now set. The earlier
R14 51-case baseline and [57 guards](../artifacts/native/backend-preparation/platform-pack-round8-root-tests.log)
remain historical, as do earlier launch failures. Target execution stays
deferred by the user's instruction to prepare here; these are preparation
checks, not macOS/Windows runtime acceptance.
Separate [round-fifteen backend plans](../artifacts/native/backend-preparation/round15-plans/root-verification.json)
pin five steps each for Linux x64, macOS ARM64/Intel and Windows MSVC to
`dfbb53e9`; all twenty steps are `not_run`. Generating plans launched no target,
media or device tests; earlier R14 plans remain historical.
The earlier [eleven-case supplement](../artifacts/native/platform-regressions-round1/README.md)
and its two draft blockers remain historical. The [completed integration review](../artifacts/native/platform-regressions-integration-round1/review-round1/REVIEW.md)
found four preparation gaps: fresh-copy/recovery isolation, runtime ID checks,
unchanged-sequence handling and compact-library wording. The corrected pinned
pack passed the root checks above. The [final independent review](../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
closed all four findings with no P1/P2 issues. The pack is ready for local
handoff; every native target case remains unrun.

## Remaining production gates

The [R13 native baseline](../artifacts/native/cache-culling-round1/baseline-root-verification.json)
observed the long clip not drawn at view 49.97–59.83 seconds, though clicking the
empty lane selected it, and
480/480 distinct owned posters while browsing. Root checked sixteen captures,
thirteen process identities gone and all 480 still fixtures' full pixels;
harness faults remain preserved. This baseline does not establish measured
GPU growth. [Poster retention](../.scratch/core-editor/issues/09-thumbnail-retention.md)
and [overlap culling](../.scratch/core-editor/issues/11-overlapping-timeline-culling.md)
now have scoped native corrections above. The
[preview dispatch policy](../.scratch/core-editor/issues/10-preview-dispatch-policy.md)
has a selected explicit-poster/retained-admission-order design with a bounded
interactive bypass. The earlier 22-focused/76-media checks remain historical;
the 96-test consumer/media stage and its package lint remain preserved. The
latest [backend retention proof](../artifacts/production-readiness/decoder-retention-round2/backend-root-verification.json)
has 112 passing media tests and current package-only denied-warning Clippy.
Two GIF-resize failures and one unrelated byte-blocked retirement failure were
reproduced and corrected. The [second review](../artifacts/production-readiness/preview-consumer-backend/review-round2.md)
found no remaining backend blocker: reclamation runs outside the queue mutex,
then rechecks currentness; portrait shared pixels and same-consumer pipe
continuity retain controls. These changes are now included in the 521-test
R15 whole-workspace/source-target/release checkpoint. Optimized worker modes
now pass their separate comparison; native throughput remains unverified.

The optional normalized three-consumer protected subset now keeps the four-video-pipe
bound in backend source. Manager assertions went from two failures (five-stream
starts 10 versus 7; eight-stream starts 16 versus 13) plus nine controls to
eleven passes. An actual worker counted one old-generation, five initial and
two next-frame starts (eight), with exact pixels; after 65 completed image jobs
pruned a warm tag, a hint plus unrelated completion applied maintenance and
the next uncached frame added no start. The
[read-only review](../artifacts/production-readiness/decoder-retention-round2/backend-review-round1.md)
found no P1/P2 production blocker; the later warm-prune test extension is
root-executed but outside that review. Counts do not independently isolate
`clear()`; source review covers its branch. Hint/latest retention is bounded
by a 67-lane union, with off-mutex lifecycle/currentness checks. R15 frontend
selection has seven passing controls for stable at-most-three granted/drawn
video owners. The separate bridge below now observes the nonempty frontend
setter and restart reset; worker behavior is independently proven. Corrected optimized/native
measurements remain unverified at the frontend/native seam; the completed
optimized worker comparison above does not exercise that bridge.

The [controller/consumer source review](../artifacts/production-readiness/preview-consumer-identity-review-response.md)
identified R14 ordinal-height rebinding during same-asset transitions.
[Issue 12](../.scratch/core-editor/issues/12-preview-consumer-identity.md)'s follow-up
paint/transport work now passes [152 desktop tests](../artifacts/production-readiness/preview-consumer-phase-b/root-green-verification.json),
including twenty identity, four transport, twenty-four texture and nine viewer
checks. Four font-delta fixture failures and the actual two-pixel Speed-slot
regression remain preserved before correction. [Formatting followed that green run](../artifacts/production-readiness/preview-consumer-phase-b/root-format-source-record.json).
The later 177-test proof also records clean all-target desktop denied-warning
Clippy. R15's startup captures now show separate transport controls at normal
and compact sizes; normal transport input passes the R15 subset above, while
compact play/speed and source-monitor input remain unrun, so
[the transport ticket](../.scratch/core-editor/issues/14-compact-viewer-transport.md)
stays open.

[Viewer texture retention](../.scratch/core-editor/issues/13-viewer-texture-retention.md)
has no byte/count cap in R14: age is 120 painted frames, not wall time. The
selected next policy is 64 MiB logical RGBA, request-time `max(kept, granted_box)`
reservation, a common active-set quality cap and at most eight inactive layers.
The [actual phase-A run](../artifacts/production-readiness/viewer-working-set-round1/phase-a/root-verification.json)
reached all thirteen intended budget assertion failures, with no compile/fixture
failure or ignored test. All prior 152 tests and eight new controls passed;
1,561 immutable artifacts were rehashed. The
[phase-B proof](../artifacts/production-readiness/viewer-working-set-round1/phase-b/root-green-verification.json)
passed all 177 desktop tests and package-only denied-warning Clippy at its source.
Twenty-five resource tests include the original twenty-one tests byte-for-byte
and new resize/audio/notice/failure controls; the original texture tests remain
unchanged. This implements logical budget accounting, not GPU, upload/staging
or process-memory bounds. The completed
[Opus review](../artifacts/production-readiness/viewer-working-set-round1/review-round1.md)
found no P1/P2 ledger defect, but identified the paused persistent-error
decode/repaint cycle. Root's [actual public-worker RED](../artifacts/production-readiness/preview-request-liveness-round1/phase-a/root-red-verification.json)
then reproduced three assertion failures with eleven controls passing: missing
clip/source demands reasked in four unchanged paused frames, while playing
owners 32–39 received no picture across eighty controlled results. PNG
identity/full pixels, valid retries, held-picture/shrink and paused forty-owner
controls passed. There were no compile/fixture/timeout failures and owned
temporary storage was empty. This filter did not rerun the prior 177 tests.
[Issue 16](../.scratch/core-editor/issues/16-preview-request-liveness.md)'s
[correction](../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
passes all 21 focused and 198 desktop tests plus denied-warning Clippy, retaining
6,410 immutable artifacts, the old 24-texture block and phase-A byte prefix
before formatting. Unchanged paused failed time/box is asked once; changed
time/bounds/source and recovery retry, own-held/oversized failure and finite
forty-playing-owner PNG admission have controls.
The [completed correction review](../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
found no P1/P2 behavior defect but identified G1: chooser tests and an empty
still-service set did not execute the nonempty frontend setter or reset.
Six prepared bookkeeping tests were declined, neither integrated nor executed.
The later [actual canonical bridge probe](../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
close two sites at frozen R15 `textures.rs`: a real service received a nonempty
set from `Textures::compose`, and received it again after `cancel`/`ask_again`.
Four cases measured 7 versus 10 decoder starts and restart 14 versus 20;
all 60 transient full-RGBA checks across twelve frames passed and all 51
logged decoder PIDs were gone. This artifact probe adds no standing suite guard:
198 desktop/521 workspace counts are unchanged. Retire/clear/Closed, project
replacement, source monitor, cuts/reorder and playing remain unproved, as do
native/GPU/wake/throughput effects. Errors carry no time, allowing one bounded extra retry; restored
same-path files while paused need a seek/resize retry, and playing offline
video can fail/renew notices for each new time. These are review limits, not
complete missing-media UX or native wake/presentation acceptance.

| Area | Required evidence |
| --- | --- |
| Complete input acceptance | Every [core-feature criterion](../.scratch/core-editor/spec.md) through real input on each supported platform, including errors and reverse actions |
| Audio/video correctness | Audible PCM integrity, sustained underrun-free output and measured A/V synchronization; mixer supports 16 simultaneous audible clips and speed changes alter pitch; exact VFR PTS/HDR need further implementation |
| Native performance | Repeated release startup/idle/seek/frame-time measurements, source-rate presentation and dropped-frame counts, large timelines/imports and process-tree/GPU memory |
| Codec deployment | Clean-machine FFmpeg availability, supported-format matrix, malformed-media bounds, licensing/notices and hardware-decoding strategy |
| Durable projects | Native interruption/full-disk/permission tests, long-history recovery, atomic replacement on all OSes and conflict policy for concurrent manual project writers |
| Desktop quality | Accessibility, complete keyboard/IME behavior, clipboard/file drops, DPI/display changes, minimized/idle behavior and shutdown cleanup |
| Distribution | OS/architecture packages, clean-machine launch, signing/notarization, notices and update/release policy |
| Provider integration | [Portable process-fault fixtures](../.scratch/native-ai/issues/01-portable-process-fault-fixtures.md) and native Windows job teardown; macOS/Windows real sign-in/model/failure/cancel behavior; semantic audiovisual analysis if advertised |

Export was outside the user's first core-feature phase and is not implemented.
No installer, signed distribution or release pipeline is implied by a native
source build.
