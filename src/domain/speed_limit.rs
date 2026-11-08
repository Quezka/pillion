//! Speed limits: which road the rider is on, and when a limit is worth saying out loud.
//!
//! The limits come from OpenStreetMap (through the routing service), so they're only as good as the map: where the map
//! has no limit, Pillion says nothing rather than guess.

/// A road and its limit, as the map knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    /// The map's id for the road. Two pieces of one street can have different ids.
    pub way: u64,
    pub name: String,
    /// km/h; `None` when the map doesn't say.
    pub limit: Option<u32>,
}

/// What the rider should hear about the limit.
#[derive(Clone, Debug, PartialEq)]
pub enum LimitEvent {
    /// A new road, with a limit different from the last one said (or the first one of the ride).
    NewLimit { limit: u32, road: String },
    /// Riding over the limit for a while.
    TooFast { limit: u32 },
}

/// Only speak again after this long on the same limit, so a string of short streets doesn't chatter.
const QUIET_AFTER_SAME_S: f64 = 180.0;
/// A road has to stay the same for this long before its limit is announced: GPS wobble near a junction can briefly
/// put the rider on the side street.
const SETTLE_S: f64 = 3.0;
/// Riding this much over the limit, for this long, earns one warning per road.
const OVER_BY_KMH: f64 = 5.0;
const OVER_FOR_S: f64 = 6.0;

/// Follows the roads the rider is on and decides what to say about their limits.
#[derive(Clone, Debug, Default)]
pub struct LimitWatcher {
    /// The road the last fix was on, and since when.
    current: Option<(Road, f64)>,
    /// The road whose limit was handled already (said, or deliberately not).
    settled_way: Option<u64>,
    last_said: Option<(u32, f64)>,
    over_since: Option<f64>,
    warned_way: Option<u64>,
}

impl LimitWatcher {
    pub fn road(&self) -> Option<&Road> {
        self.current.as_ref().map(|(road, _)| road)
    }

    /// The limit of the road the rider is on, if the map gives one.
    pub fn limit(&self) -> Option<u32> {
        self.road().and_then(|r| r.limit)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The road the rider is on now (from the map), at time `now` in seconds.
    pub fn on_road(&mut self, road: Road, now: f64) -> Option<LimitEvent> {
        match &self.current {
            Some((same, _)) if same.way == road.way => {}
            // The same street in another piece, with the same limit: not a new road for the rider.
            Some((same, since)) if same.name == road.name && !road.name.is_empty() && same.limit == road.limit => {
                let since = *since;
                self.settled_way = self.settled_way.map(|_| road.way);
                self.current = Some((road, since));
            }
            _ => {
                self.current = Some((road, now));
                self.over_since = None;
            }
        }
        self.check(now)
    }

    /// Call on every GPS fix with the speed, to settle a new road and notice riding too fast.
    pub fn on_speed(&mut self, speed_kmh: Option<f64>, now: f64) -> Option<LimitEvent> {
        if let Some(event) = self.check(now) {
            return Some(event);
        }
        let (road, _) = self.current.as_ref()?;
        let limit = road.limit?;
        let way = road.way;
        match speed_kmh {
            Some(speed) if speed > f64::from(limit) + OVER_BY_KMH => {
                let since = *self.over_since.get_or_insert(now);
                if now - since >= OVER_FOR_S && self.warned_way != Some(way) {
                    self.warned_way = Some(way);
                    return Some(LimitEvent::TooFast { limit });
                }
            }
            _ => self.over_since = None,
        }
        None
    }

    fn check(&mut self, now: f64) -> Option<LimitEvent> {
        let (road, since) = self.current.as_ref()?;
        if self.settled_way == Some(road.way) || now - since < SETTLE_S {
            return None;
        }
        self.settled_way = Some(road.way);
        let limit = road.limit?;
        let repeat = matches!(self.last_said, Some((said, at)) if said == limit && now - at < QUIET_AFTER_SAME_S);
        if repeat {
            return None;
        }
        self.last_said = Some((limit, now));
        Some(LimitEvent::NewLimit { limit, road: road.name.clone() })
    }
}

/// The map's `maxspeed` value in km/h: "50", "30 mph", or a country's default such as "IT:urban". `None` for
/// "none", "signals", "walk" and anything unreadable.
pub fn parse_maxspeed(value: &str) -> Option<u32> {
    let v = value.trim();
    if let Some(mph) = v.strip_suffix("mph") {
        return mph.trim().parse::<f64>().ok().map(|m| (m * 1.609_344).round() as u32);
    }
    if let Ok(kmh) = v.trim_end_matches("km/h").trim().parse::<u32>() {
        return (kmh > 0).then_some(kmh);
    }
    // Italian defaults (Codice della strada, art. 142), the owner's country.
    match v {
        "IT:urban" => Some(50),
        "IT:rural" => Some(90),
        "IT:trunk" => Some(110),
        "IT:motorway" => Some(130),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn road(way: u64, name: &str, limit: Option<u32>) -> Road {
        Road { way, name: name.to_string(), limit }
    }

    #[test]
    fn a_new_road_is_announced_once_it_settles() {
        let mut w = LimitWatcher::default();
        assert_eq!(w.on_road(road(1, "Via Roma", Some(50)), 0.0), None);
        assert_eq!(w.on_speed(Some(30.0), 1.0), None);
        assert_eq!(w.on_speed(Some(30.0), 3.5), Some(LimitEvent::NewLimit { limit: 50, road: "Via Roma".into() }));
        assert_eq!(w.on_speed(Some(30.0), 4.0), None);
        // A side street for a second (GPS wobble) says nothing.
        assert_eq!(w.on_road(road(2, "Vicolo", Some(30)), 5.0), None);
        assert_eq!(w.on_road(road(1, "Via Roma", Some(50)), 6.0), None);
        assert_eq!(w.on_speed(Some(30.0), 12.0), None);
    }

    #[test]
    fn same_limit_on_the_next_street_is_quiet_for_a_while() {
        let mut w = LimitWatcher::default();
        w.on_road(road(1, "Via Roma", Some(50)), 0.0);
        assert!(w.on_speed(None, 4.0).is_some());
        w.on_road(road(2, "Via Po", Some(50)), 10.0);
        assert_eq!(w.on_speed(None, 15.0), None);
        w.on_road(road(3, "Via Zamboni", Some(30)), 20.0);
        assert_eq!(w.on_speed(None, 24.0), Some(LimitEvent::NewLimit { limit: 30, road: "Via Zamboni".into() }));
        // Long after, the same 30 is worth saying again on a new road.
        w.on_road(road(4, "Via Irnerio", Some(30)), 400.0);
        assert!(w.on_speed(None, 404.0).is_some());
    }

    #[test]
    fn the_same_street_in_pieces_is_one_road() {
        let mut w = LimitWatcher::default();
        w.on_road(road(1, "Via Emilia", Some(50)), 0.0);
        assert!(w.on_speed(None, 4.0).is_some());
        assert_eq!(w.on_road(road(9, "Via Emilia", Some(50)), 30.0), None);
        assert_eq!(w.on_speed(None, 40.0), None);
    }

    #[test]
    fn unknown_limits_are_silent() {
        let mut w = LimitWatcher::default();
        w.on_road(road(1, "Strada", None), 0.0);
        assert_eq!(w.on_speed(Some(40.0), 10.0), None);
        assert_eq!(w.limit(), None);
    }

    #[test]
    fn too_fast_warns_once_per_road_after_a_while() {
        let mut w = LimitWatcher::default();
        w.on_road(road(1, "Via Zamboni", Some(30)), 0.0);
        w.on_speed(Some(28.0), 4.0);
        assert_eq!(w.on_speed(Some(40.0), 5.0), None);
        assert_eq!(w.on_speed(Some(40.0), 11.5), Some(LimitEvent::TooFast { limit: 30 }));
        assert_eq!(w.on_speed(Some(40.0), 20.0), None);
        // Slowing down and speeding up again on the same road doesn't nag.
        w.on_speed(Some(25.0), 21.0);
        assert_eq!(w.on_speed(Some(40.0), 30.0), None);
    }

    #[test]
    fn maxspeed_values() {
        assert_eq!(parse_maxspeed("50"), Some(50));
        assert_eq!(parse_maxspeed("30 mph"), Some(48));
        assert_eq!(parse_maxspeed("IT:urban"), Some(50));
        assert_eq!(parse_maxspeed("none"), None);
        assert_eq!(parse_maxspeed("0"), None);
    }
}
