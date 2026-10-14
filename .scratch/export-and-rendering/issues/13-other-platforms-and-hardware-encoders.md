# Other platforms and hardware encoders

Status: ready-for-human
Progress: open
Type: task
Blocked by: 12

## Scope

Run exports on macOS (VideoToolbox, including ProRes) and Windows (Media
Foundation, AMF, Quick Sync), and on a Linux host whose VA-API and Quick Sync
initialise. Each needs hardware this work did not have; the code probes and
drives them with the same argument builder, but none was run. Record which
encoders passed the probe, which encoded a real export, and whether the
software fallback fired.

## Comments

2026-10-14: From ticket 12 on the Linux host: NVENC H.264 and HEVC wrote
every hardware export; `av1_nvenc` passed the encoder check but was not
used. VA-API ("Failed to initialise VAAPI connection" on renderD128 and
renderD129) and QSV ("Error creating a MFX session: -9") failed the check;
AMF was not offered by this FFmpeg. The automatic software fallback under
Auto was not seen natively, as NVENC could not be made to fail from outside;
only engine tests cover it.
