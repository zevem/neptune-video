//! Bounded planning metadata. Small documents preserve the complete wire shape;
//! large documents explicitly describe a focused, incomplete view. Projection
//! borrows the immutable document and never clones clip/media strings.

use crate::{AiContext, AiError};
use editor_core::{
    AnimTrack, Asset, AssetId, BlendMode, CanvasSettings, Clip, ClipContent, ClipId, Effect,
    FolderId, Follow, FrameRate, Marker, Mask, MediaFolder, MediaKind, ParamKey, Project, Retime,
    TextAnimation, TextStyle, Time, Track, TrackId, TrackKind, Tracker, Transform, Transition,
};
use serde::{Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque},
    io::{self, Write},
    path::Path,
};
use uuid::Uuid;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_NEARBY_SEEDS: usize = 128;
const MAX_OPTIONAL_TRACKS: usize = 512;
const MAX_OPTIONAL_FOLDERS: usize = 128;
const MAX_NEARBY_MARKERS: usize = 32;
const MAX_DISPLAY_NAME_BYTES: usize = 512;
const MAX_PATH_BYTES: usize = 4096;
/// Longer keyframe tracks and tracker paths reach the provider as counts.
const MAX_INLINE_KEYS: usize = 32;

#[derive(Serialize)]
struct Input<'a, P: Serialize> {
    request: &'a str,
    selected_clip_ids: &'a [ClipId],
    playhead_microseconds: Time,
    project: P,
}

/// Serializes immutable metadata on the provider worker. Output is at most
/// 1 MiB; selected clips and their transitive link/group closure are never
/// silently truncated. Optional neighborhoods contain whole closures or none.
pub(crate) fn prompt(context: &AiContext, request: &str) -> Result<String, AiError> {
    if request.trim().is_empty() || request.len() > 32 * 1024 || request.contains('\0') {
        return Err(AiError::Invalid("Enter a request of 1–32768 bytes".into()));
    }
    if !fits(&context.selected) {
        return Err(required_too_large());
    }
    let selected = selected_locations(context)?;
    let full = Input {
        request,
        selected_clip_ids: &context.selected,
        playhead_microseconds: context.playhead,
        project: LeanProject::from(context.project.as_ref()),
    };
    if let Ok(bytes) = encode(&full) {
        return text(bytes);
    }

    let graph = Graph::new(&context.project, context.playhead);
    let required = graph.closure(selected.iter().copied());
    let required_clips: Vec<_> = required
        .iter()
        .map(|location| LeanClip(graph.clip(*location)))
        .collect();
    if !fits(&required_clips) {
        return Err(required_too_large());
    }
    // Track/folder inventories are optional; selected track states and source
    // limits are mandatory. Shrink inventories before refusing exact targets.
    let mut inventories = (MAX_OPTIONAL_TRACKS, MAX_OPTIONAL_FOLDERS);
    let mut included = required.clone();
    while !fits(&focused(
        context,
        request,
        &graph,
        &included,
        &required,
        ViewExtras {
            markers: &[],
            inventories,
            skipped: 0,
        },
    )?) {
        if inventories == (0, 0) {
            return Err(required_too_large());
        }
        inventories = (inventories.0 / 2, inventories.1 / 2);
    }

    let mut visited = required.clone();
    let mut skipped = 0;
    for seed in &graph.nearby {
        if visited.contains(seed) {
            continue;
        }
        let members = graph.closure([*seed]);
        visited.extend(members.iter().copied());
        let added: Vec<_> = members.difference(&included).copied().collect();
        included.extend(added.iter().copied());
        // Counting serialization checks exact escaped byte cost without
        // allocating one context string per optional component.
        if !fits(&focused(
            context,
            request,
            &graph,
            &included,
            &required,
            ViewExtras {
                markers: &[],
                inventories,
                skipped,
            },
        )?) {
            for location in added {
                included.remove(&location);
            }
            skipped += 1;
        }
    }

    let mut nearest: BinaryHeap<(u64, usize)> = BinaryHeap::new();
    for (index, marker) in context.project.markers.iter().enumerate() {
        keep_nearest(
            &mut nearest,
            (marker.time.0.abs_diff(context.playhead.0), index),
            MAX_NEARBY_MARKERS,
        );
    }
    let mut markers = Vec::new();
    for (_, index) in nearest.into_sorted_vec() {
        markers.push(&context.project.markers[index]);
        if !fits(&focused(
            context,
            request,
            &graph,
            &included,
            &required,
            ViewExtras {
                markers: &markers,
                inventories,
                skipped,
            },
        )?) {
            markers.pop();
        }
    }
    let value = focused(
        context,
        request,
        &graph,
        &included,
        &required,
        ViewExtras {
            markers: &markers,
            inventories,
            skipped,
        },
    )?;
    let bytes = encode(&value).map_err(|_| required_too_large())?;
    text(bytes)
}

fn required_too_large() -> AiError {
    AiError::Invalid("Selected clips and their complete linked/grouped editing context exceed the 1 MiB planning limit; use a smaller independent selection".into())
}

fn text(bytes: Vec<u8>) -> Result<String, AiError> {
    String::from_utf8(bytes)
        .map_err(|_| AiError::Invalid("Editing context cannot be encoded".into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Location {
    track: usize,
    clip: usize,
}

fn selected_locations(context: &AiContext) -> Result<BTreeSet<Location>, AiError> {
    let mut missing: HashSet<ClipId> = context.selected.iter().copied().collect();
    let mut locations = BTreeSet::new();
    for (track_index, track) in context.project.tracks.iter().enumerate() {
        for (clip_index, clip) in track.clips.iter().enumerate() {
            if missing.remove(&clip.id) {
                locations.insert(Location {
                    track: track_index,
                    clip: clip_index,
                });
            }
        }
    }
    if !missing.is_empty() {
        return Err(AiError::Invalid(
            "A captured selected clip no longer belongs to this project; capture a new context"
                .into(),
        ));
    }
    Ok(locations)
}

struct Graph<'a> {
    project: &'a Project,
    links: HashMap<Uuid, Vec<Location>>,
    groups: HashMap<Uuid, Vec<Location>>,
    nearby: Vec<Location>,
    clip_count: usize,
    duration: Time,
}

impl<'a> Graph<'a> {
    fn new(project: &'a Project, playhead: Time) -> Self {
        let mut links: HashMap<Uuid, Vec<Location>> = HashMap::new();
        let mut groups: HashMap<Uuid, Vec<Location>> = HashMap::new();
        let mut nearby = BinaryHeap::new();
        let mut clip_count = 0;
        let mut duration = Time::ZERO;
        for (track_index, track) in project.tracks.iter().enumerate() {
            for (clip_index, clip) in track.clips.iter().enumerate() {
                let location = Location {
                    track: track_index,
                    clip: clip_index,
                };
                if let Some(link) = clip.linked_id {
                    links.entry(link).or_default().push(location);
                }
                if let Some(group) = clip.group_id {
                    groups.entry(group).or_default().push(location);
                }
                let active = clip.start <= playhead && playhead < clip.end();
                let distance = if active {
                    0
                } else {
                    clip.start
                        .0
                        .abs_diff(playhead.0)
                        .min(clip.end().0.abs_diff(playhead.0))
                };
                // An active clip wins an equal-distance tie with one just ended.
                keep_nearest(
                    &mut nearby,
                    (distance, u8::from(!active), location),
                    MAX_NEARBY_SEEDS,
                );
                clip_count += 1;
                duration = duration.max(clip.end());
            }
        }
        Self {
            project,
            links,
            groups,
            nearby: nearby
                .into_sorted_vec()
                .into_iter()
                .map(|(_, _, location)| location)
                .collect(),
            clip_count,
            duration,
        }
    }

    fn clip(&self, location: Location) -> &'a Clip {
        // Locations are made only while traversing this immutable snapshot.
        &self.project.tracks[location.track].clips[location.clip]
    }

    fn closure(&self, seeds: impl IntoIterator<Item = Location>) -> BTreeSet<Location> {
        let mut found = BTreeSet::new();
        let mut queue = VecDeque::new();
        for seed in seeds {
            if found.insert(seed) {
                queue.push_back(seed);
            }
        }
        let mut links = HashSet::new();
        let mut groups = HashSet::new();
        while let Some(location) = queue.pop_front() {
            let clip = self.clip(location);
            for members in [
                clip.linked_id
                    .filter(|id| links.insert(*id))
                    .and_then(|id| self.links.get(&id)),
                clip.group_id
                    .filter(|id| groups.insert(*id))
                    .and_then(|id| self.groups.get(&id)),
            ]
            .into_iter()
            .flatten()
            {
                for member in members {
                    if found.insert(*member) {
                        queue.push_back(*member);
                    }
                }
            }
        }
        found
    }
}

fn keep_nearest<T: Ord>(heap: &mut BinaryHeap<T>, value: T, limit: usize) {
    if heap.len() < limit {
        heap.push(value);
    } else if heap.peek().is_some_and(|largest| value < *largest) {
        heap.pop();
        heap.push(value);
    }
}

#[derive(Serialize)]
struct Counts {
    tracks: usize,
    clips: usize,
    assets: usize,
    folders: usize,
    markers: usize,
    trackers: usize,
}

#[derive(Serialize)]
struct Scope {
    mode: &'static str,
    complete: bool,
    selected_related_complete: bool,
    assets_complete_for_included_clips: bool,
    total: Counts,
    omitted: Counts,
    exact_selected_related_clips: usize,
    included_nearby_related_clips: usize,
    nearby_components_skipped_for_budget: usize,
    guidance: &'static str,
}

const GUIDANCE: &str = "This is a focused, incomplete view, not a replacement project. Absent tracks, clips, assets, folders and markers still exist; omitted counts say how much is outside scope. Track order is its absolute zero-based project index, not this array's position. Each track clip_count/omitted_clips includes its unseen clips. Selected clips and transitive linked/grouped relatives are exact and complete; each included nearby relationship component is also complete. All included media clips have actual source duration/rate/dimensions/audio metadata. Optional display names or file paths are null only where omitted_fields explicitly declares them. Trackers are listed only for included clips that follow them. Never infer deletion, emptiness or global coverage from absence; request narrower context or clarification for edits requiring unseen data.";

#[derive(Serialize)]
struct FocusedInput<'a> {
    #[serde(flatten)]
    input: Input<'a, FocusedProject<'a>>,
    context_scope: Scope,
}

#[derive(Serialize)]
struct FocusedProject<'a> {
    id: Uuid,
    name: Option<&'a str>,
    omitted_fields: Vec<&'static str>,
    settings: &'a CanvasSettings,
    duration_microseconds: Time,
    assets: BTreeMap<AssetId, FocusedAsset<'a>>,
    folders: BTreeMap<FolderId, FocusedFolder<'a>>,
    tracks: Vec<FocusedTrack<'a>>,
    markers: Vec<&'a Marker>,
    trackers: BTreeMap<Uuid, LeanTracker<'a>>,
}

#[derive(Serialize)]
struct FocusedTrack<'a> {
    id: TrackId,
    order: usize,
    name: Option<&'a str>,
    omitted_fields: Vec<&'static str>,
    kind: TrackKind,
    locked: bool,
    hidden: bool,
    muted: bool,
    #[serde(skip_serializing_if = "unity")]
    gain: f32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    solo: bool,
    clip_count: usize,
    omitted_clips: usize,
    clips: Vec<LeanClip<'a>>,
}

/// The complete document with long key lists replaced by declared counts.
#[derive(Serialize)]
struct LeanProject<'a> {
    id: Uuid,
    name: &'a str,
    settings: &'a CanvasSettings,
    assets: &'a BTreeMap<AssetId, Asset>,
    folders: &'a BTreeMap<FolderId, MediaFolder>,
    tracks: Vec<LeanTrack<'a>>,
    markers: &'a [Marker],
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    trackers: BTreeMap<Uuid, LeanTracker<'a>>,
}

impl<'a> From<&'a Project> for LeanProject<'a> {
    fn from(project: &'a Project) -> Self {
        // Exhaustive, so a new document field cannot be dropped silently.
        let Project {
            id,
            name,
            settings,
            assets,
            folders,
            tracks,
            markers,
            trackers,
        } = project;
        Self {
            id: *id,
            name,
            settings,
            assets,
            folders,
            tracks: tracks
                .iter()
                .map(|track| LeanTrack::from(&**track))
                .collect(),
            markers,
            trackers: trackers
                .iter()
                .map(|(id, tracker)| (*id, LeanTracker(tracker)))
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct LeanTrack<'a> {
    id: TrackId,
    name: &'a str,
    kind: TrackKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    locked: bool,
    hidden: bool,
    muted: bool,
    #[serde(skip_serializing_if = "unity")]
    gain: f32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    solo: bool,
    clips: Vec<LeanClip<'a>>,
}

impl<'a> From<&'a Track> for LeanTrack<'a> {
    fn from(track: &'a Track) -> Self {
        let Track {
            id,
            name,
            kind,
            language,
            locked,
            hidden,
            muted,
            gain,
            solo,
            clips,
        } = track;
        Self {
            id: *id,
            name,
            kind: *kind,
            language: language.as_deref(),
            locked: *locked,
            hidden: *hidden,
            muted: *muted,
            gain: *gain,
            solo: *solo,
            clips: clips.iter().map(LeanClip).collect(),
        }
    }
}

/// Exact unless an animation track is long; then that track alone is a count.
struct LeanClip<'a>(&'a Clip);

impl Serialize for LeanClip<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let timed = matches!(&self.0.content, ClipContent::Text { words, .. } if !words.is_empty());
        if !timed && self.0.animation.iter().all(|track| !long(&track.keys)) {
            return self.0.serialize(serializer);
        }
        let Clip {
            id,
            name,
            content,
            start,
            source_in,
            duration,
            playback_rate,
            transform,
            linked_id,
            group_id,
            gain,
            blend,
            effects,
            masks,
            animation,
            retime,
            transition_in,
            transition_out,
            follow,
            fade_in,
            fade_out,
        } = self.0;
        SummarizedClip {
            id: *id,
            name,
            content: LeanContent(content),
            start: *start,
            source_in: *source_in,
            duration: *duration,
            playback_rate: *playback_rate,
            transform,
            linked_id: *linked_id,
            group_id: *group_id,
            gain: *gain,
            blend: *blend,
            effects,
            masks,
            animation: animation.iter().map(LeanAnimation).collect(),
            retime,
            transition_in,
            transition_out,
            follow,
            fade_in: *fade_in,
            fade_out: *fade_out,
        }
        .serialize(serializer)
    }
}

fn long<T>(keys: &[T]) -> bool {
    keys.len() > MAX_INLINE_KEYS
}

fn empty<T>(values: &&Vec<T>) -> bool {
    values.is_empty()
}

fn none<T>(value: &&Option<T>) -> bool {
    value.is_none()
}

/// A track at its default volume sends no `gain`, as the document does.
fn unity(gain: &f32) -> bool {
    *gain == 1.0
}

/// Mirrors `Clip`'s wire shape around summarized animation.
#[derive(Serialize)]
struct SummarizedClip<'a> {
    id: ClipId,
    name: &'a str,
    content: LeanContent<'a>,
    start: Time,
    source_in: Time,
    duration: Time,
    playback_rate: f64,
    transform: &'a Transform,
    linked_id: Option<Uuid>,
    group_id: Option<Uuid>,
    gain: f32,
    #[serde(skip_serializing_if = "BlendMode::is_normal")]
    blend: BlendMode,
    #[serde(skip_serializing_if = "empty")]
    effects: &'a Vec<Effect>,
    #[serde(skip_serializing_if = "empty")]
    masks: &'a Vec<Mask>,
    animation: Vec<LeanAnimation<'a>>,
    #[serde(skip_serializing_if = "none")]
    retime: &'a Option<Retime>,
    #[serde(skip_serializing_if = "none")]
    transition_in: &'a Option<Transition>,
    #[serde(skip_serializing_if = "none")]
    transition_out: &'a Option<Transition>,
    #[serde(skip_serializing_if = "none")]
    follow: &'a Option<Follow>,
    #[serde(skip_serializing_if = "Time::is_zero")]
    fade_in: Time,
    #[serde(skip_serializing_if = "Time::is_zero")]
    fade_out: Time,
}

/// A timed title's words are a count and a span: a proposal edits text,
/// and the editor keeps the times of the words it keeps.
struct LeanContent<'a>(&'a ClipContent);

impl Serialize for LeanContent<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "type", rename = "text")]
        struct Summary<'a> {
            text: &'a str,
            font_size: f32,
            color: [u8; 4],
            #[serde(skip_serializing_if = "TextStyle::is_plain")]
            style: &'a TextStyle,
            #[serde(skip_serializing_if = "none")]
            enter: &'a Option<TextAnimation>,
            #[serde(skip_serializing_if = "none")]
            exit: &'a Option<TextAnimation>,
            words: Option<()>,
            word_count: usize,
            spoken: [Time; 2],
            omitted_fields: [&'static str; 1],
        }
        let ClipContent::Text {
            text,
            font_size,
            color,
            style,
            enter,
            exit,
            words,
        } = self.0
        else {
            return self.0.serialize(serializer);
        };
        let (Some(first), Some(last)) = (words.first(), words.last()) else {
            return self.0.serialize(serializer);
        };
        Summary {
            text,
            font_size: *font_size,
            color: *color,
            style,
            enter,
            exit,
            words: None,
            word_count: words.len(),
            spoken: [first.at, last.end],
            omitted_fields: ["words"],
        }
        .serialize(serializer)
    }
}

struct LeanAnimation<'a>(&'a AnimTrack);

impl Serialize for LeanAnimation<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Summary {
            target: Option<Uuid>,
            key: ParamKey,
            keys: Option<()>,
            key_count: usize,
            omitted_fields: [&'static str; 1],
        }
        let AnimTrack { target, key, keys } = self.0;
        if !long(keys) {
            return self.0.serialize(serializer);
        }
        Summary {
            target: *target,
            key: *key,
            keys: None,
            key_count: keys.len(),
            omitted_fields: ["keys"],
        }
        .serialize(serializer)
    }
}

struct LeanTracker<'a>(&'a Tracker);

impl Serialize for LeanTracker<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Summary<'a> {
            id: Uuid,
            name: &'a str,
            asset_id: AssetId,
            keys: Option<()>,
            key_count: usize,
            omitted_fields: [&'static str; 1],
        }
        let Tracker {
            id,
            name,
            asset_id,
            keys,
        } = self.0;
        if !long(keys) {
            return self.0.serialize(serializer);
        }
        Summary {
            id: *id,
            name,
            asset_id: *asset_id,
            keys: None,
            key_count: keys.len(),
            omitted_fields: ["keys"],
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct FocusedAsset<'a> {
    id: AssetId,
    name: Option<&'a str>,
    path: Option<&'a Path>,
    omitted_fields: Vec<&'static str>,
    kind: MediaKind,
    duration: Time,
    width: u32,
    height: u32,
    frame_rate: Option<FrameRate>,
    has_audio: bool,
    folder_id: Option<FolderId>,
    file_size: u64,
}

impl<'a> From<&'a Asset> for FocusedAsset<'a> {
    fn from(asset: &'a Asset) -> Self {
        let name = (asset.name.len() <= MAX_DISPLAY_NAME_BYTES).then_some(asset.name.as_str());
        let path = asset
            .path
            .to_str()
            .filter(|path| path.len() <= MAX_PATH_BYTES)
            .map(|_| asset.path.as_path());
        let mut omitted_fields = Vec::new();
        if name.is_none() {
            omitted_fields.push("name");
        }
        if path.is_none() {
            omitted_fields.push("path");
        }
        Self {
            id: asset.id,
            name,
            path,
            omitted_fields,
            kind: asset.kind,
            duration: asset.duration,
            width: asset.width,
            height: asset.height,
            frame_rate: asset.frame_rate,
            has_audio: asset.has_audio,
            folder_id: asset.folder_id,
            file_size: asset.file_size,
        }
    }
}

#[derive(Serialize)]
struct FocusedFolder<'a> {
    id: FolderId,
    name: Option<&'a str>,
    parent_id: Option<FolderId>,
    omitted_fields: Vec<&'static str>,
}

impl<'a> From<&'a MediaFolder> for FocusedFolder<'a> {
    fn from(folder: &'a MediaFolder) -> Self {
        let name = (folder.name.len() <= MAX_DISPLAY_NAME_BYTES).then_some(folder.name.as_str());
        Self {
            id: folder.id,
            name,
            parent_id: folder.parent_id,
            omitted_fields: if name.is_none() { vec!["name"] } else { vec![] },
        }
    }
}

struct ViewExtras<'a, 'b> {
    markers: &'b [&'a Marker],
    inventories: (usize, usize),
    skipped: usize,
}

fn focused<'a>(
    context: &'a AiContext,
    request: &'a str,
    graph: &Graph<'a>,
    included: &BTreeSet<Location>,
    required: &BTreeSet<Location>,
    extras: ViewExtras<'a, '_>,
) -> Result<FocusedInput<'a>, AiError> {
    let ViewExtras {
        markers,
        inventories,
        skipped,
    } = extras;
    let project = context.project.as_ref();
    let mut clips: BTreeMap<usize, Vec<LeanClip>> = BTreeMap::new();
    let mut assets = BTreeMap::new();
    let mut trackers = BTreeMap::new();
    let mut folder_ids = VecDeque::new();
    for location in included {
        let clip = graph.clip(*location);
        clips
            .entry(location.track)
            .or_default()
            .push(LeanClip(clip));
        let tracker = clip
            .follow
            .as_ref()
            .and_then(|follow| project.trackers.get(&follow.tracker_id));
        if let Some(tracker) = tracker {
            trackers.insert(tracker.id, LeanTracker(tracker));
        }
        // Clean plates, LUTs and tracked sources are source context too.
        for id in clip
            .asset_id()
            .into_iter()
            .chain(clip.effects.iter().filter_map(|effect| effect.asset_id))
            .chain(tracker.map(|tracker| tracker.asset_id))
        {
            let asset = project.assets.get(&id).ok_or_else(|| {
                AiError::Invalid("An included clip is missing its source asset".into())
            })?;
            if let std::collections::btree_map::Entry::Vacant(entry) = assets.entry(id) {
                entry.insert(FocusedAsset::from(asset));
                if let Some(folder) = asset.folder_id {
                    folder_ids.push_back(folder);
                }
            }
        }
    }
    let mut folders = BTreeMap::new();
    while folders.len() < inventories.1 {
        let Some(id) = folder_ids.pop_front() else {
            break;
        };
        if folders.contains_key(&id) {
            continue;
        }
        if let Some(folder) = project.folders.get(&id) {
            folders.insert(id, FocusedFolder::from(folder));
            if let Some(parent) = folder.parent_id {
                folder_ids.push_back(parent);
            }
        }
    }
    // An otherwise unused library can still expose a bounded folder inventory.
    for (id, folder) in project.folders.iter() {
        if folders.len() >= inventories.1 {
            break;
        }
        folders
            .entry(*id)
            .or_insert_with(|| FocusedFolder::from(folder));
    }

    let mut tracks = Vec::new();
    let mut optional = 0;
    for (order, track) in project.tracks.iter().enumerate() {
        let present = clips.remove(&order);
        if present.is_none() {
            if optional >= inventories.0 {
                continue;
            }
            optional += 1;
        }
        let selected_clips = present.unwrap_or_default();
        let name = (track.name.len() <= MAX_DISPLAY_NAME_BYTES).then_some(track.name.as_str());
        tracks.push(FocusedTrack {
            id: track.id,
            order,
            name,
            omitted_fields: if name.is_none() { vec!["name"] } else { vec![] },
            kind: track.kind,
            locked: track.locked,
            hidden: track.hidden,
            muted: track.muted,
            gain: track.gain,
            solo: track.solo,
            clip_count: track.clips.len(),
            omitted_clips: track.clips.len() - selected_clips.len(),
            clips: selected_clips,
        });
    }
    let name = (project.name.len() <= MAX_DISPLAY_NAME_BYTES).then_some(project.name.as_str());
    let scope = Scope {
        mode: "focused",
        complete: false,
        selected_related_complete: true,
        assets_complete_for_included_clips: true,
        total: Counts {
            tracks: project.tracks.len(),
            clips: graph.clip_count,
            assets: project.assets.len(),
            folders: project.folders.len(),
            markers: project.markers.len(),
            trackers: project.trackers.len(),
        },
        omitted: Counts {
            tracks: project.tracks.len() - tracks.len(),
            clips: graph.clip_count - included.len(),
            assets: project.assets.len() - assets.len(),
            folders: project.folders.len() - folders.len(),
            markers: project.markers.len() - markers.len(),
            trackers: project.trackers.len() - trackers.len(),
        },
        exact_selected_related_clips: required.len(),
        included_nearby_related_clips: included.len() - required.len(),
        nearby_components_skipped_for_budget: skipped,
        guidance: GUIDANCE,
    };
    Ok(FocusedInput {
        input: Input {
            request,
            selected_clip_ids: &context.selected,
            playhead_microseconds: context.playhead,
            project: FocusedProject {
                id: project.id,
                name,
                omitted_fields: if name.is_none() { vec!["name"] } else { vec![] },
                settings: &project.settings,
                duration_microseconds: graph.duration,
                assets,
                folders,
                tracks,
                markers: markers.to_vec(),
                trackers,
            },
        },
        context_scope: scope,
    })
}

struct LimitedWriter {
    bytes: Vec<u8>,
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("Context budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let mut writer = LimitedWriter { bytes: Vec::new() };
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.bytes)
}

struct Counter {
    bytes: usize,
}

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.bytes) {
            return Err(io::Error::other("Context budget exceeded"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fits(value: &impl Serialize) -> bool {
    serde_json::to_writer(Counter { bytes: 0 }, value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Ease, Editor, Keyframe, TrackKey};
    use serde_json::{Value, json};
    use std::{path::PathBuf, sync::Arc};

    fn large_project() -> Project {
        let mut project = Project::new("Five thousand clips");
        let mut video = Track::new("Video", TrackKind::Video);
        for index in 0..5_000 {
            let mut asset = Asset::new(
                PathBuf::from(format!("/owned/media/shot-{index}.mp4")),
                MediaKind::Video,
                Time(12_000_000),
            );
            asset.width = 1920;
            asset.height = 1080;
            asset.frame_rate = Some(FrameRate {
                numerator: 30_000,
                denominator: 1001,
            });
            asset.has_audio = true;
            asset.file_size = 64_000_000 + index;
            let mut clip = Clip::from_asset(&asset, Time(index as i64 * 100_000));
            clip.duration = Time(100_000);
            clip.source_in = Time(2_000_000);
            clip.playback_rate = 0.75;
            clip.gain = 0.4;
            clip.transform.position = [12.0, -15.0];
            clip.transform.crop = [0.1, 0.2, 0.1, 0.1];
            video.clips.push(clip);
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        }
        project.tracks[0] = Arc::new(video);
        project.validate().unwrap();
        project
    }

    fn captured(project: Project, selected: Vec<ClipId>, playhead: Time) -> AiContext {
        let editor = Editor::new(project).unwrap();
        AiContext::capture(&editor, &selected, playhead)
    }

    fn clips(value: &Value) -> Vec<&Value> {
        value["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|track| track["clips"].as_array().unwrap())
            .collect()
    }

    #[test]
    fn small_project_retains_the_complete_existing_wire_shape() {
        let mut project = Project::new("Small exact document");
        let clip = Clip::text("A title", Time::ZERO, Time(3_000_000));
        let selected = vec![clip.id];
        Arc::make_mut(&mut project.tracks[2]).clips.push(clip);
        let context = captured(project, selected, Time(1_000_000));
        let before = context.project.clone();
        let encoded = prompt(&context, "Trim the title").unwrap();
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            value,
            json!({
                "request": "Trim the title",
                "selected_clip_ids": context.selected,
                "playhead_microseconds": context.playhead,
                "project": context.project.as_ref(),
            })
        );
        assert!(value.get("context_scope").is_none());
        assert!(Arc::ptr_eq(&before, &context.project));
    }

    #[test]
    fn five_thousand_clip_view_is_bounded_and_preserves_ids_and_exact_source_limits() {
        let mut project = large_project();
        let selected = vec![
            project.tracks[0].clips[0].id,
            project.tracks[0].clips[4999].id,
        ];
        Arc::make_mut(&mut project.tracks[0]).locked = true;
        let context = captured(project, selected.clone(), Time(250_050_000));
        let before = context.project.clone();
        let encoded = prompt(&context, "Inspect the selected source ranges").unwrap();
        assert!(encoded.len() <= MAX_BYTES);
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["context_scope"]["mode"], "focused");
        assert_eq!(value["context_scope"]["total"]["clips"], 5000);
        assert_eq!(value["context_scope"]["total"]["assets"], 5000);
        assert_eq!(value["context_scope"]["selected_related_complete"], true);
        assert!(value["context_scope"]["omitted"]["clips"].as_u64().unwrap() > 0);
        assert!(
            value["context_scope"]["included_nearby_related_clips"]
                .as_u64()
                .unwrap()
                > 0
        );
        let present = clips(&value);
        for id in selected {
            assert!(present.iter().any(|clip| clip["id"] == id.to_string()));
        }
        assert!(present.iter().any(|clip| clip["start"] == 250_000_000));
        for clip in present {
            let id: ClipId = serde_json::from_value(clip["id"].clone()).unwrap();
            let (track, actual) = context.project.clip(id).unwrap();
            let projected_clip: Clip = serde_json::from_value(clip.clone()).unwrap();
            assert_eq!(&projected_clip, actual);
            let asset_id = actual.asset_id().unwrap();
            let actual_asset = &context.project.assets[&asset_id];
            let projected: Asset =
                serde_json::from_value(value["project"]["assets"][asset_id.to_string()].clone())
                    .unwrap();
            assert_eq!(&projected, actual_asset);
            assert!(
                actual.source_in + actual.duration.scaled(actual.playback_rate)
                    <= projected.duration
            );
            let focused_track = value["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["id"] == track.id.to_string())
                .unwrap();
            assert_eq!(focused_track["locked"], track.locked);
            assert_eq!(focused_track["clip_count"], track.clips.len());
        }
        assert!(Arc::ptr_eq(&before, &context.project));
    }

    #[test]
    fn transitive_link_and_group_relatives_remain_exact_even_far_from_playhead() {
        let mut project = large_project();
        let selected = project.tracks[0].clips[0].id;
        let link = Uuid::new_v4();
        let group = Uuid::new_v4();
        let second_link = Uuid::new_v4();
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.clips[0].linked_id = Some(link);
        track.clips[4999].linked_id = Some(link);
        track.clips[4999].group_id = Some(group);
        track.clips[4800].group_id = Some(group);
        track.clips[4800].linked_id = Some(second_link);
        track.clips[4801].linked_id = Some(second_link);
        let expected: Vec<_> = [0, 4999, 4800, 4801]
            .map(|index| track.clips[index].clone())
            .into();
        let context = captured(project, vec![selected], Time::ZERO);
        let value: Value = serde_json::from_str(
            &prompt(&context, "Move the selection and its relatives").unwrap(),
        )
        .unwrap();
        assert_eq!(value["context_scope"]["exact_selected_related_clips"], 4);
        for clip in expected {
            let present = clips(&value)
                .into_iter()
                .find(|present| present["id"] == clip.id.to_string())
                .unwrap();
            let projected: Clip = serde_json::from_value(present.clone()).unwrap();
            assert_eq!(projected, clip);
        }
    }

    #[test]
    fn an_oversized_selected_relationship_component_is_rejected_without_truncation() {
        let mut project = large_project();
        let group = Uuid::new_v4();
        let track = Arc::make_mut(&mut project.tracks[0]);
        for clip in &mut track.clips {
            clip.group_id = Some(group);
        }
        let selected = track.clips[0].id;
        let context = captured(project, vec![selected], Time::ZERO);
        assert!(
            matches!(prompt(&context, "Move the whole selected group"), Err(AiError::Invalid(message)) if message.contains("Selected clips and their complete linked/grouped"))
        );
        assert_eq!(context.project.tracks[0].clips.len(), 5000);
    }

    #[test]
    fn an_oversized_nearby_component_is_omitted_whole_with_explicit_scope() {
        let mut project = large_project();
        let group = Uuid::new_v4();
        for clip in &mut Arc::make_mut(&mut project.tracks[0]).clips {
            clip.group_id = Some(group);
        }
        let context = captured(project, vec![], Time::ZERO);
        let encoded = prompt(&context, "Change the canvas background").unwrap();
        assert!(encoded.len() <= MAX_BYTES);
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(clips(&value).len(), 0);
        assert_eq!(value["context_scope"]["omitted"]["clips"], 5000);
        assert_eq!(
            value["context_scope"]["nearby_components_skipped_for_budget"],
            1
        );
        assert_eq!(value["project"]["tracks"][0]["omitted_clips"], 5000);
        assert_eq!(value["context_scope"]["complete"], false);
    }

    #[test]
    fn many_track_view_retains_required_absolute_order_and_all_track_states() {
        let mut project = large_project();
        for index in 0..700 {
            project.tracks.push(Arc::new(Track::new(
                format!("Extra {index}"),
                TrackKind::Text,
            )));
        }
        let clip = Clip::text(
            "Selected on a late track",
            Time(250_000_000),
            Time(1_000_000),
        );
        let selected = clip.id;
        let last = project.tracks.len() - 1;
        let track = Arc::make_mut(&mut project.tracks[last]);
        track.clips.push(clip);
        track.locked = true;
        track.hidden = true;
        track.muted = true;
        let selected_track = track.id;
        let context = captured(project, vec![selected], Time(250_000_000));
        let value: Value =
            serde_json::from_str(&prompt(&context, "Explain this locked track").unwrap()).unwrap();
        let tracks = value["project"]["tracks"].as_array().unwrap();
        assert!(
            value["context_scope"]["omitted"]["tracks"]
                .as_u64()
                .unwrap()
                > 0
        );
        for track in tracks {
            let order = track["order"].as_u64().unwrap() as usize;
            assert_eq!(track["id"], context.project.tracks[order].id.to_string());
            for state in ["locked", "hidden", "muted"] {
                assert_eq!(
                    track[state],
                    serde_json::to_value(&context.project.tracks[order]).unwrap()[state]
                );
            }
        }
        let late = tracks
            .iter()
            .find(|track| track["id"] == selected_track.to_string())
            .unwrap();
        assert_eq!(late["order"], last);
        assert_eq!(late["locked"], true);
        assert_eq!(late["hidden"], true);
        assert_eq!(late["muted"], true);
        assert_eq!(late["clips"][0]["id"], selected.to_string());
    }

    #[test]
    fn optional_names_and_paths_are_omitted_only_with_exact_field_declarations() {
        let mut project = Project::new("Named document");
        let mut asset = Asset::new(
            PathBuf::from(format!("/media/{}", "p".repeat(MAX_BYTES))),
            MediaKind::Video,
            Time(30_000_000),
        );
        asset.name = "n".repeat(MAX_BYTES);
        asset.width = 3840;
        asset.height = 2160;
        asset.frame_rate = Some(FrameRate {
            numerator: 24_000,
            denominator: 1001,
        });
        asset.has_audio = true;
        let mut clip = Clip::from_asset(&asset, Time::ZERO);
        clip.name = "Exact clip name".into();
        clip.source_in = Time(1_000_000);
        clip.duration = Time(2_000_000);
        let selected = clip.id;
        let asset_id = asset.id;
        Arc::make_mut(&mut project.tracks[0])
            .clips
            .push(clip.clone());
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        let context = captured(project, vec![selected], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Trim the selected source range").unwrap())
                .unwrap();
        assert_eq!(
            value["project"]["tracks"][0]["clips"][0],
            serde_json::to_value(clip).unwrap()
        );
        let projected = &value["project"]["assets"][asset_id.to_string()];
        assert!(projected["name"].is_null());
        assert!(projected["path"].is_null());
        assert_eq!(projected["omitted_fields"], json!(["name", "path"]));
        assert_eq!(projected["duration"], asset.duration.0);
        assert_eq!(projected["width"], asset.width);
        assert_eq!(projected["height"], asset.height);
        assert_eq!(
            projected["frame_rate"],
            serde_json::to_value(asset.frame_rate).unwrap()
        );
        assert_eq!(projected["has_audio"], true);
    }

    fn opacity_track(count: usize) -> AnimTrack {
        AnimTrack {
            target: None,
            key: ParamKey::Opacity,
            keys: (0..count)
                .map(|index| Keyframe {
                    at: Time(index as i64 * 1_000),
                    value: 0.5,
                    ease: Ease::Linear,
                })
                .collect(),
        }
    }

    fn tracked(project: &mut Project, clip: &mut Clip, count: usize) -> Uuid {
        let tracker = Tracker {
            id: Uuid::new_v4(),
            name: "Face".into(),
            asset_id: clip.asset_id().unwrap(),
            keys: (0..count)
                .map(|index| TrackKey {
                    source: Time(index as i64 * 1_000),
                    center: [0.5, 0.5],
                    scale: 1.0,
                    rotation: 0.0,
                })
                .collect(),
        };
        clip.follow = Some(Follow {
            tracker_id: tracker.id,
            position: true,
            scale: false,
            rotation: false,
            offset: [0.0, 0.0],
        });
        let id = tracker.id;
        Arc::make_mut(&mut project.trackers).insert(id, Arc::new(tracker));
        id
    }

    #[test]
    fn long_key_lists_are_declared_counts_in_a_complete_document() {
        let mut project = Project::new("Keyed");
        let asset = Asset::new(
            PathBuf::from("/media/a.mp4"),
            MediaKind::Video,
            Time(30_000_000),
        );
        let mut clip = Clip::from_asset(&asset, Time::ZERO);
        clip.animation = vec![opacity_track(MAX_INLINE_KEYS + 1), {
            let mut short = opacity_track(MAX_INLINE_KEYS);
            short.key = ParamKey::Rotation;
            short
        }];
        let long_tracker = tracked(&mut project, &mut clip, MAX_INLINE_KEYS + 1);
        let follow = clip.follow.clone();
        let short_tracker = tracked(&mut project, &mut clip, MAX_INLINE_KEYS);
        clip.follow = follow;
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        Arc::make_mut(&mut project.tracks[0])
            .clips
            .push(clip.clone());
        let context = captured(project, vec![clip.id], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Fade the clip").unwrap()).unwrap();
        assert!(value.get("context_scope").is_none());

        let shown = &value["project"]["tracks"][0]["clips"][0];
        assert_eq!(
            shown["animation"][0],
            json!({
                "target": null,
                "key": "opacity",
                "keys": null,
                "key_count": MAX_INLINE_KEYS + 1,
                "omitted_fields": ["keys"],
            })
        );
        assert_eq!(shown["animation"][1], json!(clip.animation[1]));
        // Everything else about the clip is its exact wire shape.
        let mut expected = json!(clip);
        expected["animation"] = shown["animation"].clone();
        assert_eq!(shown, &expected);

        let trackers = &value["project"]["trackers"];
        let summary = &trackers[long_tracker.to_string()];
        assert!(summary["keys"].is_null());
        assert_eq!(summary["key_count"], MAX_INLINE_KEYS + 1);
        assert_eq!(summary["omitted_fields"], json!(["keys"]));
        assert_eq!(summary["name"], "Face");
        assert_eq!(
            trackers[short_tracker.to_string()],
            json!(context.project.trackers[&short_tracker])
        );
        let mut rest = value["project"].clone();
        let mut whole = json!(context.project.as_ref());
        for document in [&mut rest, &mut whole] {
            document["tracks"][0]["clips"] = json!([]);
            document["trackers"] = json!({});
        }
        assert_eq!(rest, whole);
    }

    #[test]
    fn focused_view_summarizes_long_keys_and_lists_followed_trackers() {
        let mut project = large_project();
        let mut clip = project.tracks[0].clips[0].clone();
        clip.animation = vec![opacity_track(MAX_INLINE_KEYS + 1)];
        let followed = tracked(&mut project, &mut clip, MAX_INLINE_KEYS + 1);
        let mut far = project.tracks[0].clips[4999].clone();
        let unseen = tracked(&mut project, &mut far, 2);
        let tracked_asset = far.asset_id().unwrap();
        // A clip far from the playhead follows the selected clip's tracker source.
        let source = project.tracks[0].clips[4000].asset_id().unwrap();
        Arc::make_mut(
            Arc::make_mut(&mut project.trackers)
                .get_mut(&followed)
                .unwrap(),
        )
        .asset_id = source;
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.clips[0] = clip.clone();
        track.clips[4999] = far;
        let context = captured(project, vec![clip.id], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Ease the fade").unwrap()).unwrap();
        assert_eq!(value["context_scope"]["mode"], "focused");
        assert_eq!(value["context_scope"]["total"]["trackers"], 2);
        assert_eq!(value["context_scope"]["omitted"]["trackers"], 1);
        let shown = clips(&value)
            .into_iter()
            .find(|shown| shown["id"] == clip.id.to_string())
            .unwrap();
        assert!(shown["animation"][0]["keys"].is_null());
        assert_eq!(shown["animation"][0]["key_count"], MAX_INLINE_KEYS + 1);
        assert_eq!(shown["animation"][0]["omitted_fields"], json!(["keys"]));
        assert_eq!(shown["follow"], json!(clip.follow));
        let trackers = value["project"]["trackers"].as_object().unwrap();
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[&followed.to_string()]["key_count"], 33);
        assert!(!trackers.contains_key(&unseen.to_string()));
        let assets = value["project"]["assets"].as_object().unwrap();
        assert!(assets.contains_key(&source.to_string()));
        assert!(!assets.contains_key(&tracked_asset.to_string()));
    }

    #[test]
    fn fades_track_volume_and_solo_are_sent_when_set_and_absent_at_their_defaults() {
        let mut project = Project::new("Sound");
        let mut asset = Asset::new(
            PathBuf::from("/media/a.mp4"),
            MediaKind::Video,
            Time(30_000_000),
        );
        asset.has_audio = true;
        let mut faded = Clip::from_asset(&asset, Time::ZERO);
        faded.duration = Time(2_000_000);
        faded.fade_in = Time(250_000);
        faded.fade_out = Time(500_000);
        // A long track takes the summarized shape, which names every field itself.
        let mut summarized = faded.clone();
        summarized.id = Uuid::new_v4();
        summarized.start = Time(2_000_000);
        summarized.animation = vec![opacity_track(MAX_INLINE_KEYS + 1)];
        let mut plain = summarized.clone();
        plain.id = Uuid::new_v4();
        plain.start = Time(4_000_000);
        plain.fade_in = Time::ZERO;
        plain.fade_out = Time::ZERO;
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.gain = 0.5;
        track.solo = true;
        track.clips.extend([faded, summarized, plain]);
        let context = captured(project, vec![], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Fade the music out").unwrap()).unwrap();
        assert!(value.get("context_scope").is_none());

        let tracks = value["project"]["tracks"].as_array().unwrap();
        assert_eq!(tracks[0]["gain"], 0.5);
        assert_eq!(tracks[0]["solo"], true);
        for track in &tracks[1..] {
            assert!(track.get("gain").is_none() && track.get("solo").is_none());
        }
        let shown = tracks[0]["clips"].as_array().unwrap();
        assert!(shown[0].get("animation").is_none());
        assert!(shown[1]["animation"][0]["keys"].is_null());
        for clip in &shown[..2] {
            assert_eq!(clip["fade_in"], 250_000);
            assert_eq!(clip["fade_out"], 500_000);
        }
        assert!(shown[2]["animation"][0]["keys"].is_null());
        assert!(shown[2].get("fade_in").is_none() && shown[2].get("fade_out").is_none());
    }

    #[test]
    fn focused_view_sends_fades_track_volume_and_solo_and_omits_their_defaults() {
        let mut project = large_project();
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.gain = 2.0;
        track.solo = true;
        let faded = &mut track.clips[0];
        faded.fade_in = Time(20_000);
        faded.fade_out = Time(30_000);
        faded.animation = vec![opacity_track(MAX_INLINE_KEYS + 1)];
        let (faded, exact) = (faded.id, &mut track.clips[1]);
        exact.fade_out = Time(40_000);
        let exact = exact.id;
        let context = captured(project, vec![faded, exact], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Shorten the fades").unwrap()).unwrap();
        assert_eq!(value["context_scope"]["mode"], "focused");

        let tracks = value["project"]["tracks"].as_array().unwrap();
        assert!(tracks.len() > 1);
        assert_eq!(tracks[0]["gain"], 2.0);
        assert_eq!(tracks[0]["solo"], true);
        for track in &tracks[1..] {
            assert!(track.get("gain").is_none() && track.get("solo").is_none());
        }
        let shown = clips(&value);
        let clip = |id: ClipId| {
            *shown
                .iter()
                .find(|clip| clip["id"] == id.to_string())
                .unwrap()
        };
        assert!(clip(faded)["animation"][0]["keys"].is_null());
        assert_eq!(clip(faded)["fade_in"], 20_000);
        assert_eq!(clip(faded)["fade_out"], 30_000);
        assert!(clip(exact).get("fade_in").is_none());
        assert_eq!(clip(exact)["fade_out"], 40_000);
        let unfaded = shown
            .iter()
            .find(|clip| clip["id"] != faded.to_string() && clip["id"] != exact.to_string())
            .unwrap();
        assert!(unfaded.get("fade_in").is_none() && unfaded.get("fade_out").is_none());
    }

    #[test]
    fn unknown_selection_cannot_be_represented_as_a_complete_selection() {
        let context = captured(Project::default(), vec![Uuid::new_v4()], Time::ZERO);
        assert!(
            matches!(prompt(&context, "Delete selection"), Err(AiError::Invalid(message)) if message.contains("captured selected clip"))
        );
    }

    #[test]
    fn escaped_required_text_respects_serialized_bytes_and_is_never_trimmed() {
        let mut project = Project::default();
        let clip = Clip::text("\"".repeat(270_000), Time::ZERO, Time(1_000_000));
        let selected = clip.id;
        Arc::make_mut(&mut project.tracks[2]).clips.push(clip);
        let context = captured(project, vec![selected], Time::ZERO);
        assert!(
            matches!(prompt(&context, "Change this title"), Err(AiError::Invalid(message)) if message.contains("complete linked/grouped"))
        );
        assert_eq!(context.project.tracks[2].clips[0].name.len(), 270_000);
    }

    #[test]
    fn a_large_optional_marker_does_not_block_small_relevant_metadata() {
        let mut project = Project::default();
        let large = Marker {
            id: Uuid::new_v4(),
            time: Time::ZERO,
            label: "m".repeat(MAX_BYTES),
            color: [255; 4],
        };
        let small = Marker {
            id: Uuid::new_v4(),
            time: Time(1_000_000),
            label: "Cut".into(),
            color: [255; 4],
        };
        Arc::make_mut(&mut project.markers).extend([large, small.clone()]);
        let context = captured(project, vec![], Time::ZERO);
        let value: Value =
            serde_json::from_str(&prompt(&context, "Inspect nearby markers").unwrap()).unwrap();
        assert_eq!(value["context_scope"]["omitted"]["markers"], 1);
        assert_eq!(value["project"]["markers"], json!([small]));
    }
}
