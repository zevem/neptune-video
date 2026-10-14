# Export throughput, memory and proxy benefit

Status: ready-for-agent
Progress: open
Type: research
Blocked by: 12

## Scope

On an optimized build, with recorded host, driver and fixtures: sustained
frames per second of long 1080p and 4K exports, software and NVENC, plain and
effect-heavy; process-tree RSS and GPU memory during a job; the frame time of
the interactive window while a job renders; how long proxies take to write;
and whether proxies or Half/Quarter preview quality let 4K or high-bitrate
media play at its rate. Today only two-second whole-job timings exist.

## Comments

2026-10-14: Ticket 12 took short release-build figures
(`docs/performance.md#release-build-export-measurements`): 8 s at 1080p in
3.6 to 3.7 s on NVENC and 5.2 to 9.2 s on libx264; 100 frames at 4K in 4.6
to 4.75 s on NVENC and 6.4 to 7.3 s on libx264. They are not sustained
throughput. During a 4K libx264 job the encoder FFmpeg reached 4,942 MiB PSS
and the source reader 1,004 MiB: the encoder gets no `-threads` and the
reader does not bound its decoder threads, unlike the preview's decoders and
the proxy lane. Choosing a cap belongs here with the memory measurements.
GPU memory, the window's frame time during a job, long renders,
effect-heavy projects, proxy write time and proxy benefit remain open.

2026-10-14: Thread caps added; the memory part is partly fixed. libx264 gets
`-threads:v` min(1.5 × logical processors, 24) (`X264_MAX_THREADS`),
libsvtav1 `-svtav1-params lp=4`, and each reader pipe `-threads 4` for its
decoder; other encoders are unchanged (reasons and figures in
`docs/performance.md#export-ffmpeg-thread-caps`). Measured by running the
reader and encoder commands outside the editor on this host, median of 3
runs under another task's load: the 4K libx264 job's FFmpeg peak fell from
6,258–6,282 MiB to 4,147–4,467 MiB (encoder 3,685–3,971, reader 462–497) at
17.7 fps against 17.8; 4K SVT-AV1 from 8,873–9,163 to 5,178–5,344 MiB.
Remaining:

- About 2 GiB at 4K is not reachable with a thread cap: one x264 thread
  still used 2.3 GiB. `-rc-lookahead 20` with about 8 threads would give
  about 2.5 GiB, but changes the encoded output and roughly halves 4K
  encoder speed (14.6 fps against 22 to 29). Needs a product decision.
- No release-build export through the editor and its compositor was
  measured with the caps, so in-app speed and peak memory are unverified.
- Other hosts, SVT-AV1 before 2.0, SVT-AV1 preset 5 at 4K, and libaom, VP9
  and ProRes beyond a 4K spot check were not measured.
- GPU memory, the window's frame time during a job, long renders,
  effect-heavy projects, proxy write time and proxy benefit remain open.
