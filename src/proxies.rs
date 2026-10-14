//! The proxy lane as the window uses it: which videos get a proxy made, what
//! the viewer is told there is of each, and which proxy files it has open.
//!
//! A proxy is derived data. What there is of one lives here and in the
//! viewer's textures, never in a project; the viewer reads one only for its
//! pictures (see [`crate::textures`]), and audio, analysis, speech, titles
//! and export read the original. Asking never waits on the worker: a request
//! the lane has no room for is asked again at the next occasion to ask, which
//! is a change of the library, of the project or of the mode, or a proxy
//! finishing, never a loop.
use crate::{
    settings::{PreviewSettings, ProxyMode},
    textures::Textures,
};
use editor_core::{Asset, AssetId, MediaKind, Project};
use editor_media::{
    MediaError, ProxyEvent, ProxyService, ProxyStatus, SourceIdentity, wants_proxy,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// The videos of `project` `mode` asks a proxy of: none when off, those
/// large or heavy enough at `Auto`, and every video with a picture at
/// `Always`.
pub fn wanted(project: &Project, mode: ProxyMode) -> impl Iterator<Item = &Asset> {
    project.assets.values().filter(move |asset| match mode {
        ProxyMode::Off => false,
        ProxyMode::Auto => wants_proxy(asset),
        ProxyMode::Always => asset.kind == MediaKind::Video && asset.width > 0 && asset.height > 0,
    })
}

/// Whether `source` is what `asset` says of its file now.
fn is_of(source: &SourceIdentity, asset: &Asset) -> bool {
    *source == SourceIdentity::of(asset)
}

/// What the proxy cache holds, for the settings sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cache {
    /// The lane is running: there is a cache at all.
    pub available: bool,
    pub bytes: u64,
    pub proxies: usize,
}

/// What came of the lane since it was last looked at.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    /// A proxy became ready: what the viewer reads may change.
    pub ready: bool,
    /// Proxies asked for by hand that could not be made, said once each.
    pub failures: Vec<String>,
}

pub struct Proxies {
    service: Option<ProxyService>,
    problem: Option<String>,
    /// The budget last told the lane, in bytes.
    budget: u64,
    /// The proxy files last said to be open in a decoder.
    open: Vec<PathBuf>,
    /// The cache's directory had been read when the statuses were last
    /// looked up: before then the lane knows none of the proxies it has.
    loaded: bool,
    /// A request was refused for want of room in the lane's queue.
    behind: bool,
    /// Assets whose proxy the person removed, or whose cache they cleared:
    /// none is made of them again by the mode until they ask for one or the
    /// project is opened again.
    declined: HashSet<AssetId>,
    /// Assets whose proxy the person asked for, whose failure is said.
    by_hand: HashSet<AssetId>,
}

impl Proxies {
    /// A lane keeping its proxies in `dir`, held to the budget `preview`
    /// says. One that cannot start leaves the preview reading originals and
    /// says why in [`Self::problem`].
    pub fn start(
        wake: impl Fn() + Send + Sync + 'static,
        dir: PathBuf,
        preview: &PreviewSettings,
    ) -> Self {
        let budget = preview.proxy_budget();
        let (service, problem) = match ProxyService::new(wake, dir, budget) {
            Ok(service) => (Some(service), None),
            Err(error) => (None, Some(format!("Proxies are unavailable: {error}"))),
        };
        Self {
            service,
            problem,
            budget,
            open: Vec::new(),
            loaded: false,
            behind: false,
            declined: HashSet::new(),
            by_hand: HashSet::new(),
        }
    }

    /// A window with no proxies at all.
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            service: None,
            problem: None,
            budget: 0,
            open: Vec::new(),
            loaded: false,
            behind: false,
            declined: HashSet::new(),
            by_hand: HashSet::new(),
        }
    }

    /// Why proxies are not made or read here, when they are not.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// What the cache holds now.
    pub fn cache(&self) -> Cache {
        let Some(service) = &self.service else {
            return Cache::default();
        };
        let (bytes, proxies) = service.usage();
        Cache {
            available: true,
            bytes,
            proxies,
        }
    }

    /// Looks up what there is of a proxy of every video of `project` and
    /// tells `textures`. Memory alone is read.
    fn refresh(&self, project: &Project, textures: &mut Textures) {
        let Some(service) = &self.service else {
            return;
        };
        for asset in project.assets.values() {
            if asset.kind == MediaKind::Video {
                textures.set_proxy(asset.id, service.lookup(asset));
            }
        }
    }

    /// Asks for the proxies `preview`'s mode wants of `project` that are not
    /// ready, waiting, being made or failed already, until the lane has no
    /// room: the rest are asked at the next occasion.
    fn request(&mut self, project: &Project, preview: &PreviewSettings, textures: &mut Textures) {
        let Some(service) = &self.service else {
            return;
        };
        self.behind = false;
        for asset in wanted(project, preview.proxies) {
            if self.declined.contains(&asset.id)
                || !matches!(textures.proxy_status(asset.id), ProxyStatus::None)
            {
                continue;
            }
            match service.request(asset) {
                Ok(()) => textures.set_proxy(asset.id, service.lookup(asset)),
                Err(MediaError::Busy) => {
                    self.behind = true;
                    return;
                }
                Err(MediaError::Closed) => return,
                // Nothing the lane can make a proxy of.
                Err(_) => {}
            }
        }
    }

    /// The library changed: assets came, went or are of other files now.
    pub fn library_changed(
        &mut self,
        project: &Project,
        preview: &PreviewSettings,
        textures: &mut Textures,
    ) {
        self.refresh(project, textures);
        self.request(project, preview, textures);
    }

    /// Another project replaced the one open: what was asked for it is given
    /// up, and what the new one wants is asked.
    pub fn replaced(
        &mut self,
        project: &Project,
        preview: &PreviewSettings,
        textures: &mut Textures,
    ) {
        if let Some(service) = &self.service {
            service.cancel_all();
        }
        textures.forget_proxies();
        self.declined.clear();
        self.by_hand.clear();
        self.library_changed(project, preview, textures);
    }

    /// The mode changed to what `preview` says. Off gives up what waits; any
    /// other asks for what it wants.
    pub fn mode_changed(
        &mut self,
        project: &Project,
        preview: &PreviewSettings,
        textures: &mut Textures,
    ) {
        if preview.proxies == ProxyMode::Off {
            if let Some(service) = &self.service {
                service.cancel_all();
            }
            self.by_hand.clear();
        }
        textures.read_proxies(self.reads(preview));
        self.library_changed(project, preview, textures);
    }

    /// Whether the viewer reads proxies.
    fn reads(&self, preview: &PreviewSettings) -> bool {
        self.service.is_some() && preview.proxies != ProxyMode::Off
    }

    /// The person asked for a proxy of `asset`.
    pub fn create(&mut self, asset: &Asset, textures: &mut Textures) -> Result<(), String> {
        let Some(service) = &self.service else {
            return Err(self
                .problem
                .clone()
                .unwrap_or_else(|| "Proxies are unavailable".to_owned()));
        };
        match service.request(asset) {
            Ok(()) => {
                self.declined.remove(&asset.id);
                self.by_hand.insert(asset.id);
                // Asked for again, it is tried again, whatever its file.
                textures.forget_unreadable(asset.id);
                textures.set_proxy(asset.id, service.lookup(asset));
                Ok(())
            }
            Err(MediaError::Busy) => {
                Err("Too many proxies are waiting to be made. Try again when some are done.".into())
            }
            Err(error) => Err(format!("No proxy can be made of {}: {error}", asset.name)),
        }
    }

    /// The person removed the proxy of `asset`. The mode makes none of it
    /// again until they ask.
    pub fn remove(&mut self, asset: &Asset, textures: &mut Textures) {
        let Some(service) = &self.service else {
            return;
        };
        service.remove(asset);
        self.declined.insert(asset.id);
        self.by_hand.remove(&asset.id);
        textures.forget_unreadable(asset.id);
        textures.set_proxy(asset.id, ProxyStatus::None);
    }

    /// The person emptied the cache. Those of `project` are not made again
    /// by the mode until it is opened again or they ask.
    pub fn clear(&mut self, project: &Project, textures: &mut Textures) {
        let Some(service) = &self.service else {
            return;
        };
        service.clear();
        self.declined.extend(
            (project.assets.values())
                .filter(|asset| asset.kind == MediaKind::Video)
                .map(|asset| asset.id),
        );
        self.by_hand.clear();
        textures.forget_proxies();
    }

    /// Stops the lane for a closing window: what waits is given up and the
    /// proxy being written is stopped and its part file removed, waiting a
    /// moment at most. See [`ProxyService::shutdown`].
    pub fn shutdown(&self) {
        if let Some(service) = &self.service {
            service.shutdown();
        }
    }

    /// Holds the cache to `preview`'s budget, when that changed.
    pub fn set_budget(&mut self, preview: &PreviewSettings) {
        let budget = preview.proxy_budget();
        if let Some(service) = &self.service
            && budget != self.budget
        {
            service.set_budget(budget);
            self.budget = budget;
        }
    }

    /// Takes what the lane published, tells `textures` the person's choices
    /// and what there is of each proxy of `project`, and tells the lane
    /// which proxy files the viewer has open. Called before every pass;
    /// nothing in it waits.
    pub fn take(
        &mut self,
        project: &Project,
        preview: &PreviewSettings,
        textures: &mut Textures,
    ) -> Taken {
        textures.set_quality(preview.quality);
        textures.read_proxies(self.reads(preview));
        let mut taken = Taken::default();
        let Some(service) = &self.service else {
            return taken;
        };
        if !self.loaded && service.loaded() {
            // The proxies the cache had are known from now on.
            self.loaded = true;
            self.refresh(project, textures);
        }
        let mut finished = false;
        while let Some(event) = service.poll() {
            match event {
                ProxyEvent::Progress {
                    asset,
                    source,
                    done,
                } => {
                    if (project.assets.get(&asset)).is_some_and(|now| is_of(&source, now)) {
                        textures.set_proxy(asset, ProxyStatus::Generating(done));
                    }
                }
                ProxyEvent::Ready { asset, proxy } => {
                    finished = true;
                    if (project.assets.get(&asset)).is_some_and(|now| is_of(&proxy.source, now)) {
                        self.by_hand.remove(&asset);
                        textures.set_proxy(asset, ProxyStatus::Ready(proxy));
                        taken.ready = true;
                    }
                }
                ProxyEvent::Failed {
                    asset,
                    source,
                    message,
                } => {
                    finished = true;
                    let Some(now) = project.assets.get(&asset) else {
                        continue;
                    };
                    if is_of(&source, now) {
                        if self.by_hand.remove(&asset) {
                            taken
                                .failures
                                .push(format!("No proxy was made of {}: {message}", now.name));
                        }
                        textures.set_proxy(asset, ProxyStatus::Failed(message));
                    }
                }
                ProxyEvent::Removed { proxies } => {
                    // The budget took them out: what is said ready of them
                    // is what the lane has now, usually none.
                    for asset in project.assets.values() {
                        let gone = matches!(
                            textures.proxy_status(asset.id),
                            ProxyStatus::Ready(ready)
                                if proxies.iter().any(|proxy| proxy.path == ready.path)
                        );
                        if gone {
                            textures.set_proxy(asset.id, service.lookup(asset));
                        }
                    }
                }
            }
        }
        // Room was made in the lane's queue: what it refused is asked again.
        if finished && self.behind {
            self.request(project, preview, textures);
        }
        let open = textures.proxies_open();
        if open != self.open
            && let Some(service) = &self.service
        {
            service.set_in_use(&open);
            self.open = open;
        }
        taken
    }

    /// The proxy files last said to be open.
    #[cfg(test)]
    pub fn told_open(&self) -> &[PathBuf] {
        &self.open
    }

    /// Where a window keeps its proxies: see [`crate::settings::proxy_dir`].
    pub fn dir(told_settings: Option<&Path>) -> PathBuf {
        crate::settings::proxy_dir(told_settings)
    }
}

#[cfg(test)]
#[path = "proxies_tests.rs"]
mod tests;
