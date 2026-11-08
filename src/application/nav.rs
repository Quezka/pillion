//! A route and following it by GPS: where the rider is along it, what to say and when. Getting the route is the
//! `MapServices` port's job (`infra::osm`).

use crate::domain::geo::EARTH_RADIUS_M;
pub use crate::domain::geo::{bearing_deg, distance_m, LatLon};
pub use crate::domain::places::Place;
use crate::locale::tf;
pub const TOP_SPEED_KMH: u32 = 45;
pub const EARLY_ANNOUNCE_M: f64 = 300.0;
pub const FINAL_ANNOUNCE_M: f64 = 60.0;
/// How much warning a turn gets, in time, so that it's as useful at 20 km/h as at 45.
const EARLY_ANNOUNCE_S: f64 = 24.0;
const FINAL_ANNOUNCE_S: f64 = 5.0;
const EARLY_LIMITS_M: (f64, f64) = (150.0, 350.0);
const FINAL_LIMITS_M: (f64, f64) = (30.0, 70.0);
/// After a turn, a long way to the next one is worth saying, so the rider knows all is well.
const LONG_STRETCH_M: f64 = 1500.0;
pub const OFF_ROUTE_M: f64 = 50.0;
pub const OFF_ROUTE_FIXES: u32 = 5;
const REROUTE_RETRY_FIXES: u32 = 30;
/// What the routing service says when it can't reach a point with this kind of vehicle.
pub const NO_PATH_ERROR: &str = "No path could be found";
const ARRIVED_M: f64 = 25.0;
/// How far ahead along the route a GPS fix may jump. Keeps a road that doubles back from
/// confusing the progress.
const SEARCH_AHEAD_M: f64 = 600.0;


#[derive(Clone, Debug, Default)]
pub struct Maneuver {
    pub kind: u32,
    pub text: String,
    pub spoken_before: String,
    pub spoken_at: String,
    /// Said once the turn is done, such as "continue for 3 km".
    pub spoken_after: String,
    /// The next turn comes so soon that this one's instruction already mentions it.
    pub joined_to_next: bool,
    /// Which exit to take, on a roundabout.
    pub exit: u32,
    pub begin: usize,
}

impl Maneuver {
    pub fn is_arrival(&self) -> bool {
        matches!(self.kind, 4..=6)
    }

    pub fn is_roundabout(&self) -> bool {
        matches!(self.kind, 26 | 27)
    }

    /// The arrow for this step. Valhalla's numbers: ramps are 17 to 19, exits 20 and 21, forks 22 to 24, merges 25, 37, 38.
    pub fn symbol(&self) -> &'static str {
        match self.kind {
            4..=6 => "flag",
            9 | 18 | 23 | 37 => "turn-slight-right",
            10 | 20 => "turn-right",
            11 => "turn-sharp-right",
            12 => "u-turn-right",
            13 => "u-turn-left",
            14 => "turn-sharp-left",
            15 | 21 => "turn-left",
            16 | 19 | 24 | 38 => "turn-slight-left",
            _ => "turn-straight",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Route {
    pub destination: String,
    pub points: Vec<LatLon>,
    pub along_m: Vec<f64>,
    pub maneuvers: Vec<Maneuver>,
    pub total_m: f64,
    pub total_s: f64,
}

impl Route {
    pub fn new(destination: &str, points: Vec<LatLon>, maneuvers: Vec<Maneuver>, total_s: f64) -> Self {
        let mut along_m = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                total += distance_m(points[i - 1], *p);
            }
            along_m.push(total);
        }
        Self {
            destination: destination.to_string(),
            points,
            along_m,
            maneuvers,
            total_m: total,
            total_s,
        }
    }
}

/// Where a GPS fix sits on the route: how far from it, and how far along.
fn locate(route: &Route, at: LatLon, from_m: f64) -> (f64, f64) {
    let scale = at.0.to_radians().cos();
    let project = |p: LatLon| ((p.1 - at.1).to_radians() * scale * EARTH_RADIUS_M, (p.0 - at.0).to_radians() * EARTH_RADIUS_M);
    let mut best = (f64::MAX, from_m);
    for i in 0..route.points.len() - 1 {
        if route.along_m[i + 1] < from_m - OFF_ROUTE_M {
            continue;
        }
        if route.along_m[i] > from_m + SEARCH_AHEAD_M {
            break;
        }
        let (ax, ay) = project(route.points[i]);
        let (bx, by) = project(route.points[i + 1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 { (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let (cx, cy) = (ax + t * dx, ay + t * dy);
        let d = (cx * cx + cy * cy).sqrt();
        if d < best.0 {
            best = (d, route.along_m[i] + t * (route.along_m[i + 1] - route.along_m[i]));
        }
    }
    best
}

#[derive(Clone, Debug, PartialEq)]
pub enum NavEvent {
    Say { text: String },
    Arrived,
    Reroute,
}

/// Following a route. `next` is the maneuver the rider is heading for.
#[derive(Clone, Debug)]
pub struct Navigation {
    pub route: Route,
    pub along_m: f64,
    pub next: usize,
    early_said: bool,
    final_said: bool,
    off_route_fixes: u32,
    reroute_wait_fixes: u32,
    pub off_route: bool,
}

/// Rounds a distance for speech, to the nearest 50 m.
fn spoken_metres(m: f64) -> u32 {
    ((m / 50.0).round() as u32).max(1) * 50
}

impl Navigation {
    pub fn new(route: Route) -> Self {
        Self {
            next: route.maneuvers.len().min(1),
            route,
            along_m: 0.0,
            early_said: false,
            final_said: false,
            off_route_fixes: 0,
            reroute_wait_fixes: 0,
            off_route: false,
        }
    }

    pub fn current(&self) -> Option<&Maneuver> {
        self.route.maneuvers.get(self.next)
    }

    fn maneuver_along(&self, index: usize) -> f64 {
        let m = &self.route.maneuvers[index];
        self.route.along_m[m.begin.min(self.route.along_m.len() - 1)]
    }

    pub fn distance_to_next_m(&self) -> f64 {
        if self.next >= self.route.maneuvers.len() {
            return (self.route.total_m - self.along_m).max(0.0);
        }
        (self.maneuver_along(self.next) - self.along_m).max(0.0)
    }

    /// The index of the route point the rider has just passed.
    pub fn point_index(&self) -> usize {
        self.route.along_m.partition_point(|&a| a <= self.along_m).saturating_sub(1)
    }

    pub fn remaining_m(&self) -> f64 {
        (self.route.total_m - self.along_m).max(0.0)
    }

    pub fn remaining_s(&self) -> f64 {
        if self.route.total_m <= 0.0 {
            return 0.0;
        }
        self.route.total_s * self.remaining_m() / self.route.total_m
    }

    /// A new route couldn't be fetched, so try again a little later and not at every fix.
    pub fn reroute_failed(&mut self) {
        self.off_route = false;
        self.off_route_fixes = 0;
        self.reroute_wait_fixes = REROUTE_RETRY_FIXES;
    }

    /// The opening announcement, spoken once when the route starts.
    pub fn opening(&self) -> String {
        let km = self.route.total_m / 1000.0;
        let minutes = (self.route.total_s / 60.0).round().max(1.0);
        let first = self.route.maneuvers.first().map_or("", |m| m.spoken_before.as_str());
        tf("Route ready, {} kilometres, about {} minutes. {}", &[&format!("{km:.1}"), &format!("{minutes:.0}"), &first])
    }

    /// Where a position lands on the route's line, as a share of its index, for drawing what's been ridden.
    pub fn index_along(&self) -> usize {
        self.route.along_m.partition_point(|a| *a < self.along_m).min(self.route.points.len().saturating_sub(1))
    }

    pub fn update(&mut self, at: LatLon, speed_mps: Option<f64>) -> Vec<NavEvent> {
        let mut events = Vec::new();
        let (off_by, along) = locate(&self.route, at, self.along_m);
        if off_by > OFF_ROUTE_M {
            self.off_route_fixes += 1;
            if self.off_route_fixes >= OFF_ROUTE_FIXES + self.reroute_wait_fixes && !self.off_route {
                self.off_route = true;
                events.push(NavEvent::Reroute);
            }
            return events;
        }
        self.off_route_fixes = 0;
        self.reroute_wait_fixes = 0;
        self.off_route = false;
        self.along_m = self.along_m.max(along);

        let mut just_passed = None;
        while self.next < self.route.maneuvers.len() {
            let is_last = self.next + 1 == self.route.maneuvers.len();
            if is_last || self.maneuver_along(self.next) > self.along_m {
                break;
            }
            just_passed = Some(self.next);
            // If the last instruction already mentioned this turn, it doesn't need warning about again.
            self.early_said = self.route.maneuvers[self.next].joined_to_next;
            self.next += 1;
            self.final_said = false;
        }

        let Some(m) = self.current().cloned() else {
            return events;
        };
        let distance = self.distance_to_next_m();
        if m.is_arrival() && distance <= ARRIVED_M {
            events.push(NavEvent::Arrived);
            return events;
        }
        if let Some(passed) = just_passed {
            let after = &self.route.maneuvers[passed].spoken_after;
            if !after.is_empty() && distance > LONG_STRETCH_M {
                events.push(NavEvent::Say { text: after.clone() });
            }
        }
        let (early_m, final_m) = speed_mps.map_or((EARLY_ANNOUNCE_M, FINAL_ANNOUNCE_M), |s| {
            ((s * EARLY_ANNOUNCE_S).clamp(EARLY_LIMITS_M.0, EARLY_LIMITS_M.1), (s * FINAL_ANNOUNCE_S).clamp(FINAL_LIMITS_M.0, FINAL_LIMITS_M.1))
        });
        if distance <= final_m {
            if !self.final_said {
                self.final_said = true;
                self.early_said = true;
                events.push(NavEvent::Say { text: m.spoken_at });
            }
        } else if distance <= early_m && !self.early_said {
            self.early_said = true;
            events.push(NavEvent::Say {
                text: tf("In {} metres, {}", &[&spoken_metres(distance), &m.spoken_before]),
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::osm::decode_polyline6;
    use crate::application::ports::MapServices as _;
    use crate::infra::osm::Osm;
    fn parse_route(json: &str, d: &str) -> Result<Route, String> { Osm.parse_route(json, d) }
    fn parse_search(json: &str) -> Result<Vec<Place>, String> { Osm.parse_search(json) }
    fn route_request_url(from: LatLon, to: LatLon, nearby: bool, heading: Option<f64>) -> String {
        Osm.route_url(from, to, nearby, heading, "en")
    }
    fn search_request_url(query: &str, near: Option<LatLon>) -> String { Osm.search_url(query, near, "en") }

    fn step(kind: u32, begin: usize, name: &str) -> Maneuver {
        Maneuver {
            kind,
            text: name.to_string(),
            spoken_before: format!("{name} soon"),
            spoken_at: name.to_string(),
            begin,
            ..Default::default()
        }
    }

    /// Two 1 km legs: north, then east. Latitude and longitude step about 111 m and 80 m.
    fn sample() -> Route {
        let mut points: Vec<LatLon> = (0..=9).map(|i| (48.0 + f64::from(i) * 0.001, 2.0)).collect();
        let corner = points[9];
        points.extend((1..=12).map(|i| (corner.0, 2.0 + f64::from(i) * 0.001)));
        Route::new(
            "Test",
            points,
            vec![step(1, 0, "Start"), step(10, 9, "Turn right"), step(4, 21, "Arrive")],
            240.0,
        )
    }

    #[test]
    fn polyline_decodes_the_documented_example() {
        // Google's documented example, at the sixth decimal scale.
        let points = decode_polyline6("_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        assert_eq!(points.len(), 3);
        assert!((points[0].0 - 3.85).abs() < 1e-6);
        assert!((points[0].1 + 12.02).abs() < 1e-6);
        assert!((points[2].1 + 12.6453).abs() < 1e-3);
    }

    #[test]
    fn truncated_polyline_is_safe() {
        assert!(decode_polyline6("_p~iF").len() <= 1);
        assert!(decode_polyline6("").is_empty());
    }

    #[test]
    fn parses_a_valhalla_reply() {
        let json = r#"{"trip":{"summary":{"time":120.0},"legs":[{"shape":"_p~iF~ps|U_ulLnnqC",
          "maneuvers":[{"type":1,"instruction":"Drive east.","begin_shape_index":0},
          {"type":4,"instruction":"You have arrived.","verbal_pre_transition_instruction":"You will arrive.",
           "begin_shape_index":1}]}]}}"#;
        let r = parse_route(json, "Home").unwrap();
        assert_eq!(r.destination, "Home");
        assert_eq!(r.maneuvers.len(), 2);
        assert_eq!(r.maneuvers[1].spoken_before, "You will arrive.");
        assert_eq!(r.maneuvers[0].spoken_at, "Drive east.");
        assert_eq!(r.total_s, 120.0);
        assert!(parse_route(r#"{"error":"No path could be found"}"#, "x").is_err());
        assert!(parse_route("not json", "x").is_err());
    }

    #[test]
    fn parses_search_results() {
        let json = r#"[{"lat":"48.5","lon":"2.25","display_name":"Somewhere"},{"lat":"bad","lon":"2","display_name":"X"}]"#;
        let places = parse_search(json).unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].at, (48.5, 2.25));
    }

    #[test]
    fn urls_are_encoded_and_ask_for_the_scooter_profile() {
        let url = route_request_url((48.1, 2.2), (48.3, 2.4), false, None);
        assert!(url.contains("motor_scooter"));
        assert!(!url.contains("radius"));
        assert!(route_request_url((48.1, 2.2), (48.3, 2.4), true, None).contains("radius%22%3A300"));
        assert!(url.contains("top_speed%22%3A45"));
        assert!(!url.contains(' '));
        assert!(search_request_url("a b&c", None).ends_with("q=a%20b%26c"));
    }

    #[test]
    fn announces_each_turn_once_early_then_final_then_arrives() {
        let mut nav = Navigation::new(sample());
        assert!(nav.update((48.0, 2.0), None).is_empty());
        let mut said = Vec::new();
        let mut arrived = false;
        let mut path: Vec<LatLon> = Vec::new();
        for p in nav.route.points.clone().windows(2) {
            for i in 0..5 {
                let t = f64::from(i) / 5.0;
                path.push((p[0].0 + (p[1].0 - p[0].0) * t, p[0].1 + (p[1].1 - p[0].1) * t));
            }
        }
        path.push(*nav.route.points.last().unwrap());
        for at in path {
            for e in nav.update(at, None) {
                match e {
                    NavEvent::Say { text } => said.push(text),
                    NavEvent::Arrived => arrived = true,
                    NavEvent::Reroute => panic!("on the route"),
                }
            }
        }
        assert!(arrived);
        assert_eq!(said.iter().filter(|s| s.starts_with("In 300 metres, Turn right")).count(), 1);
        assert_eq!(said.iter().filter(|s| *s == "Turn right").count(), 1);
        let early = said.iter().position(|s| s.starts_with("In 300")).unwrap();
        let last = said.iter().position(|s| s == "Turn right").unwrap();
        assert!(early < last);
    }

    #[test]
    fn reroutes_once_after_enough_off_route_fixes() {
        let mut nav = Navigation::new(sample());
        let away = (48.0, 2.01);
        let mut reroutes = 0;
        for _ in 0..12 {
            reroutes += nav.update(away, None).iter().filter(|e| **e == NavEvent::Reroute).count();
        }
        assert_eq!(reroutes, 1);
        assert!(nav.off_route);
        nav.update((48.0, 2.0), None);
        assert!(!nav.off_route);
    }

    #[test]
    fn failed_reroute_tries_again_but_not_at_every_fix() {
        let mut nav = Navigation::new(sample());
        let away = (48.0, 2.01);
        for _ in 0..OFF_ROUTE_FIXES {
            nav.update(away, None);
        }
        assert!(nav.off_route);
        nav.reroute_failed();
        let mut fixes = 0;
        while nav.update(away, None).is_empty() {
            fixes += 1;
            assert!(fixes < 100);
        }
        assert_eq!(fixes, OFF_ROUTE_FIXES + REROUTE_RETRY_FIXES - 1);
    }

    #[test]
    fn single_gps_blip_does_not_reroute() {
        let mut nav = Navigation::new(sample());
        for _ in 0..3 {
            assert!(nav.update((48.0, 2.01), None).is_empty());
            assert!(nav.update((48.0, 2.0), None).is_empty());
        }
    }

    #[test]
    fn progress_never_goes_backwards_at_a_red_light_jitter() {
        let mut nav = Navigation::new(sample());
        nav.update((48.005, 2.0), None);
        let before = nav.along_m;
        nav.update((48.0049, 2.0), None);
        assert_eq!(nav.along_m, before);
    }

    #[test]
    fn arrows_point_the_right_way_on_ramps_and_forks() {
        let symbol = |kind| Maneuver { kind, ..Default::default() }.symbol();
        assert_eq!(symbol(18), "turn-slight-right");
        assert_eq!(symbol(19), "turn-slight-left");
        assert_eq!(symbol(22), "turn-straight");
        assert_eq!(symbol(38), "turn-slight-left");
        assert_eq!(symbol(5), "flag");
        assert!(Maneuver { kind: 26, ..Default::default() }.is_roundabout());
    }

    #[test]
    fn warning_distance_follows_the_speed() {
        let first_warning_at = |speed: f64| {
            let mut nav = Navigation::new(sample());
            for i in 0..90 {
                let at = (48.0 + f64::from(i) * 0.0001, 2.0);
                if !nav.update(at, Some(speed)).is_empty() {
                    return nav.distance_to_next_m();
                }
            }
            panic!("no warning");
        };
        assert!(first_warning_at(12.5) > 280.0);
        assert!(first_warning_at(4.0) < 170.0);
    }

    #[test]
    fn a_long_stretch_after_a_turn_is_announced_and_a_joined_turn_is_not_warned_twice() {
        let mut points: Vec<LatLon> = (0..=4).map(|i| (48.0 + f64::from(i) * 0.001, 2.0)).collect();
        let corner = points[4];
        points.extend((1..=40).map(|i| (corner.0, 2.0 + f64::from(i) * 0.001)));
        let mut turn = step(10, 4, "Turn right");
        turn.spoken_after = "Continue for 3 km".to_string();
        let route = Route::new("Test", points, vec![step(1, 0, "Start"), turn, step(4, 44, "Arrive")], 300.0);
        let mut nav = Navigation::new(route);
        let mut said = Vec::new();
        for i in 0..=12 {
            let at = (48.0 + f64::from(i) * 0.0004, 2.0);
            for e in nav.update(at, None) {
                if let NavEvent::Say { text } = e {
                    said.push(text);
                }
            }
        }
        assert!(said.iter().any(|s| s == "Continue for 3 km"), "{said:?}");

        let mut joined = step(10, 2, "First");
        joined.joined_to_next = true;
        let points: Vec<LatLon> = (0..=30).map(|i| (48.0 + f64::from(i) * 0.0005, 2.0)).collect();
        let route = Route::new("Test", points, vec![step(1, 0, "Start"), joined, step(15, 4, "Second"), step(4, 30, "Arrive")], 300.0);
        let mut nav = Navigation::new(route);
        let mut said = Vec::new();
        for i in 0..=8 {
            for e in nav.update((48.0 + f64::from(i) * 0.0002, 2.0), None) {
                if let NavEvent::Say { text } = e {
                    said.push(text);
                }
            }
        }
        assert!(!said.iter().any(|s| s.starts_with("In ") && s.contains("Second")), "{said:?}");
    }

    #[test]
    fn a_reroute_can_keep_the_way_the_rider_is_going_and_search_looks_nearby_first() {
        let url = route_request_url((48.1, 2.2), (48.3, 2.4), false, Some(90.0));
        assert!(url.contains("heading%22%3A90"));
        assert!(url.contains("heading_tolerance"));
        assert!(search_request_url("cafe", Some((48.0, 2.0))).contains("lat=48.00000&lon=2.00000"));
        assert!((bearing_deg((48.0, 2.0), (48.0, 2.1)) - 90.0).abs() < 0.5);
        assert!(bearing_deg((48.0, 2.0), (47.9, 2.0)) > 179.0);
    }
}
