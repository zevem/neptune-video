use crate::{MIN_SCHEMA_VERSION, SCHEMA_VERSION, StoreError, atomic_json, read_versioned};
use editor_core::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};
use uuid::Uuid;

// Arc serde alone duplicates the entire history on disk. Intern document roots,
// tracks and maps so a thousand transform edits don't repeat unchanged media.
#[derive(Serialize, Deserialize)]
struct Archive {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_binding: Option<crate::document::RecoveryDocumentBinding>,
    revision: u64,
    current: usize,
    saved: Option<usize>,
    projects: Vec<ProjectRecord>,
    tracks: Vec<Track>,
    assets: Vec<BTreeMap<AssetId, Asset>>,
    folders: Vec<BTreeMap<FolderId, MediaFolder>>,
    markers: Vec<Vec<Marker>>,
    // Each map version names its paths by index: a tracker shared by many
    // versions is written once.
    #[serde(default)]
    trackers: Vec<BTreeMap<Uuid, usize>>,
    #[serde(default)]
    tracker_values: Vec<Tracker>,
    past: Vec<Entry>,
    future: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
struct ProjectRecord {
    id: AssetId,
    name: String,
    settings: CanvasSettings,
    assets: usize,
    folders: usize,
    tracks: Vec<usize>,
    markers: usize,
    #[serde(default)]
    trackers: Option<usize>,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    label: String,
    before: usize,
    after: usize,
}

struct Interner<T> {
    values: Vec<T>,
    pointers: HashMap<usize, usize>,
}
impl<T: Clone> Interner<T> {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            pointers: HashMap::new(),
        }
    }
    fn insert(&mut self, value: &Arc<T>) -> usize {
        let ptr = Arc::as_ptr(value) as usize;
        if let Some(id) = self.pointers.get(&ptr) {
            return *id;
        }
        let id = self.values.len();
        self.values.push((**value).clone());
        self.pointers.insert(ptr, id);
        id
    }
}
pub(super) fn save(path: &Path, state: &SessionState) -> Result<(), StoreError> {
    save_with_binding(path, state, None)
}
pub(super) fn save_with_binding(
    path: &Path,
    state: &SessionState,
    document_binding: Option<crate::document::RecoveryDocumentBinding>,
) -> Result<(), StoreError> {
    let mut roots: Vec<Arc<Project>> = Vec::new();
    let mut pointers = HashMap::new();
    let mut root_id = |root: &Arc<Project>| {
        let ptr = Arc::as_ptr(root) as usize;
        *pointers.entry(ptr).or_insert_with(|| {
            let id = roots.len();
            roots.push(root.clone());
            id
        })
    };
    let current = root_id(&state.current);
    let saved = state.saved.as_ref().map(&mut root_id);
    let past = state
        .past
        .iter()
        .map(|h| Entry {
            label: h.label.clone(),
            before: root_id(&h.before),
            after: root_id(&h.after),
        })
        .collect();
    let future = state
        .future
        .iter()
        .map(|h| Entry {
            label: h.label.clone(),
            before: root_id(&h.before),
            after: root_id(&h.after),
        })
        .collect();
    let mut tracks = Interner::new();
    let mut assets = Interner::new();
    let mut folders = Interner::new();
    let mut markers = Interner::new();
    let mut tracker_values = Interner::new();
    let mut trackers: Vec<BTreeMap<Uuid, usize>> = Vec::new();
    let mut tracker_maps = HashMap::new();
    let projects = roots
        .iter()
        .map(|p| ProjectRecord {
            id: p.id,
            name: p.name.clone(),
            settings: p.settings.clone(),
            assets: assets.insert(&p.assets),
            folders: folders.insert(&p.folders),
            tracks: p.tracks.iter().map(|t| tracks.insert(t)).collect(),
            markers: markers.insert(&p.markers),
            trackers: Some(
                *tracker_maps
                    .entry(Arc::as_ptr(&p.trackers) as usize)
                    .or_insert_with(|| {
                        trackers.push(
                            p.trackers
                                .iter()
                                .map(|(id, tracker)| (*id, tracker_values.insert(tracker)))
                                .collect(),
                        );
                        trackers.len() - 1
                    }),
            ),
        })
        .collect();
    atomic_json(
        path,
        &Archive {
            schema_version: SCHEMA_VERSION,
            document_binding,
            revision: state.revision,
            current,
            saved,
            projects,
            tracks: tracks.values,
            assets: assets.values,
            folders: folders.values,
            markers: markers.values,
            trackers,
            tracker_values: tracker_values.values,
            past,
            future,
        },
    )
}
fn get<T>(values: &[Arc<T>], index: usize) -> Result<Arc<T>, StoreError> {
    values
        .get(index)
        .cloned()
        .ok_or_else(|| StoreError::Invalid("Broken recovery reference".into()))
}
pub(super) fn load(path: &Path) -> Result<Editor, StoreError> {
    let base = path
        .canonicalize()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    decode(read_versioned(path)?, Some(&base))
}
pub(super) fn decode(value: serde_json::Value, base: Option<&Path>) -> Result<Editor, StoreError> {
    decode_with_binding(value, base).map(|(editor, _)| editor)
}
pub(super) fn load_with_document(
    path: &Path,
    destination: &Path,
) -> Result<crate::RecoveredSession, StoreError> {
    let base = path
        .canonicalize()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let (editor, binding) = decode_with_binding(read_versioned(path)?, Some(&base))?;
    let restored = binding
        .map(|binding| binding.restore(destination))
        .transpose()?
        .flatten();
    let (document, document_changed) = match restored {
        Some((document, changed)) => (Some(document), changed),
        None => (None, false),
    };
    let editor = if document.is_none() || document_changed {
        let mut state = editor.session_state();
        state.saved = None;
        Editor::from_session(state)?
    } else {
        editor
    };
    Ok(crate::RecoveredSession {
        editor,
        document,
        document_changed,
    })
}
fn decode_with_binding(
    value: serde_json::Value,
    base: Option<&Path>,
) -> Result<(Editor, Option<crate::document::RecoveryDocumentBinding>), StoreError> {
    let archive: Archive = serde_json::from_value(value)?;
    if archive.schema_version > SCHEMA_VERSION {
        return Err(StoreError::FutureSchema(archive.schema_version));
    }
    if archive.schema_version < MIN_SCHEMA_VERSION {
        return Err(StoreError::Invalid("Unsupported recovery schema".into()));
    }
    if let Some(binding) = &archive.document_binding {
        binding.validate()?;
    }
    let tracks: Vec<_> = archive.tracks.into_iter().map(Arc::new).collect();
    let assets: Vec<_> = archive
        .assets
        .into_iter()
        .map(|mut map| {
            if let Some(base) = base {
                for asset in map.values_mut() {
                    if asset.path.is_relative() {
                        asset.path = base.join(&asset.path);
                    }
                }
            }
            Arc::new(map)
        })
        .collect();
    let folders: Vec<_> = archive.folders.into_iter().map(Arc::new).collect();
    let markers: Vec<_> = archive.markers.into_iter().map(Arc::new).collect();
    let tracker_values: Vec<_> = archive.tracker_values.into_iter().map(Arc::new).collect();
    let trackers = archive
        .trackers
        .into_iter()
        .map(|map| {
            map.into_iter()
                .map(|(id, index)| Ok((id, get(&tracker_values, index)?)))
                .collect::<Result<BTreeMap<_, _>, StoreError>>()
                .map(Arc::new)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let untracked = Arc::new(BTreeMap::new());
    let mut projects = Vec::new();
    for p in archive.projects {
        projects.push(Arc::new(Project {
            id: p.id,
            name: p.name,
            settings: p.settings,
            assets: get(&assets, p.assets)?,
            folders: get(&folders, p.folders)?,
            tracks: p
                .tracks
                .into_iter()
                .map(|i| get(&tracks, i))
                .collect::<Result<_, _>>()?,
            markers: get(&markers, p.markers)?,
            trackers: match p.trackers {
                Some(index) => get(&trackers, index)?,
                None => untracked.clone(),
            },
        }));
    }
    let entries = |entries: Vec<Entry>| -> Result<Vec<HistoryEntry>, StoreError> {
        entries
            .into_iter()
            .map(|h| {
                Ok(HistoryEntry {
                    label: h.label,
                    before: get(&projects, h.before)?,
                    after: get(&projects, h.after)?,
                })
            })
            .collect()
    };
    Ok((
        Editor::from_session(SessionState {
            current: get(&projects, archive.current)?,
            saved: archive.saved.map(|i| get(&projects, i)).transpose()?,
            past: entries(archive.past)?,
            future: entries(archive.future)?,
            revision: archive.revision,
        })?,
        archive.document_binding,
    ))
}
