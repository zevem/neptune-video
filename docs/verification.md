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
| Looks, animation and motion | Keyframes, transitions, effects, looks, speed, masks, blend, colour, keying, tracking, stabilization as preview | See [advanced editing](#advanced-editing): Linux headless tests, device readback on two adapters and native captures on virtual displays; no playback rate or other platform, and no effect was checked in an [export](#export-and-rendering) |
| Audio editing | Waveforms, fades, track volume and solo, detached audio, background music, voiceover recording, audio effects, keep pitch, loudness normalization, ducking, beats, Save audio as and stem separation, as preview | See [audio editing](#audio-editing): macOS headless tests, mixer and lane readback, launch captures and native acceptance captures of the driven release build on one host; nothing listened to, no device opened, no Linux or Windows run |
| Native AI | Agent tab conversation with the Claude Code or Codex CLI through `aster-mcp` started in the window, each edit one undo step, with the reach of an outside agent (other projects, person-added folders) ("Made as it works", the default); Claude/Codex CLI planning, captured/focused metadata, review/apply/discard/cancel and exact prepared-state commits ("Proposed for review" and caption translation) | See [in-app agent](#in-app-agent): real Claude Code 2.1.300 and Codex 0.162.1 conversations on one macOS host, debug and release builds, with captures; no Linux or Windows window. Planning: exact reviewed split IDs verified through the public runtime; `2c596cb0` both real-provider split/undo/redo and owned no-op/stale fixtures passed. Prior db30 cancel/reopen evidence stays scoped. Planning sends metadata only; the live agent sends rendered frames, contact sheets and transcripts to the provider and cannot listen to sound |
| Titles, captions and graphics | Styled titles, text animation, captions from speech with word times, karaoke, caption editing, SRT/VTT, caption languages and AI translation, lower thirds, title cards, callouts, stickers, emoji, editable templates, kept looks and brand colours, as preview | See [titles, captions and motion graphics](#titles-captions-and-motion-graphics): Linux headless tests, a real Whisper model on synthetic English speech and native captures on a virtual display; one title and two captions, burned in and embedded, in [Linux exports](#export-and-rendering); no playback rate, real provider translation or other platform |
| Export and rendering | Export dialog with poster and saved presets; render queue with pause, resume, cancel, retry and part files; MP4, MOV, WebM and sound-only files; software and hardware encoders; caption sidecars, embedding and burn-in; proxies and preview quality | See [export and rendering](#export-and-rendering): one Linux host, release build, every matrix export matched its expected values and sound was within 0.1 ms of the picture; NVENC only among hardware encoders; no macOS, Windows or long render |

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
each. It records no platform acceptance and no playback frame rate. It was
written before export existed, so everything below is preview at preview
resolution. Export now draws these features through the same compositor
([ADR 0009](adr/0009-export-renders-originals-through-the-preview-compositor.md)),
but the [export verification](#export-and-rendering) used no effect, look,
mask, transition, speed change, tracking or stabilization.

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
| Model, bounds, schema v2 | Bounds, NaN, reversal, atomic failure, v1 decode ([advanced_model](../crates/editor-core/tests/advanced_model.rs), [advanced_commands](../crates/editor-core/tests/advanced_commands.rs)); v1 files load and the next save writes the current version, a future one is refused ([schema](../crates/editor-store/tests/schema.rs)). When this section was written that was v2 and v3; since [audio editing](#audio-editing) (2026-10-13) the writer emits v3, v1 and v2 load, and v4 is refused | n/a | n/a | No | No build that knows only v1 was run against a v2 file; the refusal is shown for the next version on this build through the same path |
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
| Native AI planning | Provider schema under 23 KiB and planner text under 6 KiB, planner text names every parameter key, look and motion commands plan and commit | n/a | No | No | No real Claude or Codex request with the new instructions; Windows command-line headroom |

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

## Audio editing

Waveforms, fades, track volume and solo, detached audio, background music,
voiceover recording, audio effects, pitch-keeping speed, loudness
normalization, ducking, beats, Save audio as and stem separation from
[the audio-editing design](../.scratch/audio-editing/spec.md) are implemented
as preview features ([ADR 0008](adr/0008-sound-is-levels-in-the-mixer-and-filters-in-the-decoder.md)).
This section states what evidence exists for each. **Nothing was listened
to**, no test opened an output device or a microphone, and nothing records
platform acceptance or real-time playback. Since 2026-10-14 an export renders
the same mix offline (see [export and rendering](#export-and-rendering)); no
exported sound has been listened to either.

Every open owner decision of the epic was delegated and taken on the owner's
behalf (spec Comments, 2026-10-14), and the build follows each: instructions
under 6 KiB and schema under 23 KiB; audio effects on audio-track clips only;
speed changes pitch unless the clip has an enabled `Pitch`; solo durable and
in the history; recordings in `<document stem> media/` beside the saved
document, refused for an untitled project; -14 LUFS with a -1 dBTP ceiling;
ducking replaces a clip's volume keys after asking; centre reduction kept;
Save audio as… built; a 0 dBFS master ceiling; Demucs as the person's stem
separator; neural denoise and speech detection
([21b](../.scratch/audio-editing/issues/21b-neural-denoise.md),
[21c](../.scratch/audio-editing/issues/21c-speech-detection-for-ducking.md))
not built and left as research.

## Titles, captions and motion graphics

The features of [the titles and captions design](../.scratch/titles-and-captions/spec.md)
are implemented as preview features: titles and graphics are drawn in the
viewer, and captions leave the editor as SubRip or WebVTT files. This section
was written before export existed. Export now draws titles and can burn
captions in or embed them; the [export verification](#export-and-rendering)
checked one title and two captions, not graphics, karaoke or animated
text. This section states what
evidence exists for each feature and what was not tried. The tickets'
comments in [`.scratch/titles-and-captions/issues/`](../.scratch/titles-and-captions/issues)
list every capture and test by name.

### Where the evidence comes from

| Kind | What it is | Host and build |
| --- | --- | --- |
| Headless tests | Core, store, media, agent and desktop tests at the owning seam. Desktop tests drive the timeline, inspector, library, sheets and transport through real egui passes and a bench that mirrors `App`; `App` itself cannot be built in a test, so its wiring (`perform`, `logic`, the jobs, `on_exit`) was read, not run | One macOS host: macOS 26.5.2, Apple M1 (arm64), Rust 1.97.1, Homebrew FFmpeg and ffprobe 8.1.2 (`scripts/check-media-tools.py` passed). The `dev` test profile (opt-level 1, dependencies at 2), and the unoptimized `ci` profile for ticket 10. Each ticket ran its own packages; the last whole-crate counts reported are `editor-media` 327 and `aster-editor` 827 and 12 (ticket 21a). No whole-workspace run on one source was recorded for the epic, and no Linux or Windows run |
| Mixer readback | `Mixer::chunk` returns the samples a device would get, so fades, track volume, solo, the limiter, every filter, its delay and pitch are read back from WAV files the tests write sample by sample and FFmpeg decodes ([level_tests](../crates/editor-media/src/audio/level_tests.rs), [chain_tests](../crates/editor-media/src/audio/chain_tests.rs)). The source monitor shares the mixer | The same host; no device |
| Lane fixtures | The sound lane run against the host's FFmpeg on files FFmpeg or the test made: tones, clicks, silence, white noise, a late-starting stream, a FIFO ([sound tests](../crates/editor-media/src/sound/tests.rs), [separate_tests](../crates/editor-media/src/sound/separate_tests.rs)); the stem separator as a `sh` stand-in, and once as Demucs 4.1.0 | The same host. No real speech or music |
| Launch captures | The release build launched with `--screenshot` on its own fresh settings, session, cache and project, nothing clicked or typed, under [artifacts/audio-editing/](../artifacts/audio-editing/) | The macOS host's display, scale as the system set it: asked for at 1440x900, the window came up at about 1280x678 points; narrow 760x620. Only states a project file can hold at launch |
| Native acceptance (2026-10-14) | 94 captures of the running release build driven by real pointer and key events (menus, held gestures, hovers, the Audio section, library tabs and filters, the heads' menus), under [artifacts/audio-editing/captures/](../artifacts/audio-editing/captures/), each looked at against [design](design.md); then 12 more (`fix-*`) of the two defects it found, after they were fixed | macOS 26.5.2 (25F84), Apple M1, built-in Retina display at scale 2, Rust 1.97.1, Homebrew FFmpeg 8.1.2. `cargo build --release --locked`: the 94 from `target/release/aster` sha256 `fadbe5aa…5e763`, the 12 from sha256 `64a03608…6c038` with both fixes. Fresh task-owned projects built over `aster-agent` from FFmpeg-made media (a 12 s testsrc2 interview with a tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Normal window asked 1440x900, came up at 1280x709 points; narrow 760x620, and 760x560 for the Audio section. Dark theme only |
| Headless agent runs | `aster-agent --project … --create` sending the batches a person's action makes, over generated fixtures | The same host, `dev` build; the transcripts are in the ticket folders under `artifacts/audio-editing/` |

### Feature matrix

"Readback" is the mixer or the lane above. "Native" is a capture of the
running build that was looked at. "Measured" is a number with its conditions
in [performance](performance.md#audio-editing-budgets); everything else in
that column is unmeasured.

| Feature | Headless tests | Readback | Native capture | Measured | Known limits and unverified |
| --- | --- | --- | --- | --- | --- |
| Model, commands, schema v4 | Fade and track bounds, NaN and infinity, reversal, atomic failure, trim, split, rate and retime clamps, `sounds` and `audible_at` over mute, solo, volume 0, an owned picture and freeze, reverse and ramp, every audio kind's ranges and placement, sixteen and not seventeen ([sound](../crates/editor-core/tests/sound.rs)); v1 and v2 files, checkpoints and recovery load and the next save writes 4 (3 before the merge with titles and captions), v5 refused and left at every entry, round trips with the new fields and audio kinds ([schema](../crates/editor-store/tests/schema.rs)); clipboard text 3, reading 1 to 3 and refusing 0 and 4 ([clipboard](../src/clipboard.rs)) | n/a | n/a | No | No build that knows only v2 was run against a v3 file; the v2 fixtures were written by this tree before the version was raised. Validation keeps fades legal on an owned picture clip. Cost of `Sounding` over a large project |
| Agent surface | Context sends fades, track volume and solo and omits their defaults; serialized `set_fades`, `set_track_state` and the seven kinds validated, inspected and undone as JSON text ([protocol](../crates/editor-agent/tests/protocol.rs)); planned and committed; units sentence and the kinds' place in the text ([planner.rs](../crates/editor-agent/src/planner.rs)) | n/a | n/a | After the merge with titles and captions: planner instructions 5,793 bytes of `MAX_SYSTEM_BYTES` (6 KiB), schema 22,062 of `MAX_SCHEMA_BYTES` (23 KiB), and the Windows-escaped command line under 31 KiB, printed and asserted by the planner tests on 2026-10-14 | No real Claude or Codex request with the new text; the Windows bound is asserted, not run on Windows, and has about a hundred characters to spare. One provider test fails on this host's filesystem before the code under test runs (a non-UTF-8 directory name) |
| Mixer: level, solo, master limiter | Plan admission equals `sounds` over nine tracks and five solo choices; seventeen clips fit once a track is soloed out | 100 ms fade on its curve within 1e-4; track volume and keys exact; solo and mute admission; two full-scale tones never above 1.0; a mix within ±1.0 and the source monitor bit for bit | No device opened | The gain's return to 1 after a peak (1.42 s at twice the ceiling) | No lookahead; sample peaks, not true peaks; the 17th voice still stops the mix; CPU and underruns; nothing listened to |
| Voice filter chain and probe | Strings at defaults and extremes, stack order, disabled and unavailable kinds left out; probe with a fake runner and against the host's FFmpeg (a refused option marks that kind alone); a `sync` does not cancel it | EQ band -24.00 dB on its tone and -0.34 dB at 100 Hz; high-pass; limiter at its ceiling; compressor; centre reduction; noise between bursts 16.3 dB lower at the defaults; `afftdn` and `alimiter` delays removed to within 70 us at two rates | No | Delays (24,979 and 4,979 us at 48 kHz); the probe's eight runs, 297 ms once | Filters start cold at every seek and edit; a value FFmpeg refuses after passing the probe stops all preview sound; centre reduction is left out on a one-channel device; the last 25 ms of a file are lost through noise reduction; FFmpeg 8.1.2 only, no minimum version |
| Pitch and Keep pitch | Strings at six speeds and three shifts; the switch is one `SetEffects` entry each way; phrases | 440.0 Hz kept at speed 2, 880.0 Hz at +12; burst onsets early, never late | No | Sound early by 8.0 to 58.5 ms under `atempo`, by factor | Lead not compensated; pitch held only while speed over the pitch ratio is 0.5 to 2; no formant preservation; rough on music is expected, not heard; held, reversed and ramped stretches silent; a picture clip keeps pitch only after Detach audio |
| Detach audio | The batch, span, rate, time map and keys carried, shared link, one entry undone whole, each refusal, a batch refused whole after a lock, JSON and store round trip, the phrase, the project menu's row ([rules.rs](../src/rules.rs)); the pair planned as its audio clip alone | n/a | The linked pair at both widths ([02](../artifacts/audio-editing/02-detach-audio/)); the clip menu's row enabled and disabled, the project menu's row, the refusal toast and the detached pair, at both widths | No | Unlinking makes both heard; ownership is by link and media, not by span; first audio stream only |
| Sound lane and peak cache | Lane replacement by target, cancellation with processes reaped, `Busy`, idle wakes; cache stamps, checksums, staleness, eviction, sweeping; refusals before any process | A half-scale tone's buckets within one step; silence all zeros; a late-starting stream keeps bucket index equal to container time | n/a | Two hours of silent FLAC read in 0.98 s (`dev`) and 1.64 s (`ci`) | Mono downmix; 10 ms buckets from 8 kHz; the whole file is read; `SourceIdentity` has no audio fields; time to a first waveform and real recordings unmeasured |
| Waveforms | A tile asks once; failures remembered; stale answers dropped; the 32 MiB bound; columns through rate, trim and time map; one mesh a tile ([peaks.rs](../src/peaks.rs), [timeline](../src/ui/timeline.rs)) | n/a | Waves, the level line before peaks, minimum zoom and a failed file, at both widths ([04](../artifacts/audio-editing/04-waveforms/)); maximum zoom at both widths | No | Paint cost and time to first waveform unmeasured; a file at the bucket bound takes a quarter of the held map |
| Fades | Handle drag one entry, Escape, bounds, frames and snaps, locked track, handles never in a trim grip at any width, the curve equal to `fade_at`; inspector fields; menu rows; phrases; owned picture refused in words ([fades_tests](../src/ui/fades_tests.rs)) | Through the mixer row | Stored fades drawn over waves at both widths ([07](../artifacts/audio-editing/07-fades/)); handles at rest, a held handle with its length, a fade set, and the inspector's fade rows, at both widths | No | One curve shape; sound follows on release |
| Track solo and volume | Head arithmetic, each toggle one entry, the dimmed mute equal to `sounds` over 32 settings, the volume field and its refusals, phrases ([heads_tests](../src/ui/heads_tests.rs), [tracks.rs](../src/ui/tracks.rs)) | Through the mixer row | Audio and video heads, solo and dimmed mute, volume figures, at both widths ([08a](../artifacts/audio-editing/08a-track-heads/)); solo set by a click, the audio and video heads' volume menu, a typed 70 and the figure under the name, at both widths | No | Keyboard focus in the heads not captured; percent, not dB; no pan, meter or automation |
| Volume line and keyframe lane | Scale, drag one `SetGain`, Escape, keyed line not dragged, Alt-click one key on the curve, Key volume and its refusals, the empty Volume row, the held percent beside the point taken and clear of both fades' curves and handles or in the name's row ([volume_tests](../src/ui/volume_tests.rs), [keyframes_tests](../src/ui/keyframes_tests.rs)) | n/a | The line at rest, plain, under fades and keyed, at both widths ([08b](../artifacts/audio-editing/08b-volume-line/)); a held line with its percent, the empty Volume row, a key by Alt-click and the Key volume row, at both widths; the percent held over a fade in and between fades on the fixed build (`fix-*-02`, `fix-*-03`), clear of the fade's curve and handle | No | The percent said in the name's row, when fades leave no room, is tested headless only, as is the name giving way wholly to it on a tile too narrow for both (changed after the `fix-*` build; not captured); a line within 0 to 200 % only |
| Library filter and background music | The bed's clips, fade and refusals, one entry, store round trip; the filter by kind and its empty words, the menu row and its reason, the tile's menu from a secondary click on its play offer while a click still plays ([rules.rs](../src/rules.rs), [library.rs](../src/ui/library.rs)) | Level through the mixer row | The filter at rest and a bed made by the headless batch, at both widths ([09](../artifacts/audio-editing/09-background-music/)); Sound and Images chosen, the empty words, a sound's menu with the row enabled and disabled with its reason, and a bed made from it, at both widths; the same menu from a right-click on the tile's play offer on the fixed build (`fix-*-04`, `fix-*-05`) | No | A bed does not follow later edits |
| Audio section and presets | Rows from the table with units and no diamond, a drag one entry, logarithmic frequency, add, switch, move, reset, remove, Voice and 3-band EQ one entry each, the sixteen bound, the probe's answer, a narrow panel at 240 points, idle ([audio_tests](../src/ui/inspector/audio_tests.rs), [audio.rs](../src/audio.rs)); the library's Audio tiles; phrases | Through the chain row | The empty section with Ducking, the Add audio effect menu, Voice with High-pass open, the four cards, Noise reduction unavailable with its rows disabled (made with a wrapper FFmpeg that refuses `afftdn`, not a real build lacking it) and the Effects tab's Audio group, at 1280x709 and 760x560 | No | No EQ curve, attack or release controls; values set by design, none by ear |
| Normalize loudness | Meter fixtures (-20.0 LUFS for a stereo tone at -20 dBFS, 6 LU and an EQ band read as expected, silence and 0.3 s told apart), the `ebur128` summary parser, the stderr tail; the gain rule, keyed clips scaled, stale answers, refusals, store round trip ([normalize.rs](../src/normalize.rs)); menu, inspector and phrase | A tone through the real lane set to 282 %, "-23.0 LUFS, peak -22.8 dBFS" | The notice with what was measured and the volume set, at both widths | Twenty minutes in 2.54 s through the lane; ten minutes of stereo noise in 2.77 s | Relies on the layout of FFmpeg's `ebur128` summary; clip loudness, not programme loudness; no standard claimed; measured as stereo |
| Duck under speech | `duck_keys` and merging in core; the gate over synthetic peaks; six voice files through a held map of two; a failed file left out by name; one entry undone; store round trip; the question; menu, Ducking card and phrases ([duck.rs](../src/duck.rs)) | Keys at 2.85, 3.0, 4.3 and 4.7 s from a tone read by the real lane | The Ducking card, the question before keys are replaced, and the ducked line over the whole clip, at both widths | No | An energy gate, not speech detection; the gate's cost on the UI thread unmeasured; written once |
| Beats | Six tempos of synthetic clicks within 1 BPM and 5 ms of each click, phase, `None` for silence, tones and noise; lane replacement; ticks through rate, trim and time map; snapping that leaves a moved clip's own beats; at most 256 markers in one entry; eight held grids ([beats_tests](../src/ui/beats_tests.rs)) | FFmpeg click tracks through the lane: 127.78 BPM and 100.00 BPM with a late stream, every beat within 5 ms | Beat ticks at two zooms, a move and a trim held snapped to a beat, at both widths | Beats at the bucket bound in 0.49 s (`dev`) and 4.11 s (`ci`) | No real music; above about 170 BPM a click track reads half; no downbeats |
| Save audio as | WAV of a video's whole sound and of a span; cancel, replace, failure and `.part` rules; aliases of the project's media refused; the card at two widths; library, clip and project menus ([extract.rs](../src/extract.rs)) | A 3 s video's tone written as 3.008 s at 1000.0 Hz and -9.02 dBFS RMS; a span's tone 24,001 frames in of 24,000 | The row in the clip, project and library menus | The two fixture figures | Windows never run (its flush handle and resolved-path identity are untried); FIFO cancellation is Unix only; no RF64; a `.part` stays if the editor exits mid-write; the save dialog and the progress card not captured: the system save panel would show the person's own folders, and the host has no private display |
| Separate stems | With a `sh` stand-in: stems moved and probed, cancel and replace leave nothing, a stem never replaces a file, failures in the tool's words, a part folder in the way; placement as one entry undone whole, stale answers, refusals; menu, card and phrase ([stems.rs](../src/stems.rs)) | Demucs 4.1.0 (`htdemucs`) through the lane on 20 s of two tones: two 20.000 s stems | The menu row disabled with its reason, at both widths | 5.65 and 6.04 s with the model held, 15.72 s fetching it; 180 s of tones in 26.7 s by the tool alone | Stand-in tests are Unix only; quality not claimed (tones, not music or speech); memory and device the tool chose; the weights' licence unsettled ([installation](installation.md#stem-separator)); the progress card, a finished separation and placed stems not captured: no separator is installed on the capture host |
| Recording and voiceover | The media folder and take names ([media_folder](../crates/editor-store/tests/media_folder.rs)); `RecordService` with a stand-in source: a ramp bit for bit, overruns counted, `stop` never waits, a crash leaves a playable file, existing file refused, a full disk, the size limit ([record tests](../crates/editor-media/src/record/tests.rs)); placement, the 300 ms anchor, an import in flight, refusals, undo leaves the file, store round trip, idle ([src/record.rs](../src/record.rs)); R and the control at three widths ([transport_tests](../src/ui/transport_tests.rs)) | n/a | The control at rest and its hover, at both widths | Callback 0.3 to 12.2 us into a stalled writer; `stop` 1.3 to 3.5 us; a crashed writer's file read as 2.387 s of 2.5 s | No microphone opened and no take heard against the timeline; device round trip not measured or compensated; microphone permission and a packaged build's usage string; window closed during a take; recording, past the end and a placed take not captured: Record opens the host's microphone and may raise the permission prompt, which needs a person with a microphone |
| Command-panel phrases | Every listed example parses but background music, which needs a sound in the library and has its own test ([agent.rs](../src/agent.rs)) | n/a | No | No | Recording and Save audio as have no phrase; the panel's phrases are the person's, and an agent or the headless CLI can start no measurement, duck, beat search, recording or separation |

### Unverified for every row

- **Listening.** No sound of any feature was heard: no filter, preset,
  limiter, `atempo`, fade curve, duck, bed or stem was judged by ear. Every
  figure is a reading of samples.
- **Devices.** No test opened an output device or a microphone; no take was
  recorded from a real input, and no device round trip was measured.
- **Linux and Windows.** Nothing in this section was linked or run there.
  The other sections of this record are Linux evidence; this one is macOS
  evidence alone.
- **Interaction in the running app.** Native acceptance on 2026-10-14
  drove the release build on the macOS host above with real pointer and key
  events and captured menus, held gestures, hovers, the Audio section,
  library tabs and filters, notices and the heads' menus (94 captures, then
  12 of the two fixes; see "Where the evidence comes from"). Not captured:
  a recording, its take and the control during a take (ticket 19: needs a
  person with a microphone); the save dialog and the save card (ticket 20:
  the system save panel would show the person's real folders); a
  separation's card and placed stems (ticket 21a: no separator installed);
  the held percent in the name's row; keyboard focus rings of this work;
  the light theme; any recording of motion. Captures are stills: dragging,
  scrubbing and playback were not recorded as video.
- **Real media.** Fixtures are tones, clicks, silence and noise made by
  FFmpeg or the tests. No speech, music, camera recording or long compressed
  file was tried.
- **Playback load and A/V sync.** No CPU, underrun or frame-rate figure for
  sixteen filtered voices, and no measurement of sound against picture in the
  running app.
- **FFmpeg builds.** Homebrew FFmpeg 8.1.2 alone; no minimum version, and the
  probe proves each kind at its defaults only.
- **Optimized builds.** Captures are of release builds; every measurement is
  from the `dev` test profile, which is partly optimized (opt-level 1,
  dependencies at 2) and not a release build, or from the unoptimized `ci`.

| Headless tests | Core, store, media, agent and desktop tests at the owning seam. Desktop benches run real egui passes over the inspector, library tabs and speech panel and find controls through AccessKit | One Linux container: 4 vCPU Intel Xeon at 2.8 GHz, 15 GiB, Ubuntu 24.04.5, Rust 1.97.1, `dev` profile with dependencies at `opt-level` 2. Final runs: editor-core 165, editor-store 78, editor-agent 86 with the provider fixture, desktop 672 and `aster-agent` 12 passed; editor-media 247 of 248, the one failure being `decode::tests::a_paced_walk_backwards_at_preview_size_is_measured`, which fails the same way on the base commit and is outside this work |
| Real speech model | A test that runs only when `ASTER_WHISPER_MODEL` and `ASTER_SPEECH_SAMPLE` are set: words, their timeline times and a cancel during recognition | Same host, Tiny q5_1, `talk.wav` (8.5 s of English synthesized by Flite) |
| Native captures | The running debug build driven by XTest pointer and keyboard and looked at, with task-owned settings, presets, models and projects | Xvfb with lavapipe (software Vulkan), no window manager, at 1440x900 and at 760x560 or 760x700. Kept in the session's scratch directory, not in the repository |
| Optimized build | `cargo build --release --locked --bin aster` of the final source | Built on the same host in 7 min 44 s from clean. The `aster` binary is 34,147,232 bytes, about 1.7 MB of it whisper.cpp and ggml. Run under the same Xvfb: the Karaoke box look drawn, and captions from speech with Tiny and the language detected appeared between 4.2 and 4.9 s after the click. That run found two caption tracks with one name. After the fix, the rebuilt binary (34,148,904 bytes, 3 min 9 s incremental) named the second "English captions 2" in the track chooser at 1440x900 and at 760x700 |

### Feature matrix

"Native" is a capture of the running build that was looked at. "Measured" is
a number with its conditions in
[performance](performance.md#titles-captions-and-speech-budgets); everything
else in that column is unmeasured.

| Feature | Headless tests | Native capture | Measured | Known limits and unverified |
| --- | --- | --- | --- | --- |
| Model, commands, schema v3 | Bounds, NaN, defaults, realignment, split/trim/rate rebasing, reversal and atomic failure ([titles](../crates/editor-core/tests/titles.rs)); a version 2 title loads plain, titles, graphics, captions and fonts round-trip, the next save writes the current version (4 since the audio work was merged) and version 5 is refused ([schema](../crates/editor-store/tests/schema.rs)) | n/a | No | No build that knows only version 2 was run against a version 3 file; the refusal is shown for version 4 through the same path |
| Text overlays: fonts, colours, outlines, shadows, backgrounds | Picture tests with the bundled font: ink bounds, outline grows ink, shadow offset, background box, wrap, per-character fallback ([text tests](../crates/editor-media/src/text/tests.rs)); eight inspector tests ([text_tests](../src/ui/inspector/text_tests.rs)) | Text section at both sizes; lower third, titles and outlines drawn in the viewer from lane pictures | No | Time to draw a picture; right-to-left, vertical and complex-script layout not reviewed; synthetic bold is a stroke; the emoji test prints `SKIP` on a host without a colour emoji face |
| Text animation presets | Progress per kind and edge, unit spread and frame snapping in the plan; per-unit motion in the picture | In style list and Typewriter chosen; a Pop entrance correctly invisible at its first frame; `animate in pop` from the command panel | No | No animation was watched at a measured frame rate; whether the lane keeps up with 30 or 60 fps is unknown |
| Captions from speech with word times | 23 speech tests: catalog, model header shapes, languages, quiet cuts, word times, the one-job lane with cancel, downloads through a fake `curl` (checked, resumed, damaged, missing, cancelled); the real-model test above; 5 speech-panel benches | Tiny already present: six captions in about 5.5 s and one undo; Stop; Base downloaded through the proxy and its SHA-256 matched; captions waiting behind a redo; `generate captions` by command | Model load, mix, detector, recognition and cancel with Tiny | English synthetic speech only; Small, Medium and Large v3 Turbo never run; no music, noise, accents or long recordings; whisper.cpp's word times were not compared with hand-timed words; no GPU |
| Animated karaoke captions | Word state in the plan; word, box and fill highlights in the picture | A Box-highlighted caption with colour emoji; a karaoke look dropped on a caption; the current-cue bar with a karaoke caption | No | Never watched during playback at a measured rate |
| Manual caption editing, timing and styling | 18 Captions tab tests, including a bench of 2000 captions that draws 5 to 40 rows | Typing a caption, typed times, row, track, language, style and export menus, the empty state, at both sizes | No | Screen reader; caption editing on a locked track natively. The timeline's track heads cut long names, so "English captions" and "English captions 2" both read "English ca…" there; the chooser shows them whole |
| SRT and VTT import and export | Round trips through files written and read back, malformed files refused whole, VTT word tags, encodings, the 8 MiB cap, language tags in file names | Export menu only | No | The system file pickers were not driven: there is no portal service on Xvfb, so a real import or export through the window was not done |
| Caption languages and translation | Language names and BCP 47 shapes; `copy_captions` in the core; the translation request and its refusals; the track menu choosing and typing a language; command phrases | `pt-br` typed into a new track; Translate into Spanish against a fake provider, the plan, Apply (Spanish track above the hidden English one, "[es] …" in the viewer) and one Undo at 1440; Translate into German at 760 | Provider command line: 30,499 of 31,744 characters | No real Claude or Codex model translated anything; Whisper's translate-to-English was not run on speech in another language |
| Lower thirds, title cards, callouts, stickers, emoji | 13 Text tab tests: adding, placement, canvas scaling, restyling, refusals, search, drops, idle repaint ([text_tab_tests](../src/ui/text_tab_tests.rs)) | Tiles drawn by the lane, scrolled to lower thirds, title cards and emoji; a tile dragged to a track with its ghost; a rotated colour-emoji sticker in the viewer | No | Emoji need a colour emoji face on the system; graphics have no image elements |
| Motion graphics templates with editable parameters | Graphic section tests: texts, shared colours, one font, staggered animation | Graphic section at 1440 | No | Parts cannot be moved or resized in the inspector; only their texts, shared colours, font and animation |
| Kept looks, templates and brand colours | 3 preset-file tests (a round trip without word times, a newer or damaged file moved aside before a save, what a preset cannot hold refused) and 2 inspector tests | Keep popup, Yours after keeping, brand colours in the picker, presets read back after a restart | No | Typing a preset name and Return were not delivered to the window in that session; covered by benches |
| Command panel and native planner | Phrases parse and plan; the schema stays under 23 KiB (`MAX_SCHEMA_BYTES` since the merge with the audio work) and the command line under 31 KiB; planner text names the new commands | `generate captions` and `animate in pop` at 1440 | No | Real provider requests with the new planner text; `translate captions to …` was driven through the track menu, not typed |

### Unverified for every row

- **macOS and Windows.** Nothing here was built or run there. CI runs on pull
  requests and `main`, and this work has not been through it, so the
  whisper.cpp build with Xcode or MSVC, `curl` on those systems, their system
  font folders and colour emoji fonts are untried.
- **Playback.** No animated title, karaoke caption or graphic has a measured
  presentation rate, and the time to draw one picture was not measured.
- **A hardware GPU, a window manager, a physical display and HiDPI.** Every
  capture is a private virtual display at scale 1 on software Vulkan.
- **Real speech.** The only speech heard was synthesized English. Accuracy in
  other languages, with music or noise, and on recordings longer than one
  five-minute piece is unknown.
- **File dialogs.** Font import, subtitle import and export, and model file
  choice open system pickers that were not driven.
- **Accessibility.** Controls are reachable by AccessKit names in the benches;
  no screen reader was used.

## Export and rendering

The features of [the export and rendering design](../.scratch/export-and-rendering/spec.md)
and [ADR 0009](adr/0009-export-renders-originals-through-the-preview-compositor.md)
were driven on 2026-10-14 on one Linux host, release build, for
[ticket 12](../.scratch/export-and-rendering/issues/12-native-verification.md).
The [full record](../artifacts/export-phase7/verify-20261014-093405/results.md)
keeps ffprobe output for each file, the scripts, logs and captures; it is
local, as `artifacts/` is not tracked by git. Nothing here covers macOS,
Windows, a Wayland compositor or any hardware encoder other than NVENC.

Later the same day, fixes for the defects below were checked in a second
release build, with the window driven by XTest
([follow-up record](../artifacts/export-phase7/polish-20261014-102125/results.md),
also local). The export FFmpeg thread caps were measured with a script that
runs the export's source reader and encoder commands directly, not through the
editor.

### Host and build

| Item | Value |
| --- | --- |
| Host | Dell G15 5530, i9-13900HX (32 threads), 30 GiB RAM, Ubuntu 26.04.1, Linux 7.0.0-38-generic |
| GPU | NVIDIA GeForce RTX 4060 Laptop, driver 595.99.02. The window ran on the NVIDIA GPU through Vulkan: `nvidia-smi` listed `aster` as a client. Xvfb has no DRI3, so Mesa could not present there |
| FFmpeg | 8.0.1-3ubuntu2 from `.tools`, named by `ASTER_FFMPEG` and `ASTER_FFPROBE` |
| Build | `cargo build --release --locked --bins`, pinned toolchain 1.97.1 |
| Final binaries (SHA-256) | `target/release/aster` `3452f930b345c90bf9e659c3f7683ab5564478da649b0123665c03d596f2e572`; `target/release/aster-agent` `0a75889e28ab0a4a070766a0d4006ae22158cba30014b53d73621cadfeeb4048`. The build before the speed fix below was `aster` `be85dfc5d54c269e52c85f9f80e523f6846bde07dfb6901ff0346e508f479a84`; output files from both builds were byte-identical |
| Isolation | Private Xvfb displays `:147` (1440x900) and `:148` (760x560), no window manager, `dbus-run-session`, and a fresh HOME, XDG folders, `--settings` and `--agent-session` for each window |
| Follow-up build (defects 4 to 8) | `target/release/aster` `07b644ca2a25918deb075899ed25d19bad3ad5cc06629ca5b9cf42a8eef9ad1b`; `aster-agent` unchanged. Private Xvfb displays `:151` (1440x900) and `:152` (760x560), isolated in the same way. The record does not say whether this build already had the thread caps of defect 3 |
| Thread-cap measurements (defect 3) | Same host, FFmpeg 8.0.1 from `.tools`, SVT-AV1 2.3.0. Another task's `aster` kept the load average between 2 and 21, so each figure is the median of 3 interleaved runs |

### Checks

Each suite ran once, `aster-editor` twice; no test failed or behaved
inconsistently.

| Command | Result |
| --- | --- |
| `cargo test -p editor-core --locked` | 192 passed, 0 failed |
| `cargo test -p editor-media --locked`, with `ASTER_FFMPEG` set | 316 passed, 0 failed |
| `cargo test -p aster-editor --locked` | 871 passed before the speed fix, 872 after (its regression test) |
| `python3 scripts/test-workspace.py --package editor-agent` | 97 passed, 0 failed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Clean, last run after both fixes |
| `python3 scripts/check-architecture.py` | Passed |
| `rustfmt --edition 2024 --check` on `src/main.rs`, `src/bin/aster-agent.rs` and `crates/*/src/lib.rs` | Failed at first on module order in `crates/editor-core/src/lib.rs`; clean after the fix |
| `cargo build --release --locked --bins` | Built |

After the fixes for defects 3 to 8:

| Command | Result |
| --- | --- |
| `cargo test -p aster-editor --locked` | 868 passed, 0 failed |
| `cargo test -p editor-media --locked --lib -- export::`, with `ASTER_FFMPEG` and `ASTER_FFPROBE` set | 50 passed, 0 failed. The capability probe test among them makes every software encoder encode, so FFmpeg accepts `-threads:v 24` and `lp=4` |
| `cargo clippy -p aster-editor --all-targets --locked -- -D warnings` and the same for `editor-media` | Clean |
| `rustfmt --edition 2024 --check` on the changed files | Clean |

### Export matrix

Three fixtures were made with FFmpeg `lavfi`: a 1920x1080 29.97 fps clip of
240 frames with one white frame (90, at 3.003 s) and a click in the sound at
the same time; a 3840x2160 25 fps clip and a 1080x1920 30 fps clip, 4 s each.
The project was built over `aster-agent --connect`: the first fixture on
linked video and audio tracks, a title from 0.5 to 2.5 s, and an `en` caption
track with two cues from 4.0 to 7.0 s. Each export was queued with
`queue_export` and checked with `ffprobe -count_frames`. Every job ended
`done` on its first attempt. "Job" is the job's own `elapsed_seconds`,
FFmpeg start, sound render and output check included.

| Setting | Encoder | Result | Job |
| --- | --- | --- | --- |
| MP4 H.264 software, 1080p | libx264 | High, yuv420p, 1920x1080, 30000/1001, 240 of 240 frames, 8.008 s; AAC 48 kHz stereo | 9.24 s |
| MP4 H.264 Auto | h264_nvenc | Main, 240 frames | 3.67 s |
| MP4 H.265 Auto and software | hevc_nvenc, libx265 | Main, tag `hvc1` on both, 240 frames | 3.47 s, 5.59 s |
| MOV ProRes 422 HQ with PCM | prores_ks | `apch` (HQ), yuv422p10le; pcm_s24le, 384,384 samples (8.008 s x 48,000 exactly) | 9.25 s |
| WebM VP9 with Opus | libvpx-vp9, libopus | VP9 Profile 0, 240 frames; Opus 48 kHz, stream start -0.007 s | 14.8 s |
| MP4 AV1 software, range 2 to 4 s | libsvtav1 | 61 frames as expected; white frame at index 31 as expected | 4.55 s |
| M4A, WAV, MP3, FLAC | aac, pcm_s24le, libmp3lame, flac | AAC 8.008 s; 384,384 samples exactly; 44.1 kHz, 353,153 decoded (8.008 x 44,100 = 353,152.8); 384,384 exactly | 0.83, 0.10, 0.21, 0.13 s |
| 720p and 1440p | h264_nvenc | 1280x720 and 2560x1440, 240 frames | 3.48 s, 6.04 s |
| Custom 1001x563 | h264_nvenc | 1002x564: each side rounded up to even, as the design says | 2.48 s |
| 25 fps from the 29.97 fps project | h264_nvenc | 201 frames, 8.04 s | 4.14 s |
| Range 2 to 5 s | h264_nvenc | 91 frames; white frame at index 31 (90 - 59) | 2.09 s |
| TikTok preset on the 16:9 canvas | h264_nvenc | 1080x1920; the 400-pixel band at the top is black (Y at most 16) | 3.45 s |
| SRT sidecar and embedded, burn-in off | h264_nvenc | `.en.srt` beside the file with 2 cues; `mov_text` stream, language `eng`; captions absent from the picture | 4.43 s |
| VTT sidecar and MOV embedded, burn-in on, 720p | h264_nvenc | `.en.vtt` with word times; `tx3g` stream; captions drawn in the picture | 2.97 s |
| 4K project (3840x2160, 25 fps), range 10 to 14 s | h264_nvenc, libx264, hevc_nvenc | 100 frames each | 4.59 to 4.75, 6.42 to 7.32, 4.65 s |
| Portrait canvas 1080x1920, range 16 to 20 s | h264_nvenc | Project size 1080x1920; 720p is 720x1280; 120 frames each | 2.04 s, 1.24 s |
| Export button in the sheet | h264_nvenc | The window's own path wrote `$HOME/Untitled.mp4`: 420 frames, 14.014 s, in sync | Not recorded |

Every video file is tagged BT.709 with limited (tv) range; ProRes stores no
range. The title was seen at 1.5 s and the caption at 4.5 s in frames taken
from the files.

**Fill.** In the follow-up build, the TikTok preset with Fill on the 16:9
canvas wrote 1080x1920, 1200 frames. Frame 300 scores 42.2 dB PSNR against the
source scaled to 1920 high and cropped at the centre, and 4.0 dB against a
letterboxed layout; the top and bottom 300 rows have mean Y 127, so there are
no black bars.

**Originals, not proxies.** The 4K exports score 41.4 dB (libx264) and
41.9 dB (NVENC) PSNR against the 4K source; the 1280x720 proxy scaled up to
4K scores 33.6 dB against it. Export read the original. The portrait export
scores 40.8 dB against its source.

### Sound and picture sync

The white frame and the click were found in each file and compared.

| Export | Click minus white frame |
| --- | --- |
| Every whole-timeline video export with sound (8 s at each size and codec, and the 14 s export from the sheet) | +0.02 ms |
| The two range exports | +0.03 ms |
| WebM with Opus | -6.98 ms as measured; the Opus stream starts at -7 ms, so about 0 |
| 25 fps from 29.97 fps | The one-frame flash (3.003 to 3.036 s) falls between output frames 75 (3.000 s) and 76 (3.040 s), so no output frame shows it, as it should. Picture timing was checked instead: output frames 50, 75, 76, 100 and 150 match source frames 59, 89, 91, 119 and 179, and the click stays at 3.003 s |

### Queue, refusals and quitting

Driven through the agent operations of the final build unless noted.

- **Cancel**: the job reached `cancelled` 0.24 s after the request; its part
  file and `.aster-render-<job>` folder were removed and its two FFmpeg
  processes ended. Retry ran it again (attempt 2); a second cancel cleaned up
  the same way.
- **Pause and resume**: no frame advanced in 5 s and the job's FFmpeg
  processes used no CPU over 2 s. Resume finished with 240 frames in sync.
- **Folder removed while queued**: the job failed with "The destination
  folder does not exist" and left nothing behind.
- **Refusals**: a relative path, a wrong extension and a missing folder were
  `invalid_export`; an existing file `destination_exists`; a destination that
  is a source asset, even with overwrite, `invalid_export`; WAV with video,
  Opus at 44.1 kHz and a ProRes bitrate `invalid_export`; a range past the
  end `empty_export`; an unknown job `unknown_job`; a request without guards
  `revision_required`. `overwrite: true` replaced the file.
- **Quitting during a render** (window): the "Rendering will stop" question,
  then Stop and quit and Don't save; the process exited and no part file,
  working folder or FFmpeg process was left.

In the follow-up build, through the window's own buttons:

- **Add to queue, then Start all**: the notice "… is held in the render
  queue", a Held row, and Start all rendered it, 1200 of 1200 frames.
- **Pause and Resume**: paused at frame 957 (1440x900) and 560 (760x560); no
  frame advanced in 3 s; Resume finished all 1200 frames.
- **A failed job, retried**: with the destination folder removed after Add
  to queue, Start all ended the job Failed, "The destination folder does not
  exist". With the folder made again, Retry finished it (attempts 2).
- **Stop and quit** (760x560, a 4K job): the export FFmpeg processes were
  gone 6 ms and 11 ms after the click, and the part file and working folder
  after 165 ms and 170 ms (two runs). The job was cancelled at frame 180 of
  1200 while the save question was up, and no output file was written.
  Cancel on the save question kept the window open with the job Cancelled
  and Retry offered; Don't save quit, leaving the folder empty.

### Timings and memory

Whole-job times for 8 s at 1080p were 5.2 to 9.2 s with libx264, varying
widely between identical runs, and 3.55 to 3.74 s with NVENC; 100 frames at
4K took 4.6 to 4.75 s with NVENC and 6.4 to 7.3 s with libx264. During the 4K
libx264 job the encoder FFmpeg used up to 4,942 MiB and the source reader
1,004 MiB, before the thread caps. Conditions and the rest are in
[performance](performance.md#release-build-export-measurements).

With the thread caps, the reader and encoder commands of a 4K libx264 job
(Recommended, Balanced), run outside the editor, peaked at 4,147 to
4,467 MiB together, against 6,258 to 6,282 MiB before; the 4K SVT-AV1 job at
5,178 to 5,344 MiB, against 8,873 to 9,163 MiB. The figures for each encoder
are in [performance](performance.md#export-ffmpeg-thread-caps).

### Captures

Taken from the final build on the private displays and looked at. In
[`out/`](../artifacts/export-phase7/verify-20261014-093405/out):

| Size | Captures |
| --- | --- |
| 1440x900 | `export-sheet-1440x900.png` (poster showing the title at 1.5 s), `queue-rendering-1440x900.png` (frame, speed and time-left line), `queue-done-1440x900.png`, `settings-1440x900.png` (Preview section), `library-proxy-generating-1440x900.png` and `library-proxy-1440x900.png` (proxy queued and ready), `toolbar-ring-1440x900.png` and `toolbar-ring-zoom.png` |
| 760x560 | `main-760x560.png`, `export-sheet-760x560.png`, `export-sheet-hover-left-760x560.png` and its zoom, `queue-rendering-760x560.png`, `queue-done-760x560.png`, `settings-760x560.png`, `settings-preview-760x560.png`, `library-proxy-760x560.png`, `toolbar-ring-760x560.png` and its zoom, `quit-with-job-760x560.png`, `quit2-save-prompt-760x560.png`, `after-stop-and-quit-760x560.png` |

`queue-rendering-scrolled-list-1440x900.png` is from the build before the
speed fix. Frames taken from the exported files are in
[`frames/`](../artifacts/export-phase7/verify-20261014-093405/frames).

From the follow-up build, in
[`out/`](../artifacts/export-phase7/polish-20261014-102125/out) (`w-` at
1440x900, `n-` at 760x560):

| Check | Captures |
| --- | --- |
| Held row and Start all | `w-03` to `w-05`, `n-04` to `n-06` |
| Paused row | `w-06-paused.png`, `n-07-paused.png` |
| Failed row and Retry | `w-08` to `w-11`, `n-08` to `n-10` |
| Fill with the TikTok preset | `w-12` to `w-14` |
| Scroll bar beside the summary values | `n-02-sheet-hover-left.png`, `n-03-sheet-hover-bar.png` and their zooms |
| Running job first, many finished | `w-16-many-finished-active.png`, `n-12-many-finished-active.png` |
| Notices under an open sheet, none over the open queue | `w-07-done-no-notice.png`, `w-17-notice-under-sheet.png`, `n-11-notice-under-export-sheet.png`, `n-13-sheet-open-during-render.png` |
| Stop and quit | `n-14`, `n-17` to `n-20`, `w-18` to `w-21` |
| Folder named as in the queue | `w-01-sheet.png`, `w-04-queue-held.png`, `n-01-sheet.png`, `n-05-queue-held.png` |

### Defects found

Fixed during the run:

1. **Speed and time left were never shown.** `Control::hold` in
   `src/export/queue.rs` reported a wait of about 100 to 200 ns even when the
   job was not paused, which emptied the speed window before every frame.
   Agents got `speed_fps` 0 and no `eta_seconds`, and the queue row showed
   only "Frame N of M". It now returns zero unless the job was paused; the
   test `hold_reports_no_wait_unless_it_was_paused` fails on the old code and
   passes now. The row now reads, for example, "Frame 343 of 420 · 16.7 fps ·
   0:05 left".
2. **Formatting**: module order in `crates/editor-core/src/lib.rs`.

Fixed after the run, the same day. Each fix has a regression test; the
captures are listed above:

3. **Export FFmpeg had no thread cap.** The encoder got no `-threads`, and
   the source reader capped its filter and output threads but not its
   decoder. Now libx264 gets x264's own count, 1.5 times the logical
   processors, at most 24 (`X264_MAX_THREADS`); libsvtav1 gets
   `-svtav1-params lp=4`; and each reader pipe decodes with 4 threads
   (`DECODE_THREADS`). Other encoders are unchanged. Tests:
   `x264_takes_its_own_thread_count_up_to_a_cap`,
   `svt_av1_runs_at_a_bounded_level_of_parallelism` and
   `a_pipe_decodes_with_a_bounded_number_of_threads` fail without the
   change; `encoders_that_bound_their_own_threads_are_left_alone` guards
   the rest. The memory is lower but not near 2 GiB at 4K (see Open).
4. **Clipped values at 760x560.** The widened scroll bar covered the last
   letter of each summary value. The sheet's scroll areas now keep a gutter
   for the bar: the values end at x=282 and the bar sits at about 286 to 292,
   with the pointer over the column or on the bar
   (`a_scroll_bar_widened_under_the_pointer_covers_nothing`).
5. **Running job out of sight.** The queue now lists the job under way
   first, then waiting and held jobs in render order, then finished jobs
   newest first, and scrolls a job that starts into sight once. With 9
   finished jobs at 1440x900 and 7 at 760x560 the active row was first
   (`the_job_under_way_comes_first_then_the_waiting_then_the_finished`,
   `a_job_retried_and_done_again_is_the_most_recent`,
   `a_job_that_starts_is_scrolled_into_sight_once`).
6. **Notices over the queue.** While a sheet is open, notices now sit at
   the bottom of the window, under it, and while the queue is open a
   finished or failed job posts no notice, its row saying so. At 760x560
   "Exported fail-n.mp4" fit in the 40-pixel band under the export sheet
   (`src/ui/notices_tests.rs`,
   `a_finished_job_is_told_by_a_notice_unless_the_queue_shows_it`).
7. **Stop and quit waited for the save question.** It now cancels every job
   at once, waiting ones first, then asks about unsaved changes; staying
   leaves the jobs cancelled, with Retry. Timings are under
   [Queue, refusals and quitting](#queue-refusals-and-quitting)
   (`stop_and_quit_cancels_every_job_at_once_and_starts_none`; it passes
   without the waiting-first order too, so that order has no failing test).
8. **Folder named two ways.** The sheet's folder button showed the folder's
   base name and the queue the shortened path; both now show
   `~/…/Trip/Final`, the full path on hover
   (`the_folder_is_named_as_the_render_queue_names_it`).

Open:

- **4K encoder memory.** The 4K libx264 encoder still peaks at 3,685 to
  3,971 MiB (2.3 GiB with one thread). About 2.5 GiB would need `-rc-lookahead 20` and
  about 8 threads, which changes the encoded output and roughly halves 4K
  encoder speed; that is a product decision
  ([ticket 14](../.scratch/export-and-rendering/issues/14-throughput-and-memory.md)).
- **One slow job.** A 1080p NVENC job of 1200 frames, with the export sheet
  open at 1440x900, reported "Done in 1:06"; the same job took 0:13 to 0:21
  in every other run, 0:18 with the sheet open at 760x560. It was not
  reproduced and its log shows nothing
  ([ticket 15](../.scratch/export-and-rendering/issues/15-implementation-follow-ups.md)).

### Review

A review in four parts (state and integrity, security, tools against the
window, jobs and lifecycle) checked each finding a second time. 29
findings were confirmed, and each was fixed in the build above:

- **Work in no file.** Recovered work, or an edit whose save was refused,
  could be deleted by close, open, attach, handover or shutdown. It is
  now kept: those calls refuse, and `discard` and shutdown leave it in the
  recovery copy. Save As works while the file is in conflict. A refused
  save renews the editing context, and the session now follows it.
  `project_open` loads the new project before letting the old one go.
- **The window** now answers reads while the person edits; only changes
  wait.
- **Undo.** The agent's own undo and redo now step over several entries.
  An edit saved without its recovery copy keeps its result and IDs.
- **Writing and reading files.**
  - Every numbered frame file is checked.
  - Pictures and caption files are written through a new hidden file
    renamed into place, so dangling links are refused.
  - FFmpeg formats that read other files (playlists, concat lists,
    manifests) or draw text as video are never media.
  - `AddAssets` needs importable media and probes it again.
  - Folder walks do not read links that lead outside.
  - Fonts outside the roots stop renders.
  - Attaching to a window whose project path cannot be resolved is
    refused.
- **Tools.**
  - `text_edit` with a template keeps the template's look.
  - A loudness or transcript result applies only to the project and clip
    state it was measured on.
  - Ripple trims are bounded by the source, not by neighbours.
  - `clip_add` on a new track honours `at`.
  - Moves to another track keep the window's rule of one track and the
    same kind.
  - `text_add` places and fits for the length asked.
  - Fades with both edges are one check.
  - Drop-frame timecode and huge hours are refused.
- **Jobs.**
  - Local exports survive a project switch or a handover.
  - Speech jobs run one at a time, and a panicking speech worker answers.
  - Jobs are cancelled before the server exits.
  - Frames are encoded off the protocol threads, a call's frames have a
    pixel budget and stop when it is cancelled, and contact sheets fit
    2048 pixels.
  - Resource notifications are sent at most twice a second.

### Not verified

- **macOS and Windows.** Nothing in this section was built or run there:
  VideoToolbox, Media Foundation, Metal, Direct3D, their file pickers,
  Explorer and Finder reveal, and Windows paths
  ([ticket 13](../.scratch/export-and-rendering/issues/13-other-platforms-and-hardware-encoders.md)).
- **Other hardware encoders.** AMF was not offered; VA-API ("Failed to
  initialise VAAPI connection" on both render nodes) and QSV ("Error creating
  a MFX session") failed the encoder check on this host. Only NVENC's H.264
  and HEVC encoders wrote files; `av1_nvenc` passed the check and was not
  used.
- **The automatic fallback from hardware to software** under Auto: covered by
  engine tests only, as NVENC could not be made to fail from outside.
- **libaom AV1** (offered, not exported), and **96 kHz and mono** sound.
- **The thread caps in the editor.** No release-build export through the
  editor and its GPU compositor was measured with them: the figures are from
  the reader and encoder commands run on their own, on synthetic sources
  (`testsrc2` with noise, H.264 at about 300 Mbit/s and HEVC at 4K), with
  speed varying up to ±15 % between runs. Other hosts, SVT-AV1 releases
  before 2.0, SVT-AV1 preset 5 at 4K, and libaom, VP9 and ProRes beyond a
  spot check at 4K were not measured. Below 17 logical processors the x264
  cap changes nothing.
- **Picture parity with the viewer** beyond looking at it; the ±2 parity is a
  headless test.
- **Long renders** of minutes or hours, **GPU memory** of a job, how much a
  job slows the interactive window, effect-heavy projects and **camera
  footage**: every fixture is synthetic
  ([ticket 14](../.scratch/export-and-rendering/issues/14-throughput-and-memory.md)).
- **Preview quality and proxy switching during playback**: the badges were
  captured, playback with them was not measured.
- **A real display, a window manager, a Wayland compositor and HiDPI**: every
  window was on Xvfb at scale 1.
- **Sound devices**: sync was measured in the files; nothing was played
  through a device.
- **Free space, an interrupted machine and a killed editor** during a job.

## Bundled media tools

Aster now runs the `ffmpeg` and `ffprobe` beside its executable, or in a checkout
build the ones `scripts/fetch-media-tools.py` placed in `.tools/media/<os>-<arch>`,
before `PATH` ([media executables](installation.md#media-executables)). Measured
on 2026-10-14 on the export host above (Ubuntu 26.04.1, Linux 7.0.0-38-generic,
no FFmpeg on `PATH`):

| Check | Result |
| --- | --- |
| `python3 scripts/fetch-media-tools.py` | Downloaded and digest-checked the `linux-x86_64` pin in 22 s; both tools report `n8.1.3-9-g29e619e767`. Linked: glibc libraries and `libgcc_s` only. Every encoder and audio filter Aster names is listed by `-encoders` / `-filters` |
| `cargo test -p editor-media --locked`, no override | 446 passed after one fixture fix: FFmpeg 8.1's ffprobe reports a WebM Opus file's start as 0 where 8.0.1 reported -0.007 s, so `opus_with_its_lead_in_keeps_the_files_time_from_any_point` now accepts both; its timing checks pass with 8.0.1 and 8.1.3 |
| `cargo test -p aster-editor --locked`, `ASTER_FFMPEG` / `ASTER_FFPROBE` naming the fetched tools | 1104 + 13 passed, 0 failed |
| `python3 scripts/test-package-release.py` | 18 passed, including the bundled-tools cases |
| `cargo clippy -p editor-media --all-targets --locked -- -D warnings`, `check-architecture.py` | Clean, passed |
| Release `aster` `c21e8a531b607a3c93ac557c036a16736e724ac643bf4e27e49d81f570a05d13` on private Xvfb displays, fresh HOME/XDG and `--settings`, `PATH=/usr/bin:/bin` | From `target/release`, and copied into a `bin/` with only the two tools beside it: `clip.mp4` imported with its poster at 1440x900 and 760x560; dragged to the timeline by XTest and played with Space, frames decoded and no FFmpeg error showed. Copied alone into a `bin/`, the same import found no tool and the library stayed empty |

Not verified: the other five pins beyond their published digests, any package
built with `--media-tools` (no build record was made), audible sound (the
isolated session had no audio device), macOS Gatekeeper with unsigned bundled
tools, and the GPL source offer a published package needs.

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

## Native MCP

[`aster-mcp`](mcp.md) ([ADR 0010](adr/0010-mcp-reaches-the-editor-through-the-agent-protocol.md),
[spec](../.scratch/native-mcp/spec.md)) was driven on 2026-10-14 on one
macOS host with a debug build. Nothing here covers Linux, Windows, a release
build or an Intel Mac ([ticket 05](../.scratch/native-mcp/issues/05-linux-and-windows-acceptance.md)).

### Host and build

| Item | Value |
| --- | --- |
| Host | Apple Silicon Mac, 8 CPUs, 8 GB RAM, macOS 26.5.2 (Darwin 25.5.0), Retina display |
| Build | `cargo build --locked --bins` (dev profile), pinned toolchain 1.97.1. Final binaries (SHA-256): `target/debug/aster` `fbe83a90cd092ffcbea95e0c5eb1f770abf18fe08793d27f1c398a5ec3137fae`; `target/debug/aster-mcp` `1aa890ddd3fbdbb0d7e4cf1a128b774e04407434b5371fdf00fb0a16ad68f367` |
| FFmpeg | 9.0.2 from `.tools/media/macos-aarch64`, found beside the checkout |
| Clients | the rmcp 3.5.1 client (tests), Claude Code 2.1.300, Codex CLI 0.162.1, a JSON-RPC script over stdio, curl over HTTP |
| Isolation | a fresh temporary folder per test or run as the only root, with its own `--desktop-settings`, `--models`, sessions and connection files; windows started with `--settings` inside it |

### Checks

| Check | Result |
| --- | --- |
| `cargo test --locked --test mcp` (9 end-to-end tests: create, import, timeline edits, effects, transitions, keyframes, titles, music, batches, dry runs, stale revisions, frames, contact sheet, scene and loudness jobs, preview and export, undo, checkpoints, resources, reopening in a second server and with the editor's loader, roots and overwrites, a killed server, a stopped editor's recovery copy kept through close, create and a stopped server, the agent's undo and redo over several entries, dangling links, playlists passed off as media, links in imported folders, HTTP auth and a session) | passed |
| The same with `ASTER_TEST_DESKTOP=1`: a real `aster` window, `project_open` routed to it, an edit, a frame, undo, redo and save through it | passed |
| `cargo test --locked --lib` (with the `mcp::` unit tests for paths, times, jobs, the contact-sheet layout and the command schema) | 1122 passed |
| `cargo test --locked -p editor-media` (with the new scene, details, playlist and import-predicate tests) | 451 passed |
| `cargo test --locked -p editor-agent` with the provider fixture | 75 passed, 1 failed: `provider::tests::an_unrepresentable_native_path_is_refused_before_creating_an_instruction_file` (`Illegal byte sequence`, os error 92), which fails the same way without this work on this host |
| `cargo clippy --locked -p aster-editor -p editor-media --lib --bins --tests`, `cargo fmt --check` for the touched packages, `scripts/check-architecture.py` | clean |
| `scripts/test-package-release.py`, `scripts/test-native-test-runner.py` with `aster-mcp` among the binaries | passed with `TMPDIR` outside `/var`; with the default `TMPDIR` they fail on this host before and after this work, because `/var` is a symbolic link |
| HTTP with curl: no token, wrong token, `Host: evil.example`, `Origin: https://evil.example`, a 5 MB body | 401, 401, 403, 403, 413; a valid token opened a session. The connection file was mode 0600 and was removed on SIGTERM |
| SIGTERM, SIGHUP and SIGINT to a stdio server with a project open | exit 0; lock released and recovery copy removed |
| `tools/list` | 39 tools, 40,854 bytes on the wire: 106 bytes under the 40 KiB the test enforces, so a new tool needs room made for it |
| Whisper `tiny` installed by `transcription_models`, then `transcribe_start` on macOS `say` speech and `captions {from_transcript}` | words with frame times; five captions |
| An export cancelled 0.5 s into rendering | ended `cancelled`; no file left at the destination |

### Real clients

Claude Code and Codex each built an edited video from raw footage using
only the Aster tools
([record](../.scratch/native-mcp/evidence/real-clients-2026-10-14.md)).
Each run created the project, inspected the media, built and trimmed the
timeline with a dissolve, transcribed and captioned the speech, added a
title and music, looked at frames and fixed what it saw, and exported a
preview and the final file. Claude Code made 41 calls, one of them refused
and corrected. Codex made 39 calls, none refused, once its editing tools
were approved in its configuration. Both runs used an earlier build.
Claude Code ran the same brief again on the final build: 38 calls, none
refused, saved at revision 12. Its `promo.mp4` is 1280x720, 30 fps,
9.47 s, with sound, and no recovery copy was left.

### Window and person

A debug `aster` window with a task-owned `--settings` was driven with key
presses sent through System Events:

- The person's Cmd+Z and M (marker) reached the agent at once.
- The agent's `history {op: undo}` refused the person's entry with
  `not_agent_entry`.
- The agent's transition, look and title appeared in the window as they
  were made.
- The agent's `project_save` saved through the window.

Captures of the final build (hashes above), at 1440x900 unless noted, in
[`.scratch/native-mcp/captures`](../.scratch/native-mcp/captures):

- `1-person-opens-agent-project.png`: a project an agent made, left with
  SIGTERM, opened by a person: no restore prompt.
- `2-agent-edits-live-in-window.png`: after the person's marker and save,
  the agent's wipe, look and title appearing in the person's window.
- `3-after-agent-undo-and-save.png`: the agent's own title undone and the
  project saved through the window.
- `4-narrow-window-agent-project.png`: the saved project reopened at
  820x640.
- `5-person-opens-claude-code-project.png`: the project Claude Code made
  on the final build, opened by a person: saved, no restore prompt, with
  its captions, title, dissolve and faded music.

Not captured: a window refusing an agent's edit during a person's drag
or dialog (`person_editing`, `desktop_busy`). The refusal is covered by
code and by the protocol, but no key presses in this run held a drag or
opened a dialog.

### Defects found and fixed

1. `project_open` refused `project_busy` when a window had the file open
   with unsaved work; it now attaches to the window.
2. A failed `project_open` had already closed the open project; the new
   project is now read first.
3. Opening a file whose recovery copy held a stopped editor's unsaved
   work would have replaced that copy at the first edit; it now refuses
   `recovery_available`.
4. Servers stopped by a signal (as Claude Code and Codex stop them) left
   their lock and recovery copy; signals now release the project.
5. `render_export_start` failed `capabilities_pending` in a server's first
   seconds; it now waits for the encoder check.
6. `tools/list` was 102 KB; it is now under 40 KiB.
7. Smaller ones: a one-command edit kept its builder's label ("Normalize
   loudness" for a volume change); `job {op: cancel}` answered the state
   before the cancel took effect; caption export needed a track ID when
   the project had one caption track; an out-of-range text style gave no
   ranges; undo with nothing to undo said `not_agent_entry`.

### Not verified

- Linux, Windows, Intel Macs and release builds. On Windows the stdio
  descriptor move, the connection file's permissions and the crash test
  do not apply as written.
- Whether a minimized or hidden window answers within the host's 5 s.
- Clients other than Claude Code, Codex and the rmcp client; the
  `2026-07-28` protocol revision's stateless HTTP mode.
- Performance under many concurrent HTTP clients; frame rendering where no
  graphics adapter opens.
- Transcription with models other than `tiny`; GPU and hardware-encoder
  paths beyond the preview export's `hevc_videotoolbox`.

## In-app agent

By default ("Made as it works"), the Agent tab's Ask mode edits through
[`aster-mcp`](mcp.md) started in its own window
([ADR 0011](adr/0011-the-agent-tab-edits-through-aster-mcp-started-in-its-window.md),
[feature folder](../.scratch/in-app-agent/)). It is a conversation with the
installed Claude Code or Codex CLI whose only tools are the same 39 tools
outside MCP clients use. Every edit in the window goes through the window's
agent session and is one undo step in the window's history. Like an outside
agent it may also make, open, save, export and close other projects and
come back to its window, and it may use the folders the person adds in
Settings › Agent › Folders, which reach the running conversation through a
roots file. "Proposed for review" keeps the earlier one-proposal planner,
and caption translation always uses it. It was driven on 2026-10-14 with
debug builds and on 2026-10-15 with release builds, on one macOS host. No
window was run on Linux, Windows or an Intel Mac; the cross-target checks
below are type checks and tests only.

The planner sends project metadata only. The live agent's tools return the
project's editing data, rendered frames, contact sheets, transcripts, level
and loudness measurements, and file names and paths inside its roots, and all
of that reaches the provider under the person's own sign-in and terms
([what reaches the provider](native-ai.md#what-reaches-the-provider)). Aster
uploads no source media file. The agent cannot read files outside its roots,
run shell commands, browse the web or use other MCP servers.

### Host and build

| Item | Value |
| --- | --- |
| Host | Apple Silicon Mac (arm64), 8 CPUs, 8 GB RAM, macOS 26.5.2 (Darwin 25.5.0) |
| Release build | `cargo build --release --locked --bins`, pinned toolchain 1.97.1, run from `target/release` (2026-10-15). Final build G (SHA-256): `aster` `4ac9f25dd9c484a7b9c5ab9536d1af68773330aee0cdb94c4d98aadabf78d5e0`; `aster-mcp` `350d1ab77af9f7491869f9b372bbc0cbda20f9e1d20b8baa0c3dea1278d508b6`; `aster-agent` `6f7c7fb7a0af2620accb4ec21c7207281504ad11687cf203d5cb47f58dba614a`: run 20, capture 29. Build E (`aster` `6cbc3c01…`, `aster-mcp` `387e6d3d…`), before defect 12: runs 12–17, captures 16–25. Build F (`aster` `3c184d51…`, same `aster-mcp`): runs 18–19, captures 26–28. G changes only the folders file's wait (written and waited for only when the folders changed before a conversation starts), the window-project refusal's wording and comments |
| Debug build | `cargo build --locked --bins` (dev profile), pinned toolchain 1.97.1. Debug build D (SHA-256): `target/debug/aster` `3ceedd91bc57ebb4ef33f315beea9e4d84cfb8eda9e4784d63c071457a489f87`; `target/debug/aster-mcp` `a8dea38a9f59f96cd6183c4ab833501eb2dd05020c163bb48b3e8df409658124`; `target/debug/aster-agent` `f1a4901ed32cce7acd2f6afc44ce0e62d970dbf1f070d6f268e4effd97ee0147` |
| Builds per capture | Captures 01–05: builds before A. Build A (`aster` `94e5cee8…`, `aster-mcp` `03c65742…`): run 8 and the first two requests of run 9, captures 06–09. Build B (`aster` `cfb88ea4…`, `aster-mcp` `827ebb47…`), with the shorter working row and the "changes" wording: captures 10–11. Build C (`aster` `06af350d…`, `aster-mcp` `f0d5557b…`) adds the restart note: run 11, captures 12–13. Build D adds the panel introduction's full folder list and the Windows launcher message (defects 10 and 11): captures 14–15 |
| Providers | Claude Code 2.1.300 (`claude-opus-5-5`, effort Medium; Low, then Medium, in run 11) and Codex CLI 0.162.1 (`gpt-6.1-sol`), each under the host's existing sign-in |
| Isolation | a fresh task-owned folder per run with `footage/` holding `interview.mp4`, `street.mp4`, `bars.mp4` and `music.m4a`, and the Whisper `tiny` model; for the release runs also an `extra/` folder outside the project's. Windows at 1280x800 points (wide) and 820x640 points (narrow) |

### Checks

| Check | Result |
| --- | --- |
| `cargo test --locked --lib` (with `markdown_marks_are_read_not_shown` and `a_step_ends_once_and_steps_left_running_stop_with_the_request` in `src/ui/agent.rs`; `the_agents_folders_are_written_whole_for_its_owner_only_and_replaced_when_they_change` and `a_window_knows_its_own_agents_requests_from_another_windows` in `src/ai.rs`) | 1126 passed |
| `ASTER_TEST_DESKTOP=1 cargo test --locked --test mcp` (12 tests). `a_server_started_for_a_closed_window_still_makes_projects_of_its_own` runs without a display: a server started for a gone window answers `desktop_closed` while it follows it, then makes, edits, closes and opens a project of its own and does not look for the window again. `the_agent_tab_server_starts_in_its_window_and_may_leave_and_come_back` needs one: it edits the window's project with authority `desktop`, undoes and redoes, saves a copy and waits for `project_status.last_save`, makes and edits a project of its own while the window's file stays as it was, and comes back with `desktop {op: attach}` naming no window. `the_folders_in_a_roots_file_are_read_again_when_it_changes`: no file, no folder; a folder named later is usable in the same session, `/` is left out, a folder taken out is refused again. These replace the first round's pinned-server tests | passed, five runs in a row on the final code |
| `cargo test --locked -p editor-agent --lib` with `ASTER_PROVIDER_TEST_FIXTURE`. New: three conversation tests against a fake `claude` script (several turns keep one process, ending starts afresh, Stop ends within 3 s, a missing provider fails), three Claude stream tests and three Codex tests, one of them checking that the shell stays off and `code_mode_host` stays on | 84 passed, 1 failed: `provider::tests::an_unrepresentable_native_path_is_refused_before_creating_an_instruction_file` (os error 92), the same host failure recorded under [native MCP](#native-mcp), not caused by this work |
| `cargo clippy --locked -p aster-editor -p editor-agent --lib --bins --tests -- -D warnings`, `cargo fmt --check`, `scripts/check-architecture.py` | clean |
| `cargo check` and `cargo clippy -- -D warnings` of `editor-agent` (lib, bins, examples, tests) for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, from this Mac | clean. The root package could not be checked for Windows from here: `whisper-rs-sys` needs CMake and a Windows C toolchain |
| Linux container (Colima, Debian bookworm, aarch64, `rust:1.97.1-bookworm`, the CI's packages plus `cmake` and `libclang-dev`): clippy of `aster-editor` and `editor-agent` with `-D warnings`, `cargo test --locked --test mcp` without a display, the `ai::` and `mcp::` unit tests, and `editor-agent`'s tests | Clippy clean. `--test mcp`: 12 passed with Aster's pinned FFmpeg 8.1 for linux-aarch64 and Mesa's software Vulkan (lavapipe); with Debian's FFmpeg 5.1 the export test fails because that FFmpeg refuses the decoder's `-thread_queue_size` placement, and without a Vulkan driver `frame_render` has no adapter. No display: the two window tests return early, so no Linux window was exercised. `ai::` and `mcp::` unit tests: 25 passed. `editor-agent`: 87 passed, including the test that fails with os error 92 on this Mac |

### Native runs

Runs 1–11 were typed into the Agent tab of a debug `aster` window on
2026-10-14; runs 12–19 into a release window on 2026-10-15, with the full
requests in [the release runs](../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md).

1. Claude, wide window, an earlier build: a full promo brief. In about 2
   minutes it imported the footage, built interview, street (2 s) and bars
   with a dissolve, ripple-trimmed the interview, transcribed it and made
   5 captions, titled it "Aster Promo" and then enlarged the title with an
   outline, added the music in two pieces with a fade, exported
   `promo_preview.mp4` (960x540) and `promo.mp4` (1280x720, 30/1 fps,
   9.472 s, with sound) and saved. 11 edits.
2. The same conversation: "Change the title text to Aster Promo 2026 and
   make it yellow." (1 edit), then "Actually, undo the colour you just
   picked but keep the new words." It remembered the earlier turn, undid its
   own step and applied the words again (2 changes).
3. Stop during "Waiting for a job" in a request for a contact sheet, a fade
   out and an export of `promo-v2.mp4`. The edit already made stayed, and
   the queued export finished in the window's render queue. No `claude` or
   `aster-mcp` process was left.
4. Codex: the first attempt answered "I couldn't access Aster's tools"
   (defect 1 below). After the fix it added a marker "Codex was here" at
   3 s and made the street clip black and white, then checked a frame.
   2 edits.
5. The person pressed Cmd+Z in the window: the agent's last edit, the
   black-and-white effect, was undone and Redo became available.
6. Codex, narrow window: a marker "Narrow" at 1 s and a frame check. The
   steps and the note fit.
7. "Proposed for review", Claude: "Rename the marker at 1 second to
   Opening." gave a proposal (Remove marker, Add marker) with Discard and
   Apply. Nothing was applied.
8. Build A, a fresh folder, Claude, wide window: import the footage, put
   interview then bars with a half-second dissolve, a title "Final build" for
   2 s, `music.m4a` at -12 dB, look at a frame, export `final.mp4` beside the
   project and save. 7 edits. ffprobe read `final.mp4` as 1280x720, 30/1 fps,
   7.509 s, with video and audio; the project was saved.
9. Builds A and B, Codex, narrow window: "Make the Final build title bigger
   and move it near the top, check a frame at 1 s" (2 edits), then "Undo only
   the move you just made, keep the bigger size." It undid only the move, in
   the same thread. Then a marker "Cut" at 4 s, bars faded in over 0.5 s and
   frames looked at, at 4 and 5 s.
10. After the windows closed, no `claude`, `aster-mcp` or `ffmpeg` process
    of the task remained. Two Codex app-server processes started on
    September 27 and October 10 belong to the person and were left alone.
11. Build C, Claude, effort Low: "How many clips are on the timeline? Do not
    change anything." gave a correct answer and the note "no changes". After
    effort changed to Medium, "What did I ask you a moment ago?" showed the
    restart note, and the agent said it could not tell. No task process was
    left afterwards.
12. Release build E, Claude, wide: asked to make a separate `teaser.aster`
    without changing the person's project, cut street then bars with a
    title, export `teaser.mp4`, then come back and add a marker. It made the
    project with local authority, exported `teaser.mp4` (1280x720, 30/1 fps,
    7.019 s, with sound), came back and added the marker. Note: "1 change,
    which undo or redo takes back · 3 changes in other projects". The roots
    file was mode 0600 and named the project's folder, `footage/` and the
    six usual folders.
13. The same conversation: importing `extra/b-roll.mp4` was refused
    `outside_roots`, and the agent named Settings › Agent and changed
    nothing. The person added `extra` with "Add folder…" in the native
    dialog; the roots file named it within the frame. "I added that folder
    in Settings. Try again please." imported it and put it at 7.49 s, in the
    same `claude` process, with no restart note. A folder chosen by mistake
    on the first try reached the roots file too, and "Remove" took it out of
    both again.
14. Build E, Codex, narrow: a half-second fade out on the b-roll, an export
    to `codex.mp4` (1280x720, 30/1 fps, 11.520 s, with sound) and a save; the
    window showed "Saved" and the file held the fade.
15. Build E, Codex, narrow: Stop at 22 s, after ten markers and five
    captions: "Stopped. The 11 changes already made stay; undo and redo take
    them back."
16. Build E, Codex, narrow: Stop 7 s into a contact sheet and export:
    "Stopped. Nothing was changed."; no export was written. After each Stop
    no Codex or `aster-mcp` process of the task was left.
17. Build E, Codex, narrow: open `teaser.aster` in a new window with
    `desktop {op: open}`, retitle it "Teaser 2", save, come back and count the
    person's clips (8). Note: "no changes here · 1 change in another
    project". The second window hid the edit's row (defect 12).
18. Build F, Claude, wide: the same with "Teaser 3". The second window listed
    "Agent: Edit text" and "Agent: Save"; the home window's note read "no
    changes here · 1 change in another project".
19. Build F, narrow: Settings › Agent with an added folder fits at 820x640.
    After the windows were quit no task process, session file or roots
    file was left.
20. Build G, Claude, wide, a new window and conversation: "Which folders may
    you use right now? Check project_status, tell me whether a folder named
    extra is among them, and change nothing." It listed the project's
    folders, `extra` and the six usual folders, and changed nothing: the
    server read the folders file the window wrote before it started.

### Captures

In [`.scratch/in-app-agent/captures`](../.scratch/in-app-agent/captures):

- `01-claude-steps-live.png`: run 1, Claude's steps appearing in the panel
  as it builds the promo.
- `02-claude-follow-up-same-conversation.png`: run 2, the colour undone and
  the new words kept in the same conversation.
- `03-stopped-mid-request.png`: run 3, the Stop note in its earlier wording
  (defect 3).
- `04-person-undoes-agent-edit.png`: run 5, the person's Cmd+Z took back the
  agent's black-and-white effect; Redo is available.
- `05-proposed-for-review-mode.png`: run 7, the reviewed proposal with
  Discard and Apply, nothing applied.
- `06-final-claude-working.png`: run 8 working on build A; the row still
  cut to "3 edi…" (defect 5).
- `07-final-claude-done.png`: run 8 done.
- `08-final-codex-narrow.png`: run 9 at 820x640, the title bigger and moved.
- `09-final-codex-follow-up.png`: run 9, only the move undone.
- `10-final-narrow-working-row.png`: build B, the working row "Working… 15s"
  fits at 820x640.
- `11-final-codex-done-note.png`: build B, the note "Codex · gpt-6.1-sol ·
  2 changes; undo and redo take them back".
- `12-final-agent-menu.png`: build C, the agent menu: Answered by, Effort,
  Edits (Made as it works, Proposed for review), New conversation, Model….
- `13-final-restart-note.png`: build C, the restart note after the effort
  changed, and the agent saying it could not tell.
- `14-final-intro-wide.png`: build D, a new window's introduction naming all
  six folders.
- `15-final-intro-narrow.png`: build D at 820x640; the second paragraph is
  further down the panel's scrolling log.
- `16-release-claude-makes-another-project.png`: run 12, back in the window
  with the note naming changes in another project.
- `17-release-folder-outside-refused.png`: run 13, the refusal naming
  Settings › Agent.
- `18-release-settings-folder-added.png`: run 13, Settings › Agent with the
  added folder, Remove and "Add folder…".
- `19-release-same-conversation-uses-folder.png`: run 13, the import done in
  the same conversation.
- `20-release-codex-narrow-export-saved.png`: run 14, exported and saved at
  820x640.
- `21-release-codex-working-before-stop.png` and
  `22-release-codex-stopped-changes-stay.png`: run 15, before and after Stop.
- `23-release-codex-stopped-nothing-changed.png`: run 16.
- `24-release-codex-back-from-another-window.png`: run 17, the note "no
  changes here · 1 change in another project".
- `25-release-second-window-missing-row-defect.png`: run 17, the second
  window without the edit's row (defect 12).
- `26-release-second-window-shows-agent-rows.png`: run 18, both rows.
- `27-release-claude-back-in-its-window.png`: run 18, the home window.
- `28-release-settings-folders-narrow.png`: run 19.
- `29-release-g-new-conversation-reads-folders.png`: run 20.

Not captured: the in-app agent refused while the person holds a drag or a
dialog is open (`person_editing`, `desktop_busy`), and an outside MCP client
editing the same window as the in-app agent at the same time. Both go through
the window's existing refusal and one-at-a-time request path; no run held a
drag, opened a dialog or connected a second client.

### Defects found and fixed during the runs

1. Codex 0.162.1 calls MCP tools through its code-mode host. Conversations
   started with the planner's disabled features, which include
   `code_mode_host`, so the first Codex run answered "I couldn't access
   Aster's tools". Conversations now keep `code_mode_host` on, with the shell
   tool still off; a test checks both.
2. The window listed its own agent's exports as "Agent: Export …" rows, the
   rows it shows for outside MCP clients. Requests from this window's agent
   (request IDs starting `aster-tab-`) now show only as the agent's steps;
   outside clients keep their rows.
3. The Stop note read "Stopped. The 1 edit already made stays; undo takes
   them back." One change and several changes now have their own wording.
4. Notes said "edits", but the count includes the agent's own undo. Notes
   and the working row now say "changes".
5. At 820x640 the working row was cut to "3 edi…". It now reads
   "Working… Ns · N changes" and fits.
6. A pinned server looked at the path given to `project_create` or
   `project_open` before refusing. Superseded on 2026-10-15: the server no
   longer refuses them at all.
7. A failed run of a desktop test left its `aster` window open. Both
   desktop tests now close their window when they fail.
8. When the provider, model, effort or roots changed, the next request
   started a new conversation without saying so. The panel now says so.
   Since 2026-10-15 the folders are a live roots file and no longer start a
   new conversation; the note reads "A new conversation started, because
   the agent's provider, model or effort changed. It remembers nothing of
   the last one."
9. `an_agent_edits_the_project_open_in_a_window`, committed with native MCP,
   asserted `dirty: false` straight after `project_save` to a window. A
   window saves on its own time and answers with `saving: true`, so the test
   failed in 2 of 16 runs. Both desktop tests now wait, as an agent should,
   until `project_status` shows `saving: false` and `last_save` for the
   file.
10. The panel's introduction named only the Movies, Desktop, Documents and
    Downloads folders; `agent_roots` also allows Music and Pictures. It now
    names all six, and says the agent sees frames, transcripts and sound
    levels rather than sound.
11. On Windows, the refusal of a Claude `.cmd` or `.bat` launcher said
    "Claude planning requires…" when it refused a conversation too. It now
    says "Claude needs the native Windows CLI…" for both. Not run on
    Windows.
12. Every window started its agent's server with the same `--tag
    aster-tab`, and took any `aster-tab-` request for its own agent's, whose
    steps it shows in its own panel. When one window's agent edited a
    project in a second window, the second window hid that edit's row (run
    17). Each window now tags its agent with its own instance,
    `aster-tab-<instance>`, and claims only those requests; a unit test
    checks another window's tag, the bare prefix and an outside client's.

### Limits

The in-app agent's limits are listed once, in
[native AI](native-ai.md#limitations): Stop ends the conversation, Aster's
instructions are visible in the provider's arguments, the agent cannot
listen, its undo after leaving and coming back reaches only its entries
since it came back, and sign-in, model access and usage limits are the
person's own.

### Not verified

- Any window on Linux, Windows or an Intel Mac. On Windows the planner
  refuses a Claude `.cmd` shim because its schema is in the arguments. A
  conversation puts no schema there but resolves the executable the same
  way, so the code refuses a `.cmd` or `.bat` launcher for it too; nothing
  was tried on Windows. On Windows the roots file is not limited to its
  owner, and the server tells its versions apart by modified time and
  length only.
- Codex versions other than 0.162.1, which needs `code_mode_host` for MCP
  tools, and Claude Code versions other than 2.1.300.
- The 12-hour conversation lifetime and the 600 s limit without a word from
  the provider; both are code limits that no run reached.
- The refusals and the concurrent outside client named under captures.
- Claude on a long request in a narrow window; Claude ran wide, Codex
  narrow.

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
| Audio/video correctness | Audible PCM integrity, sustained underrun-free output and measured A/V synchronization; mixer supports 16 simultaneous audible clips, and speed changes alter pitch unless a clip keeps it; exact VFR PTS/HDR need further implementation |
| Audio editing | Listening to every filter, preset, fade, the limiter, `atempo`, a duck and stems; a real microphone and output device on each platform with the device round trip measured; the native captures still missing from tickets 19, 20 and 21a of [audio editing](../.scratch/audio-editing/issues/); CPU and underruns under sixteen filtered voices; FFmpeg builds other than Homebrew 8.1.2; Linux and Windows runs of the sound lane, the recorder and Save audio as |
| Native performance | Repeated release startup/idle/seek/frame-time measurements, source-rate presentation and dropped-frame counts, large timelines/imports and process-tree/GPU memory |
| Codec deployment | Clean-machine FFmpeg availability, supported-format matrix, malformed-media bounds, licensing/notices and hardware-decoding strategy |
| Durable projects | Native interruption/full-disk/permission tests, long-history recovery, atomic replacement on all OSes and conflict policy for concurrent manual project writers |
| Desktop quality | Accessibility, complete keyboard/IME behavior, clipboard/file drops, DPI/display changes, minimized/idle behavior and shutdown cleanup |
| Distribution | OS/architecture packages, clean-machine launch, signing/notarization, notices and update/release policy |
| Provider integration | [Portable process-fault fixtures](../.scratch/native-ai/issues/01-portable-process-fault-fixtures.md) and native Windows job teardown; Windows real sign-in/model/failure/cancel behavior, and macOS on a release build (the [in-app agent](#in-app-agent) ran real Claude Code and Codex conversations, Stop included, on a macOS debug build only); in-app agent conversations on Linux and Windows, a Claude `.cmd` shim, other Claude Code and Codex versions, and a Stop that interrupts without ending the conversation; semantic audio analysis if advertised |

Export was outside the user's first core-feature phase. It is now implemented,
with Linux-only evidence in [export and rendering](#export-and-rendering);
export on macOS and Windows, other hardware encoders and long renders remain
gates. No installer, signed distribution or release pipeline is implied by a native
source build.
