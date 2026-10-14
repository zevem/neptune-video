# Automatic captions (speech lane)

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 09

## Scope

whisper.cpp through whisper-rs; model catalog with sizes and SHA-256; verified curl download with progress and cancel; offline 16 kHz mix of a timeline range; DTW word times; cues from words; one batch adds the caption track.

## Comments

- Choice of engine:
  - whisper.cpp 1.8.3 is built into the binary through whisper-rs 0.16, from source and with no default features. Nothing is installed at run time.
  - Word times come from whisper.cpp's DTW token alignment. The alignment heads are picked from the model's header, falling back to the top text layers for a file the catalog does not know.
  - The Silero detector (`ggml-silero-v5.1.2.bin`, 0.9 MB, downloaded with the first model) keeps silence and music from Whisper.
  - WhisperX was not used. Its word times come from a second wav2vec2 alignment model run in Python and PyTorch, which cannot ship inside a native editor without a large runtime.
- Models:
  - The panel offers Tiny (32 MB), Base (60 MB), Small (191 MB), Medium (540 MB) and Large v3 Turbo (575 MB), all multilingual and quantized. "Model file…" takes any whisper.cpp `.bin`.
  - A catalog model is downloaded once on request with the system `curl` (`ASTER_CURL` overrides it). It is resumed from `<file>.part` and kept only when its size and SHA-256 match. A damaged file is removed and the notice says so.
  - Models live in `models/` next to the settings file when `--settings` is given, otherwise in the platform's local data directory.
- Build: whisper.cpp needs CMake, a C and C++ compiler and libclang at build time. `.cargo/config.toml` builds it for a CPU family, not for the build machine: AVX2, FMA, F16C and BMI2 on x86-64, NEON on 64-bit ARM. An x86-64 processor without AVX2 cannot run the binary. GPU back-ends are not enabled.
- Measurements on this host (4 vCPU Xeon 2.8 GHz VM, dev profile with dependencies at opt-level 2, Tiny q5_1, 7 s of speech in an 11.55 s timeline):

  | Stage | Time |
  | --- | --- |
  | Model load | 140 ms |
  | Offline 16 kHz mix | about 300 ms |
  | Detector | about 180 ms |
  | Whisper, language detected | 3.7 s |
  | Whisper, language given, greedy | 2.0 s |
  | Whisper, language given, beam 5 | 2.3 s |

  - Beam search (5) was kept. It took about 15% longer than greedy and wrote "nobody" where greedy wrote "no body".
  - Cancel stops a download or the timeline's decoders at once. whisper.cpp checks its abort callback only when an encoder or decoder pass ends, and its language detection cannot be aborted, so a cancel during recognition took 3.34 s with Tiny here. It will take longer with larger models.
  - The real-model test bounds the cancel at 30 s.
- Native verification: the debug build ran under Xvfb on this Linux container, with task-owned settings, models and a 9:16 project holding `talk.wav` (8.5 s of speech placed at 1 s).
  - 1440×900 window:
    1. The empty Captions tab offers "Generate from speech".
    2. The panel showed Tiny already downloaded. "Generate captions" showed "Writing what is said…" above the tab.
    3. Six captions arrived in about 5.5 s from the click. Undo was offered, and the viewer drew the cue at 0:02.
  - 760×700 window, with the library open at 220×415:
    1. The panel scrolled to its button.
    2. Stop showed "Stopping…", then "Captions were not made." The project stayed Saved, with nothing to undo.
    3. A second run made the track, and one Undo removed all of it.
    4. Base was then chosen. "Download and generate" fetched 59,707,625 bytes from Hugging Face through this container's proxy, with the percentage moving, and the file's SHA-256 matched the catalog.
    5. Because a redo was pending, the five captions waited in "Captions are ready … Redo first to keep both". "Add captions" added them and said "Added 5 captions in English."
  - With the language detected, Tiny wrote "Three things no body", and Base wrote "Free things nobody tells" for "Three things nobody tells".
  - Sound reports no device on this host.
- Two faults were found during native verification and fixed, each with a test:
  - The speech panel's content sat in a child Ui whose size never reached its scroll area. On a short library, "Generate captions" was out of reach. The new bench test (240×360) fails without the fix.
  - The shared switch blended the translucent `pressed` colour as if it were opaque. An off switch was near-invisible in dark mode on a panel and solid black in light mode. Its track is now drawn translucent, with the accent over it as it slides on.
- Tests:
  - `crates/editor-media/src/speech/tests.rs`, 23 tests. They cover the catalog, header shapes, languages, condensed timing and quiet cuts, word times, the one-job lane with cancel and drop, and downloads through a fake curl (checked, resumed, damaged, curl's error, curl missing, cancel, only what is missing). There is also an offline-mix level test.
  - One test runs only when `ASTER_WHISPER_MODEL` and `ASTER_SPEECH_SAMPLE` are set. It passed with Tiny, and checks the words, their timeline times and a cancel during recognition.
  - Desktop: 666 passed. Of these, 5 are speech-panel benches, 6 are unit tests in `src/speech.rs`, and 1 app test covers when captions wait. `cargo clippy --bin aster --tests` and `cargo fmt --check` are clean.
  - editor-media: 247 passed, 1 ignored, and 1 failed. The failing test, `decode::tests::a_paced_walk_backwards_at_preview_size_is_measured`, fails the same way on the base commit on this host, with decode steps of 600 to 700 ms. It is unrelated to this change and was not skipped.
- Not verified:
  - Building or running on Windows or macOS, and `curl` being there. Windows 10 1803 and later and macOS ship one, but this was not checked.
  - Recognition progress for under 30 s of speech: whisper.cpp reports it per 30 s window, so it stays at 0% and then completes.
  - Accuracy beyond English, speed of the larger models, and a real recording longer than one five-minute piece. The piece cutting is covered by unit tests only.
  - GPU acceleration.
