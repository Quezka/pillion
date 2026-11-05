//! A pretend ride for trying the app without the moped. It sends moped frames through the real
//! parser, and the app labels it clearly as a demo.

use crate::engine::Engine;
use crate::locale::{t, tf};
use crate::nav::{LatLon, Maneuver, Route};
use crate::protocol::{build_live_frame, Live, Pack};

const STEP_S: f64 = 0.5;
const SPEED_MPS: f64 = 8.3;
pub const START_ALONG_M: f64 = 650.0;
const STOP_BEFORE_END_M: f64 = 80.0;
const WH_PER_METRE: f64 = 1.0 / 30.0;
const START_REMAINING_AH: f64 = 14.4;
const FULL_AH: f32 = 22.5;
const START_ODOMETER_M: u32 = 1_284_300;
const TRIP_SO_FAR_M: u32 = 5_200;
const TRIP_CAPACITY_USED_AH: f32 = 3.5;
const FIRST_NOTIFICATION_S: f64 = 0.0;

pub struct DemoRide {
    started: Option<f64>,
    next_step: f64,
    counter: u8,
    notified: bool,
    route_points: Vec<LatLon>,
    route_along: Vec<f64>,
}

fn maneuver(kind: u32, begin: usize, text: &str, street: &str) -> Maneuver {
    let full = tf("{} onto {}.", &[&text, &street]);
    Maneuver {
        kind,
        text: full.clone(),
        spoken_before: full,
        spoken_at: tf("{}.", &[&text]),
        begin,
    }
}

fn demo_route() -> Route {
    let mut points: Vec<LatLon> = Vec::new();
    let mut at = (48.2000_f64, 16.3700_f64);
    // Straight, right, gentle bend, left, then straight to the end.
    for (dlat, dlon, steps) in [
        (0.0007, 0.0002, 14),
        (0.0001, 0.0009, 9),
        (0.0005, 0.0006, 7),
        (0.0007, -0.0002, 12),
    ] {
        for _ in 0..steps {
            points.push(at);
            at = (at.0 + dlat, at.1 + dlon);
        }
    }
    points.push(at);
    let maneuvers = vec![
        maneuver(1, 0, t("Head north"), "Harbour Road"),
        maneuver(10, 14, t("Turn right"), "Mill Street"),
        maneuver(16, 30, t("Bear left"), "Park Avenue"),
        maneuver(4, points.len() - 1, t("Arrive"), "Park Avenue"),
    ];
    Route::new(t("Demo destination"), points, maneuvers, 460.0)
}

impl DemoRide {
    pub fn new() -> (Self, Route) {
        let route = demo_route();
        let demo = Self {
            started: None,
            next_step: 0.0,
            counter: 0,
            notified: false,
            route_points: route.points.clone(),
            route_along: route.along_m.clone(),
        };
        (demo, route)
    }

    fn position_at(&self, along: f64) -> LatLon {
        let last = self.route_points.len() - 1;
        let i = self.route_along.partition_point(|a| *a <= along).clamp(1, last);
        let (a0, a1) = (self.route_along[i - 1], self.route_along[i]);
        let t = if a1 > a0 { ((along - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 0.0 };
        let (p0, p1) = (self.route_points[i - 1], self.route_points[i]);
        (p0.0 + (p1.0 - p0.0) * t, p0.1 + (p1.1 - p0.1) * t)
    }

    fn live_at(ridden_m: f64) -> Live {
        let used_ah = (ridden_m * WH_PER_METRE / 50.0) as f32;
        let remaining = START_REMAINING_AH as f32 - used_ah;
        let mut live = Live {
            odometer_m: START_ODOMETER_M + ridden_m as u32,
            switched_on: true,
            model_number: 1,
            motor_rpm: 2400,
            motor_temp: 41.0,
            controller_temp: 36.0,
            aux_voltage: 12.8,
            riding_mode: 1,
            ..Live::default()
        };
        live.packs[0] = Pack {
            fitted: true,
            percent: (remaining / FULL_AH * 100.0).round() as u8,
            low: false,
            voltage: 51.5,
            current_ma: -18_500,
            temp_max: 27.0,
            temp_min: 24.5,
            remaining_ah: remaining,
            full_ah: FULL_AH,
            health: 96,
        };
        live
    }

    pub fn step(&mut self, engine: &mut Engine, now: f64) {
        let started = *self.started.get_or_insert_with(|| {
            // Open the ride partway, so the trip and consumption figures already mean something.
            let mut before = Self::live_at(0.0);
            before.odometer_m -= TRIP_SO_FAR_M;
            before.packs[0].remaining_ah += TRIP_CAPACITY_USED_AH;
            engine.on_rx(&build_live_frame(&before, 0));
            now
        });
        if now < self.next_step {
            return;
        }
        self.next_step = now + STEP_S;
        let ridden = (now - started) * SPEED_MPS;
        self.counter = self.counter.wrapping_add(1);
        engine.on_rx(&build_live_frame(&Self::live_at(ridden), self.counter));
        let total = *self.route_along.last().unwrap_or(&0.0);
        // The demo waits just before the end, so a long look at it doesn't arrive and go blank.
        let along = (START_ALONG_M + ridden).min(total - STOP_BEFORE_END_M);
        engine.on_gps(self.position_at(along), Some(SPEED_MPS));
        if !self.notified && now - started >= FIRST_NOTIFICATION_S {
            self.notified = true;
            engine.on_notification("demo.chat", "Chat", "Sam", t("See you at six!"), false);
        }
    }
}
