## Compilation coordination — R15

Root owns all Cargo validation.
No parallel Cargo. The frozen [R15 build record](../../artifacts/production-readiness/final/round15-build-record.json)
and [root verification](../../artifacts/production-readiness/final/round15-backend-root-verification.json)
record 521 passing tests (198 desktop, 12 CLI, 81 agent, 52 core, 112 media,
66 store), formatting, architecture, actual FFmpeg/ffprobe 8.0.1 and Linux plus
three source-target denied-warning Clippy checks: nine sequential stages with
Linux optimized linking. macOS/Windows were neither linked nor run.

Source `dfbb53e92f61a5da79309b3fc6c148c52a0415a9649c85148fefe768713af6c2`
(83 files), `neptune-video` `362ba180a2093f39dfffd71dd2f3b2b30fa1704cd6afe952c36275aa28e3f364`,
`neptune-video-agent` `00906bbabde45b204f77f2dc84e41316d87084292408d8cbf59e74d24fffc76a`.
The [release graph](../../artifacts/production-readiness/final/round15-release-libraries.json)
attributes optimized libraries to the exact Cargo-built binary. Root formatted
only two UI test files after GREEN; [pre-format copies/diffs](../../artifacts/production-readiness/final/round15-preformat/result.json)
preserve the original test blocks. [Thirteen R15 native editing/input checks](../../artifacts/native/round15-editing-smoke/root-verification.json)
pass with five viewed captures and eleven sampled identities gone; they
establish no multi-owner/cache/wake, hardware GPU, audio or fps proof. Earlier
cache/overlap acceptance remains R14 (144 captures/19 attempts/75
recorded identities gone); [its binaries and three libraries](../../artifacts/production-readiness/final/round14-retained-release/record.json)
were retained before the R15 release replacement. R15 has [three root-verified
startup captures](../../artifacts/native/round15-window-captures/root-visual-verification.json):
normal/compact visuals and separate Play/Speed controls, with library collapse
at 760×560 and an unavailable-audio notice. They establish no input, transitions,
wake/cache/GPU or audio proof; fifteen sampled process identities were gone.

## Current interfaces and completed source checks

- `request_poster(asset,time,size)` uses explicit purpose. Posters decode one
  at a time on their own `neptune-video-posters` worker, oldest admission
  first; viewer pictures never wait for one. A lane changing purpose moves to
  the other worker with its admission rank. Each kind has its own
  32-pending bound. Ranks bound dispatch opportunities, not latency or delivery.
- `PreviewConsumer`, `request_preview(consumer,&asset,time,size)` and
  `retire_preview(consumer)` preserve continuing owner/cut bindings separately
  from shared decoded pixels. Legacy requests still coalesce by asset/bounds.
  `FrameReady.consumer` is optional; `Error` also carries optional consumer/size,
  but no time. Explicit results/errors must match current owner, asset and box.
- The viewer accounts `max(kept, granted_box)` against 64 MiB logical RGBA,
  chooses one active-set quality cap and retains at most eight inactive spares.
  Preserve own-held/cut pictures and report actual one-texel exhaustion. This
  is not an upload/staging/driver or measured GPU-memory bound.
- `set_preview_retention([Option<PreviewConsumer>;3]) -> Result<(),MediaError>`
  replaces a normalized subset inside four video pipes. Empty hints use LRU;
  unchanged sets do not notify. Latest union hints is bounded to 67 lanes.
  Hint membership does not authorize results. Retirement/generation and idle/
  blocked-publication maintenance reclaim outside the queue mutex, then recheck
  currentness. [112-media proof](../../artifacts/production-readiness/decoder-retention-round2/backend-root-verification.json)
  includes manager RED/GREEN, full pixels and real-worker warm-tag controls.
- [Issue16 GREEN](../../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
  passes 21 real-worker/image/selection and all 198 desktop tests plus lint.
  Unchanged paused failed time/box is asked once; changed time/box/source,
  recovery retry and held/oversized failure retain controls. Forty playing PNG
  owners each receive a picture without raising the 32-pending limit.
- `Track::clips_overlapping(start,end)` borrows start-sorted clips in stable
  half-open order, filters ends and excludes touching/empty/reversed ranges.
  Cost is O(log n + start-qualified prefix); no interval index was added.

## Measurement and remaining acceptance

The [optimized R15 comparison](../../artifacts/production-readiness/multilayer-preview-round3/root-measurement-verification.json)
passed all 81 legacy/explicit/protected cases. Five-video sequential medians
were 185.490/179.190/67.395 ms; every measured five-video sequential batch in
all modes exceeded 33.333 ms. This is public worker evidence, not native FPS,
GPU/audio, supersession/cohort/poster competition or frontend hint effects.
The [completed independent audit](../../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
agrees with record arithmetic/build attribution and checks 3,040 observed
process/thread identities gone, including setup. Original R14 27-case artifacts and
preparation-only round-two/round-three records remain immutable.

The [completed source review](../../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
found no P1/P2 behavior defect but P2 evidence gap G1: six chooser tests use no
service and the actual still-service test has an empty hint set. Nonempty
those tests did not execute nonempty setter/`retention_told` reset effects. Six
prepared bookkeeping tests were declined, not integrated or executed. The
[actual canonical bridge](../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
now observe the nonempty call and reset after `cancel`/`ask_again` at frozen
R15: 7/10 and restart 14/20 decoder starts, 60 full-pixel checks, all 51 logged
decoders gone. This artifact probe adds no suite guard; retire/clear/Closed,
source/cut/reorder and playing remain unproved. Time-less errors allow one
bounded extra retry; playing offline video can fail for each new time, and
restored same-path media while paused needs seek/resize retry.

Issue16 source review is complete with residual G1 sites open; native-pack
pins/offline checks pass. [Final independent review](../../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
closed all four preparation findings with no P1/P2.
Actual slow/in-flight video, native wake/idle, GPU/upload bounds, bursts/paced
presentation and complete R15 native input remain open. [Four R15 backend plans](../../artifacts/native/backend-preparation/round15-plans/root-verification.json)
contain twenty `not_run` steps. Target execution is user-deferred. The last verified
UI-pack baseline was R14/51 cases/57 guards; the eleven-case supplement was draft.
The pinned 64-case catalog passed [106 offline guards](../../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json);
all native cases are `not_run`. Generated fixture bytes and [ten Linux native
startup openings](../../artifacts/native/platform-regressions-integration-round1/fixture-opener-round1/root-attempt-01/root-verification.json)
are verified, not catalog passes. Source/fixture pins are set.
No native macOS/Windows proof or whole-product completion is implied.

## Historical handoffs

The original interfaces and coordination notes below preserve earlier work.
Their pending/grant statements are historical; the R15 section above controls
current source/build/acceptance claims. Root still owns compilation coordination.

# Backend contract — Neptune Video

Coordinate via this file and ui-dependencies.md. All paths relative to repo root.

## UPDATE for UI: live agent integration (2026-10-12)

Read `.scratch/core-editor/ui-review.md` for integration findings before finishing. `Playback::tick_with_audio(now:Instant,duration:Time,audio_position:Option<Time>)->bool` now follows audio clock and maintains wall-clock fallback; use AudioService::position(). SaveResult now includes context_id and project_id (check both for current editor). AgentRequest also requires expected_context_id from inspect.result.context_id; each loaded Editor gets a new context even when reopening same project ID/revision.

UPDATE round2: `Command::RestoreProject { project:Box<Project> }` is implemented NOW. Load checkpoint off-thread with store.load_project, then dispatch RestoreProject (same project.id enforced) for one undoable restoration. Keep current desktop path/context, selection reconcile as usual. `editor_store::RecoveryLease` is being implemented to hold adjacent .lock advisory ownership for active recovery destinations; root will re-export when done. Root reserved compilewindow for UI round2; backend agents won't launch Cargo until it finishes. Autosave completion must advance last-success revision only on successful write; failed writes must retry after interval even when no new edit occurs (current old recovery_revision set on submission can suppress retries). Keep pending-request/coalescing separate from successful-save revision. Also verify library folders are manageable from UI (create virtual folder + MoveAssetToFolder actions), not only import-generated folder browsing. Requirement media-library folders implies organization; minimal folder-context/menu controls appropriate.

Asset source previews requirement: current inspector thumbnail+metadata is only a still poster. Provide actual VIDEO/GIF/AUDIO source preview transport from library without inserting into timeline (context menu Preview / optional space onselectedasset, orsourceoverlay). Backend `Project::asset_preview(&Asset)->Result<Project,EditError>` is ready NOW: transient singleclip/singletrack project with asset metadata/resolution/rate. Use existing MediaService/AudioService+Playback against Arc<previewProject>, rendertransport/source monitor; opening pauses main transport, closing restores main paused. No fake waveform or source-media writes. Original user explicitly requested asset previews so source playback needs reachability.

Recovery lease READY now: `editor_store::RecoveryLease::acquire(path:&Path)->Result<Self,StoreError>` creates/locks adjacent `path + ".lock"`, guard releases when dropped (no PID heuristics). `RecoveryLease::is_active(path:&Path)->Result<bool,StoreError>` probes existing lock without creating directories/files. Use the SAME recovery base (the path passed to save_recovery/SaveWorker.request) in acquire and probe; strip .recovery suffix from discovery before probing. Acquire off-thread when adopting a recovery, hold `Arc<RecoveryLease>` per active editor recovery destination, skip active copies when scanning; acquire again atomically when restoring because probe is momentary. `SaveWorker::request_with_lease(path:PathBuf,editor:&Editor,recovery:bool,lease:Arc<RecoveryLease>)->Result` captures clone in pending write to retain ownership across context replacement/shutdown. Public old request remains for tests/headless. Root re-exported APIs; four lease tests written. Never unlink a held lock file (inode race).

Root has added `editor-agent` dependency to desktop; crate ready now. PLEASE integrate in App runtime. `editor_agent::AgentHost::new(wake)->Result<AgentHost,AgentError>`; `poll()->Option<AgentCall>`; `AgentCall::execute(self,&mut Editor)` calls the shared protocol with revision guarding and sends reply. Drain in App::logic, compare editor.revision before/after to clear caches/selection stale IDs + resync audio exactly like UI commands. Host::publish(&Path)->Result creates private session descriptor; default unique path config_dir()/sessions/<host.descriptor().instance_id>.json; optional CLI `--agent-session PATH` for verification. App holds host; dropping stops transport/removes its own descriptor. Show descriptor path in Agent panel when useful with Copy button; no need display token.

External provider integration: `neptune-video-agent --connect /absolute/session.json` newline JSON stdio bridge to LIVE app. AgentRequest `{request_id:"1",operation:"inspect"}` then `{request_id:"2",operation:"apply",expected_revision:0,expected_project_id:"<project.id from inspect>",command:{type:"rename_project",name:"Film"}}`; Mutation project identity and revision are both required. Undo/redo require expected_revision and expected_project_id, too. `editor_agent::handle(&mut Editor,AgentRequest)->AgentResponse` also available for raw JSON panel if desired. This makes the editor accessible to real AI agents without pretending local command interpreter is a hosted LLM. Optional assistant provider configuration is outside the requested core phase.

Save correctness: call `editor.register_save()` BEFORE `SaveWorker.request(...,&editor,false)` so ack can mark exactly the captured snapshot saved while later edits stay dirty. Recovery does not mark saved. SaveResult destination/revision must be checked against project generation/path before applying. Worker now has `flush(Duration)->Result` and bounded two-second Drop.

## Model (editor_core root re-exports)

`Time(pub i64)` integer microseconds: `ZERO`, `from_seconds(f64) -> Time`, `as_seconds() -> f64`. Public arithmetic +/-. IDs are UUID type aliases `AssetId`, `TrackId`, `ClipId`, `FolderId`, `MarkerId`. Use `uuid::Uuid::new_v4()`.

`FrameRate { numerator: u32, denominator: u32 }`, default 30/1; `frame_time(i64)->Time`, `frame_at(Time)->i64`, `fps()->f64`.

`CanvasSettings { width:u32,height:u32,frame_rate:FrameRate,background:[u8;4], audio_sample_rate:u32 }`, default 1920x1080,30fps, [14,15,18,255],48000. `preset("16:9"|"9:16"|"1:1")->Self`.

`Asset { id:AssetId,name:String,path:PathBuf,kind:MediaKind,duration:Time,width:u32,height:u32,frame_rate:Option<FrameRate>,has_audio:bool,folder_id:Option<FolderId>,file_size:u64 }`; `MediaKind::{Video,Audio,Image,Gif}`. `Asset::new(path,kind,duration)->Self` defaults metadata. Images have positive still duration 5s, GIFs actual duration. Paths absolute at import.

`MediaFolder { id:FolderId,name:String,parent_id:Option<FolderId> }`.

`Project { id:Uuid,name:String,settings:CanvasSettings,assets:Arc<BTreeMap<AssetId,Asset>>,folders:Arc<BTreeMap<FolderId,MediaFolder>>,tracks:Vec<Arc<Track>>,markers:Arc<Vec<Marker>> }`; `Project::new(name)`, `duration()->Time`, `clip(id)->Option<(&Track,&Clip)>`, `track(id)->Option<&Track>`, `validate()->Result`. Empty project has V1 video, A1 audio, T1 text tracks.

`Track { id:TrackId,name:String,kind:TrackKind,locked:bool,hidden:bool,muted:bool,clips:Vec<Clip> }`; `Track::new(name,kind)`; `TrackKind::{Video,Audio,Text}`.

`Clip { id:ClipId,name:String,content:ClipContent,start:Time,source_in:Time,duration:Time,playback_rate:f64,transform:Transform,linked_id:Option<Uuid>,group_id:Option<Uuid>,gain:f32 }`; `Clip::from_asset(&Asset,start)->Self`, `Clip::text(text,start,duration)->Self`, `end()->Time`, `source_time(playhead)->Time`.

`ClipContent` serde tagged `type` snake_case: `Media {asset_id:AssetId}`, `Text {text:String,font_size:f32,color:[u8;4]}`. `Transform {position:[f32;2],scale:[f32;2],rotation:f32,opacity:f32,flip_x:bool,flip_y:bool,crop:[f32;4]}`; crop [left,top,right,bottom] normalized, position pixels relative canvas center, rotation degrees, default scales1 opacity1 crop0.

`Marker { id:MarkerId,time:Time,label:String,color:[u8;4] }`.

## Editor

`Editor::new(Project)->Result<Self,EditError>`, `Editor::default()`. `project()->&Project`, `snapshot()->Arc<Project>`, `revision()->u64`, `is_dirty()->bool`, `mark_saved(revision:u64)`, `dispatch(Command)->Result<EditOutcome,EditError>`, `undo()->bool`, `redo()->bool`, `can_undo()`, `can_redo()`, `undo_label()->Option<&str>`, `redo_label()`, `history()->impl Iterator<Item=&HistoryEntry>`. HistoryEntry label:String and before/after Arc<Project>. Snapshots structurally share unchanged tracks and assets. Atomic command validation; no partial mutations; no UI/media buffers in document/history. `copy(&[ClipId])->Result<Clipboard,EditError>`, `paste(&Clipboard,at:Time,target_track:Option<TrackId>)->Result<EditOutcome,EditError>`. Clipboard preserves relative tracks/times and remaps relationships/IDs.

`Command` serde tagged `type`, snake_case. Variants and exact fields:

- `AddAssets { assets:Vec<Asset> }`, `RemoveAssets { asset_ids:Vec<AssetId> }`
- `AddFolder { folder:MediaFolder }`, `MoveAssetToFolder { asset_id:AssetId,folder_id:Option<FolderId> }`
- `AddTrack { track:Track }`, `RemoveTrack { track_id:TrackId }`, `ReorderTrack {track_id:TrackId,index:usize}`
- `SetTrackState { track_id:TrackId,locked:Option<bool>,hidden:Option<bool>,muted:Option<bool>,name:Option<String> }`
- `AddClip { track_id:TrackId,clip:Clip }`
- `DeleteClips { clip_ids:Vec<ClipId>,ripple:bool }`
- `MoveClips { clip_ids:Vec<ClipId>,delta:Time,target_track:Option<TrackId> }` (groups and links move together; target_track relocates explicitly selected clips only)
- `TrimClip { clip_id:ClipId,source_in:Time,duration:Time,ripple:bool }` (linked clips trim together)
- `SplitClip {clip_id:ClipId,at:Time}` (linked clips split together with fresh corresponding link IDs)
- `SetTransform {clip_id:ClipId,transform:Transform}`
- `SetPlaybackRate {clip_id:ClipId,rate:f64}` (preserves consumed source span)
- `SetGain {clip_id:ClipId,gain:f32}`
- `SetText {clip_id:ClipId,text:String,font_size:f32,color:[u8;4]}`
- `LinkClips {clip_ids:Vec<ClipId>}`, `UnlinkClips {clip_ids:Vec<ClipId>}`, `GroupClips {clip_ids:Vec<ClipId>}`, `UngroupClips {clip_ids:Vec<ClipId>}`
- `AddMarker {marker:Marker}`, `RemoveMarker {marker_id:MarkerId}`
- `RenameProject {name:String}`, `SetSettings {settings:CanvasSettings}`
- `Batch {label:String,commands:Vec<Command>}` (one history entry; all-or-nothing)

`snap_time(project:&Project,time:Time,tolerance:Time,exclude:&[ClipId])->Time` nearest playhead-independent clip edge/marker; UI adds playhead snapping as needed.

`Playback::default()`: `position:Time`, `playing:bool`, `speed:f64`; `play()`, `pause()`, `seek(Time)`, `step(frames:i64,FrameRate)`, `set_speed(f64)->Result`, `tick(now:Instant,duration:Time)->bool`; no decode performed. Frame stepping rational.

`Project::active_clips(Time)->impl Iterator<Item=(&Track,&Clip)>` visible, active clips in track order. Audio runtime must separately include muted/hidden semantics as appropriate.

## Persistence (editor_store)

`save_project(path:&Path,project:&Project)->Result` atomic replacement, `load_project(path:&Path)->Result<Project>`, `save_recovery(path:&Path,editor:&Editor)->Result` (adjacent .recovery, all session history); `load_recovery(path:&Path)->Result<Editor>`; `recovery_path(path:&Path)->PathBuf`; `remove_recovery(path)`; `duplicate_project(project,name)->Project` fresh project id.

`checkpoint(path:&Path,project:&Project,label:&str)->Result<PathBuf>`; `list_checkpoints(path:&Path)->Result<Vec<CheckpointInfo>>` with path, label; load checkpoint with load_project.

`SaveWorker::new(wake:impl Fn()+Send+Sync+'static)->Self`, `request(path:PathBuf,editor:&Editor,recovery:bool)->Result<(),StoreError>`, `poll()->Option<SaveResult>`; SaveResult `{path:PathBuf,revision:u64,recovery:bool,result:Result<(),StoreError>}`. Bounded coalescing background saves; UI only marks saved successful non-recovery revision; dirty detection based on document identity.

## Media (editor_media)

`Frame { width:u32,height:u32,rgba:Arc<[u8]> }` immutable RGBA8 pixels; `byte_len()->usize`.

`MediaService::new(wake:impl Fn()+Send+Sync+'static)->Result<Self,MediaError>`, `import(paths:Vec<PathBuf>)->Result<u64,MediaError>`, `request_frame(asset:&Asset,time:Time,size:[u32;2])->Result<u64,MediaError>`, `poll()->Option<MediaEvent>`, `cancel()->u64`, `generation()->u64`. UI must call cancel when switching projects, and reject event generations unequal to service generation. Requests latest-win per asset+size, allowing library thumbnail and preview lanes independently. Size preserves aspect, caps each dimension at 4096 and RGBA bytes at 64 MiB. Returned request u64 is the project/session generation, not a unique frame id. Time on FrameReady is the requested source time; keep old frame while decoding the next.

`MediaEvent::{ Imported {generation:u64,assets:Vec<Asset>,folders:Vec<MediaFolder>,errors:Vec<ImportError>}, FrameReady {generation:u64,asset_id:AssetId,time:Time,size:[u32;2],frame:Frame}, Error {generation:u64,asset_id:Option<AssetId>,message:String} }`; `ImportError {path:PathBuf,message:String}`. Import folder IDs arrive before/with asset folder references: dispatch a Batch of AddFolder then AddAssets. Imports canonicalize paths and recurse directories without following symlink directories. Supported stills decode natively, animated GIF metadata/preview natively, video/audio metadata through ffprobe. FFmpeg command binaries are resolved via `NEPTUNE_VIDEO_FFMPEG` / `NEPTUNE_VIDEO_FFPROBE` overrides, then the copies shipped beside the `neptune-video` executable (or, for a checkout build, fetched into `.tools/media/<os>-<arch>`), then PATH; missing tools produce actionable errors. Sequential video preview retains decoder processes between frames and restarts only for backward/distant seeks or resize; bounded frame cache and decoder slots. Errors contain actual failed path/tool context.

`AudioService::new(wake:impl Fn()+Send+Sync+'static)->Result<Self,MediaError>`, `sync(project:Arc<Project>,position:Time,playing:bool,speed:f64)`, `poll()->Option<String>`, `stop()`, `position()->Option<Time>` (device clock after initial preload, preferred playback master while audible preview runs). Construct once; sync on playback toggle/seek/speed and document change (not every frame). It takes playback time as its starting anchor, runs device output and decode/mixing in background, handles every unmuted audio-bearing clip (including video's original audio), source trim, gain, clip rate and playback rate. Hidden video tracks suppress video only, not sound; mute controls sound. Native output requires a working audio device and FFmpeg. Mixes to device stereo/mono; playback rate changes resample pitch. AudioService failures should remain visible without stopping silent video preview.

## Runtime

Use eframe 0.36.2 App::ui (not old update). Real wgpu/native desktop. Follow Neptune AsyncFileDialog on main thread then worker await. Idle paused UI sleeps; playback requests repaint_after frame budget. No dynamic process/filesystem work in draw paths. UI owns transient selection/zoom/clipboard/text input. UI should explain errors unobtrusively.
