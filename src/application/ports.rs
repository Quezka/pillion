//! What the Engine needs from the outside world, as traits. The web services behind them live in `infra`, and the
//! wiring (`crate::wiring`) hands the Engine real ones; tests can hand it their own.
//!
//! The phone does the actual fetching (the Engine only emits a Command with the address), so a port is two halves:
//! the request to make, and how to read the answer.

use crate::domain::geo::LatLon;
use crate::domain::places::Place;
use crate::domain::speed_limit::Road;

use super::nav::Route;

/// Places, routes, the roads around the rider and how far the battery reaches.
pub trait MapServices: Send {
    /// `nearby` also accepts roads around the destination, for places a moped can't drive right up to. `heading` is
    /// the way the rider is already going, so a new route doesn't begin with a U-turn. `lang` is "en", "it" or "ru".
    fn route_url(&self, from: LatLon, to: LatLon, nearby: bool, heading: Option<f64>, lang: &str) -> String;
    fn parse_route(&self, json: &str, destination: &str) -> Result<Route, String>;
    /// The place search, with `near` (where the rider is) to put close places first.
    fn search_url(&self, query: &str, near: Option<LatLon>, lang: &str) -> String;
    fn parse_search(&self, json: &str) -> Result<Vec<Place>, String>;
    /// The road under the rider, going `heading`.
    fn road_url(&self, at: LatLon, heading: Option<f64>) -> String;
    fn parse_road(&self, json: &str) -> Result<Option<Road>, String>;
    /// The roads (and their limits) along a route, one entry per point of `route.points`.
    fn route_roads_url(&self, route: &Route) -> String;
    fn parse_route_roads(&self, json: &str, points: usize) -> Result<Vec<Option<Road>>, String>;
    /// The area reachable on roads within `km`, as outlines.
    fn reach_url(&self, from: LatLon, km: f64) -> String;
    fn parse_reach(&self, json: &str) -> Result<Vec<Vec<LatLon>>, String>;
}

/// A newer version, when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    /// The file's SHA-256 as the publisher reports it, when it does.
    pub sha256: String,
    pub notes: String,
}

/// Where new versions are published.
pub trait ReleaseFeed: Send {
    fn latest_url(&self) -> String;
    /// `Some` when the reply holds a version newer than `current`.
    fn parse_latest(&self, json: &str, current: &str) -> Result<Option<Release>, String>;
}
