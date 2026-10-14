# Speech detection for ducking

Status: ready-for-human
Progress: open
Type: research

## Question

The ducking gate (ticket 15) is signal energy and reacts to any loud sound.
A voice-activity model would react to speech only. Is one worth its cost
here, and which: what runtime and model, under which licences, at what
size, and where would it run so that the gate stays a pure function over
derived data?

A natural home is a fifth byte in the peak file (a voice probability per
10 ms bucket, written by the `Peaks` job, the cache's algorithm version
raised), which the gate would read in place of `rms`. The answer should say
whether that holds.

Candidates named by the proposals, none verified:

| Route | To check |
| --- | --- |
| `nnnoiseless`'s per-frame voice probability (BSD-3-Clause, weights embedded) | Provenance of the weights; it wants 48 kHz, the peak job decodes at 8 kHz, so a second decode or a higher rate |
| `ort` or `tract-onnx` with a small VAD model | Runtime size, model licence, operator coverage |
| No model: a band-limited energy gate (300 Hz to 3 kHz) with a spectral flatness test | Whether it is enough better than `rms` to matter; needs no decision |

The owner decisions are those of
[advanced-editing ticket 16](../../advanced-editing/issues/16-automatic-subject-segmentation.md).

## Answer

Not answered as research. Decided on the owner's behalf on 2026-10-14 (the
owner delegated every open decision of the epic): speech detection for
ducking is not built in this epic. The editor keeps the shipped energy gate
over the cached `rms` (ticket 15), which any loud sound opens, and adds no
runtime, model, crate or network request. The question above stays open
research for a later decision; none of its candidates was run and no licence
text was read.

## Comments

2026-10-13 (review): Split from the former ticket 21. Nothing was run and
no licence text was read; the statements above are the proposals'.

2026-10-14 (ticket 22): The owner delegated the decision; it was taken on
the owner's behalf: not built, and the shipped behaviour stays (Answer
above, spec owner decision 11). Progress stays open because the research
question is unanswered. The documents describe speech detection for ducking
as not a feature.
