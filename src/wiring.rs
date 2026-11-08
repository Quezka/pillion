//! The composition root: the only place that knows every layer and plugs the real services into the Engine.

use crate::application::engine::Engine;
use crate::infra::osm::Osm;
use crate::infra::update::GitHubReleases;

impl Engine {
    /// The Engine with the real web services: OpenStreetMap's for maps, GitHub's for updates.
    pub fn new() -> Self {
        Self::with(Box::new(Osm), Box::new(GitHubReleases))
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
