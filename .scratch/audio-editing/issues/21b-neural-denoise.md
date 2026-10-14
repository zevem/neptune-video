# Neural denoise

Status: ready-for-human
Progress: open
Type: research

## Question

`Denoise` is spectral subtraction (`afftdn`) with a noise floor the person
sets. A recurrent denoiser (RNNoise) or a larger model (DeepFilterNet
class) does better on speech and needs no floor. What would one take in
this native Rust editor: which runtime and model, under which licences, at
what size, and how would its output reach the mixer?

The answer should follow
[advanced-editing ticket 16](../../advanced-editing/issues/16-automatic-subject-segmentation.md),
which waits on the same owner decisions: licence policy, native inference
library size per platform, a first-use download as the editor's first
network request, and sign-off on training-data provenance.

Candidates named by the proposals, none verified:

| Route | To check |
| --- | --- |
| `nnnoiseless` (BSD-3-Clause, RNNoise port, weights embedded, no runtime or download) | Provenance of the trained weights. It needs 48 kHz frames in-process, so it does not fit the per-voice FFmpeg chain: it means an in-process stage in the mixer or a rendered derived file |
| FFmpeg `arnndn` | Needs an RNNoise model file FFmpeg does not ship; where would the editor get one. Fits the chain and the execution probe as it is |
| `ort` or `tract-onnx` with a larger model | Runtime size, model licences, speed |

Shape any answer must respect: the editor still starts, edits and plays
when the runtime or model is absent, and says so; GPL code and
non-commercial model licences are out; a prototype ticket comes before any
commitment.

## Answer

Not answered as research. Decided on the owner's behalf on 2026-10-14 (the
owner delegated every open decision of the epic): neural denoise is not
built in this epic. The editor keeps the shipped spectral noise reduction
(`Denoise`, FFmpeg `afftdn` with a noise floor the person sets, ticket 11),
and adds no runtime, model, crate or network request. The question above
stays open research for a later decision; none of its candidates was run and
no licence text was read.

## Comments

2026-10-13 (review): Split from the former ticket 21. Nothing was run and
no licence text was read; the statements above are the proposals'.

2026-10-14 (ticket 22): The owner delegated the decision; it was taken on
the owner's behalf: not built, and the shipped behaviour stays (Answer
above, spec owner decision 11). Progress stays open because the research
question is unanswered. The documents describe neural denoise as not a
feature.
