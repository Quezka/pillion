//! Routes from the free OpenStreetMap services, and following them by GPS.

use serde_json::Value;

use crate::locale::{self, tf};

const EARTH_RADIUS_M: f64 = 6_371_000.0;
pub const TOP_SPEED_KMH: u32 = 45;
pub const EARLY_ANNOUNCE_M: f64 = 300.0;
pub const FINAL_ANNOUNCE_M: f64 = 60.0;
pub const OFF_ROUTE_M: f64 = 50.0;
pub const OFF_ROUTE_FIXES: u32 = 5;
const REROUTE_RETRY_FIXES: u32 = 30;
const NEARBY_RADIUS_M: u32 = 300;
/// What the routing service says when it can't reach a point with this kind of vehicle.
pub const NO_PATH_ERROR: &str = "No path could be found";
const ARRIVED_M: f64 = 25.0;
/// How far ahead along the route a GPS fix may jump. Keeps a road that doubles back from
/// confusing the progress.
const SEARCH_AHEAD_M: f64 = 600.0;

pub type LatLon = (f64, f64);

#[derive(Clone, Debug, Default)]
pub struct Maneuver {
    pub kind: u32,
    pub text: String,
    pub spoken_before: String,
    pub spoken_at: String,
    pub begin: usize,
}

impl Maneuver {
    pub fn is_arrival(&self) -> bool {
        matches!(self.kind, 4..=6)
    }

    /// Direction for the arrow, in degrees clockwise from straight ahead.
    pub fn arrow_angle(&self) -> f32 {
        match self.kind {
            9 | 19 | 23 => 45.0,
            10 | 20 => 90.0,
            11 => 135.0,
            12 => 180.0,
            13 => -180.0,
            14 => -135.0,
            15 | 21 => -90.0,
            16 | 22 | 24 => -45.0,
            _ => 0.0,
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

fn distance_m(a: LatLon, b: LatLon) -> f64 {
    let mean_lat = ((a.0 + b.0) / 2.0).to_radians();
    let dx = (b.1 - a.1).to_radians() * mean_lat.cos();
    let dy = (b.0 - a.0).to_radians();
    (dx * dx + dy * dy).sqrt() * EARTH_RADIUS_M
}

/// Valhalla's shape: Google polyline encoding with six decimals.
pub fn decode_polyline6(shape: &str) -> Vec<LatLon> {
    let mut points = Vec::new();
    let (mut lat, mut lon) = (0i64, 0i64);
    let mut bytes = shape.bytes();
    let mut next = || -> Option<i64> {
        let (mut result, mut shift) = (0i64, 0u32);
        loop {
            let b = i64::from(bytes.next()?) - 63;
            result |= (b & 0x1F) << shift;
            shift += 5;
            if b < 0x20 || shift > 60 {
                break;
            }
        }
        Some(if result & 1 == 1 { !(result >> 1) } else { result >> 1 })
    };
    while let (Some(dlat), Some(dlon)) = (next(), next()) {
        lat += dlat;
        lon += dlon;
        points.push((lat as f64 / 1e6, lon as f64 / 1e6));
    }
    points
}

fn text_field(m: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| m[*k].as_str())
        .unwrap_or_default()
        .to_string()
}

/// `nearby` also accepts roads around the destination, for places a moped can't drive right up to.
pub fn route_request_url(from: LatLon, to: LatLon, nearby: bool) -> String {
    let radius = if nearby { format!(r#","radius":{NEARBY_RADIUS_M}"#) } else { String::new() };
    let body = format!(
        concat!(
            r#"{{"locations":[{{"lat":{},"lon":{}}},{{"lat":{},"lon":{}{}}}],"#,
            r#""costing":"motor_scooter","costing_options":{{"motor_scooter":{{"top_speed":{}}}}},"#,
            r#""directions_options":{{"units":"kilometers","language":"{}"}}}}"#
        ),
        from.0, from.1, to.0, to.1, radius, TOP_SPEED_KMH, locale::current().route_code()
    );
    format!("https://valhalla1.openstreetmap.de/route?json={}", percent_encode(&body))
}

pub fn search_request_url(query: &str) -> String {
    format!(
        "https://nominatim.openstreetmap.org/search?format=jsonv2&limit=6&accept-language={}&q={}",
        locale::current().code(),
        percent_encode(query)
    )
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    pub at: LatLon,
}

pub fn parse_search(json: &str) -> Result<Vec<Place>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("unexpected search reply")?;
    Ok(list
        .iter()
        .filter_map(|p| {
            let lat = p["lat"].as_str()?.parse().ok()?;
            let lon = p["lon"].as_str()?.parse().ok()?;
            Some(Place { name: p["display_name"].as_str()?.to_string(), at: (lat, lon) })
        })
        .collect())
}

pub fn parse_route(json: &str, destination: &str) -> Result<Route, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(message) = v["error"].as_str() {
        return Err(message.to_string());
    }
    let leg = &v["trip"]["legs"][0];
    let shape = leg["shape"].as_str().ok_or("the route has no shape")?;
    let points = decode_polyline6(shape);
    if points.len() < 2 {
        return Err("the route is too short".to_string());
    }
    let list = leg["maneuvers"].as_array().ok_or("the route has no directions")?;
    let maneuvers: Vec<Maneuver> = list
        .iter()
        .map(|m| Maneuver {
            kind: m["type"].as_u64().unwrap_or(0) as u32,
            text: text_field(m, &["instruction"]),
            spoken_before: text_field(m, &["verbal_pre_transition_instruction", "instruction"]),
            spoken_at: text_field(
                m,
                &["verbal_succinct_transition_instruction", "verbal_pre_transition_instruction", "instruction"],
            ),
            begin: m["begin_shape_index"].as_u64().unwrap_or(0) as usize,
        })
        .collect();
    Ok(Route::new(destination, points, maneuvers, v["trip"]["summary"]["time"].as_f64().unwrap_or(0.0)))
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

    pub fn update(&mut self, at: LatLon) -> Vec<NavEvent> {
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

        while self.next < self.route.maneuvers.len() {
            let is_last = self.next + 1 == self.route.maneuvers.len();
            if is_last || self.maneuver_along(self.next) > self.along_m {
                break;
            }
            self.next += 1;
            self.early_said = false;
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
        if distance <= FINAL_ANNOUNCE_M {
            if !self.final_said {
                self.final_said = true;
                self.early_said = true;
                events.push(NavEvent::Say { text: m.spoken_at });
            }
        } else if distance <= EARLY_ANNOUNCE_M && !self.early_said {
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

    fn step(kind: u32, begin: usize, name: &str) -> Maneuver {
        Maneuver {
            kind,
            text: name.to_string(),
            spoken_before: format!("{name} soon"),
            spoken_at: name.to_string(),
            begin,
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
        let url = route_request_url((48.1, 2.2), (48.3, 2.4), false);
        assert!(url.contains("motor_scooter"));
        assert!(!url.contains("radius"));
        assert!(route_request_url((48.1, 2.2), (48.3, 2.4), true).contains("radius%22%3A300"));
        assert!(url.contains("top_speed%22%3A45"));
        assert!(!url.contains(' '));
        assert!(search_request_url("a b&c").ends_with("q=a%20b%26c"));
    }

    #[test]
    fn announces_each_turn_once_early_then_final_then_arrives() {
        let mut nav = Navigation::new(sample());
        assert!(nav.update((48.0, 2.0)).is_empty());
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
            for e in nav.update(at) {
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
            reroutes += nav.update(away).iter().filter(|e| **e == NavEvent::Reroute).count();
        }
        assert_eq!(reroutes, 1);
        assert!(nav.off_route);
        nav.update((48.0, 2.0));
        assert!(!nav.off_route);
    }

    #[test]
    fn failed_reroute_tries_again_but_not_at_every_fix() {
        let mut nav = Navigation::new(sample());
        let away = (48.0, 2.01);
        for _ in 0..OFF_ROUTE_FIXES {
            nav.update(away);
        }
        assert!(nav.off_route);
        nav.reroute_failed();
        let mut fixes = 0;
        while nav.update(away).is_empty() {
            fixes += 1;
            assert!(fixes < 100);
        }
        assert_eq!(fixes, OFF_ROUTE_FIXES + REROUTE_RETRY_FIXES - 1);
    }

    #[test]
    fn single_gps_blip_does_not_reroute() {
        let mut nav = Navigation::new(sample());
        for _ in 0..3 {
            assert!(nav.update((48.0, 2.01)).is_empty());
            assert!(nav.update((48.0, 2.0)).is_empty());
        }
    }

    #[test]
    fn progress_never_goes_backwards_at_a_red_light_jitter() {
        let mut nav = Navigation::new(sample());
        nav.update((48.005, 2.0));
        let before = nav.along_m;
        nav.update((48.0049, 2.0));
        assert_eq!(nav.along_m, before);
    }
}
