//! Geometry on the map: distances, directions, points around a centre.

pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Latitude and longitude, in degrees.
pub type LatLon = (f64, f64);

pub fn distance_m(a: LatLon, b: LatLon) -> f64 {
    let mean_lat = ((a.0 + b.0) / 2.0).to_radians();
    let dx = (b.1 - a.1).to_radians() * mean_lat.cos();
    let dy = (b.0 - a.0).to_radians();
    (dx * dx + dy * dy).sqrt() * EARTH_RADIUS_M
}

/// The direction from one point to another, in degrees clockwise from north.
pub fn bearing_deg(a: LatLon, b: LatLon) -> f64 {
    let mean_lat = ((a.0 + b.0) / 2.0).to_radians();
    let dx = (b.1 - a.1).to_radians() * mean_lat.cos();
    let dy = (b.0 - a.0).to_radians();
    dx.atan2(dy).to_degrees().rem_euclid(360.0)
}

/// How far apart two directions are, 0 to 180 degrees.
pub fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// The point `distance_m` away from `from` in the direction `bearing` (degrees from north). Close enough for the
/// few tens of kilometres a moped's range covers.
pub fn offset(from: LatLon, bearing: f64, distance_m: f64) -> LatLon {
    let b = bearing.to_radians();
    let dlat = distance_m * b.cos() / EARTH_RADIUS_M;
    let dlon = distance_m * b.sin() / (EARTH_RADIUS_M * from.0.to_radians().cos().max(1e-6));
    (from.0 + dlat.to_degrees(), from.1 + dlon.to_degrees())
}

/// A circle on the map as `points` points: the range ring when the road-reachable area isn't known.
pub fn circle(center: LatLon, radius_m: f64, points: usize) -> Vec<LatLon> {
    (0..points).map(|i| offset(center, 360.0 * i as f64 / points as f64, radius_m)).collect()
}

/// The shortest distance from `at` to the segment `a`–`b`, in metres.
pub fn distance_to_segment_m(at: LatLon, a: LatLon, b: LatLon) -> f64 {
    let scale = at.0.to_radians().cos();
    let project = |p: LatLon| ((p.1 - at.1).to_radians() * scale * EARTH_RADIUS_M, (p.0 - at.0).to_radians() * EARTH_RADIUS_M);
    let (ax, ay) = project(a);
    let (bx, by) = project(b);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    (cx * cx + cy * cy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_circles_land_at_the_right_distance() {
        let bologna = (44.4949, 11.3426);
        for bearing in [0.0, 45.0, 90.0, 200.0] {
            let p = offset(bologna, bearing, 10_000.0);
            assert!((distance_m(bologna, p) - 10_000.0).abs() < 20.0);
            assert!(angle_between(bearing_deg(bologna, p), bearing) < 1.0);
        }
        let ring = circle(bologna, 25_000.0, 36);
        assert_eq!(ring.len(), 36);
        assert!(ring.iter().all(|p| (distance_m(bologna, *p) - 25_000.0).abs() < 60.0));
    }

    #[test]
    fn angles_wrap_round() {
        assert_eq!(angle_between(350.0, 10.0), 20.0);
        assert_eq!(angle_between(90.0, 270.0), 180.0);
    }

    #[test]
    fn segment_distance() {
        let a = (44.0, 11.0);
        let b = offset(a, 90.0, 1000.0);
        let beside = offset(offset(a, 90.0, 500.0), 0.0, 30.0);
        assert!((distance_to_segment_m(beside, a, b) - 30.0).abs() < 0.5);
    }
}
