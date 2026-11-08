//! The free OpenStreetMap web services Pillion uses, behind the `MapServices` port: Valhalla (routes, the road under the
//! rider and its speed limit, the area the battery reaches), Photon (place search, typo-tolerant, close places first)
//! with Nominatim as the fallback answer format.
//!
//! The phone fetches the addresses built here and hands back the answer, which is read here too.

use serde_json::Value;

use crate::application::nav::{Maneuver, Route, TOP_SPEED_KMH};
use crate::application::ports::MapServices;
use crate::domain::geo::LatLon;
use crate::domain::places::Place;
use crate::domain::speed_limit::Road;

const VALHALLA: &str = "https://valhalla1.openstreetmap.de";
const HEADING_TOLERANCE_DEG: u32 = 60;
const NEARBY_RADIUS_M: u32 = 300;
/// A route's shape is sent in the address, so long routes are thinned to this many points for the speed limits.
const ROAD_SAMPLES: usize = 350;
const SEARCH_RESULTS: usize = 8;

/// The MapServices the app uses.
pub struct Osm;

impl MapServices for Osm {
    fn route_url(&self, from: LatLon, to: LatLon, nearby: bool, heading: Option<f64>, lang: &str) -> String {
        let way = heading.map_or(String::new(), |h| format!(r#","heading":{h:.0},"heading_tolerance":{HEADING_TOLERANCE_DEG}"#));
        let radius = if nearby { format!(r#","radius":{NEARBY_RADIUS_M}"#) } else { String::new() };
        let body = format!(
            concat!(
                r#"{{"locations":[{{"lat":{},"lon":{}{}}},{{"lat":{},"lon":{}{}}}],"#,
                r#""costing":"motor_scooter","costing_options":{{"motor_scooter":{{"top_speed":{}}}}},"#,
                r#""directions_options":{{"units":"kilometers","language":"{}"}}}}"#
            ),
            from.0, from.1, way, to.0, to.1, radius, TOP_SPEED_KMH, route_language(lang)
        );
        format!("{VALHALLA}/route?json={}", percent_encode(&body))
    }

    fn parse_route(&self, json: &str, destination: &str) -> Result<Route, String> {
        parse_valhalla_route(json, destination)
    }

    fn search_url(&self, query: &str, near: Option<LatLon>, lang: &str) -> String {
        // Photon speaks English, German and French; for Italian and Russian its local names are the right ones.
        let lang = if matches!(lang, "en" | "de" | "fr") { lang } else { "default" };
        let around = near.map_or(String::new(), |p| format!("&lat={:.5}&lon={:.5}&location_bias_scale=0.4", p.0, p.1));
        format!("https://photon.komoot.io/api/?limit={SEARCH_RESULTS}&lang={lang}{around}&q={}", percent_encode(query))
    }

    fn parse_search(&self, json: &str) -> Result<Vec<Place>, String> {
        parse_places(json)
    }

    fn road_url(&self, at: LatLon, heading: Option<f64>) -> String {
        let way = heading.map_or(String::new(), |h| format!(r#","heading":{h:.0},"heading_tolerance":{HEADING_TOLERANCE_DEG}"#));
        let body = format!(r#"{{"locations":[{{"lat":{:.6},"lon":{:.6}{way}}}],"costing":"motor_scooter","verbose":true}}"#, at.0, at.1);
        format!("{VALHALLA}/locate?json={}", percent_encode(&body))
    }

    fn parse_road(&self, json: &str) -> Result<Option<Road>, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if let Some(message) = v["error"].as_str() {
            return Err(message.to_string());
        }
        let Some(edge) = v[0]["edges"].as_array().and_then(|edges| edges.first()) else {
            return Ok(None);
        };
        let info = &edge["edge_info"];
        let Some(way) = info["way_id"].as_u64() else {
            return Ok(None);
        };
        Ok(Some(Road { way, name: first_name(&info["names"]), limit: limit(&info["speed_limit"]) }))
    }

    fn route_roads_url(&self, route: &Route) -> String {
        let samples = sample(&route.points, ROAD_SAMPLES);
        let shape: Vec<LatLon> = samples.iter().map(|&i| route.points[i]).collect();
        let body = format!(
            concat!(
                r#"{{"encoded_polyline":"{}","costing":"motor_scooter","shape_match":"map_snap","#,
                r#""filters":{{"attributes":["edge.way_id","edge.names","edge.speed_limit","matched.edge_index"],"action":"include"}}}}"#
            ),
            encode_polyline6(&shape).replace('\\', "\\\\")
        );
        format!("{VALHALLA}/trace_attributes?json={}", percent_encode(&body))
    }

    fn parse_route_roads(&self, json: &str, points: usize) -> Result<Vec<Option<Road>>, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if let Some(message) = v["error"].as_str() {
            return Err(message.to_string());
        }
        let edges: Vec<Option<Road>> = v["edges"]
            .as_array()
            .ok_or("no roads in the answer")?
            .iter()
            .map(|e| e["way_id"].as_u64().map(|way| Road { way, name: first_name(&e["names"]), limit: limit(&e["speed_limit"]) }))
            .collect();
        let matched: Vec<Option<Road>> = v["matched_points"]
            .as_array()
            .ok_or("no matched points in the answer")?
            .iter()
            .map(|m| m["edge_index"].as_u64().and_then(|i| edges.get(i as usize).cloned().flatten()))
            .collect();
        // Back from the thinned shape to every point of the route: each takes the road of the sample before it.
        let samples = sample_indices(points, ROAD_SAMPLES);
        let mut out = Vec::with_capacity(points);
        let mut k = 0;
        for i in 0..points {
            while k + 1 < samples.len() && samples[k + 1] <= i {
                k += 1;
            }
            out.push(matched.get(k).cloned().flatten());
        }
        Ok(out)
    }

    fn reach_url(&self, from: LatLon, km: f64) -> String {
        let body = format!(
            concat!(
                r#"{{"locations":[{{"lat":{:.5},"lon":{:.5}}}],"costing":"motor_scooter","#,
                r#""costing_options":{{"motor_scooter":{{"top_speed":{}}}}},"contours":[{{"distance":{:.1}}}],"#,
                r#""polygons":true,"denoise":0.3,"generalize":150}}"#
            ),
            from.0, from.1, TOP_SPEED_KMH, km.clamp(1.0, 200.0)
        );
        format!("{VALHALLA}/isochrone?json={}", percent_encode(&body))
    }

    fn parse_reach(&self, json: &str) -> Result<Vec<Vec<LatLon>>, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if let Some(message) = v["error"].as_str() {
            return Err(message.to_string());
        }
        let mut rings = Vec::new();
        for feature in v["features"].as_array().ok_or("no area in the answer")? {
            let geometry = &feature["geometry"];
            let polygons: Vec<&Value> = match geometry["type"].as_str() {
                Some("MultiPolygon") => geometry["coordinates"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
                _ => vec![&geometry["coordinates"]],
            };
            for polygon in polygons {
                // The outer ring only: holes (lakes, a hill with no roads) are too small to matter at this scale.
                if let Some(outer) = polygon[0].as_array() {
                    let ring: Vec<LatLon> =
                        outer.iter().filter_map(|p| Some((p[1].as_f64()?, p[0].as_f64()?))).collect();
                    if ring.len() >= 3 {
                        rings.push(ring);
                    }
                }
            }
        }
        Ok(rings)
    }
}

fn route_language(lang: &str) -> &'static str {
    match lang {
        "it" => "it-IT",
        "ru" => "ru-RU",
        _ => "en-US",
    }
}

fn first_name(names: &Value) -> String {
    names.as_array().and_then(|n| n.first()).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Valhalla says 0 for "unknown".
fn limit(value: &Value) -> Option<u32> {
    value.as_u64().filter(|&l| l > 0 && l < 200).map(|l| l as u32)
}

/// Indices of at most `n` points spread evenly over `len`, always keeping the first and the last.
fn sample_indices(len: usize, n: usize) -> Vec<usize> {
    if len <= n {
        return (0..len).collect();
    }
    let mut out: Vec<usize> = (0..n).map(|i| i * (len - 1) / (n - 1)).collect();
    out.dedup();
    out
}

fn sample(points: &[LatLon], n: usize) -> Vec<usize> {
    sample_indices(points.len(), n)
}

/// Search answers: Photon's GeoJSON, or Nominatim's list (the older format, still read for safety).
fn parse_places(json: &str) -> Result<Vec<Place>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(features) = v["features"].as_array() {
        return Ok(features.iter().filter_map(photon_place).collect());
    }
    let list = v.as_array().ok_or("unexpected search reply")?;
    Ok(list
        .iter()
        .filter_map(|p| {
            let lat = p["lat"].as_str()?.parse().ok()?;
            let lon = p["lon"].as_str()?.parse().ok()?;
            let full = p["display_name"].as_str()?;
            let (name, detail) = full.split_once(", ").unwrap_or((full, ""));
            Some(Place { name: name.to_string(), detail: detail.to_string(), at: (lat, lon) })
        })
        .collect())
}

fn photon_place(f: &Value) -> Option<Place> {
    let c = f["geometry"]["coordinates"].as_array()?;
    let at = (c.get(1)?.as_f64()?, c.first()?.as_f64()?);
    let p = &f["properties"];
    let s = |k: &str| p[k].as_str().unwrap_or_default().trim().to_string();
    let street = match (s("street"), s("housenumber")) {
        (street, number) if !street.is_empty() && !number.is_empty() => format!("{street} {number}"),
        (street, _) => street,
    };
    let name = if !s("name").is_empty() { s("name") } else if !street.is_empty() { street.clone() } else { s("city") };
    if name.is_empty() {
        return None;
    }
    let town = if !s("city").is_empty() { s("city") } else { s("county") };
    let detail: Vec<String> = [street, town]
        .into_iter()
        .filter(|x| !x.is_empty() && *x != name)
        .collect();
    Some(Place { name, detail: detail.join(", "), at })
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


/// The other way round, for sending a shape.
pub fn encode_polyline6(points: &[LatLon]) -> String {
    let mut out = String::new();
    let (mut last_lat, mut last_lon) = (0i64, 0i64);
    let put = |value: i64, out: &mut String| {
        let mut v = if value < 0 { !(value << 1) } else { value << 1 };
        while v >= 0x20 {
            out.push(char::from(((0x20 | (v & 0x1F)) + 63) as u8));
            v >>= 5;
        }
        out.push(char::from((v + 63) as u8));
    };
    for &(lat, lon) in points {
        let (la, lo) = ((lat * 1e6).round() as i64, (lon * 1e6).round() as i64);
        put(la - last_lat, &mut out);
        put(lo - last_lon, &mut out);
        last_lat = la;
        last_lon = lo;
    }
    out
}

fn text_field(m: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| m[*k].as_str())
        .unwrap_or_default()
        .to_string()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn parse_valhalla_route(json: &str, destination: &str) -> Result<Route, String> {
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
        .map(|m| {
            let joined_to_next = m["verbal_multi_cue"].as_bool().unwrap_or(false);
            // When two turns come together, the short form would leave out the second one.
            let at_keys: &[&str] = if joined_to_next {
                &["verbal_pre_transition_instruction", "instruction"]
            } else {
                &["verbal_succinct_transition_instruction", "verbal_pre_transition_instruction", "instruction"]
            };
            Maneuver {
                kind: m["type"].as_u64().unwrap_or(0) as u32,
                text: text_field(m, &["instruction"]),
                spoken_before: text_field(m, &["verbal_pre_transition_instruction", "instruction"]),
                spoken_at: text_field(m, at_keys),
                spoken_after: text_field(m, &["verbal_post_transition_instruction"]),
                joined_to_next,
                exit: m["roundabout_exit_count"].as_u64().unwrap_or(0) as u32,
                begin: m["begin_shape_index"].as_u64().unwrap_or(0) as usize,
            }
        })
        .collect();
    Ok(Route::new(destination, points, maneuvers, v["trip"]["summary"]["time"].as_f64().unwrap_or(0.0)))
}

