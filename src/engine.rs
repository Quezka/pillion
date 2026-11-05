//! Everything that keeps running during a ride, with or without a screen. The platform layer feeds
//! it events through the `on_*` methods and carries out the `Command`s it queues.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::demo::DemoRide;
use crate::locale::{self, t, tf, Lang};
use crate::nav::{self, LatLon, NavEvent, Navigation, Place};
use crate::protocol::{self as proto, ClockTime, DashFlags, Frame, Live};

const CONNECT_TIMEOUT_S: f64 = 15.0;
const CLOCK_PERIOD_S: f64 = 1.0;
const FLAGS_DELAY_S: f64 = 0.1;
const DATA_STALE_S: f64 = 10.0;

const DEFAULT_KM_PER_KWH: f64 = 30.0;
const MIN_VALID_KM_PER_KWH: f64 = 5.0;
const AVG_PACK_VOLTAGE: f64 = 50.0;
const CHARGE_BASE_HOURS: i32 = 8;
const CHARGE_MODEL_FACTOR: f64 = 1.125;
const MEASURED_AFTER_KM: f64 = 2.0;
const RIDE_END_AFTER_S: f64 = 5.0;
const NOMINAL_PACK_AH: f64 = 20.0;
const RANGE_MIN_PERCENT_ONE_PACK: u32 = 15;
const RANGE_MIN_PERCENT_TWO_PACKS: u32 = 30;

const BATTERY_STEP: u8 = 10;
const BATTERY_STEP_LOW: u8 = 5;
const BATTERY_LOW_VOICE: u8 = 20;
const CONSUMPTION_EVERY_KM: f64 = 5.0;
const SPOKEN_TEXT_LIMIT: usize = 220;
const LOG_LENGTH: usize = 30;
const FIELD_SEPARATOR: char = ',';
const NOTIF_BURST_S: f64 = 15.0;
const RECENT_PLACES: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Scan,
    StopScan,
    Connect(String),
    Disconnect,
    Write(Vec<u8>),
    Speak { text: String, urgent: bool },
    SetSpeechRate(f32),
    FetchRoute { url: String, destination: String },
    Search { url: String },
    AskText { key: String, title: String, hint: String },
    OpenNotificationAccess,
    SetLanguage(&'static str),
    EnableBluetooth,
    OpenLocationSettings,
    OpenAppSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    NeedsPairing,
    Idle,
    Scanning,
    Connecting,
    LoggingIn,
    Online,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallState {
    Idle,
    Ringing,
    OffHook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifLevel {
    Off,
    AppAndSender,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BluetoothState {
    On,
    Off,
    Denied,
}

#[derive(Clone, Debug)]
struct NotifBurst {
    package: String,
    label: String,
    extra: u32,
    until: f64,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub ccu_id: String,
    pub passkey: String,
    pub address: String,
    pub voice_mode: bool,
    pub say_battery: bool,
    pub say_consumption: bool,
    pub say_navigation: bool,
    pub say_calls: bool,
    pub notif_level: NotifLevel,
    pub speech_rate: f32,
    pub muted_apps: HashSet<String>,
    pub dash_slots: Vec<String>,
    /// Every finished ride added up, for the average that the range is based on.
    pub total_km: f64,
    pub total_kwh: f64,
    /// Call lights on the dash. The original has this switch too, off until the rider sets it.
    pub dash_calls: bool,
    pub appearance: Appearance,
    pub high_contrast: bool,
    /// `None` follows the phone.
    pub language: Option<Lang>,
    pub recents: Vec<Place>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ccu_id: String::new(),
            passkey: String::new(),
            address: String::new(),
            voice_mode: true,
            say_battery: true,
            say_consumption: true,
            say_navigation: true,
            say_calls: true,
            notif_level: NotifLevel::Full,
            speech_rate: 1.0,
            muted_apps: HashSet::new(),
            dash_slots: Vec::new(),
            total_km: 0.0,
            total_kwh: 0.0,
            dash_calls: true,
            appearance: Appearance::System,
            high_contrast: false,
            language: None,
            recents: Vec::new(),
        }
    }
}

impl Settings {
    fn to_text(&self) -> String {
        let flag = |b: bool| u8::from(b);
        let level = match self.notif_level {
            NotifLevel::Off => 0,
            NotifLevel::AppAndSender => 1,
            NotifLevel::Full => 2,
        };
        let mut muted: Vec<&str> = self.muted_apps.iter().map(String::as_str).collect();
        muted.sort_unstable();
        format!(
            "ccu={}\npass={}\naddr={}\nvoice={}\nbattery={}\nconsumption={}\nnav={}\ncalls={}\nnotif={}\nrate={}\nmuted={}\nslots={}\ntotal_km={}\ntotal_kwh={}\ndash_calls={}\nappearance={}\ncontrast={}\nlanguage={}\n{}",
            self.ccu_id,
            self.passkey,
            self.address,
            flag(self.voice_mode),
            flag(self.say_battery),
            flag(self.say_consumption),
            flag(self.say_navigation),
            flag(self.say_calls),
            level,
            self.speech_rate,
            muted.join(&FIELD_SEPARATOR.to_string()),
            self.dash_slots.join(&FIELD_SEPARATOR.to_string()),
            self.total_km,
            self.total_kwh,
            flag(self.dash_calls),
            match self.appearance {
                Appearance::System => "system",
                Appearance::Light => "light",
                Appearance::Dark => "dark",
            },
            flag(self.high_contrast),
            self.language.map_or("system", Lang::code),
            self.recents
                .iter()
                .map(|p| format!("recent={};{};{}\n", p.at.0, p.at.1, p.name.replace('\n', " ")))
                .collect::<String>(),
        )
    }

    fn from_text(text: &str) -> Self {
        let mut s = Self::default();
        fn list(value: &str) -> impl Iterator<Item = String> + '_ {
            value.split(FIELD_SEPARATOR).filter(|x| !x.is_empty()).map(str::to_string)
        }
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let on = value == "1";
            match key {
                "ccu" => s.ccu_id = value.to_string(),
                "pass" => s.passkey = value.to_string(),
                "addr" => s.address = value.to_string(),
                "voice" => s.voice_mode = on,
                "battery" => s.say_battery = on,
                "consumption" => s.say_consumption = on,
                "nav" => s.say_navigation = on,
                "calls" => s.say_calls = on,
                "notif" => {
                    s.notif_level = match value {
                        "0" => NotifLevel::Off,
                        "1" => NotifLevel::AppAndSender,
                        _ => NotifLevel::Full,
                    }
                }
                "rate" => s.speech_rate = value.parse().unwrap_or(1.0),
                "muted" => s.muted_apps = list(value).collect(),
                "slots" => s.dash_slots = list(value).collect(),
                "total_km" => s.total_km = value.parse().unwrap_or(0.0),
                "total_kwh" => s.total_kwh = value.parse().unwrap_or(0.0),
                "appearance" => {
                    s.appearance = match value {
                        "light" => Appearance::Light,
                        "dark" => Appearance::Dark,
                        _ => Appearance::System,
                    }
                }
                "contrast" => s.high_contrast = on,
                "dash_calls" => s.dash_calls = on,
                "language" => s.language = Lang::from_code(value),
                "recent" => {
                    let mut parts = value.splitn(3, ';');
                    let lat = parts.next().and_then(|v| v.parse().ok());
                    let lon = parts.next().and_then(|v| v.parse().ok());
                    if let (Some(lat), Some(lon), Some(name)) = (lat, lon, parts.next()) {
                        s.recents.push(Place { name: name.to_string(), at: (lat, lon) });
                    }
                }
                _ => {}
            }
        }
        s
    }
}

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub package: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct NotifEntry {
    pub label: String,
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Trip {
    start_odometer_m: Option<u32>,
    start_capacity_ah: f64,
    pub km: f64,
    pub kwh: f64,
    next_consumption_km: f64,
}

pub struct Engine {
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    pub link: Link,
    pub live: Option<Live>,
    pub trip: Trip,
    pub found_devices: Vec<(String, String)>,
    pub flags: DashFlags,
    pub apps: Vec<AppEntry>,
    pub notifications: VecDeque<NotifEntry>,
    pub spoken: VecDeque<String>,
    pub nav: Option<Navigation>,
    pub nav_status: String,
    pub search_results: Vec<Place>,
    pub gps: Option<LatLon>,
    pub speed_kmh: Option<f64>,
    pub phone_battery: u8,
    pub notification_access: bool,
    pub text_prompt: Option<(String, String)>,
    pub demo_active: bool,
    pub bad_checksums: u32,
    /// Room Android's status and button bars take from the top and bottom of the screen.
    pub system_bars_px: (f32, f32),
    /// The phone has no working speech engine, so nothing can be spoken.
    pub voice_missing: bool,
    pub bluetooth: BluetoothState,
    pub location_on: bool,
    /// What the phone itself is set to. `None` where the platform doesn't say.
    pub system_dark: Option<bool>,
    system_language: Option<Lang>,
    spoken_language: Option<Lang>,
    notif_bursts: Vec<NotifBurst>,
    demo: Option<DemoRide>,
    outbox: VecDeque<Command>,
    now: f64,
    link_since: f64,
    last_rx: f64,
    next_clock: f64,
    flags_due: Option<f64>,
    clock: Option<ClockTime>,
    counter: u8,
    phone_uuid: String,
    reconnecting: bool,
    announced_percent: Option<u8>,
    low_said: bool,
    warning_said: u8,
    errors_said: Vec<u16>,
    switched_on_said: Option<bool>,
    range_warned: bool,
    destination: Option<(String, LatLon)>,
    route_pending: bool,
    rerouting: bool,
    route_nearby: bool,
    ride_off_since: Option<f64>,
    call: CallState,
    was_ringing: bool,
}

fn random_uuid() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = t.to_be_bytes();
    }
    proto::to_hex(&bytes)
}

/// Keeps speech short and on a character boundary.
fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_string(),
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
            settings_path: None,
            link: Link::NeedsPairing,
            live: None,
            trip: Trip::default(),
            found_devices: Vec::new(),
            flags: DashFlags::default(),
            apps: Vec::new(),
            notifications: VecDeque::new(),
            spoken: VecDeque::new(),
            nav: None,
            nav_status: String::new(),
            search_results: Vec::new(),
            gps: None,
            speed_kmh: None,
            phone_battery: 100,
            notification_access: false,
            text_prompt: None,
            demo_active: false,
            bad_checksums: 0,
            system_bars_px: (0.0, 0.0),
            voice_missing: false,
            bluetooth: BluetoothState::On,
            location_on: true,
            system_dark: None,
            system_language: None,
            spoken_language: None,
            notif_bursts: Vec::new(),
            demo: None,
            outbox: VecDeque::new(),
            now: 0.0,
            link_since: 0.0,
            last_rx: 0.0,
            next_clock: 0.0,
            flags_due: None,
            clock: None,
            counter: 0,
            phone_uuid: random_uuid(),
            reconnecting: false,
            announced_percent: None,
            low_said: false,
            warning_said: 0,
            errors_said: Vec::new(),
            switched_on_said: None,
            range_warned: false,
            destination: None,
            route_pending: false,
            rerouting: false,
            route_nearby: false,
            ride_off_since: None,
            call: CallState::Idle,
            was_ringing: false,
        }
    }

    pub fn load_settings(&mut self, path: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            self.settings = Settings::from_text(&text);
        }
        self.settings_path = Some(path);
        self.apply_language();
        self.push(Command::SetSpeechRate(self.settings.speech_rate));
        if self.has_pairing() && self.link == Link::NeedsPairing {
            self.connect();
        }
    }

    pub fn language(&self) -> Lang {
        self.settings.language.or(self.system_language).unwrap_or(Lang::En)
    }

    pub fn set_language(&mut self, language: Option<Lang>) {
        self.settings.language = language;
        self.save_settings();
        self.apply_language();
    }

    /// The phone's own language and light or dark setting.
    pub fn on_system(&mut self, language: Option<Lang>, dark: bool) {
        self.system_language = language;
        self.system_dark = Some(dark);
        self.apply_language();
    }

    fn apply_language(&mut self) {
        let language = self.language();
        locale::set(language);
        if self.spoken_language != Some(language) {
            self.spoken_language = Some(language);
            self.push(Command::SetLanguage(language.code()));
        }
    }

    pub fn on_bluetooth(&mut self, state: BluetoothState) {
        if state == BluetoothState::Off && self.bluetooth == BluetoothState::On && self.has_pairing() {
            self.say(t("Bluetooth is off."), true);
        }
        self.bluetooth = state;
    }

    pub fn enable_bluetooth(&mut self) {
        self.push(Command::EnableBluetooth);
    }

    pub fn open_location_settings(&mut self) {
        self.push(Command::OpenLocationSettings);
    }

    pub fn open_app_settings(&mut self) {
        self.push(Command::OpenAppSettings);
    }

    pub fn test_voice(&mut self) {
        self.say(t("This is how I'll sound on your ride. If you can hear me in your earpiece, you're all set."), true);
    }

    pub fn set_notification_access(&mut self, allowed: bool) {
        self.notification_access = allowed;
    }

    pub fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            // A failed save only costs the settings, not the ride.
            let _ = std::fs::write(path, self.settings.to_text());
        }
    }

    pub fn poll_command(&mut self) -> Option<Command> {
        self.outbox.pop_front()
    }

    fn push(&mut self, command: Command) {
        self.outbox.push_back(command);
    }

    fn set_link(&mut self, link: Link) {
        self.link = link;
        self.link_since = self.now;
    }

    fn next_counter(&mut self) -> u8 {
        let c = self.counter;
        self.counter = self.counter.wrapping_add(1);
        c
    }

    fn say(&mut self, text: impl Into<String>, urgent: bool) {
        let text = text.into();
        self.spoken.push_front(text.clone());
        self.spoken.truncate(LOG_LENGTH);
        // During a call only what can't wait is spoken over it.
        let on_a_call = self.call == CallState::OffHook;
        if self.settings.voice_mode && (urgent || !on_a_call) {
            self.push(Command::Speak { text, urgent });
        }
    }

    // ----- Moped connection -----

    pub fn has_pairing(&self) -> bool {
        !self.settings.ccu_id.is_empty() && !self.settings.passkey.is_empty()
    }

    /// `true` when the code was understood and saved.
    pub fn set_pairing_code(&mut self, code: &str) -> bool {
        let Some((ccu, key)) = proto::decode_pairing_code(code) else {
            return false;
        };
        self.settings.ccu_id = ccu;
        self.settings.passkey = key;
        self.settings.address.clear();
        self.reconnecting = false;
        self.save_settings();
        self.set_link(Link::Idle);
        self.connect();
        true
    }

    pub fn connect(&mut self) {
        if !self.has_pairing() {
            self.set_link(Link::NeedsPairing);
            return;
        }
        self.found_devices.clear();
        if self.settings.address.is_empty() {
            self.set_link(Link::Scanning);
            self.push(Command::Scan);
        } else {
            self.set_link(Link::Connecting);
            self.push(Command::Connect(self.settings.address.clone()));
        }
    }

    pub fn disconnect(&mut self) {
        self.push(Command::StopScan);
        self.push(Command::Disconnect);
        self.set_link(if self.has_pairing() { Link::Idle } else { Link::NeedsPairing });
    }

    pub fn forget_moped(&mut self) {
        self.disconnect();
        self.settings.ccu_id.clear();
        self.settings.passkey.clear();
        self.settings.address.clear();
        self.reconnecting = false;
        self.save_settings();
        self.set_link(Link::NeedsPairing);
    }

    pub fn on_ble_found(&mut self, name: &str, address: &str) {
        if !proto::DEVICE_NAMES.iter().any(|n| name.contains(n)) {
            return;
        }
        if !self.found_devices.iter().any(|(_, a)| a == address) {
            self.found_devices.push((name.to_string(), address.to_string()));
        }
        if self.link == Link::Scanning && !self.settings.ccu_id.is_empty() && name.contains(&self.settings.ccu_id) {
            self.choose_device(address);
        }
    }

    pub fn choose_device(&mut self, address: &str) {
        self.settings.address = address.to_string();
        self.save_settings();
        self.push(Command::StopScan);
        self.set_link(Link::Connecting);
        self.push(Command::Connect(address.to_string()));
    }

    pub fn on_ble_ready(&mut self) {
        let counter = self.next_counter();
        let frame = proto::login_frame(
            &self.settings.ccu_id,
            &self.settings.passkey,
            &self.phone_uuid,
            self.reconnecting,
            counter,
        );
        self.set_link(Link::LoggingIn);
        self.push(Command::Write(frame));
        self.reconnecting = true;
    }

    pub fn on_ble_disconnected(&mut self) {
        match self.link {
            Link::Online | Link::LoggingIn => {
                let was_online = self.link == Link::Online;
                self.set_link(Link::Connecting);
                if was_online {
                    self.say(t("Moped connection lost."), true);
                }
            }
            _ => {}
        }
    }

    pub fn on_rx(&mut self, bytes: &[u8]) {
        let Some(frame) = Frame::parse(bytes) else {
            return;
        };
        if bytes.last() != Some(&proto::checksum(&bytes[..bytes.len() - 1])) {
            self.bad_checksums += 1;
        }
        if let Some(accepted) = proto::login_result(&frame) {
            if accepted {
                self.set_link(Link::Online);
                self.next_clock = self.now;
                self.announced_percent = None;
            } else {
                let retrying = self.reconnecting;
                self.reconnecting = false;
                self.set_link(Link::Refused);
                self.say(t("The moped refused the pairing code."), true);
                self.push(Command::Disconnect);
                if retrying {
                    self.connect();
                }
            }
            return;
        }
        if let Some(live) = proto::parse_live(&frame) {
            self.last_rx = self.now;
            if self.link != Link::Online && !self.demo_active {
                self.set_link(Link::Online);
            }
            self.on_live(live);
        }
    }

    // ----- Battery and consumption -----

    fn on_live(&mut self, live: Live) {
        let capacity: f64 = live.packs.iter().filter(|p| p.fitted).map(|p| f64::from(p.remaining_ah)).sum();
        let riding = self.trip.start_odometer_m.is_some();
        let charged = riding && capacity > self.trip.start_capacity_ah + 0.05;
        if charged || (!riding && live.switched_on) {
            // A ride starts when the main switch is on. A battery that was charged or swapped starts one too.
            self.trip = Trip {
                start_odometer_m: Some(live.odometer_m),
                start_capacity_ah: capacity,
                km: 0.0,
                kwh: 0.0,
                next_consumption_km: CONSUMPTION_EVERY_KM,
            };
        }
        if let Some(start_odometer) = self.trip.start_odometer_m {
            self.trip.km = f64::from(live.odometer_m.saturating_sub(start_odometer)) / 1000.0;
            self.trip.kwh = ((self.trip.start_capacity_ah - capacity) * AVG_PACK_VOLTAGE / 1000.0).max(0.0);
        }
        self.live = Some(live);
        self.announce_changes();
    }

    /// Measured km/kWh for this ride, once it has run long enough to mean something.
    pub fn measured_efficiency(&self) -> Option<f64> {
        if self.trip.km < MEASURED_AFTER_KM || self.trip.kwh <= 0.0 {
            return None;
        }
        Some(self.trip.km / self.trip.kwh)
    }

    /// Km per kWh over all finished rides plus this one. The original's figure is an average like
    /// this too, but kept on Yamaha's server.
    pub fn average_efficiency(&self) -> Option<f64> {
        let km = self.settings.total_km + self.trip.km;
        let kwh = self.settings.total_kwh + self.trip.kwh;
        (km >= MEASURED_AFTER_KM && kwh > 0.0).then(|| km / kwh)
    }

    /// The efficiency the range uses, with the original's fallback.
    pub fn efficiency(&self) -> f64 {
        self.average_efficiency()
            .filter(|e| *e >= MIN_VALID_KM_PER_KWH)
            .unwrap_or(DEFAULT_KM_PER_KWH)
    }

    pub fn percent(&self) -> Option<u8> {
        let live = self.live.as_ref()?;
        let fitted = || live.packs.iter().filter(|p| p.fitted);
        let total_full: f64 = fitted().map(|p| f64::from(p.full_ah)).sum();
        if fitted().count() == 0 {
            return None;
        }
        let percent = if total_full > 0.0 {
            fitted().map(|p| f64::from(p.percent) * f64::from(p.full_ah)).sum::<f64>() / total_full
        } else {
            fitted().map(|p| f64::from(p.percent)).sum::<f64>() / fitted().count() as f64
        };
        Some(percent.round() as u8)
    }

    /// Range left, or `None` where the original withholds it: output limited, or charge too low to
    /// estimate.
    pub fn range_km(&self) -> Option<f64> {
        let live = self.live.as_ref()?;
        let fitted: Vec<_> = live.packs.iter().filter(|p| p.fitted).collect();
        let total_percent: u32 = fitted.iter().map(|p| u32::from(p.percent)).sum();
        let enough = match fitted.len() {
            0 => false,
            1 => total_percent > RANGE_MIN_PERCENT_ONE_PACK,
            _ => total_percent > RANGE_MIN_PERCENT_TWO_PACKS,
        };
        if live.output_limited || !enough {
            return None;
        }
        let efficiency = self.efficiency();
        let metres: f64 = fitted
            .iter()
            .map(|p| efficiency * f64::from(p.percent) * 0.01 * f64::from(p.full_ah) * AVG_PACK_VOLTAGE)
            .sum();
        Some(metres / 1000.0)
    }

    /// What a full charge should give, from each fitted battery's nominal capacity.
    pub fn full_range_km(&self) -> Option<f64> {
        let live = self.live.as_ref()?;
        let packs = live.packs.iter().filter(|p| p.fitted && p.percent > 0).count();
        (packs > 0).then(|| self.efficiency() * NOMINAL_PACK_AH * AVG_PACK_VOLTAGE * packs as f64 / 1000.0)
    }

    /// Hours to full for a battery. The whole-hours division is the original's.
    pub fn charge_hours(percent: u8, health: u8) -> f64 {
        let whole_hours = CHARGE_BASE_HOURS * (100 - i32::from(percent.min(100))) / 100;
        f64::from(whole_hours * i32::from(health)) * CHARGE_MODEL_FACTOR / 100.0
    }

    pub fn power_w(&self) -> Option<f64> {
        let live = self.live.as_ref()?;
        Some(
            live.packs
                .iter()
                .filter(|p| p.fitted)
                .map(|p| f64::from(p.voltage) * (f64::from(p.current_ma) / 1000.0).abs())
                .sum(),
        )
    }

    pub fn data_is_stale(&self) -> bool {
        self.live.is_some() && self.now - self.last_rx > DATA_STALE_S
    }

    fn battery_sentence(&self) -> String {
        match (self.percent(), self.range_km()) {
            (Some(p), Some(r)) => tf("Battery {} percent, about {} kilometres left.", &[&p, &format!("{r:.0}")]),
            (Some(p), None) => tf("Battery {} percent.", &[&p]),
            _ => t("No battery reading yet.").to_string(),
        }
    }

    fn announce_changes(&mut self) {
        let Some(live) = self.live.clone() else {
            return;
        };
        if let Some(percent) = self.percent() {
            let step = if percent <= BATTERY_LOW_VOICE { BATTERY_STEP_LOW } else { BATTERY_STEP };
            match self.announced_percent {
                None => {
                    self.announced_percent = Some(percent);
                    if self.settings.say_battery {
                        let text = tf("Moped connected. {}", &[&self.battery_sentence()]);
                        self.say(text, false);
                    }
                }
                Some(last) if percent > last => self.announced_percent = Some(percent),
                Some(last) if last.saturating_sub(percent) >= step => {
                    self.announced_percent = Some(percent);
                    if self.settings.say_battery {
                        self.say(self.battery_sentence(), false);
                    }
                    self.check_range_for_route();
                }
                _ => {}
            }
        }
        let low = live.packs.iter().any(|p| p.fitted && p.low);
        if low && !self.low_said {
            self.say(t("Battery low."), true);
        }
        self.low_said = low;

        if self.settings.say_consumption && self.trip.next_consumption_km > 0.0 && self.trip.km >= self.trip.next_consumption_km {
            self.trip.next_consumption_km += CONSUMPTION_EVERY_KM;
            if let Some(eff) = self.measured_efficiency() {
                let text = tf(
                    "{} kilometres so far, using {} kilometres per kilowatt hour.",
                    &[&format!("{:.0}", self.trip.km), &format!("{eff:.0}")],
                );
                self.say(text, false);
            }
        }

        if live.warning != 0 && self.warning_said == 0 {
            self.say(t("Warning. The moped has turned on its system warning."), true);
        }
        self.warning_said = live.warning;

        let errors = live.active_faults();
        let new: Vec<u16> = errors.iter().filter(|c| !self.errors_said.contains(c)).copied().collect();
        if !new.is_empty() {
            let digits = |c: u16| format!("{c:04X}").chars().map(String::from).collect::<Vec<_>>().join(" ");
            let list: Vec<String> = new.iter().map(|c| digits(*c)).collect();
            self.say(tf("Warning. The moped reports fault {}.", &[&list.join(", ")]), true);
        }
        self.errors_said = errors;

        if let Some(was_on) = self.switched_on_said {
            if was_on != live.switched_on {
                let text = if live.switched_on { t("Moped switched on.") } else { t("Moped switched off.") };
                self.say(text, false);
            }
        }
        self.switched_on_said = Some(live.switched_on);
    }

    // ----- Navigation -----

    pub fn ask_search(&mut self) {
        self.ask_text("search", t("Where to?"), t("Address or place"));
    }

    pub fn ask_pairing(&mut self) {
        self.ask_text("pair", t("Pairing code"), t("24 characters from the QR sticker"));
    }

    fn ask_text(&mut self, key: &str, title: &str, hint: &str) {
        self.push(Command::AskText {
            key: key.to_string(),
            title: title.to_string(),
            hint: hint.to_string(),
        });
    }

    pub fn on_text(&mut self, key: &str, value: &str) {
        match key {
            "search" if !value.trim().is_empty() => {
                self.nav_status = t("Searching...").to_string();
                self.push(Command::Search { url: nav::search_request_url(value.trim()) });
            }
            "pair" if !self.set_pairing_code(value) => {
                self.text_prompt = Some((
                    t("That code didn't work").to_string(),
                    t("It should have 24 characters, or 20 if it's already unscrambled.").to_string(),
                ));
            }
            _ => {}
        }
    }

    pub fn on_search_reply(&mut self, reply: Result<&str, &str>) {
        match reply.map_err(str::to_string).and_then(nav::parse_search) {
            Ok(places) => {
                self.nav_status = if places.is_empty() { t("Nothing found.").to_string() } else { String::new() };
                self.search_results = places;
            }
            Err(_) => self.nav_status = t("Search failed. Check the phone's internet connection.").to_string(),
        }
    }

    pub fn navigate_to(&mut self, place: &Place) {
        self.search_results.clear();
        self.settings.recents.retain(|p| p.name != place.name);
        self.settings.recents.insert(0, place.clone());
        self.settings.recents.truncate(RECENT_PLACES);
        self.save_settings();
        self.destination = Some((place.name.clone(), place.at));
        self.range_warned = false;
        self.rerouting = false;
        self.route_nearby = false;
        self.request_route();
    }

    fn request_route(&mut self) {
        let Some((name, at)) = self.destination.clone() else {
            return;
        };
        let Some(from) = self.gps else {
            self.route_pending = true;
            self.nav_status = t("Waiting for GPS...").to_string();
            return;
        };
        self.route_pending = false;
        self.nav_status = t("Finding a route...").to_string();
        self.push(Command::FetchRoute {
            url: nav::route_request_url(from, at, self.route_nearby),
            destination: name,
        });
    }

    pub fn on_route_reply(&mut self, reply: Result<&str, &str>, destination: &str) {
        // The rider may have stopped navigating while the request was on its way.
        if self.destination.is_none() {
            return;
        }
        let rerouting = std::mem::replace(&mut self.rerouting, false);
        match reply.map_err(str::to_string).and_then(|json| nav::parse_route(json, destination)) {
            Ok(route) => {
                let nav = Navigation::new(route);
                let nearby = t("A moped can't get right to that spot, so this goes to the nearest road.");
                self.nav_status = if self.route_nearby { nearby.to_string() } else { String::new() };
                if self.settings.say_navigation {
                    let text = if rerouting {
                        nav.route.maneuvers.first().map_or(String::new(), |m| m.spoken_before.clone())
                    } else if self.route_nearby {
                        format!("{nearby} {}", nav.opening())
                    } else {
                        nav.opening()
                    };
                    self.say(text, true);
                }
                self.nav = Some(nav);
                self.check_range_for_route();
            }
            Err(message) if message.contains(nav::NO_PATH_ERROR) && !self.route_nearby => {
                self.rerouting = rerouting;
                self.route_nearby = true;
                self.request_route();
            }
            Err(message) => {
                self.nav_status = tf("No route: {}", &[&message]);
                self.say(t("I couldn't get a route."), true);
                if let Some(nav) = self.nav.as_mut() {
                    nav.reroute_failed();
                }
            }
        }
    }

    pub fn stop_navigation(&mut self) {
        self.nav = None;
        self.destination = None;
        self.route_pending = false;
        self.nav_status.clear();
    }

    fn check_range_for_route(&mut self) {
        let Some(nav) = &self.nav else {
            return;
        };
        if self.range_warned || self.percent().is_none() {
            return;
        }
        let remaining = nav.remaining_m() / 1000.0;
        let text = match self.range_km() {
            Some(range) if remaining > range => tf(
                "Careful. This route has {} kilometres left, and the battery has about {}.",
                &[&format!("{remaining:.0}"), &format!("{range:.0}")],
            ),
            Some(_) => return,
            None => tf(
                "Careful. This route has {} kilometres left, and the battery is too low to say if it will make it.",
                &[&format!("{remaining:.0}")],
            ),
        };
        self.range_warned = true;
        self.say(text, true);
    }

    pub fn on_gps(&mut self, at: LatLon, speed_mps: Option<f64>) {
        self.gps = Some(at);
        self.speed_kmh = speed_mps.map(|s| s * 3.6);
        if self.route_pending {
            self.request_route();
        }
        let Some(nav) = self.nav.as_mut() else {
            return;
        };
        let events = nav.update(at);
        for event in events {
            match event {
                NavEvent::Say { text } => {
                    if self.settings.say_navigation {
                        self.say(text, true);
                    }
                }
                NavEvent::Arrived => {
                    if self.settings.say_navigation {
                        self.say(t("You have arrived."), true);
                    }
                    self.stop_navigation();
                    return;
                }
                NavEvent::Reroute => {
                    if self.settings.say_navigation {
                        self.say(t("Off route. Finding a new way."), true);
                    }
                    self.rerouting = true;
                    self.request_route();
                }
            }
        }
    }

    // ----- Notifications and calls -----

    pub fn on_notification(&mut self, package: &str, label: &str, title: &str, text: &str, is_call: bool) {
        if is_call {
            if self.settings.say_calls && self.flags.ringing() && !title.is_empty() {
                self.say(tf("Call from {}.", &[&title]), true);
            }
            return;
        }
        if !self.apps.iter().any(|a| a.package == package) {
            self.apps.push(AppEntry { package: package.to_string(), label: label.to_string() });
        }
        if self.settings.muted_apps.contains(package) {
            return;
        }
        self.notifications.push_front(NotifEntry {
            label: label.to_string(),
            title: title.to_string(),
            text: text.to_string(),
        });
        self.notifications.truncate(LOG_LENGTH);
        if let Some(slot) = self.dash_slot_for(package) {
            self.flags.set_app(slot, true);
        }
        if self.settings.notif_level == NotifLevel::Off {
            return;
        }
        // A busy group chat shouldn't talk over the whole ride: the first is read, the rest are counted.
        if let Some(burst) = self.notif_bursts.iter_mut().find(|b| b.package == package) {
            burst.extra += 1;
            return;
        }
        self.notif_bursts.push(NotifBurst {
            package: package.to_string(),
            label: label.to_string(),
            extra: 0,
            until: self.now + NOTIF_BURST_S,
        });
        let sentence = match self.settings.notif_level {
            NotifLevel::Off => return,
            NotifLevel::AppAndSender => format!("{label}. {title}."),
            NotifLevel::Full => format!("{label}. {title}: {}", clip(text, SPOKEN_TEXT_LIMIT)),
        };
        self.say(sentence, false);
    }

    pub fn on_notification_removed(&mut self, package: &str) {
        if let Some(slot) = self.settings.dash_slots.iter().position(|p| p == package) {
            self.flags.set_app(slot, false);
        }
    }

    /// The dash has six app lights. Apps claim them in the order they first notify.
    fn dash_slot_for(&mut self, package: &str) -> Option<usize> {
        if let Some(slot) = self.settings.dash_slots.iter().position(|p| p == package) {
            return Some(slot);
        }
        if self.settings.dash_slots.len() >= proto::APP_SLOTS {
            return None;
        }
        self.settings.dash_slots.push(package.to_string());
        self.save_settings();
        Some(self.settings.dash_slots.len() - 1)
    }

    pub fn toggle_app_muted(&mut self, package: &str) {
        if !self.settings.muted_apps.remove(package) {
            self.settings.muted_apps.insert(package.to_string());
        }
        self.save_settings();
    }

    pub fn on_call(&mut self, state: CallState) {
        if state == self.call {
            return;
        }
        match state {
            CallState::Ringing => {
                self.flags.call_ringing();
                self.was_ringing = true;
                if self.settings.say_calls {
                    self.say(t("Incoming call."), true);
                }
            }
            CallState::OffHook => {
                if self.was_ringing {
                    self.flags.call_stopped_ringing();
                }
                self.was_ringing = false;
            }
            CallState::Idle => {
                if self.was_ringing {
                    self.flags.call_stopped_ringing();
                    if self.settings.say_calls {
                        self.say(t("Missed call."), false);
                    }
                }
                self.was_ringing = false;
            }
        }
        self.call = state;
    }

    pub fn on_phone_battery(&mut self, percent: u8) {
        self.phone_battery = percent.min(100);
    }

    pub fn on_clock(&mut self, time: ClockTime) {
        self.clock = Some(time);
    }

    // ----- Voice -----

    pub fn open_notification_access(&mut self) {
        self.push(Command::OpenNotificationAccess);
    }

    pub fn set_voice_mode(&mut self, on: bool) {
        self.settings.voice_mode = on;
        self.save_settings();
        if on {
            let text = self.status_report();
            self.say(tf("Voice mode on. {}", &[&text]), true);
        }
    }

    pub fn change_speech_rate(&mut self, delta: f32) {
        self.settings.speech_rate = (self.settings.speech_rate + delta).clamp(0.6, 1.8);
        self.save_settings();
        self.push(Command::SetSpeechRate(self.settings.speech_rate));
    }

    /// Everything a rider would glance at, as one spoken sentence.
    pub fn status_report(&self) -> String {
        let mut parts = Vec::new();
        match self.link {
            Link::Online => parts.push(self.battery_sentence()),
            Link::NeedsPairing | Link::Refused => parts.push(t("The moped is not paired.").to_string()),
            _ => parts.push(t("The moped is not connected.").to_string()),
        }
        if let Some(eff) = self.measured_efficiency() {
            parts.push(tf(
                "This ride: {} kilometres at {} kilometres per kilowatt hour.",
                &[&format!("{:.1}", self.trip.km), &format!("{eff:.0}")],
            ));
        }
        if let Some(nav) = &self.nav {
            if let Some(m) = nav.current() {
                parts.push(tf(
                    "In {} metres, {}",
                    &[&format!("{:.0}", (nav.distance_to_next_m() / 10.0).round() * 10.0), &m.spoken_at],
                ));
            }
            parts.push(tf(
                "{} kilometres and {} minutes to go.",
                &[&format!("{:.1}", nav.remaining_m() / 1000.0), &format!("{:.0}", (nav.remaining_s() / 60.0).ceil())],
            ));
        }
        if let Some(live) = &self.live {
            if live.warning != 0 {
                parts.push(t("The moped has a warning on.").to_string());
            }
        }
        parts.join(" ")
    }

    /// The earpiece's button or a tap in voice mode.
    pub fn speak_status(&mut self) {
        let text = self.status_report();
        self.say(text, true);
    }

    pub fn repeat_last(&mut self) {
        if let Some(text) = self.spoken.front().cloned() {
            self.say(text, true);
        }
    }

    // ----- Demo -----

    pub fn start_demo(&mut self) {
        let (demo, route) = DemoRide::new();
        self.demo = Some(demo);
        self.demo_active = true;
        self.gps = Some(route.points[0]);
        let mut nav = Navigation::new(route);
        nav.along_m = crate::demo::START_ALONG_M;
        self.nav = Some(nav);
        self.set_link(Link::Online);
        self.tick(self.now);
    }

    pub fn stop_demo(&mut self) {
        self.demo = None;
        self.demo_active = false;
        self.live = None;
        self.trip = Trip::default();
        self.nav = None;
        self.gps = None;
        self.speed_kmh = None;
        self.notifications.clear();
        self.apps.clear();
        self.flags = DashFlags::default();
        self.announced_percent = None;
        self.set_link(if self.has_pairing() { Link::Idle } else { Link::NeedsPairing });
    }

    // ----- Rides -----

    /// A ride ends once the main switch has been off, or the link down, for a few seconds running.
    fn check_ride_end(&mut self) {
        let switched_on = self.live.as_ref().is_some_and(|l| l.switched_on);
        if self.trip.start_odometer_m.is_none() || (self.link == Link::Online && switched_on) {
            self.ride_off_since = None;
            return;
        }
        let since = *self.ride_off_since.get_or_insert(self.now);
        if self.now - since < RIDE_END_AFTER_S {
            return;
        }
        self.ride_off_since = None;
        self.trip.start_odometer_m = None;
        self.settings.total_km += self.trip.km;
        self.settings.total_kwh += self.trip.kwh;
        self.save_settings();
        if self.settings.say_consumption {
            if let Some(eff) = self.measured_efficiency() {
                let text = tf(
                    "Ride finished. {} kilometres at {} kilometres per kilowatt hour.",
                    &[&format!("{:.1}", self.trip.km), &format!("{eff:.0}")],
                );
                self.say(text, false);
            }
        }
    }

    // ----- Time -----

    fn finish_notif_bursts(&mut self) {
        let now = self.now;
        let (finished, waiting): (Vec<_>, Vec<_>) = self.notif_bursts.drain(..).partition(|b| now >= b.until);
        self.notif_bursts = waiting;
        for burst in finished.into_iter().filter(|b| b.extra > 0) {
            self.say(tf("{} more from {}.", &[&burst.extra, &burst.label]), false);
        }
    }

    pub fn tick(&mut self, now: f64) {
        self.now = now;
        self.finish_notif_bursts();
        if let Some(mut demo) = self.demo.take() {
            demo.step(self, now);
            self.demo = Some(demo);
            return;
        }
        self.check_ride_end();
        match self.link {
            Link::Scanning | Link::Connecting | Link::LoggingIn if now - self.link_since > CONNECT_TIMEOUT_S => {
                if self.link == Link::LoggingIn {
                    self.push(Command::Disconnect);
                }
                self.connect();
            }
            Link::Online => {
                if now >= self.next_clock {
                    self.next_clock = now + CLOCK_PERIOD_S;
                    self.flags_due = Some(now + FLAGS_DELAY_S);
                    if let Some(time) = self.clock {
                        let counter = self.next_counter();
                        self.push(Command::Write(proto::clock_frame(time, self.phone_battery, counter)));
                    }
                }
                if self.flags_due.is_some_and(|due| now >= due) {
                    self.flags_due = None;
                    let bits = self.flags.next_bits(self.settings.dash_calls);
                    let counter = self.next_counter();
                    self.push(Command::Write(proto::flags_frame(bits, counter)));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::{Maneuver, Route};
    use crate::protocol::{build_frame, build_live_frame, Pack};

    const CODE: &str = "ABCDEFGHIJKLMN123456";

    fn pack(percent: u8, full: f32) -> Pack {
        Pack {
            fitted: true,
            percent,
            full_ah: full,
            remaining_ah: full * f32::from(percent) / 100.0,
            voltage: 50.0,
            health: 100,
            ..Pack::default()
        }
    }

    fn live(percent: u8, odometer_m: u32) -> Live {
        let mut l = Live { odometer_m, switched_on: true, ..Live::default() };
        l.packs[0] = pack(percent, 20.0);
        l
    }

    fn feed(e: &mut Engine, l: &Live) {
        let bytes = build_live_frame(l, 0);
        e.on_rx(&bytes);
    }

    fn speech(e: &mut Engine) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(c) = e.poll_command() {
            if let Command::Speak { text, .. } = c {
                out.push(text);
            }
        }
        out
    }

    fn online() -> Engine {
        let mut e = Engine::new();
        assert!(e.set_pairing_code(CODE));
        e.on_ble_found("YSCCU_ABCDEFGHIJKLMN", "AA:BB");
        e.on_ble_ready();
        let accept = build_frame(proto::START_LOGIN_RESULT, &[(proto::GROUP_LOGIN_RESULT, vec![1])], 0);
        e.on_rx(&accept);
        assert_eq!(e.link, Link::Online);
        while e.poll_command().is_some() {}
        e
    }

    #[test]
    fn connection_runs_the_original_sequence() {
        let mut e = Engine::new();
        assert_eq!(e.link, Link::NeedsPairing);
        assert!(e.set_pairing_code(CODE));
        assert_eq!(e.link, Link::Scanning);
        assert_eq!(e.poll_command(), Some(Command::Scan));
        e.on_ble_found("SomeHeadphones", "00:11");
        assert!(e.found_devices.is_empty());
        e.on_ble_found("YSCCU_ABCDEFGHIJKLMN", "AA:BB");
        assert_eq!(e.link, Link::Connecting);
        assert_eq!(e.poll_command(), Some(Command::StopScan));
        assert_eq!(e.poll_command(), Some(Command::Connect("AA:BB".to_string())));
        e.on_ble_ready();
        let Some(Command::Write(first)) = e.poll_command() else {
            panic!("expected the login");
        };
        assert_eq!(&first[5..19], b"ABCDEFGHIJKLMN");
        assert_eq!(first[57], 1);
        // Reconnecting in the same run uses the zeroed form.
        e.on_ble_ready();
        let Some(Command::Write(second)) = e.poll_command() else {
            panic!("expected the login");
        };
        assert_eq!(&second[5..19], b"00000000000000");
        assert_eq!(second[57], 0);
    }

    #[test]
    fn device_with_other_id_is_listed_but_not_connected() {
        let mut e = Engine::new();
        e.set_pairing_code(CODE);
        e.on_ble_found("YSCCU_OTHERMOPED0000", "CC:DD");
        assert_eq!(e.link, Link::Scanning);
        assert_eq!(e.found_devices.len(), 1);
    }

    #[test]
    fn refused_login_is_announced_and_stops() {
        let mut e = Engine::new();
        e.set_pairing_code(CODE);
        e.choose_device("AA:BB");
        e.reconnecting = false;
        e.link = Link::LoggingIn;
        let refuse = build_frame(proto::START_LOGIN_RESULT, &[(proto::GROUP_LOGIN_RESULT, vec![0])], 0);
        e.on_rx(&refuse);
        assert_eq!(e.link, Link::Refused);
        assert!(speech(&mut e).iter().any(|s| s.contains("refused")));
    }

    #[test]
    fn range_uses_the_original_formula() {
        let mut e = online();
        feed(&mut e, &live(100, 1000));
        // 30 km/kWh * 100% * 20 Ah * 50 V = 30 km.
        assert!((e.range_km().unwrap() - 30.0).abs() < 0.01);
    }

    #[test]
    fn two_batteries_add_range_and_blend_percent() {
        let mut e = online();
        let mut l = live(100, 1000);
        l.packs[1] = pack(50, 20.0);
        feed(&mut e, &l);
        assert!((e.range_km().unwrap() - 45.0).abs() < 0.01);
        assert_eq!(e.percent(), Some(75));
    }

    #[test]
    fn range_is_withheld_when_charge_is_low_or_output_limited() {
        let mut e = online();
        feed(&mut e, &live(15, 1000));
        assert_eq!(e.range_km(), None);
        assert!(e.status_report().contains("Battery 15 percent."));
        feed(&mut e, &live(16, 1000));
        assert!(e.range_km().is_some());
        let mut two = live(15, 1000);
        two.packs[1] = pack(15, 20.0);
        feed(&mut e, &two);
        assert_eq!(e.range_km(), None);
        two.packs[1] = pack(16, 20.0);
        feed(&mut e, &two);
        assert!(e.range_km().is_some());
        let mut limited = live(80, 1000);
        limited.output_limited = true;
        feed(&mut e, &limited);
        assert_eq!(e.range_km(), None);
    }

    #[test]
    fn full_range_uses_nominal_capacity_per_fitted_battery() {
        let mut e = online();
        feed(&mut e, &live(50, 1000));
        // 30 km/kWh * 20 Ah * 50 V = 30 km.
        assert!((e.full_range_km().unwrap() - 30.0).abs() < 0.01);
        let mut two = live(50, 1000);
        two.packs[1] = pack(40, 20.0);
        feed(&mut e, &two);
        assert!((e.full_range_km().unwrap() - 60.0).abs() < 0.01);
    }

    fn ride_of_3_km(e: &mut Engine) {
        feed(e, &live(80, 10_000));
        let mut after = live(70, 13_000);
        after.packs[0].remaining_ah = 14.0;
        feed(e, &after);
    }

    #[test]
    fn ride_ends_after_five_seconds_with_the_switch_off_and_adds_to_the_average() {
        let mut e = online();
        ride_of_3_km(&mut e);
        let mut off = live(70, 13_000);
        off.packs[0].remaining_ah = 14.0;
        off.switched_on = false;
        feed(&mut e, &off);
        e.tick(100.0);
        e.tick(104.0);
        assert!(e.trip.start_odometer_m.is_some());
        speech(&mut e);
        e.tick(105.0);
        assert!(e.trip.start_odometer_m.is_none());
        assert!((e.settings.total_km - 3.0).abs() < 0.01);
        assert!((e.settings.total_kwh - 0.1).abs() < 0.001);
        assert!(speech(&mut e).iter().any(|t| t.starts_with("Ride finished.")));
        // Parked and off: no new ride starts.
        feed(&mut e, &off);
        assert!(e.trip.start_odometer_m.is_none());
        // Switching back on starts a new one, and the range keeps the average from before.
        feed(&mut e, &live(70, 13_000));
        assert!(e.trip.start_odometer_m.is_some());
        assert_eq!(e.trip.km, 0.0);
        assert!((e.efficiency() - 30.0).abs() < 0.5);
    }

    #[test]
    fn short_dropout_keeps_the_ride_going() {
        let mut e = online();
        ride_of_3_km(&mut e);
        e.tick(100.0);
        e.on_ble_disconnected();
        e.tick(103.0);
        assert!(e.trip.start_odometer_m.is_some());
        e.on_ble_ready();
        e.link = Link::Online;
        e.tick(104.0);
        e.tick(120.0);
        assert!(e.trip.start_odometer_m.is_some());
        e.on_ble_disconnected();
        e.tick(121.0);
        e.tick(126.0);
        assert!(e.trip.start_odometer_m.is_none());
    }

    #[test]
    fn ignored_fault_codes_stay_silent() {
        let mut e = online();
        let mut l = live(80, 1000);
        feed(&mut e, &l);
        speech(&mut e);
        l.warning = 1;
        l.vehicle_errors = vec![0xC111];
        feed(&mut e, &l);
        let said = speech(&mut e);
        assert!(said.iter().any(|s| s.contains("system warning")));
        assert!(!said.iter().any(|s| s.contains("fault")));
    }

    #[test]
    fn efficiency_is_measured_after_two_km_and_ignores_silly_values() {
        let mut e = online();
        feed(&mut e, &live(100, 10_000));
        assert_eq!(e.efficiency(), DEFAULT_KM_PER_KWH);
        // 3 km on 2 Ah * 50 V = 0.1 kWh gives 30 km/kWh; here 4 Ah gives 15 km/kWh.
        let mut l = live(80, 13_000);
        l.packs[0].remaining_ah = 16.0;
        feed(&mut e, &l);
        assert!((e.measured_efficiency().unwrap() - 15.0).abs() < 0.1);
        assert!((e.efficiency() - 15.0).abs() < 0.1);
        // Below 5 km/kWh falls back to the default.
        let mut poor = live(10, 13_100);
        poor.packs[0].remaining_ah = 2.0;
        feed(&mut e, &poor);
        assert!(e.measured_efficiency().unwrap() < MIN_VALID_KM_PER_KWH);
        assert_eq!(e.efficiency(), DEFAULT_KM_PER_KWH);
    }

    #[test]
    fn charging_restarts_the_trip() {
        let mut e = online();
        feed(&mut e, &live(50, 10_000));
        feed(&mut e, &live(45, 12_000));
        assert!(e.trip.km > 1.9);
        feed(&mut e, &live(90, 12_000));
        assert_eq!(e.trip.km, 0.0);
    }

    #[test]
    fn charge_time_follows_the_formula() {
        assert!((Engine::charge_hours(50, 100) - 4.5).abs() < 1e-6);
        assert_eq!(Engine::charge_hours(100, 100), 0.0);
        // 8 * 37 / 100 rounds down to 2 whole hours first, as in the original.
        assert!((Engine::charge_hours(63, 96) - 2.16).abs() < 1e-6);
        assert_eq!(Engine::charge_hours(99, 100), 0.0);
    }

    #[test]
    fn battery_is_announced_in_steps_and_more_often_when_low() {
        let mut e = online();
        feed(&mut e, &live(55, 1000));
        assert!(speech(&mut e)[0].contains("Battery 55 percent"));
        feed(&mut e, &live(50, 1000));
        assert!(speech(&mut e).is_empty());
        feed(&mut e, &live(45, 1000));
        assert!(speech(&mut e)[0].contains("Battery 45"));
        feed(&mut e, &live(20, 1000));
        assert!(speech(&mut e)[0].contains("Battery 20"));
        feed(&mut e, &live(15, 1000));
        assert!(speech(&mut e)[0].contains("Battery 15"));
    }

    #[test]
    fn warnings_and_faults_speak_once_with_the_code_spelled_out() {
        let mut e = online();
        let mut l = live(80, 1000);
        feed(&mut e, &l);
        speech(&mut e);
        l.warning = 1;
        l.vehicle_errors = vec![0x0A12];
        feed(&mut e, &l);
        let said = speech(&mut e);
        assert!(said.iter().any(|s| s.contains("system warning")));
        assert!(said.iter().any(|s| s.contains("fault 0 A 1 2")));
        feed(&mut e, &l);
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn low_flag_speaks_on_the_rising_edge() {
        let mut e = online();
        let mut l = live(40, 1000);
        l.packs[0].low = true;
        feed(&mut e, &l);
        assert!(speech(&mut e).iter().any(|s| s == "Battery low."));
        feed(&mut e, &l);
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn consumption_is_spoken_every_five_km() {
        let mut e = online();
        feed(&mut e, &live(90, 1000));
        speech(&mut e);
        let mut l = live(89, 6_100);
        l.packs[0].remaining_ah = 16.0;
        feed(&mut e, &l);
        assert!(speech(&mut e).iter().any(|s| s.contains("per kilowatt hour")));
    }

    #[test]
    fn voice_mode_off_stays_silent_but_keeps_the_log() {
        let mut e = online();
        e.settings.voice_mode = false;
        e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        assert!(speech(&mut e).is_empty());
        assert_eq!(e.spoken.len(), 1);
    }

    #[test]
    fn notification_reaches_dash_flag_and_voice_with_levels() {
        let mut e = online();
        e.on_notification("a.b", "Chat", "Sam", "Hello there", false);
        assert!(e.flags.app_on(0));
        assert_eq!(speech(&mut e), vec!["Chat. Sam: Hello there".to_string()]);
        e.settings.notif_level = NotifLevel::AppAndSender;
        e.tick(100.0);
        e.on_notification("a.b", "Chat", "Sam", "Secret", false);
        assert_eq!(speech(&mut e), vec!["Chat. Sam.".to_string()]);
        e.on_notification_removed("a.b");
        assert!(!e.flags.app_on(0));
    }

    #[test]
    fn muted_app_is_silent_and_keeps_no_text() {
        let mut e = online();
        e.toggle_app_muted("a.b");
        e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        assert!(speech(&mut e).is_empty());
        assert!(e.notifications.is_empty());
        assert!(!e.flags.app_on(0));
    }

    #[test]
    fn apps_claim_six_dash_slots_then_stop() {
        let mut e = online();
        for i in 0..8 {
            e.on_notification(&format!("app.{i}"), "App", "T", "x", false);
        }
        assert_eq!(e.settings.dash_slots.len(), proto::APP_SLOTS);
        assert!(e.flags.app_on(5));
        // The seventh app still speaks, it just has no light.
        assert!(!speech(&mut e).is_empty());
    }

    #[test]
    fn long_notifications_are_clipped_safely() {
        let long = "é".repeat(500);
        let clipped = clip(&long, SPOKEN_TEXT_LIMIT);
        assert_eq!(clipped.chars().count(), SPOKEN_TEXT_LIMIT + 3);
    }

    #[test]
    fn call_sequence_sets_flags_and_voice() {
        let mut e = online();
        e.on_call(CallState::Ringing);
        assert_eq!(e.flags.clone().next_bits(true) & proto::INCOMING_ON, proto::INCOMING_ON);
        e.flags.next_bits(true);
        e.on_notification("dialer", "Phone", "Sam", "", true);
        e.on_call(CallState::Idle);
        assert_eq!(e.flags.clone().next_bits(true) & proto::MISSED_ON, proto::MISSED_ON);
        let said = speech(&mut e);
        assert_eq!(said, vec!["Incoming call.", "Call from Sam.", "Missed call."]);
    }

    #[test]
    fn answered_call_sends_the_same_flag_as_the_original_but_says_nothing() {
        let mut e = online();
        e.on_call(CallState::Ringing);
        e.flags.next_bits(true);
        speech(&mut e);
        e.on_call(CallState::OffHook);
        assert_eq!(e.flags.next_bits(true) & proto::MISSED_ON, proto::MISSED_ON);
        e.on_call(CallState::Idle);
        assert_eq!(e.flags.next_bits(true) & proto::MISSED_ON, 0);
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn outgoing_call_touches_no_call_light() {
        let mut e = online();
        e.on_call(CallState::OffHook);
        e.on_call(CallState::Idle);
        assert_eq!(e.flags.next_bits(true) & (proto::MISSED_ON | proto::INCOMING_ON), 0);
    }

    #[test]
    fn the_call_light_setting_reaches_the_dash_message() {
        let mut e = online();
        e.settings.dash_calls = false;
        e.on_clock(ClockTime { year: 2026, month: 1, day: 2, hour: 3, minute: 4, second: 5 });
        e.on_call(CallState::Ringing);
        while e.poll_command().is_some() {}
        e.tick(100.0);
        e.tick(100.1);
        let mut flags = None;
        while let Some(command) = e.poll_command() {
            if let Command::Write(bytes) = command {
                if bytes[2..4] == [0x05, 0x8B] {
                    flags = Some(u16::from_be_bytes([bytes[5], bytes[6]]));
                }
            }
        }
        assert_eq!(flags.unwrap() & proto::INCOMING_ON, 0);
        assert!(!Settings::from_text(&e.settings.to_text()).dash_calls);
    }

    #[test]
    fn clock_then_flags_go_out_every_second_while_online() {
        let mut e = online();
        e.on_clock(ClockTime { year: 2026, month: 1, day: 2, hour: 3, minute: 4, second: 5 });
        e.tick(100.0);
        let Some(Command::Write(clock)) = e.poll_command() else {
            panic!("expected the clock");
        };
        assert_eq!(&clock[2..4], &[0x05, 0x8A]);
        assert!(e.poll_command().is_none());
        e.tick(100.1);
        let Some(Command::Write(flags)) = e.poll_command() else {
            panic!("expected the flags");
        };
        assert_eq!(&flags[2..4], &[0x05, 0x8B]);
        e.tick(100.5);
        assert!(e.poll_command().is_none());
        e.tick(101.0);
        assert!(matches!(e.poll_command(), Some(Command::Write(_))));
    }

    #[test]
    fn stalled_connection_retries_after_the_timeout() {
        let mut e = Engine::new();
        e.set_pairing_code(CODE);
        while e.poll_command().is_some() {}
        e.tick(5.0);
        assert!(e.poll_command().is_none());
        e.tick(16.0);
        assert_eq!(e.poll_command(), Some(Command::Scan));
    }

    #[test]
    fn route_needs_gps_and_waits_for_it() {
        let mut e = online();
        e.navigate_to(&Place { name: "Home".to_string(), at: (48.1, 2.1) });
        assert_eq!(e.nav_status, "Waiting for GPS...");
        assert!(e.poll_command().is_none());
        e.on_gps((48.0, 2.0), Some(5.0));
        assert!(matches!(e.poll_command(), Some(Command::FetchRoute { .. })));
        assert_eq!(e.speed_kmh, Some(18.0));
    }

    fn short_route() -> Route {
        let points: Vec<LatLon> = (0..=20).map(|i| (48.0 + f64::from(i) * 0.001, 2.0)).collect();
        let m = |kind, begin, text: &str| Maneuver {
            kind,
            text: text.to_string(),
            spoken_before: text.to_string(),
            spoken_at: text.to_string(),
            begin,
        };
        Route::new("Home", points, vec![m(1, 0, "Go north"), m(4, 20, "Arrive")], 300.0)
    }

    const NO_PATH: &str = r#"{"error_code":442,"error":"No path could be found for input"}"#;

    fn route_url(e: &mut Engine) -> String {
        match e.poll_command() {
            Some(Command::FetchRoute { url, .. }) => url,
            other => panic!("expected a route request, got {other:?}"),
        }
    }

    #[test]
    fn unreachable_destination_retries_once_for_the_nearest_road() {
        let mut e = online();
        e.on_gps((48.0, 2.0), None);
        e.navigate_to(&Place { name: "Palace".to_string(), at: (48.1, 2.1) });
        assert!(!route_url(&mut e).contains("radius"));
        e.on_route_reply(Ok(NO_PATH), "Palace");
        assert!(route_url(&mut e).contains("radius"));
        // The second refusal is final, and it says so.
        e.on_route_reply(Ok(NO_PATH), "Palace");
        assert!(e.poll_command().is_some());
        assert!(e.nav_status.starts_with("No route"));
    }

    #[test]
    fn reply_after_stopping_navigation_is_ignored() {
        let mut e = online();
        e.on_gps((48.0, 2.0), None);
        e.navigate_to(&Place { name: "Home".to_string(), at: (48.1, 2.1) });
        e.stop_navigation();
        let json = r#"{"trip":{"summary":{"time":120.0},"legs":[{"shape":"_p~iF~ps|U_ulLnnqC",
          "maneuvers":[{"type":1,"instruction":"Go.","begin_shape_index":0}]}]}}"#;
        e.on_route_reply(Ok(json), "Home");
        assert!(e.nav.is_none());
    }

    #[test]
    fn arriving_ends_navigation_with_a_sentence() {
        let mut e = online();
        e.nav = Some(Navigation::new(short_route()));
        for i in 0..=20 {
            e.on_gps((48.0 + f64::from(i) * 0.001, 2.0), None);
        }
        assert!(e.nav.is_none());
        assert!(speech(&mut e).iter().any(|s| s == "You have arrived."));
    }

    #[test]
    fn warns_when_the_route_is_longer_than_the_range() {
        let mut e = online();
        feed(&mut e, &live(2, 1000));
        speech(&mut e);
        e.nav = Some(Navigation::new(short_route()));
        e.check_range_for_route();
        assert!(speech(&mut e).iter().any(|s| s.starts_with("Careful.")));
        e.check_range_for_route();
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn warns_with_figures_when_the_range_is_known_and_short() {
        let mut e = online();
        let mut l = live(20, 1000);
        l.packs[0] = pack(20, 2.0);
        feed(&mut e, &l);
        speech(&mut e);
        e.nav = Some(Navigation::new(short_route()));
        e.check_range_for_route();
        assert!(speech(&mut e).iter().any(|s| s.contains("the battery has about 1.")));
    }

    #[test]
    fn no_warning_when_the_route_fits() {
        let mut e = online();
        feed(&mut e, &live(90, 1000));
        speech(&mut e);
        e.nav = Some(Navigation::new(short_route()));
        e.check_range_for_route();
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn a_burst_of_messages_is_read_once_then_counted() {
        let mut e = online();
        e.tick(100.0);
        speech(&mut e);
        for _ in 0..4 {
            e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        }
        e.on_notification("c.d", "Mail", "Ann", "Hi", false);
        assert_eq!(speech(&mut e), vec!["Chat. Sam: Hello".to_string(), "Mail. Ann: Hi".to_string()]);
        e.tick(114.0);
        assert!(speech(&mut e).is_empty());
        e.tick(115.0);
        assert_eq!(speech(&mut e), vec!["3 more from Chat.".to_string()]);
        // Dash lights and the list still get every one.
        assert_eq!(e.notifications.len(), 5);
        assert!(e.flags.app_on(1));
        e.on_notification("a.b", "Chat", "Sam", "Again", false);
        assert_eq!(speech(&mut e), vec!["Chat. Sam: Again".to_string()]);
    }

    #[test]
    fn during_a_call_only_urgent_speech_gets_through() {
        let mut e = online();
        e.on_call(CallState::Ringing);
        e.on_call(CallState::OffHook);
        speech(&mut e);
        e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        assert!(speech(&mut e).is_empty());
        assert_eq!(e.spoken.front().map(String::as_str), Some("Chat. Sam: Hello"));
        e.say("Turn left.", true);
        assert_eq!(speech(&mut e), vec!["Turn left.".to_string()]);
    }

    #[test]
    fn recent_places_stay_unique_newest_first_and_capped() {
        let mut e = online();
        for i in 0..7 {
            e.navigate_to(&Place { name: format!("Place {i}"), at: (48.0, 2.0) });
        }
        e.navigate_to(&Place { name: "Place 3".to_string(), at: (48.0, 2.0) });
        let names: Vec<&str> = e.settings.recents.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Place 3", "Place 6", "Place 5", "Place 4", "Place 2"]);
    }

    #[test]
    fn bluetooth_going_off_is_said_once_and_only_when_paired() {
        let mut e = Engine::new();
        e.on_bluetooth(BluetoothState::Off);
        assert!(speech(&mut e).is_empty());
        e.on_bluetooth(BluetoothState::On);
        let mut e = online();
        e.on_bluetooth(BluetoothState::Off);
        assert_eq!(speech(&mut e), vec!["Bluetooth is off.".to_string()]);
        e.on_bluetooth(BluetoothState::Off);
        assert!(speech(&mut e).is_empty());
    }

    #[test]
    fn test_voice_speaks_even_before_pairing() {
        let mut e = Engine::new();
        e.test_voice();
        assert_eq!(speech(&mut e).len(), 1);
    }

    #[test]
    fn status_report_covers_what_a_rider_needs() {
        let mut e = online();
        let mut l = live(60, 1000);
        feed(&mut e, &l);
        l.warning = 1;
        feed(&mut e, &l);
        e.nav = Some(Navigation::new(short_route()));
        let report = e.status_report();
        assert!(report.contains("Battery 60 percent"));
        assert!(report.contains("to go"));
        assert!(report.contains("warning"));
        assert!(Engine::new().status_report().contains("not paired"));
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let mut s = Settings::default();
        s.ccu_id = "ABC".to_string();
        s.passkey = "123".to_string();
        s.voice_mode = false;
        s.notif_level = NotifLevel::AppAndSender;
        s.speech_rate = 1.2;
        s.muted_apps.insert("a.b".to_string());
        s.dash_slots = vec!["c.d".to_string(), "e.f".to_string()];
        s.appearance = Appearance::Dark;
        s.high_contrast = true;
        s.language = Some(Lang::It);
        s.recents = vec![Place { name: "Home; sweet, home".to_string(), at: (48.5, 16.25) }];
        let back = Settings::from_text(&s.to_text());
        assert_eq!(back.ccu_id, "ABC");
        assert_eq!(back.passkey, "123");
        assert!(!back.voice_mode);
        assert!(back.say_battery);
        assert_eq!(back.notif_level, NotifLevel::AppAndSender);
        assert!((back.speech_rate - 1.2).abs() < 1e-6);
        assert!(back.muted_apps.contains("a.b"));
        assert_eq!(back.dash_slots.len(), 2);
        assert_eq!(back.appearance, Appearance::Dark);
        assert!(back.high_contrast);
        assert_eq!(back.language, Some(Lang::It));
        assert_eq!(back.recents[0].name, "Home; sweet, home");
        assert_eq!(back.recents[0].at, (48.5, 16.25));
        assert_eq!(Settings::from_text("language=system").language, None);
    }

    #[test]
    fn demo_runs_through_the_real_parser() {
        let mut e = Engine::new();
        e.start_demo();
        assert!(e.percent().is_some());
        assert!(e.nav.is_some());
        e.stop_demo();
        assert!(e.live.is_none());
        assert_eq!(e.link, Link::NeedsPairing);
    }
}
