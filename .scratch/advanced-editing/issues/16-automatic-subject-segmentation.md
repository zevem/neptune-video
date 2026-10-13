# Automatic subject segmentation

Status: ready-for-human
Progress: resolved
Type: research
Blocked by: 13

## Question

What would it take to add automatic person or subject cut-out (background
removal without a key colour or a clean plate) to this native Rust editor:
which inference runtimes and models fit the architecture, under which
licences, at what cost, and how would the result reach the compositor?

Nothing here is implemented. Today background removal is `ChromaKey` or
`DifferenceMatte`, and the Color section says "No automatic subject cut-out:
use a key colour or a clean plate."

## Answer

### Summary

It is feasible without changing the architecture: a matte is derived analysis
data, like a stabilization path, and the compositor already draws a second
picture per effect for the clean plate. The work is a new analysis job, a
matte cache, one effect kind and one shader branch, plus a native inference
runtime and a model file that the product must be willing to ship. The
blockers are not technical. They are (1) which licences the product accepts,
(2) roughly 10 to 85 MB of new native library per platform plus a model, and
(3) acceptance of a result that is visibly worse than a keyed green screen on
hair, motion blur and fast movement. None of the candidate models was run
here, so every quality and speed statement below is the upstream's claim or
an expectation, and is marked.

### What I checked, and how

Checked on 2026-10-13 against the primary source named in each row (the
project's repository page, licence file, model card or release API). I ran no
model and no runtime, and I did not read any licence text in full beyond its
name and the README's statement. "Unverified" means I did not find it in a
primary source during this ticket. This is not legal advice: a licence name
on a repository says nothing certain about the data a model was trained on.

### How it would plug in

The design follows [ADR 0006](../../../docs/adr/0006-advanced-editing-is-generic-data-resolved-once.md):
derived data stays outside the project and enters the picture as a source.

1. **Effect.** A new `EffectKind` (say `SubjectMatte`) with two or three
   parameters from the existing table (`Softness`, `Feather`, perhaps
   `Threshold`) and no asset. It sits in the Key family beside chroma key and
   clean plate, so "Remove background" gains a third method. An unknown
   effect kind fails to deserialize in a build that does not know it, which
   reads as a damaged file, not a future one: the schema version must go to 3
   with the kind so that older builds refuse the file through their
   future-schema protection.
2. **Analysis.** A third job on the analysis lane in
   `crates/editor-media/src/analysis.rs`, next to tracking and stabilization:
   one FFmpeg process decodes the asset once at the model's input size, the
   runtime infers a matte a frame, and the job is cancellable with coalesced
   progress, exactly as `SteadyJob` is. The result carries project, request
   and `SourceIdentity`, so a stale answer is dropped. Interactive frames
   never wait on it; until a matte exists the layer is drawn whole and the
   section says so, as an unanalysed stabilized clip is.
3. **Matte cache.** Mattes are far larger than a camera path: 512x288 at
   eight bits is 147 KiB a frame, about 265 MiB a minute at 30 fps
   uncompressed. They should be written as a greyscale video (FFV1 or
   another lossless or near-lossless codec the bundled FFmpeg has; which one
   is unverified) into a bounded cache beside the stabilization cache, one
   file a media path and model version, least recently read evicted first,
   stamped like `SteadyCache`. A byte cap of the order of 1 to 2 GiB would be
   a product decision.
4. **Viewer.** The matte is then just another decoded video. The viewer asks
   for it as a new `Viewing::Matte(clip, effect)` at the layer's source time,
   in the same `ViewerBudget` as the layers, exactly as `Viewing::Plate` is
   asked for today; it is kept as a decoded frame and never as a texture.
5. **Compositor.** `Draw` gains the matte beside `plates`; the effect pass
   samples it as alpha with the layer's own sampling, so it follows crop,
   flips and the stabilization `Sampling` (a matte is in source space, as the
   camera path is). One `OP_` branch in `effect.wgsl`, one readback test.
   It counts in `ComposeBudget` as a plate does.
6. **Commands and agents.** No new command: `SetEffects` carries the effect.
   As with stabilization, an agent could add the effect but not start the
   analysis.

Live inference on each displayed frame was set aside: it would need a second
GPU client beside wgpu or a CPU budget inside playback, its results would
arrive a frame late and unevenly, and a paused frame would differ from a
played one. An offline matte is deterministic, cacheable and costs playback
one more decoder, which is itself a limit: five decoder pipes are retained,
and a matte takes one for each keyed clip on screen.

### Candidate runtimes

| Runtime | Licence (source) | Fit | Packaging | Risk |
| --- | --- | --- | --- | --- |
| ONNX Runtime through the `ort` crate | ONNX Runtime: MIT ([LICENSE](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)). `ort`: MIT or Apache-2.0 ([repository](https://github.com/pykeio/ort)). Both permit redistribution in a commercial desktop app with their notices | Runs every candidate model that has an ONNX export. CPU everywhere; execution providers for CoreML (listed as preview), DirectML, CUDA and others ([provider list](https://onnxruntime.ai/docs/execution-providers/)). Lives entirely in `editor-media` behind the lane; no core or UI dependency | A C++ library a platform. The 1.31.0 CPU release archives are 11.4 MB (linux-x64 `.tgz`), 36.5 MB (osx-arm64 `.tgz`) and 82.9 MB (win-x64 `.zip`) compressed ([release API](https://api.github.com/repos/microsoft/onnxruntime/releases/latest)); what part of that is headers or debug symbols is unverified. CUDA archives are 245 to 450 MB and should not be shipped. `ort` links a static build named by `ORT_LIB_PATH`, or loads a library at run time with its `load-dynamic` feature and `ORT_DYLIB_PATH` ([linking guide](https://ort.pyke.io/setup/linking)) | The crate can download a prebuilt library at build time; which targets that covers and from where is unverified (its page returned 404). A download in a build conflicts with `--locked` reproducible CI, so the library would be vendored or built. `ort` wraps 1.31.0 today; its own release stability is unverified |
| `tract` | MIT or Apache-2.0 ([repository](https://github.com/sonos/tract)) | Pure Rust, loads ONNX, no C++ toolchain. The README lists Metal and CUDA runtimes besides CPU | Compiled into the binary; size impact unverified, expected a few MB | Operator coverage for each candidate model is unverified and is the whole question; expected slower than ONNX Runtime on CPU (unverified) |
| `burn` with its wgpu backend | MIT and Apache-2.0 ([repository](https://github.com/tracel-ai/burn)) | Would use the GPU stack Aster already ships, on every platform wgpu reaches, with no vendor provider | Pure Rust | Its ONNX import "currently supports a limited set of ONNX operators" (README). Whether it can share Aster's wgpu 30 device or pulls a second wgpu version into the binary is unverified. Highest technical risk |
| `candle` | MIT or Apache-2.0 ([repository](https://github.com/huggingface/candle)) | Pure Rust; CPU with MKL or Accelerate, CUDA; has a Segment Anything example | Pure Rust | Its ONNX support is described only as "ONNX model evaluation"; `ort` calls its candle backend's operator support limited ([backends](https://ort.pyke.io/backends)). A matting model would likely be ported by hand |

`ort` also offers `ort-tract` and `ort-candle` as alternative backends behind
the same API, described as experimental. That makes "write against `ort`,
choose the engine later" possible in principle; unverified in practice.

### Candidate models

Sizes are the upstream's where stated. Speeds are the upstream's claims on
its hardware, not this editor's.

| Model | Licence and redistribution | Size | Upstream speed | Temporal behaviour | Notes |
| --- | --- | --- | --- | --- | --- |
| MediaPipe Selfie Segmentation | Apache-2.0 for the model ([model card](https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Selfie%20Segmentation.pdf)); redistribution in a commercial app is permitted with the notice | 249 KB general (256x256), 244 KB landscape (144x256) | About 33 to 35 ms on a Pixel 6, CPU or GPU ([task page](https://developers.google.com/edge/mediapipe/solutions/vision/image_segmenter)) | Per frame; flicker expected | TFLite, so it needs conversion to ONNX or a TFLite runtime (conversion unverified). The card puts people further than 4 m and several people at different scales out of scope, and says masks are not pixel perfect. People only |
| MODNet | "The code, models, and demos in this repository ... are released under the Apache License 2.0" ([README](https://github.com/ZHKKKe/MODNet)); no non-commercial clause stated | About 25 MB for the published portrait model (unverified; the README states 7M only for an unpublished demo model) | "Real-time", no figure given | Per frame; the paper describes a one-frame-delay smoothing trick (unverified here) | Trimap-free portrait matting with soft alpha, so hair is attempted. People only. ONNX export exists in the repository (unverified). Training data of the released weights is not fully public (unverified) |
| Robust Video Matting (RVM) | GPL-3.0 ([repository](https://github.com/PeterL1n/RobustVideoMatting)): "Code is re-released under GPL-3.0 license" | MobileNetV3 and ResNet50 variants; sizes not on the page (about 15 MB and 100 MB, unverified) | HD 104 FPS and 4K 74 to 76 FPS on a GTX 1080 Ti, tensor throughput only; ONNX models provided, opset 12, tested on CPU and CUDA | Recurrent: built for video, the strongest temporal stability of the list | GPL-3.0 on the code is incompatible with shipping inside a closed-source app; whether it reaches the weights, and whether the authors license otherwise, is unverified. Usable as-is only if Aster is distributed under GPL-3.0-compatible terms. People only |
| U²-Net | Apache-2.0 ([repository](https://github.com/xuebinqin/U-2-Net)) | `u2net` 176.3 MB, `u2netp` 4.7 MB | Not stated | Per frame; flicker expected | Salient object, so any subject and not only people. A separate `u2net_human_seg` is trained on the Supervisely Person dataset, whose terms are unverified, and "won't give you hair-level accuracy" |
| BiRefNet | MIT for the repository ([repository](https://github.com/ZhengPeng7/BiRefNet)); no separate statement for weights or training data | Swin-L backbone (hundreds of MB, unverified); lite and matting variants exist | 17 FPS at 1024x1024 on an RTX 4090; 58 to 96 ms there; 3.5 to 4.8 GB of GPU memory | Per frame | Best still-image edge quality of the list by reputation (unverified here). Too heavy for CPU video. ONNX files are published. The terms of its training datasets (DIS5K and others) are unverified and are the open licence question |
| SAM 2 / SAM 2.1 | "The SAM 2 model checkpoints ... are licensed under Apache 2.0" ([repository](https://github.com/facebookresearch/sam2)) | 38.9 M parameters (tiny) to 224.4 M (large); about 156 MB to 900 MB at 32-bit floats (computed, not stated) | 91.2 FPS (tiny) to 39.5 FPS (large) on an A100 | Has a memory across frames and propagates a prompted object through a video | Not automatic: it needs a click or a box. The track region the Motion section already draws could be that prompt. Hard masks, not hair-level matte. ONNX export of the video predictor is unverified |
| BRIA RMBG-2.0 | CC BY-NC 4.0; commercial use "subject to a commercial agreement with BRIA" ([model page](https://huggingface.co/briaai/RMBG-2.0)) | n/a | n/a | n/a | Excluded unless a licence is bought |

### Expected cost by class

None of this was measured on this host. The classes are for planning only.

| Class | Examples | Expected on a desktop CPU | Consequence |
| --- | --- | --- | --- |
| Tiny segmentation, 256 px | MediaPipe | A few milliseconds a frame (extrapolated from 33 ms on a phone; unverified) | Analysis of a clip faster than it plays; coarse, hard-edged matte |
| Portrait matting, about 512 px | MODNet, RVM MobileNetV3 | Tens of milliseconds a frame (unverified) | Around real time for the analysis; usable matte with soft edges |
| Large still-image models, 1024 px | U²-Net full, BiRefNet | Hundreds of milliseconds to seconds a frame (unverified); BiRefNet needs gigabytes of GPU memory upstream | Minutes of analysis for a minute of video without a GPU provider; needs temporal smoothing |
| Promptable video | SAM 2.1 | Unverified; upstream figures are A100 only | Practical only with a GPU provider |

A GPU execution provider changes these by an order of magnitude but is a
separate packaging matter on each platform (CoreML on macOS, DirectML on
Windows, CUDA or nothing on Linux), and it would contend with the
compositor's wgpu device for the same GPU; that contention is unmeasured.

### Binary size and packaging

| Platform | With ONNX Runtime (CPU) | Notes |
| --- | --- | --- |
| Linux x64 | About 11 MB compressed archive | A `.so` beside the binary, or static. No GPU provider that is reasonable to bundle; CUDA is hundreds of MB |
| macOS arm64 | About 37 MB compressed archive | A `.dylib` inside the app bundle must be signed and notarized with it. CoreML provider is in the standard build (unverified) and is listed as preview. No Intel archive in the 1.31.0 release assets |
| Windows x64 | About 83 MB compressed archive | A `.dll` beside the executable, signed. DirectML is a separate package (unverified) |

Aster has no installer, signing or release pipeline yet
([verification](../../../docs/verification.md#remaining-production-gates)),
so each of these lands on work that does not exist. With `tract` or `burn`
there is no native library and these rows reduce to the model file.

The model is the other half. Embedding it grows every download; fetching it
on first use adds a network path, a hash check, a cache and an offline
failure state to an editor that today makes no network request for editing.
A model from 250 KB to 25 MB can be embedded; one of hundreds of MB cannot
sensibly be.

### Recommendation

1. **Decide the licence policy first.** Aster's manifests declare no licence
   and the tree has no LICENSE file. If the product is to be closed source or
   permissively licensed, RVM (GPL-3.0) is out and BRIA is out; the usable
   set is MediaPipe, MODNet, U²-Net and SAM 2 (all Apache-2.0 by their own
   statement), with BiRefNet pending a check of its weights and data.
2. **Prototype before committing** (a `prototype` ticket, about a week): run
   MODNet and MediaPipe through `ort` on CPU, outside the editor, over the
   same three or four real clips, and record milliseconds a frame, matte
   quality on hair and motion, and flicker. Run the same ONNX files through
   `tract` to learn whether the pure-Rust route is open. This turns every
   "unverified" in the tables above into a number.
3. **If it goes ahead**, build it as an offline analysis into a matte cache
   (steps 1 to 6 above) with `ort` loading ONNX Runtime dynamically, CPU
   only, so the editor still starts and edits when the library or model is
   absent and the feature says so. Start with people only (MODNet, with
   MediaPipe as the small fallback) and call it "Remove background (person)".
   Add a general subject model and GPU providers only after the first one is
   accepted on real footage.
4. **Do not ship RVM** unless the product is GPL-compatible, although it is
   the best technical fit for video; use it as the quality reference in the
   prototype.

Expected limits to state in the product whatever is chosen: preview
resolution only, no export; mattes computed at 256 to 512 pixels and
upscaled; visible errors on hair, fast motion, several people and anything
far from the camera; a per-frame model flickers; analysis takes about as
long as the clip or longer on CPU.

### Decisions for the product owner

1. Under what licence will Aster be distributed, and may it bundle
   Apache-2.0 and MIT components with their notices? (Rules RVM in or out.)
2. Is 10 to 85 MB of native inference library a platform, plus a model of up
   to about 25 MB, acceptable in the download? Or must it be an optional
   component fetched on first use, which adds the editor's first network
   dependency?
3. Is person-only cut-out enough for a first version, or is any-subject
   required?
4. Is a matte of the quality these models give at preview resolution worth
   shipping before an export path exists?
5. Who signs off on the training-data provenance of the chosen weights?

Until these are answered this ticket needs a person, not an agent.

## Comments

2026-10-13: Answer written for ticket 17's documentation pass. No code, no
dependency and no model was added. Sources read: the repositories of ONNX
Runtime, `ort`, `tract`, `burn`, `candle`, MODNet, RobustVideoMatting,
U-2-Net, BiRefNet and SAM 2; the MediaPipe Selfie Segmentation model card
(pages 1 to 3) and image-segmenter task page; the BRIA RMBG-2.0 model page;
the ONNX Runtime 1.31.0 release asset list and execution-provider index; and
`ort`'s linking and backends guides. `ort`'s prebuilt-binaries page could not
be fetched.
