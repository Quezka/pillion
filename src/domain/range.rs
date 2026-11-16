//! Range calculator (owner ask 2026-11-16): will the battery get me there, and how long to charge for a trip.
//! Pure; inputs = what the engine knows (or what the rider typed when the moped is off).

/// Kept in hand on arrival: the original warns low near 10 %.
pub const RESERVE_PERCENT: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TripCheck {
    /// km the trip takes (both ways when round trip)
    pub needed_km: f64,
    /// battery % expected on arrival (back home when round trip); can go below 0 = won't make it
    pub arrive_percent: f64,
    /// true = arrives with at least the reserve
    pub ok: bool,
}

/// `percent` now, `range_km` that percent gives (engine's estimate).
pub fn trip_check(percent: f64, range_km: f64, distance_km: f64, round_trip: bool) -> Option<TripCheck> {
    if percent <= 0.0 || range_km <= 0.0 || distance_km < 0.0 {
        return None;
    }
    let needed_km = if round_trip { distance_km * 2.0 } else { distance_km };
    let km_per_percent = range_km / percent;
    let arrive_percent = percent - needed_km / km_per_percent;
    Some(TripCheck { needed_km, arrive_percent, ok: arrive_percent >= RESERVE_PERCENT })
}

/// Charge % a trip needs (with the reserve), from what a full charge gives.
pub fn percent_for_km(km: f64, full_range_km: f64) -> Option<f64> {
    (full_range_km > 0.0).then(|| (km / full_range_km * 100.0 + RESERVE_PERCENT).clamp(0.0, 100.0))
}

/// Minutes to charge from `from` % to `to` %, `full_hours` = empty to full for this battery (health included).
/// Linear like the original's estimate (its whole-hours rounding is kept in `Engine::charge_hours`).
pub fn charge_minutes(from: f64, to: f64, full_hours: f64) -> f64 {
    ((to - from).max(0.0) / 100.0 * full_hours * 60.0).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_trip_is_fine_and_a_long_round_trip_is_not() {
        let near = trip_check(80.0, 40.0, 10.0, false).unwrap();
        assert_eq!(near.needed_km, 10.0);
        assert!((near.arrive_percent - 60.0).abs() < 1e-9 && near.ok);
        let far = trip_check(50.0, 25.0, 12.0, true).unwrap();
        assert!((far.arrive_percent - 2.0).abs() < 1e-9 && !far.ok);
        assert_eq!(trip_check(0.0, 0.0, 5.0, false), None);
    }

    #[test]
    fn charge_needed_for_a_trip_and_the_time_it_takes() {
        assert_eq!(percent_for_km(25.0, 50.0), Some(60.0));
        assert_eq!(percent_for_km(500.0, 50.0), Some(100.0));
        assert_eq!(charge_minutes(20.0, 80.0, 9.0), 324.0);
        assert_eq!(charge_minutes(90.0, 80.0, 9.0), 0.0);
    }
}
