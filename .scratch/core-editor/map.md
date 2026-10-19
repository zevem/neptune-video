# Core editor map

## Notes

- The starting repository contained agent skills and local-tracker guidance;
  it had no application source or Git checkout at initial inspection.
- User requires a Rust native editor, Linux/macOS/Windows targets and Neptune
  reference investigation.
- [Spec](spec.md) preserves the full initial feature set and acceptance criteria.

## Decisions-so-far

- [Native Rust ownership](../../docs/adr/0001-native-rust-editor-boundaries.md):
  independent domain core, desktop composition, media runtime and persistence.
- [Time and commands](../../docs/adr/0002-timeline-time-and-reversible-commands.md):
  integer timeline time, rational frames and one reversible command boundary.
- Canonical triage remains the five default status strings. Work progress is a
  separate field so claiming/resolving work does not redefine triage roles.
- `editor-core`, `editor-media`, `editor-store` and `editor-agent` are separate
  Rust crates, with an `eframe`/`egui`/`wgpu` native desktop and two binaries.
- Agent mutations require durable project identity, a transient open-session
  context and expected revision. Reopening a project invalidates old plans.
- Recovery archives preserve structurally shared history; checkpoint restore
  is reversible. [OS recovery leases](../../docs/adr/0003-recovery-ownership-uses-os-locks.md)
  suppress another live editor's recovery offers and extend through pending
  writes.
- Media uses native still/GIF decoding, external FFmpeg software video/audio,
  bounded cached frames and CPAL mixing/device-clock integration. Explicit
  Audio-track members own sound for same-asset linked pairs.
- [Linux native evidence](../../docs/verification.md#native-linux-evidence)
  includes XTest clipboard, checkpoints, duplication, sorting/folders and crash
  recovery ownership and nonmutating source preview. Final optimized checks
  passed 13 X11 smoke cases, shutdown/recovery-history retention and a fresh
  Weston URI drop; 169 workspace tests and source/build checks passed.
- [Performance](../../docs/performance.md) has core metadata and software decode
  results plus two paused CPU/RSS observations; repeated native GPU/input/audio/
  idle distributions are still gates.
- [Audible decoder admission](issues/05-audio-voice-budget.md) now preflights
  half-open sample intervals and reaps ended children before replacement.
  Owned PCM and transient child-handle tests prove the existing 16-voice policy
  independent of clip order, including sequential cuts and continuing voices.
  The final source checkpoint passed 333 tests; native target execution and
  acoustic/A/V performance remain separate.
- [Scheduled audio publication](issues/04-audio-clock-publication.md) rejects
  obsolete/expired PCM and generation activation; monotonic handoff prevents a
  startup playhead rewind. Red/green private tests, the 333-test checkpoint,
  scoped native Linux measurements and a separate physical zero-PCM clock run
  prove their named limits. Acoustic/DAC/display synchronization remains open.

- [Fatal output clock release](issues/06-audio-output-failure.md) latches stream
  failure, rejects raced publication and future requests, preserves recoverable
  errors and wakes owned cleanup. Device-free red/green tests, the 345-test R10
  build and its separate healthy clock and silent native editing runs pass.
  Native fatal-callback triggering and physical A/V remain open; the silent
  callback-stall correction is recorded below.
- [Silent callback liveness](issues/07-audio-callback-liveness.md) resolves the
  native virtual server-loss freeze. Nine red/green seam regressions and the
  354-test R12 checkpoint pass; fresh private loss tests show wall-clock video
  continuation, one notice and natural decoder teardown, including worker-only
  observation. Healthy physical zero-PCM clocks also pass their named scope.
- [Paused preview delivery](issues/08-paused-preview-delivery.md) now retains
  admitted identity and uses cancellable byte/count backpressure. Eleven seam
  regressions, 365 workspace tests, two public five-PNG diagnostics and three
  paused native five-layer launches pass their respective scopes. Exact pixels,
  source/binaries, captures and process cleanup were independently verified.
- [Poster retention](issues/09-thumbnail-retention.md) is bounded to 256
  posters/64 MiB logical texels. R14 tests and scoped Linux captures confirm
  fitted admission, paging/revisits and selected inspector retrieval after
  eviction; source-monitor poster retrieval is indirect evidence.
- [Overlap culling](issues/11-overlapping-timeline-culling.md) now draws and
  selects the long clip in both normal and narrow native viewports. R14's
  shared half-open query preserves stable timeline order; no logarithmic
  interval index is claimed.
- [R15 headless/release checkpoint](../../artifacts/production-readiness/final/round15-build-record.json)
  passes 521 tests and nine sequential architecture/media/fmt/lint/source-target/
  Linux release checks on 83 files (`dfbb53e9`, `neptune-video` `362ba180`). This includes
  consumer, budget, dispatch, compact transport and failure/admission corrections.
  R15's [thirteen native editing/input controls](../../artifacts/native/round15-editing-smoke/root-verification.json)
  pass with five viewed captures and eleven sampled process identities gone;
  cache/overlap acceptance remains R14. [Three startup captures](../../artifacts/native/round15-window-captures/root-visual-verification.json)
  separately verify normal/compact presentation. Complete input, wake/multi-owner/
  cache proof remains; macOS/Windows execution is deferred by the user.
- [Pinned native-test preparation](../../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json)
  has 64 unrun cases and 106 passing offline guards; ten generated projects
  have Linux startup observations. [Final independent review](../../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
  closed all four preparation findings; preparation establishes no target-platform acceptance.

## Fog

- Run native macOS/Windows and comprehensive Wayland input/chooser/file-drop
  acceptance; configured CI is not platform runtime evidence.
- Measure actual sound-device output, A/V drift, GPU/display latency, repeated
  idle/minimized behavior, random seeks and large-project release workloads.
- Decide FFmpeg deployment/licensing, supported format coverage, hardware/HDR/
  exact VFR handling and packaging/signing/update policy.
- Validate the implemented conflict protection on native macOS/Windows and
  slow, failing or cloud-synced storage.
- Continue [native acceptance](issues/01-native-core-acceptance.md),
  [performance](issues/02-performance-baseline.md) and
  [distribution](issues/03-codec-and-distribution.md) gates.
- Exercise a genuine fatal native output callback on an isolated owned virtual
  route; prove clock fallback and natural decoder cleanup separately from the
  resolved [private-state regression](issues/06-audio-output-failure.md).
- Verify native use of the implemented [bounded preview dispatch policy](issues/10-preview-dispatch-policy.md)
  with explicit poster demand, retained first-admission
  order and at most two interactive bypasses before global oldest. Preserve
  delivery/currentness and separate dispatch opportunities from latency,
  successful publication and multi-layer throughput. The verified optimized
  27-case video probe exposes a four-to-five lane sequential latency jump;
  structural manager/worker correction now passes at its seam. The 81-case
  optimized legacy/explicit/protected comparison passed all 81 cases with
  root proof; protected five-video sequential median was 67.395 ms versus
  185.490 ms legacy, still over the frame budget in every measured five-video
  sequential batch. The [completed independent audit](../../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
  agrees with the data; native proof remains and R14 is preserved.
- Keep [preview consumer identity](issues/12-preview-consumer-identity.md)
  stable when reused-asset clips leave or change order, while preserving
  same-asset cut continuity. Actual paint-seam RED reproduced six transition
  failures with seven passing controls; corrected identity and compact
  transport passed the earlier 152-test stage and are included in R15's
  198 desktop tests. Final native confirmation remains.
- Bound [viewer texture retention](issues/13-viewer-texture-retention.md)
  independently of posters, queues and decoders. Request-time reservations and
  a common automatic quality cap are implemented after thirteen actual budget
  assertion failures. Logical budget accounting is included in R15 after
  the earlier 177-test green; native/GPU/wake proof remains. Preserve full
  composition and own-held/cut pictures.
- Native-confirm [compact viewer transport](issues/14-compact-viewer-transport.md):
  R14 captures show Play overlapping Speed at 800×600; R15 startup captures now
  show separate controls, with both panels at 800×600 and library collapse at
  760×560. Normal transport input passes the R15 smoke subset; compact
  play/speed/source-monitor input and transitions remain unrun.
- Measure and native-confirm [cyclic decoder churn correction](issues/15-decoder-retention-churn.md) using a
  replaceable protected subset of at most three consumers and default LRU.
  Actual manager RED precedes correction; frontend adoption follows backend
  validation. Structural manager
  RED confirms ten starts for two five-video rounds versus four for four;
  correction retains three compatible owners and reaches seven starts.
  Current 112 media tests and package Clippy pass within R15. Seven frontend
  choice controls pass; the [actual canonical bridge](../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
  now observes the nonempty hint and `cancel`/`ask_again` reset (7/10 and 14/20
  decoder starts), independently reviewed. Six prepared bookkeeping tests were
  declined; this artifact probe adds no suite guard. Retire/clear/Closed, source/
  cut/reorder and playing remain unproved. The public worker comparison passed separately;
  native comparison remains.
- Native-confirm [failed preview requests and fair frontend admission](issues/16-preview-request-liveness.md).
  Read-only Opus review confirms the paused persistent-error cycle and flags
  playing owners beyond 32; root reproduced both with three assertion failures
  and eleven passing public-worker controls. Correction passes all 21 focused
  and 198 desktop tests, retained in the full 521-test R15 build. Source review
  finds no P1/P2 behavior defect; the later bridge closes only the two G1 sites above.
  Accepted queue fairness, native wake delivery and playback throughput remain
  distinct claims.
- [Playing-preview starvation](issues/17-playing-preview-starvation.md) is
  corrected by explicit playback admission with immediate seek fences. The
  updated Linux release passes 538 workspace tests, the actual five-video
  native feedback and thirteen editing checks. Four decoder pipes remain;
  multi-layer throughput and full native platform acceptance stay open.
- [Published-first playback read-ahead](../../artifacts/production-readiness/publish-first-read-ahead-round1/FINDINGS.md)
  is integrated in the current Linux build: 549 workspace tests, exact pixels
  in three matched before/after trials, all-five native advancement and thirteen
  native editing checks pass. The four-pipe/cache budgets remain. Measured
  low-resolution progression improves with a small protected-owner latency
  tradeoff; sustained high-resolution presentation and hardware/platform gates
  remain open.
- The user chose private/proprietary source and requested a pause after the
  verified Linux handoff. The current portable trial is reproducible, passes
  thirteen packaging guards and thirteen native editing checks through its
  extracted launcher. Larger-preview feedback passes exact pixels but does
  not meet source-rate playback. Resume from decoder capacity/transport,
  distribution notices and hardware/native platform acceptance; full production
  readiness remains unresolved.
