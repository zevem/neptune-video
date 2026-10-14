# Native macOS and Windows UI acceptance pack

**Status: prepared on Linux on 2026-10-12. Nothing in this pack has run on
macOS or Windows.** Every case is `not_run`. The pack establishes no native
acceptance, and a Linux cross-typecheck, a Wine run or a remote shell does not
substitute for running it.

**Current pin (twelfth version): the publish-first read-ahead checkpoint.**
This paragraph stands as written when the twelfth version was created. The
source pin is `89aa6908…1673` (87 files) and the build record is
[publish-first-read-ahead-round1/build-record.json](../artifacts/production-readiness/publish-first-read-ahead-round1/build-record.json)
(`b0b1c3e5…4f38`; Linux `aster` `61dc6abd…09c5`, `aster-agent` `00906bba…c76a`,
549 workspace tests, as that record states them). The twelfth version changed
the pin and its wording only: `FROZEN_SOURCE` and its comment in the runner,
`prepared` in `cases.json`, the frozen digest and note in
`report-template.json`, the status record and this guide. The 64 case
objects, the 106 guard tests, both drivers and `supplementary-fixtures.json`
(`bd985f5b…be34`) are the eleventh version's bytes. Five source files differ
from the eleventh version's pin (`changed_from_r16` in the record), all in
`crates/editor-media/src`: `decode.rs`, `process.rs`, `service.rs` and the new
`decode/prefetch_tests.rs` and `service/prefetch_tests.rs`. The author compared
the 87 files on disk with the record's per-file hashes (all equal) and did not
recompute the combined digest; the runner does when the root runs it. No file
under `src/` differs, so what the cases cite from the desktop source is in
files with the hashes they had at the reading described below, and the cases
were not re-read one by one. They assert nothing about decoders, pipes, caches
or read-ahead. The fixtures were not regenerated and have not been opened with
this build. The author ran nothing: the twelfth version's guards, `validate`,
plans, dry runs and refusals are the root's to run, and the root's Linux
native checks of this build were not complete, so no native result for it is
stated here. Not shown by the record: high-resolution or hardware decoding, a
native frame rate, sound on a device. Every macOS and Windows case is
`not_run`: 0 executed. The eleventh version's five changed files are kept in
[before-round12/](../artifacts/native/platform-test-preparation/before-round12/MANIFEST.json).

**Since the twelfth version was made (no pack byte changed).** For the build
this pin names the root recorded two Linux native checks under Xvfb, neither
of them a case of this pack:
[five clips playing](../artifacts/native/playing-multilayer-round2/publish-first-r17-attempt-01/playing.json)
(`observed`, `all_five_advancing`: 40 reads, none missed, 21, 21, 20, 17 and
15 distinct frames per tile, all five ending on frame 119; the root viewed
four captures) and
[13 editing and input checks](../artifacts/native/publish-first-editing-r17-attempt-02/root-verification.json)
(`passed`; the root was viewing its captures when this was written; its
first attempt stopped on the harness's own project argument before Aster
started and is no product result). Neither measures sound on a device, a GPU
or a frame rate, and the playing figures are no improvement claim over the
eleventh version's build. The twelfth version's guards, `validate`, plans,
dry runs and refusals had not been reported to the author. Every macOS and
Windows case is `not_run`: 0 executed.

**The eleventh version was pinned to the playback-admission checkpoint.** The
paragraph that follows stands as written when the eleventh version was
created, before the root ran anything on it; the paragraph after it gives the
state since. The
source pin is `b7fc3308…f87d` (85 files) and the build record is
[playback-admission-round1/build-record.json](../artifacts/production-readiness/playback-admission-round1/build-record.json)
(`40ea3dfc…5a13`; Linux `aster` `160efa02…1da5`, `aster-agent` `00906bba…c76a`,
538 workspace tests, as that record states them). The author hashed the 85
files on disk and they equal the record. The eleventh version changed the pin
and its wording only: `FROZEN_SOURCE` and its comment in the runner,
`prepared` in `cases.json`, the frozen digest and note in
`report-template.json`, the status record and this guide. The 64 case
objects, the 106 guard tests, both drivers and `supplementary-fixtures.json`
(`bd985f5b…be34`) are the tenth version's bytes. The fixtures were not
regenerated and have not been opened with the newly pinned build; their
opening by the round-fifteen Linux build, and every root check described
below, belong to the tenth version and are historical for this pin. Seven
source files differ from round fifteen (`changed_from_r15` in the record,
`src/app.rs` among them) and the cases were not re-read against them. The
eleventh version's guards, `validate`, plans, dry runs and refusals have not
been run by the author; the root runs them. No Linux native result exists
here for the newly pinned build, and no case has run on macOS or Windows.
The tenth version's five changed files are kept in
[before-round11/](../artifacts/native/platform-test-preparation/before-round11/MANIFEST.json).

**After the eleventh version was made (no pack byte changed).** The root ran its
guards and `validate` on Linux: 106 tests, OK, exit 0
([log](../artifacts/production-readiness/playback-admission-round1/platform-pack-guards.log))
and 64 cases, 62 acceptance and 2 diagnostic, 0 problems, exit 0
([log](../artifacts/production-readiness/playback-admission-round1/platform-pack-validate.log)).
Plans, dry runs and refusals have not been reported for this version. The
author then read the 13 supplementary case objects against the pinned source
`b7fc3308…f87d`: every key, label, constant, file and test name they cite is
there as cited, and no case needs a change (`round11_case_reconfirmation` in
`preparation-status.json` lists them by case). Of the seven files that differ
from round fifteen the cases cite three: `src/app.rs`, `src/textures.rs` and
`src/ui/media_consumer_tests.rs`. CMP-01, RUS-01, OWN-02 and LIV-01 still say
they were written from `dfbb53e9…f6c2`; that is where they were written from
and it is kept. This was a reading: it shows that the instructions name what
the source has, not that the build behaves as a case expects. For that version's
pinned Linux build the root recorded two native checks under Xvfb, neither of them a
case of this pack:
[five clips playing](../artifacts/native/playing-multilayer-round2/root-attempt-01/playing.json)
(`observed`, `all_five_advancing`: 40 reads at about 10 Hz, 21, 21, 21, 17 and
17 distinct frames per tile, all five ending on frame 119; the root viewed its
four captures) and
[13 editing and input checks](../artifacts/native/playback-admission-editing/root-verification.json)
(`passed`; its captures had not been viewed by the root when this was
written). Neither measures sound on a device, a GPU or a frame rate. The ten
supplementary fixtures have still not been opened with this build. Every
macOS and Windows case is `not_run`: 0 executed. The two files this paragraph
and its status entry changed are kept as they were in
[round11-validation/](../artifacts/native/platform-test-preparation/round11-validation/MANIFEST.json).

**The tenth version was pinned to round fifteen.** Its source pin was
`dfbb53e9…f6c2` (83 files), its build record is the
[round-fifteen build record](../artifacts/production-readiness/final/round15-build-record.json)
(`f8ef24ec…0e53`), and `supplementary-fixtures.json` (`bd985f5b…be34`) pins the
ten supplementary projects and their 482 media files. The pin file is a byte
copy of the one the root produced with the pinning script after it had opened
all ten projects with the round-fifteen Linux build
([opening record](../artifacts/native/platform-regressions-integration-round1/fixture-opener-round1/root-attempt-01/root-verification.json),
[pin record](../artifacts/native/platform-regressions-integration-round1/root-fixture-attempt-01/root-pin-candidate-verification.json)).
That opening is a startup check on Linux only: each project loaded, the
inspected project equalled the file, and one capture was viewed. It is not a
result of any case here and not macOS or Windows evidence. The round-fifteen
source is the one the 13 supplementary cases were written from, so the key
names, labels and constants they mention are those of the pinned source,
read and not run.

The pack has 64 cases, 63 per platform: 62 acceptance and 2 diagnostic. The
51 cases of the eighth version are unchanged objects in the same order. The
ninth version added the 13 supplementary cases and their runner support
without a pin; the root ran its 92 guard tests, `validate`, plans, dry runs
and wrong-host refusals on Linux
([record](../artifacts/native/platform-regressions-integration-round1/root-unpinned-attempt-01/root-verification-corrected.json)).
The tenth version corrects four findings of the
[first review](../artifacts/native/platform-regressions-integration-round1/review-round1/REVIEW.md), installs the pins, and has 106
guard tests: the 56 earliest are byte-identical, 6 of the ninth version's
were adjusted to the corrections, and 14 are new.

**Current state, recorded after the root's run:** the root ran the tenth
version's offline checks on Linux at 2026-10-12 23:21 UTC
([record](../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json),
SHA-256 `77da2242…30d3`, `errors: []`; raw output beside it). 106 guard tests
ran and passed; `validate` reported 64 cases and 0 problems with no pending
pin; the three plans listed 63 cases each; the three dry runs exited 0; the
three `run --execute` attempts were refused with exit 3 for the wrong host and
left no run folder; 196 immutable inputs, the 83 source files and the
round-fifteen binaries were unchanged. The bytes tested were runner
`92b85283…196f`, guards `c0043ba4…d0d3`, catalog `5bdc3b25…d5d8` and pins
`bd985f5b…be34`, which were the bytes in the pack until the eleventh version
moved the pin (guards and pins are still those bytes). A
[second review](../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
of that code found no P1 or P2 and the four earlier P2 findings closed. These
are fake-tool guards and preparation checks on Linux. No case has run on macOS
or Windows; all 64 remain `not_run`.

As written by the author when the tenth version was made, before that run:
"The tenth version's guards, `validate`, plans, dry runs and refusals have not
been run: the author parsed the Python, loaded the JSON and hashed files.
Until the root runs them their expected results are a design statement." This
guide and `preparation-status.json` were edited after the root's run to add
the paragraph above and nothing else, so their hashes differ from the ones
that record lists for them.

A Linux binary is never the subject of this pack: a macOS or Windows build has
its own hash, which the runner records, and inherits none of the Linux
evidence in [verification](verification.md). Cross-target Clippy passes on
Linux are typechecks; nothing was linked or run for those targets. Every
native macOS and Windows gate, including each case here, is `not_run`;
execution on those hosts is deferred.

Earlier pins, for the record: round nine `b29ccb69…e210c` until the fifth
version, round ten `08647cfc…6e707` in the fifth, round twelve
`2b43797d…7203f` in the sixth, round thirteen `5cc20680…5014a` in the seventh
and round fourteen `f74d6560…21106` in the eighth. No case, driver, gate or
test changed with any of those pins. The tenth pinned round fifteen
`dfbb53e9…f6c2` and the eleventh the playback-admission checkpoint
`b7fc3308…f87d` (85 files). The ninth version pinned nothing; it is
kept byte for byte in
[before-round10/](../artifacts/native/platform-test-preparation/before-round10/MANIFEST.json),
and the eighth in
[before-round9/](../artifacts/native/platform-test-preparation/before-round9/MANIFEST.json).

The pack covers UI acceptance through real input on a real desktop. Headless
correctness on the same hosts belongs to the backend owner and is not
duplicated here: use [test-workspace.py](../scripts/test-workspace.py), the
[media preflight](../scripts/check-media-tools.py) and the configured
[CI jobs](../.github/workflows/ci.yml).

## Files

All under [artifacts/native/platform-test-preparation](../artifacts/native/platform-test-preparation/):

| File | Role | State |
| --- | --- | --- |
| [run_platform_acceptance.py](../artifacts/native/platform-test-preparation/run_platform_acceptance.py) | Entry point: `plan`, `validate`, `run` | Tenth version: parsed only. The ninth version's `plan`, `validate`, dry run and wrong-host refusal ran on Linux; `run --execute` never got past its refusal |
| [test_run_platform_acceptance.py](../artifacts/native/platform-test-preparation/test_run_platform_acceptance.py) | 106 tests of the runner's refusals, of every way it withholds a pass, of which interpreter it starts for a Python child, and of pinned projects, a copy per case, identifier relations, setup, sizes, sequences and diagnostics | The 92 of the ninth version ran on Linux under the root. **The tenth version's 106 are unexecuted** until the root runs them. Temporary files, fake process handles and a fake driver; not a native run |
| [aster_input_macos.py](../artifacts/native/platform-test-preparation/aster_input_macos.py) | CoreGraphics events and `screencapture -l` for the owned window | Never executed |
| [aster_input_windows.py](../artifacts/native/platform-test-preparation/aster_input_windows.py) | `SendInput` and screen capture for the owned window | Never executed |
| [cases.json](../artifacts/native/platform-test-preparation/cases.json) | 64 cases, 63 per platform (62 acceptance, 2 diagnostic): trigger, expected outcome, state checks, required artifacts, failure scope | Authored from source; unexecuted |
| [supplementary-fixtures.json](../artifacts/native/platform-test-preparation/supplementary-fixtures.json) | Pins of the ten supplementary projects and their media | Byte copy of the root's pin file `bd985f5b…be34`. The root generated the fixtures, opened each project once on Linux and verified the hashes, identifiers and their relations; no macOS or Windows host has read them |
| [report-template.json](../artifacts/native/platform-test-preparation/report-template.json) | Report shape; every case defaults to `not_run` | Template |
| [preparation-status.json](../artifacts/native/platform-test-preparation/preparation-status.json) | What this preparation did and did not do, and what is still unknown | Record |
| [before-round2/](../artifacts/native/platform-test-preparation/before-round2/MANIFEST.json) | Byte copies and hashes of the first version of this pack and guide | Record; unused by the runner |
| [before-round3/](../artifacts/native/platform-test-preparation/before-round3/MANIFEST.json) | Byte copies and hashes of the second version, whose two child-launch tests failed under an inherited `__PYVENV_LAUNCHER__` | Record; unused by the runner |
| [round3-offline-checks.log](../artifacts/native/platform-test-preparation/round3-offline-checks.log) | The Linux checks behind the third version, with and without an injected launcher variable | Record |
| [before-round4/](../artifacts/native/platform-test-preparation/before-round4/MANIFEST.json) | Byte copies and hashes of the third version, which started the host application as Python when `sys.orig_argv[0]` named it too | Record; unused by the runner |
| [round4-offline-checks.log](../artifacts/native/platform-test-preparation/round4-offline-checks.log) | The Linux checks behind the fourth version, with every name the interpreter reports about itself pointing at a stand-in application | Record |
| [before-round5/](../artifacts/native/platform-test-preparation/before-round5/MANIFEST.json) | Byte copies and hashes of the fourth version, which pinned the round-nine source and so refused the current checkout | Record; unused by the runner |
| [round5-offline-checks.log](../artifacts/native/platform-test-preparation/round5-offline-checks.log) | The Linux checks behind the fifth version: the same 57 tests, `validate`, plans, dry runs and refusals after the pin moved to the round-ten source, and the recomputed digest | Record |
| [before-round6/](../artifacts/native/platform-test-preparation/before-round6/MANIFEST.json) | Byte copies and hashes of the fifth version, which pinned the round-ten source and so refuses the current checkout | Record; unused by the runner |
| [round6-offline-checks.log](../artifacts/native/platform-test-preparation/round6-offline-checks.log) | The Linux checks behind the sixth version: the same 57 tests, `validate`, plans, dry runs and refusals after the pin moved to the round-twelve source, the recomputed 73-file digest and the byte comparison with the fifth version | Record |
| [before-round7/](../artifacts/native/platform-test-preparation/before-round7/MANIFEST.json) | Byte copies and hashes of the sixth version, which pinned the round-twelve source and so refuses the current checkout | Record; unused by the runner |
| [round7-offline-checks.log](../artifacts/native/platform-test-preparation/round7-offline-checks.log) | The Linux checks behind the seventh version: the same 57 tests, `validate`, plans, dry runs and refusals after the pin moved to the round-thirteen source, the recomputed 74-file digest and the byte comparison with the sixth version | Record |
| [before-round8/](../artifacts/native/platform-test-preparation/before-round8/MANIFEST.json) | Byte copies and hashes of the seventh version, which pinned the round-thirteen source and so refuses the current checkout | Record; unused by the runner |
| [round8-offline-checks.log](../artifacts/native/platform-test-preparation/round8-offline-checks.log) | The Linux checks behind the eighth version: the same 57 tests, `validate`, plans, dry runs and refusals after the pin moved to the round-fourteen source, the recomputed 75-file digest and the byte comparison with the seventh version | Record |
| [before-round9/](../artifacts/native/platform-test-preparation/before-round9/MANIFEST.json) | Byte copies and hashes of the eighth version: 51 cases, 57 tests, the round-fourteen pin | Record; read by one guard test, unused by the runner |
| [before-round10/](../artifacts/native/platform-test-preparation/before-round10/MANIFEST.json) | Byte copies and hashes of the ninth version: 64 cases, 92 tests, no pin, the placeholder pin file | Record; unused by the runner and the guards |
| [before-round11/](../artifacts/native/platform-test-preparation/before-round11/MANIFEST.json) | Byte copies and hashes of the five files the eleventh version changed: the tenth version's runner, catalog, template, status record and this guide, with the round-fifteen pin | Record; unused by the runner and the guards |
| [before-round12/](../artifacts/native/platform-test-preparation/before-round12/MANIFEST.json) | Byte copies and hashes of the five files the twelfth version changed: the eleventh version's runner, catalog, template, status record and this guide, with the playback-admission pin | Record; unused by the runner and the guards |
| [implementation-round1/](../artifacts/native/platform-regressions-integration-round1/implementation-round1/RESULT.json) | How the ninth version was produced, its hashes, the commands the root runs and what stays open | Record, outside the pack |
| [implementation-round2/](../artifacts/native/platform-regressions-integration-round1/implementation-round2/RESULT.json) | The corrections and the pin of the tenth version: each finding and its fix, hashes before and after, the commands the root runs and what stays open | Record, outside the pack |

Python 3.9 or newer, standard library only. The runner builds, installs and
downloads nothing.

## Who provides what

| Need | macOS (ARM64 and Intel, each on its own hardware) | Windows x86-64 MSVC | Provided by |
| --- | --- | --- | --- |
| Session | Logged-in GUI session at the console; not SSH or launchd | Logged-in, unlocked interactive session; not session 0, a service, or a task that runs logged off | Host owner |
| Checkout | The frozen checkout copied without newline or content conversion; the runner recomputes the source digest and reports a mismatch | Same (disable `core.autocrlf` if Git is used) | Host owner |
| Build | `cargo build --release --locked --bins` with Rust 1.97.1 and Xcode command-line tools | Same, MSVC toolchain with the Visual Studio C++ tools and Windows SDK | Build owner, before the run, from the frozen checkout. The runner hashes the result and cannot tell what it was built from |
| Build record | Optional JSON with `source_sha256` and `binaries.<name>.sha256`, as [record-build.py](../artifacts/production-readiness/final/record-build.py) writes | Same | Build owner. Passed as `--build-record`; compared, never treated as proof |
| Media tools | `ffmpeg` and `ffprobe` from `scripts/fetch-media-tools.py`, `PATH` or `ASTER_FFMPEG` / `ASTER_FFPROBE` | Same | Host owner |
| Input permission | Accessibility for the terminal that runs Python | Python at the same integrity level as Aster | **The person**, in System Settings. The pack only asks whether it is granted |
| Capture permission | Screen Recording for that terminal | Unlocked desktop | **The person** |
| Folder prompts | Files and Folders prompts for Aster, if macOS shows them | None | **The person** answers; note each under MAC-01 |
| Devices | Speakers or headphones; optionally a second display, a CJK input method, VoiceOver | Same with Narrator | Host owner |
| Provider requests | Installed, already signed-in `claude` / `codex` | Same; Claude needs the native `claude.exe` | **Host owner authorizes per run** with `--authorize-provider-requests` |

An agent on that host can prepare everything in the "Host owner" and "Build
owner" rows it is authorized for. It must not grant, script or edit macOS
privacy (TCC) permissions, answer consent prompts, sign in to a provider, or
read provider credential or account state. Without Accessibility the nine
OS-input automated cases stay `not_run`. Without Screen Recording every case
whose artifacts include a capture stays `not_run`, which is every case except
ENV-01: a case cannot pass without its captures.

On Windows over Remote Desktop the runner records `remote_session: true`.
GPU, DPI and audio-device results from a remote session are not console
results and must be labelled as such in ENV-03, DPI-01 and PLY-02.

## Run it

Check the plan anywhere (reads files only):

```sh
python3 artifacts/native/platform-test-preparation/run_platform_acceptance.py validate
python3 artifacts/native/platform-test-preparation/run_platform_acceptance.py plan --target aarch64-apple-darwin
```

Check the runner's own refusals anywhere. This starts no Aster process, sends
no input and is not a native run:

```sh
python3 -B artifacts/native/platform-test-preparation/test_run_platform_acceptance.py
```

Starting either script from a terminal opened inside another application needs
no preparation. Some applications (T3 Code's AppImage on the preparation host)
leave `__PYVENV_LAUNCHER__` in the environment of the shells they start;
CPython then reports that application as `sys.executable` and
`sys._base_executable`, and on the preparation host `sys.orig_argv[0]` named
it as well. No name the interpreter reports about itself is treated as
independent of the others.

- **Linux.** The running image (`/proc/self/exe`) decides when it is an
  executable file named as a Python interpreter. `sys.executable` is started
  only when it is that image, which is the case for a directly started Python
  and for a virtual environment made with links or with copies. Otherwise the
  image is started, whatever the other names say.
- **macOS, Windows, or an image that is a loader or an embedding
  application.** There is no such image to ask. `sys.executable` is started
  when it is what the process was started as or lies inside the running
  installation; failing that, what the process was started as, then a
  different base interpreter, then the installation's own interpreter.
- **Everywhere.** A file is started as Python only when its name, after links
  are followed, is one CPython installs (`python`, `python3.14`, `pythonw.exe`,
  `Python` and the like). When nothing qualifies no Python child is started
  and the report says why. This check is by name; it does not prove the file
  is an interpreter.

The variable is removed for those children only; your shell is not changed.
The choice is recorded in the report under `environment.host.python_child`.
The Linux path was exercised with Python 3.14, real temporary virtual
environments and a temporary stand-in application. The macOS and Windows path
was exercised only with made-up facts on Linux; on those hosts an application
that every reported name points at and that is itself named like Python would
still be started.

On the target host, from the repository root, with a new empty parent
directory for the run. Omitting `--execute` prints the plan and exits.

```sh
# macOS Apple Silicon (use x86_64-apple-darwin on an Intel Mac)
python3 artifacts/native/platform-test-preparation/run_platform_acceptance.py run \
  --target aarch64-apple-darwin \
  --binary target/release/aster --agent-binary target/release/aster-agent \
  --run-root "$HOME/aster-acceptance" --operator "Name" --execute
```

```powershell
# Windows x86-64 MSVC
py -3 artifacts\native\platform-test-preparation\run_platform_acceptance.py run `
  --target x86_64-pc-windows-msvc `
  --binary target\release\aster.exe --agent-binary target\release\aster-agent.exe `
  --run-root "$env:USERPROFILE\aster-acceptance" --operator "Name" --execute
```

| Option | Effect |
| --- | --- |
| `--case ID` (repeatable) | Run only these cases |
| `--no-prompt` | Automated cases only; manual cases stay `not_run` |
| `--ascii-root` | Control run without the Unicode/space root, to separate a path fault from everything else |
| `--authorize-provider-requests claude,codex` | Allow real requests for AI-01 to AI-05; omitted, they stay `not_run` |
| `--build-record FILE` | Compare the builder's own record with the source digest and binary hashes, and report the result |
| `--diagnostic-source-mismatch` | Run although the checkout is not the frozen source. Acceptance is withheld |

The runner refuses to execute, in this order and before it creates or
launches anything (exit 3):

1. The host OS is not the target's. This is decided first, before any file is
   read or OS facility is called; on Linux no driver is imported.
2. `--operator` is missing.
3. A binary is missing, or its header is not a launchable executable for the
   target CPU. A PE file needs the `PE` signature where the DOS header
   points, header ranges inside the file, an optional-header magic that fits
   the machine, and the executable-not-DLL characteristics. A universal
   Mach-O is believed only after each slice its table names was read and
   found to hold an executable of the claimed CPU.
4. The hardware is not native for the target. Windows: the native machine
   from `IsWow64Process2` must be x64, so x64 emulation on ARM64 Windows is
   refused; `PROCESSOR_ARCHITECTURE` is not consulted because emulated
   processes see `AMD64`. macOS: the Intel target is refused when
   `hw.optional.arm64` is 1; the ARM64 target needs it to be 1; a Python
   translated by Rosetta is refused for both. An unreadable answer refuses.
5. The pack's source pin is `pending_next_checkpoint`. The tenth version's is set.
6. The checkout's source digest is not the frozen digest, or a source file is
   missing.
7. A pinned fixture has a different hash.
8. A selected case needs a supplementary project whose pin is missing or
   pending, whose project file or media has another hash, whose project
   identifier is not the pinned one, whose pin names a file outside the
   fixture directory, or whose media that is pinned as absent exists.
9. A pinned project a selected case opens does not have the identifier its
   pin relates it to: one pinned to share another project's identifier does
   not, the project it names has no settled pin, or two projects that are not
   pinned as sharing have the same identifier. This is checked from the pins
   before launch and again before each owned copy, while step 8 ties each
   file's own identifier to its pin.

`--diagnostic-source-mismatch` passes steps 5 and 6 and nothing else. Such a
run can never establish acceptance.

| Exit | Meaning |
| --- | --- |
| 0 | Every acceptance case listed for the platform passed and integrity held. Diagnostic cases carry no verdict |
| 1 | Not established: a case failed, was blocked or not run, or an integrity condition below did not hold |
| 3 | Refused before launch; no run folder, no report |
| 4 | Only `--case` selections ran and they passed, or only diagnostics were selected. This is not platform acceptance |
| 5 | The runner itself faulted; the report holds what was collected |

## What a run does

1. Writes a first `report.json` (rewritten after every case), then records
   identity: source digest (same scope as
   [record-build.py](../artifacts/production-readiness/final/record-build.py)),
   binary sizes, hashes and parsed headers, the OS's architecture answers,
   build provenance, and the hash of every file in the pack.
2. Records the environment: OS, SDK and tool versions, signature state,
   display and audio inventory, session and permission state, and the output
   of the media preflight.
3. Creates one new run folder. All owned paths sit under
   `Aster tëst 日本語 %41 run/`, so launch, settings, session descriptor, project,
   recovery and media all exercise spaces, accents, CJK and a literal percent.
   The fixtures are copies of the recorded Linux
   [project](../artifacts/native/fixture.aster) and
   [video](../artifacts/native/fixtures/shot.mp4) with the asset path rewritten,
   plus a generated WAV and PNG and the recorded GIF.
4. Launches `aster PROJECT --settings … --agent-session … --size 1440x900`
   as a child and keeps its handle. A case with `setup` gets a fresh window on
   its own project at its own size, launched before the case's first
   snapshot, and so does the first case without `setup` that follows it.
5. Runs the automated cases in a fixed order on the untouched fixture, then
   relaunches and walks the operator through the manual cases.
6. Ends only its own handles, rehashes source, binaries, pack and media, and
   writes the final `report.json`. The run folder is evidence and stays.

Each step of the ending is contained. A stop that fails, a file that
vanished or a fault in the runner is recorded under `run_errors` or
`cleanup.errors`, the remaining handles are still stopped, and the report is
still written: into the run folder, else beside it, else to standard error.

During a manual case the operator performs the trigger with real devices and
uses these runner commands:

| Key | Effect |
| --- | --- |
| `s` | Record the mid-case state the case's checks refer to |
| `c` / `i` | Capture the owned window / print revision, dirty, undo, redo, project and context identifiers and the window's child processes |
| `k` | Kill the owned window (crash simulation) |
| `r` | Relaunch on the case's own project at the case's own size; `r nomedia`, `r narrow` (760×560), `r noproject`, `r WxH`. An unknown word or a size outside 200 to 16000 is refused before anything is stopped |
| `q NAME SECONDS` | Capture the owned window back to back for at most `SECONDS` (bound: 120 s and 2,000 frames). The first frame is taken before the runner says it is capturing; then the operator performs the trigger |
| `m LABEL` | Record what `ps` or `Get-Process` prints for the owned PID, and the names of its direct children |
| `2` | Open a second owned window on the same project |
| `x` | An independent writer (`aster-agent --project`) renames and saves the project |
| `d` | Done: record the after state and capture, then give `p`/`f`/`b`/`n` and an observation |

The operator's verdict is a claim; the stored status is what the evidence
supports. Nothing is promoted, and `f`, `b` and `n` are stored as given.

| Operator says `p` and… | Stored as | `failure_kind` |
| --- | --- | --- |
| every state check holds and every required snapshot and capture is on disk | `passed` | |
| a recorded state check contradicts it | `failed` | `state_check` |
| the after or mid snapshot a check refers to is missing | `blocked` | `evidence_missing` |
| a required before or after capture is unavailable | `blocked` | `evidence_missing` |
| a size the case requires has no usable capture from a window launched at it | `blocked` | `evidence_missing` |
| a required sequence is absent, has an unusable first frame or fewer than two usable frames, shows no change where one is expected, or has no usable frame the operator placed inside the transition; for a sequence that expects no change, the operator named neither frames nor `none` | `blocked` | `evidence_missing` |
| a project file's hash differs from the one before the case | `failed` | `state_check` |
| a project file's hash could not be taken, before or after | `blocked` | `evidence_missing`: never read as unchanged |

`blocked` for missing evidence is a harness or prerequisite fault, not a
product verdict. The runner names what is missing before it asks for the
verdict. If the owned window is not running at `d` (after `k`, or a crash),
relaunch with `r` first: without a window there is no after state or capture.
An observation is required for `p`, `f` and `b`.

## Supplementary projects, sizes, sequences and diagnostics

**Pinned projects.** Twelve of the 13 supplementary cases open projects other
than the pack's own; CMP-01 opens the pack's.
They are generated once by the root, never on a native host, and pinned in
`supplementary-fixtures.json` with the hash of each project and media file and
each project's identifier. The runner reads the pinned files and never writes
them. For each case it makes a fresh owned copy from the pinned bytes, in that
case's own folder under the run root, with every asset path rewritten to owned
media, and opens only that copy. Four projects are used by more than one case
and two of those cases edit; no case opens a copy another case used, so an
edit, a mistaken save or a recovery file one case leaves is not what the next
one starts from. Inside its case a copy is made once: `r` reopens it as the
case left it. Nothing an earlier case owns is deleted. Owned media is shared
by the copies and rehashed at the end like the pack's own, and so is every
pinned project file. A file a project names on purpose although it does not
exist (`absent`) must be absent in the pinned directory and in the owned
media when a copy is made and again at the end of the run; one that exists
then withholds acceptance.

**Identifiers.** `reuse-alt-source` is pinned to carry the project identifier
of `reuse`, and every other project has its own. The runner refuses to launch,
and to copy, when that does not hold. `x`, the external writer, saves the
pack's own project and is refused in a window that has another project open.

**State checks.** `project_id_unchanged` compares the inspected project
identifier with the one before the case. `project_files_unchanged` compares
the hash of every owned project file. The names beside those files are
recorded and not judged, so a recovery file the product writes is seen without
being called a fault.

**Sizes.** A capture record carries the size its window was launched at and
that window's PID. A later drag of the window is not known to the runner; the
picture's own pixel size is the evidence then.

**Sequences.** `q` takes frames back to back, in the foreground, with no
thread, no input and no chosen rate; each frame carries the time it was
started and the time it returned, and the record states the longest gap. A
sequence is a set of samples. A fault shorter than a gap is not seen, clean
frames are not evidence that nothing went wrong, and a one-paint routing fault
is excluded only by the headless paint-seam tests the root runs. After `d` the
operator says which frames show the transition. The runner checks the form of
that answer and that the named frames differ from the frame before the
trigger; the answer can withhold a pass and is never taken as proof. One
sequence, CUT-01 `split-paused`, expects no change in the picture. For it the
operator answers `none` when no frame shows a change, or names the frames
that do and judges them in the verdict; the first frame, two usable frames
and forward times are still required, and `none` places nothing in any
sequence that expects a change.

**Diagnostics.** RES-01 and LIV-02 have `kind: diagnostic`. They record what
was seen and judge nothing. The operator is offered `o`, `b` and `n`; `p` and
`f` are not accepted. With two process readings and its snapshots and captures
on disk a diagnostic is stored as `observed`, which is not one of the four
statuses, is never counted under `counts.passed` and establishes nothing. A
contradicted state check stores it as `failed`. A report in which a diagnostic
is `passed` or `failed`, is `observed` without its evidence, or in which a
case carries another kind than the catalog gives it, withholds acceptance. A
diagnostic left `not_run` is named under `summary.diagnostics` and does not
block the acceptance cases. When a diagnostic is among the listed cases the
outcome is `all_acceptance_cases_passed`, or `selected_acceptance_cases_passed`
for a subset, and the acceptance sentence counts the diagnostics observed and
not observed; `all_cases_passed` is said only of a report without one. A
selection of diagnostics alone is `no_acceptance_case_selected`, exit 4. A process reading is a figure of one OS tool for
one process: not a GPU, driver, upload or texture figure, and not comparable
between tools or hosts.

| Case | Kind | Project | Shows | Does not show |
| --- | --- | --- | --- | --- |
| OVL-01 | acceptance | `overlap` | A long clip stays drawn and selectable among later short clips, at 1440×900 and 760×560 | The exact trim edge; any cost of the query |
| PST-01 | acceptance | `posters` | Each of 480 stills shows its own poster again when its clip is in view | That at most 256 posters are kept: a private count, shown only by source tests |
| PST-02 | acceptance | `library` | The selected asset's inspector poster returns without input | That the poster was given up in between |
| CMP-01 | acceptance | pack | Play and speed are separate controls at 800×600 with both panels drawn, at 760×560 and at 1440×900. The source-monitor step applies at 760×560 only when the library is drawn there; a collapsed library is recorded, not failed | Timing, sound, decoding; which panels 760×560 draws on a native host (in the pinned source, read and not run, the library needs 760 points of content and the window leaves 748) |
| RUS-01, RUS-02 | acceptance | `reuse`, `reuse-equal` | In the captured frames, clips of one asset keep their own pictures | A one-paint fault; track reordering, for which the window has no control |
| CUT-01 | acceptance | `cut` | A cut in one asset plays through without a blank in the captured frames | Decoder or pipe reuse |
| OWN-01, OWN-02 | acceptance | `reuse`, `cut`, `reuse-alt-source` | The settled picture is the one for the latest seek, size and project file | Private request, generation and cache state; a file changed behind an asset inside one open project, for which there is no interface |
| VWR-01 | acceptance | `layers-stills-40`, `layers-video-6` | Every layer of a paused composition is drawn, at any quality | Any memory figure; the same while playing |
| LIV-01 | acceptance | `missing` | A notice about a missing file leaves and does not return while paused | Request, error and wake counts |
| RES-01, LIV-02 | **diagnostic** | `reuse`, `missing`, `layers-stills-40` | Readings and frames, recorded | Anything: no threshold exists |

## For the root: validation of the tenth version

Nothing below was done by the author. Run from the repository root, on Linux.
For the eleventh version steps 1 to 3 are unchanged; the root has since run
steps 1 and 2 on it with the expected results (logs in the current-pin
paragraphs above) and step 3 has not been reported; in step 4
the runner, catalog and template hashes are those of
`pack_sha256_after_round11` in `preparation-status.json`.
For the twelfth version steps 1 to 3 are again unchanged and none had been
run when it was made; in step 4 the hashes are those of
`pack_sha256_after_round12`.
The root ran steps 1 to 3 on the tenth version with the expected results
([record](../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json));
the steps stay here for a repeat after any change.

1. Guards: `PYTHONDONTWRITEBYTECODE=1 python3 -B artifacts/native/platform-test-preparation/test_run_platform_acceptance.py`.
   Expected: 106 tests. A failure among the 56 earliest means a runner change
   was not minimal; among the 6 adjusted or the 14 new ones it is a fault in
   the tenth version or in its guard.
2. `validate`: expected
   `{"cases": 64, "by_mode": {"automated": 13, "manual": 51}, "by_kind": {"acceptance": 62, "diagnostic": 2}, "problems": 0}`,
   exit 0, with no `PENDING:` and no `PROBLEM:` line.
3. Plans for the three targets: 63 lines each. Dry runs: exit 0, and
   `Pins: set`. `run --execute` on Linux: exit 3 for the wrong host, and no
   run folder.
4. Hashes: `sha256sum -c` over
   [implementation-round2/immutable-inputs-before.sha256](../artifacts/native/platform-regressions-integration-round1/implementation-round2/immutable-inputs-before.sha256)
   from `artifacts/native`, and the pack hashes in
   [implementation-round2/MANIFEST.json](../artifacts/native/platform-regressions-integration-round1/implementation-round2/MANIFEST.json).
   `cmp` of `supplementary-fixtures.json` with the root's pin file.
5. A Linux run of the new cases would check the harness and the fixtures. It
   is not native acceptance, and neither is opening the fixtures on Linux.

A later checkpoint moves the pins the same way: archive this version first,
replace `supplementary-fixtures.json` with the output of
`prepare_integration_fixtures.py pins` only if a fixture changed, set
`FROZEN_SOURCE` in the runner, `prepared` in `cases.json` and
`identity.source_sha256_frozen` in `report-template.json`, reconfirm against
the new source every key, label and constant the 13 cases name, and rerun
steps 1 to 4.

## What counts as UI input

- **Counts:** the operator's own keyboard, pointer, trackpad, file manager and
  dialogs; and OS events the driver posts to the owned window (nine automated
  cases: PRJ-02, HIS-01, TL-01, CLP-01 to CLP-03, ADV-01, PLY-01, TRN-01).
- **Does not count:** the live agent protocol. The runner uses it to read
  state before and after each case. ENV-01, ENV-02, IMP-04 and PRT-01 use only
  launch arguments or the protocol and say so in their `input` field.
- The drivers' typed-text helper bypasses input methods. IME-01 is manual.

Pointer targets for the automated cases are content points observed on
Linux/X11 for this fixture at 1440×900 in
[native-smoke.py](../scripts/native-smoke.py). They are uncalibrated on macOS
and Windows. For automated cases:

| What happened | Status | `failure_kind` |
| --- | --- | --- |
| The driver refused to act | `blocked` | `driver` |
| A snapshot or required capture could not be recorded, or inspect returned another shape | `blocked` | `evidence_missing` |
| An expectation did not hold (also a missed pointer target) | `failed` | `product_or_driver`: a person reads the capture before calling it a product defect |
| The owned window exited without the runner ending it | `failed` | `owned_process_exited` |
| Any other runner fault | `blocked` | `harness_or_product` |

## Coverage

| Area | Cases |
| --- | --- |
| Launch, identity, GPU and compositor picture | ENV-01, ENV-02, ENV-03 |
| Projects: create, open, save, Save As, duplicate, settings, dirty close | PRJ-01 to PRJ-05 |
| Exact reversal, dirty state, conflict, recovery, checkpoints | PRJ-06, REC-01 to REC-03, HIS-01, HIS-02 |
| Import: chooser, folder, drop, arguments, missing tools | IMP-01 to IMP-05 |
| Library: thumbnails, search, sort, folders, source preview | LIB-01 to LIB-03 |
| Timeline: frame step, markers, zoom, snapping, many tracks | TL-01 to TL-03 |
| Clips: move, trim, split, copy/cut/paste, duplicate, delete | CLP-01 to CLP-05, HIS-01, TL-01 |
| Ripple, link, group, lock, hide, mute | ADV-01, ADV-02 |
| Preview, scrub, speed, audio device and display lifecycle | PLY-01 to PLY-03 |
| Regressions since round fourteen: overlap culling, poster retrieval, compact transport, reused assets, cuts, replacement, many layers, a missing file | OVL-01, PST-01, PST-02, CMP-01, RUS-01, RUS-02, CUT-01, OWN-01, OWN-02, VWR-01, LIV-01 |
| Diagnostics, never a pass | RES-01, LIV-02 |
| Transform, reflection, crop, canvas | TRN-01, TRN-02, CNV-01 |
| Unicode and space paths | Every case through the run root; PRJ-01, IMP-01, WIN-01 by name |
| Keyboard, IME, DPI, clipboard, dialogs, drops, accessibility | KEY-01, IME-01, DPI-01, CLP-04, PRJ-01, PRJ-03, IMP-01 to IMP-03, ACC-01 |
| Platform specifics | WIN-01 (console windows, long paths), MAC-01 (signature, prompts) |
| Protocol guards and descriptor privacy (not UI) | PRT-01 |
| Native AI review, exact apply, cancel, stale, discard, teardown, failure | AI-01 to AI-06 |

## Evidence in the report

Each report holds: source, binary, pack and media hashes before and after,
with every changed or vanished file named; OS, SDK, toolchain, signature,
adapter and audio inventory; session and permission state; every owned
process with its command, PID, how it ended, its children before the stop and
any still listed afterwards; and per case the status, failure kind, failure
scope, input kind, operator observation, state checks, missing evidence,
state files and captures with their hashes. A surviving child process is
reported and never signalled by the runner.

`summary.acceptance` reads `every listed case passed on this host only, for
the binaries hashed under identity; build provenance: …` only when all of
these hold, and `summary.withheld_because` lists each one that does not:

- every case listed for the platform was selected, every acceptance case
  among them passed, the run was not interrupted and the runner recorded no
  error;
- the checkout's source digest equals the frozen digest;
- source, both binaries and the pack have the same hashes after the run as
  before, and every owned media file is unchanged;
- every passed case still has its required captures on disk, a usable
  capture at each size it requires, and each required sequence with frames
  that still have their recorded hashes;
- no diagnostic is stored as `passed` or `failed`, none is `observed` without
  its evidence, and every case has the kind the catalog gives it;
- every pinned supplementary project file has its pinned hash after the run,
  and no file pinned as absent on purpose exists then;
- every owned handle exited, no child of one is still listed, and cleanup
  recorded no error;
- a supplied build record is readable and names this source digest and these
  binary hashes.

Build provenance is never established by the runner. Without a record it is
`unknown`. With a consistent record it is `declared_by_record_unattested`:
the builder says so, and the hashes agree with what the builder wrote. A
Linux build record names Linux binaries and is `record_inconsistent` for any
native binary. Cases keep `not_run` with a reason; nothing is promoted.
A report written before the run finished says `run_state: in progress` and
`not established`, whatever its cases say.

## Not prepared, and not established by a passing run

- Audio latency, A/V drift, underruns and presentation timing. PLY-02 is a
  listening check, not a measurement.
- Which wgpu backend the binary chose. It is not reported by the binary;
  ENV-03 records loaded modules and the adapter inventory.
- Signed-out, rate-limited and model-access provider failures. They need the
  host owner to present such an account; the pack does not alter sign-in state.
- The owned no-op provider fixture used on Linux has no native counterpart
  here, so the "No changes" card is not covered.
- Signing, notarization, installers, clean-machine launch and FFmpeg deployment.
- VFR, HDR, hardware decode, proxies, colour, long sessions, slow or failing
  storage, network or cloud-synced volumes.
- OS versions, hardware, scales, layouts and input methods other than those a
  report names.
- The drivers themselves, and the two architecture probes. Their first
  native run tests them too; only the runner's decisions on their answers
  were tested here, with fake answers.
- What a capture shows. A PNG on disk with a non-zero size counts as present;
  a blank or wrong-window picture is for the person reading it to catch.
- What happens between two frames of a sequence, and how fast either driver's
  capture returns. Neither driver has ever captured anything.
- Poster, texel, decoder, request, wake and cache counts, and GPU, driver and
  upload memory. No case reads them; where they are tested, it is by source
  tests the root runs.
- Whether `ps` and `Get-Process` answer for the owned PID in every session.
- Track reordering and a file changed behind an asset inside one open
  project: the window has no control for either in the source that was read.
- The supplementary fixtures: unexecuted generator code, never opened.
- That a binary was built from the frozen source.
- ARM64EC and ARM64X Windows binaries, whose header names x64.
- Deeper process descendants: children are listed one level below the owned
  PID, and a listed PID may have been reused.
