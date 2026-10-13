//! Derived interval lookup for an immutable project root. This is not saved
//! project state: replacing a track rebuilds only that track's prefix bounds.
use crate::{Clip, Project, Time, Track};
use std::{collections::HashMap, sync::Arc};

struct IndexedTrack {
    track: Arc<Track>,
    /// Monotonic maximum end at each start-sorted clip. Unlike individual ends,
    /// these bounds remain searchable when a long clip contains shorter clips.
    prefix_ends: Vec<Time>,
}

impl IndexedTrack {
    fn new(track: Arc<Track>) -> Self {
        let mut end = Time::ZERO;
        let prefix_ends = track
            .clips
            .iter()
            .map(|clip| {
                end = end.max(clip.end());
                end
            })
            .collect();
        Self { track, prefix_ends }
    }

    fn active(&self, time: Time) -> impl Iterator<Item = &Clip> {
        let last = self.track.clips.partition_point(|clip| clip.start <= time);
        let first = self.prefix_ends[..last].partition_point(|end| *end <= time);
        self.track.clips[first..last]
            .iter()
            .filter(move |clip| time < clip.end())
    }
}

/// A query view tied to an owned immutable project snapshot. Clip and track
/// order, hidden-track exclusion and half-open endpoints match
/// [`Project::active_clips`]. The source must have validated, sorted tracks.
///
/// Construction takes O(n) time and one `Time` per clip. Warm queries allocate
/// nothing and skip prefixes whose maximum end is already behind the playhead.
/// Nested long clips can still require visiting a start-qualified suffix.
pub struct ProjectQuery {
    project: Arc<Project>,
    tracks: Vec<IndexedTrack>,
}

impl ProjectQuery {
    pub fn new(project: Arc<Project>) -> Self {
        let tracks = project
            .tracks
            .iter()
            .cloned()
            .map(IndexedTrack::new)
            .collect();
        Self { project, tracks }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Reuse bounds only when the exact immutable track root is unchanged.
    /// Undo, redo, same-ID replacements and reordered tracks all use this seam.
    pub fn update(&mut self, project: Arc<Project>) {
        if Arc::ptr_eq(&self.project, &project) {
            return;
        }
        let mut previous: HashMap<_, _> = std::mem::take(&mut self.tracks)
            .into_iter()
            .map(|indexed| (indexed.track.id, indexed))
            .collect();
        self.tracks = project
            .tracks
            .iter()
            .map(|track| {
                previous
                    .remove(&track.id)
                    .filter(|indexed| Arc::ptr_eq(&indexed.track, track))
                    .unwrap_or_else(|| IndexedTrack::new(Arc::clone(track)))
            })
            .collect();
        self.project = project;
    }

    pub fn active_clips(&self, time: Time) -> impl Iterator<Item = (&Track, &Clip)> {
        self.tracks
            .iter()
            .filter(|indexed| !indexed.track.hidden)
            .flat_map(move |indexed| {
                indexed
                    .active(time)
                    .map(move |clip| (indexed.track.as_ref(), clip))
            })
    }
}
