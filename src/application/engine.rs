//! Everything that keeps running during a ride, with or without a screen. The platform layer feeds
//! it events through the `on_*` methods and carries out the `Command`s it queues.

use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::demo::DemoRide;
use crate::locale::{self, t, tf, Lang};
use crate::nav::{self, LatLon, NavEvent, Navigation, Place};
use super::ports::{MapServices, Release, ReleaseFeed};
use crate::domain::geo;
use crate::domain::places::{remember_recent, BookmarkKind, Bookmarks};
use crate::domain::speed_limit::{LimitEvent, LimitWatcher, Road};
use crate::protocol::{self as proto, ClockTime, DashFlags, Frame, Live, BatteryRecord, Vehicle};

const BATTERY_RECORDS_KEPT: usize = 4;
const UPDATE_RETRY_S: f64 = 600.0;
const RECORDED_LIVE_FRAMES: usize = 1500;
const RECORDED_RARE_FRAMES: usize = 40;
const TILE_RETRY_S: f64 = 30.0;
const MOVING_KMH: f64 = 7.0;
const HEADING_MIN_MOVE_M: f64 = 8.0;
/// A map picture: zoom level, column, row.
pub type TileKey = (u8, u32, u32);

const CONNECT_TIMEOUT_S: f64 = 15.0;
/// The first time, Pillion looks for the moped this long before giving up until asked again.
const SCAN_GIVE_UP_S: f64 = 45.0;
/// Asking the map for the road under the rider: at most this often, and only after moving this far.
const ROAD_EVERY_S: f64 = 5.0;
const ROAD_EVERY_M: f64 = 60.0;
/// The area the battery reaches is asked again after the rider or the range moves this much.
const REACH_AGAIN_KM: f64 = 3.0;
const REACH_EVERY_S: f64 = 120.0;
/// Routes longer than this get a "halfway there".
const HALFWAY_FROM_M: f64 = 4000.0;
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
    /// Any other web request: the answer comes back to `on_fetch` with the same key.
    Fetch { key: String, url: String },
    SetPower(Power),
    /// Keep a low-power request open for the moped at this address, without scanning.
    WaitFor(String),
    AskText { key: String, title: String, hint: String, dark: bool },
    ScanQr { title: String, torch: String, type_it: String, cancel: String },
    CheckUpdate { url: String },
    InstallUpdate { url: String, sha256: String },
    VoiceDownload,
    VoiceRemove,
    VoiceEngine(VoiceEngine),
    FetchTile { z: u8, x: u32, y: u32 },
    Share(String),
    Haptic,
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
    /// The moped is off (or out of reach): nothing is searched. Android keeps one low-power request open with the
    /// Bluetooth chip, so the link comes back by itself when the moped is switched on.
    Waiting,
    /// The first search found nothing: stopped until the rider asks again.
    NotFound,
}

/// How much the phone should do for Pillion right now. The Java side turns this into GPS settings and how often it
/// polls the Engine, which is most of Pillion's battery use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    /// Riding or navigating: GPS every second, quick polling.
    Ride,
    /// The screen is open but nothing's moving: a slow GPS fix for the map, quick polling for taps.
    Look,
    /// Screen off and not riding: no GPS, slow polling.
    Rest,
}

impl Power {
    /// How often the engine ticks and the phone polls it, in milliseconds.
    pub fn pace_ms(self) -> u64 {
        match self {
            Power::Ride | Power::Look => 50,
            Power::Rest => 1000,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MopedColor {
    Teal,
    White,
    Black,
}

fn clock_day(time: ClockTime) -> String {
    format!("{:04}-{:02}-{:02}", time.year, time.month, time.day)
}

/// Where getting a new version of the app stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    CheckFailed,
    Available,
    Downloading(u8),
    NeedsPermission,
    Installing,
    InstallFailed,
}

/// Which speech engine talks. Auto picks the phone's own voice when it has a good one, otherwise the natural voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceEngine {
    Auto,
    System,
    Natural,
}

impl VoiceEngine {
    pub fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::System => "system",
            Self::Natural => "natural",
        }
    }

    fn from_code(code: &str) -> Self {
        match code {
            "system" => Self::System,
            "natural" => Self::Natural,
            _ => Self::Auto,
        }
    }
}

/// Where the natural voice for the current language stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoicePack {
    None,
    Downloading(u8),
    Ready,
    Failed,
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
    pub moped_color: MopedColor,
    pub voice_engine: VoiceEngine,
    pub update_auto: bool,
    update_checked_on: String,
    pub appearance: Appearance,
    pub high_contrast: bool,
    /// `None` follows the phone.
    pub language: Option<Lang>,
    pub recents: Vec<Place>,
    pub bookmarks: Bookmarks,
    /// Say the speed limit when riding onto a road with a different one.
    pub say_limits: bool,
    /// Say so when riding over the limit for a while.
    pub warn_speeding: bool,
    /// The glass look (see-through surfaces over a soft wash of colour).
    pub glass: bool,
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
            moped_color: MopedColor::Teal,
            voice_engine: VoiceEngine::Auto,
            update_auto: true,
            update_checked_on: String::new(),
            appearance: Appearance::System,
            high_contrast: false,
            language: None,
            recents: Vec::new(),
            bookmarks: Bookmarks::default(),
            say_limits: true,
            warn_speeding: true,
            glass: true,
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
            "update_auto={}\nupdate_checked={}\nvoice_engine={}\nccu={}\npass={}\naddr={}\nvoice={}\nbattery={}\nconsumption={}\nnav={}\ncalls={}\nnotif={}\nrate={}\nmuted={}\nslots={}\ntotal_km={}\ntotal_kwh={}\ndash_calls={}\nmoped_color={}\nappearance={}\ncontrast={}\nlanguage={}\n{}",
            flag(self.update_auto),
            self.update_checked_on,
            self.voice_engine.code(),
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
            match self.moped_color {
                MopedColor::Teal => "teal",
                MopedColor::White => "white",
                MopedColor::Black => "black",
            },
            match self.appearance {
                Appearance::System => "system",
                Appearance::Light => "light",
                Appearance::Dark => "dark",
            },
            flag(self.high_contrast),
            self.language.map_or("system", Lang::code),
            self.recents
                .iter()
                .map(|p| format!("recent={};{};{}\n", p.at.0, p.at.1, p.full_name().replace('\n', " ")))
                .chain(self.bookmarks.items.iter().map(|b| format!(
                    "bookmark={};{};{};{}\n",
                    b.kind.code(), b.place.at.0, b.place.at.1, b.place.full_name().replace('\n', " ")
                )))
                .chain([format!("limits={}\nspeeding={}\nglass={}\n", flag(self.say_limits), flag(self.warn_speeding), flag(self.glass))])
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
                // Before 0.6 there was only an on/off switch, on unless the rider turned it off.
                "natural_voice" => s.voice_engine = if on { VoiceEngine::Auto } else { VoiceEngine::System },
                "voice_engine" => s.voice_engine = VoiceEngine::from_code(value),
                "update_auto" => s.update_auto = on,
                "update_checked" => s.update_checked_on = value.to_string(),
                "moped_color" => {
                    s.moped_color = match value {
                        "black" => MopedColor::Black,
                        "white" => MopedColor::White,
                        _ => MopedColor::Teal,
                    };
                }
                "language" => s.language = Lang::from_code(value),
                "recent" => {
                    let mut parts = value.splitn(3, ';');
                    let lat = parts.next().and_then(|v| v.parse().ok());
                    let lon = parts.next().and_then(|v| v.parse().ok());
                    if let (Some(lat), Some(lon), Some(name)) = (lat, lon, parts.next()) {
                        s.recents.push(Place::new(name, (lat, lon)));
                    }
                }
                "bookmark" => {
                    let mut parts = value.splitn(4, ';');
                    let kind = BookmarkKind::from_code(parts.next().unwrap_or_default());
                    let lat = parts.next().and_then(|v| v.parse().ok());
                    let lon = parts.next().and_then(|v| v.parse().ok());
                    if let (Some(lat), Some(lon), Some(name)) = (lat, lon, parts.next()) {
                        s.bookmarks.save(kind, Place::new(name, (lat, lon)));
                    }
                }
                "limits" => s.say_limits = on,
                "speeding" => s.warn_speeding = on,
                "glass" => s.glass = on,
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
    fitted: [bool; 2],
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
    pub voice_pack: VoicePack,
    pub update_state: UpdateState,
    /// A daily check that fails isn't worth mentioning, unlike one the rider asked for.
    update_quietly: bool,
    update_tried_at: Option<f64>,
    pub update: Option<Release>,
    map: Box<dyn MapServices>,
    releases: Box<dyn ReleaseFeed>,
    /// Speed limits of the roads under the rider.
    pub limits: LimitWatcher,
    road_asked: Option<(LatLon, f64)>,
    road_pending: bool,
    /// The road at each point of the route being followed, when the map gave them.
    route_roads: Vec<Option<Road>>,
    /// The area the battery reaches on roads, as outlines, and where and for how far it was asked.
    pub reach: Vec<Vec<LatLon>>,
    reach_for: Option<(LatLon, f64, f64)>,
    reach_pending: bool,
    /// Whether the screen is on and Pillion is showing; the background service runs without it.
    pub screen_on: bool,
    power: Option<Power>,
    pending_bookmark: Option<LatLon>,
    halfway_said: bool,
    pub vehicle: Option<Vehicle>,
    pub battery_records: Vec<BatteryRecord>,
    pub recording: bool,
    recorded_live: VecDeque<String>,
    recorded_rare: Vec<String>,
    /// The way the rider is going, in degrees clockwise from north.
    pub heading_deg: Option<f64>,
    last_fix: Option<LatLon>,
    tiles_asked: HashMap<TileKey, Option<f64>>,
    /// Map pictures that have arrived, for the screen to pick up.
    pub tiles_ready: Vec<(TileKey, String)>,
    pub bluetooth: BluetoothState,
    pub location_on: bool,
    /// What the phone itself is set to. `None` where the platform doesn't say.
    pub system_dark: Option<bool>,
    system_language: Option<Lang>,
    spoken_language: Option<Lang>,
    notif_bursts: Vec<NotifBurst>,
    snack: Option<String>,
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

impl Engine {
    /// An engine using these services; `crate::wiring` gives the app the real ones.
    pub fn with(map: Box<dyn MapServices>, releases: Box<dyn ReleaseFeed>) -> Self {
        Self {
            map,
            releases,
            limits: LimitWatcher::default(),
            road_asked: None,
            road_pending: false,
            route_roads: Vec::new(),
            reach: Vec::new(),
            reach_for: None,
            reach_pending: false,
            screen_on: true,
            power: None,
            pending_bookmark: None,
            halfway_said: false,
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
            voice_pack: VoicePack::None,
            update_state: UpdateState::Idle,
            update_quietly: false,
            update_tried_at: None,
            update: None,
            vehicle: None,
            battery_records: Vec::new(),
            recording: false,
            recorded_live: VecDeque::new(),
            recorded_rare: Vec::new(),
            heading_deg: None,
            last_fix: None,
            tiles_asked: HashMap::new(),
            tiles_ready: Vec::new(),
            bluetooth: BluetoothState::On,
            location_on: true,
            system_dark: None,
            system_language: None,
            spoken_language: None,
            notif_bursts: Vec::new(),
            snack: None,
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
            self.say_anyway(t("Bluetooth is off."), true);
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
        // It speaks even with speaking turned off, or it wouldn't test anything.
        let text = t("This is how I'll sound on your ride. If you can hear me in your earpiece, you're all set.");
        self.push(Command::Speak { text: text.to_string(), urgent: true });
        self.snack = Some(t("Listen for it in your earpiece.").to_string());
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

    /// Ride speech: directions, battery, limits, notifications. The rider only hears it while the moped is
    /// connected (or on the demo ride), so the phone stays quiet in a pocket off the moped.
    fn say(&mut self, text: impl Into<String>, urgent: bool) {
        let riding = self.link == Link::Online || self.demo_active;
        self.speak_logged(text.into(), urgent, riding);
    }

    /// Speech about the link itself, or something the rider asked for: it's spoken even when the moped isn't connected.
    fn say_anyway(&mut self, text: impl Into<String>, urgent: bool) {
        self.speak_logged(text.into(), urgent, true);
    }

    fn speak_logged(&mut self, text: String, urgent: bool, allowed: bool) {
        self.spoken.push_front(text.clone());
        self.spoken.truncate(LOG_LENGTH);
        // During a call only what can't wait is spoken over it.
        let on_a_call = self.call == CallState::OffHook;
        if allowed && self.settings.voice_mode && (urgent || !on_a_call) {
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
        self.snack = Some(t("Code saved. Looking for your moped.").to_string());
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
                // Usually the moped was switched off. Don't hunt for it: wait quietly until it's back.
                self.wait_for_moped();
                if was_online && self.switched_on_said != Some(false) {
                    self.say_anyway(t("Moped connection lost."), true);
                }
            }
            _ => {}
        }
    }

    /// Stop searching and leave one low-power request open with Android, which reconnects by itself.
    fn wait_for_moped(&mut self) {
        if self.settings.address.is_empty() {
            self.push(Command::StopScan);
            self.set_link(Link::NotFound);
        } else {
            self.set_link(Link::Waiting);
            self.push(Command::WaitFor(self.settings.address.clone()));
        }
        self.limits.reset();
    }

    pub fn on_rx(&mut self, bytes: &[u8]) {
        if self.recording {
            self.record_frame(bytes);
        }
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
                self.say_anyway(t("The moped refused the pairing code."), true);
                self.push(Command::Disconnect);
                if retrying {
                    self.connect();
                }
            }
            return;
        }
        if let Some(vehicle) = proto::parse_vehicle(&frame) {
            self.vehicle = Some(vehicle);
            return;
        }
        if let Some(record) = proto::parse_battery_record(&frame) {
            self.battery_records.retain(|r| r.id != record.id);
            self.battery_records.push(record);
            self.battery_records.truncate(BATTERY_RECORDS_KEPT);
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

    // ----- Recording what the moped sends -----

    fn record_frame(&mut self, bytes: &[u8]) {
        let line = format!("{:.2} {}", self.now, proto::to_hex(bytes));
        if bytes.first() == Some(&proto::START_LIVE) {
            self.recorded_live.push_back(line);
            if self.recorded_live.len() > RECORDED_LIVE_FRAMES {
                self.recorded_live.pop_front();
            }
        } else if self.recorded_rare.len() < RECORDED_RARE_FRAMES {
            self.recorded_rare.push(line);
        }
    }

    pub fn recorded_count(&self) -> usize {
        self.recorded_live.len() + self.recorded_rare.len()
    }

    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
        if on {
            self.recorded_live.clear();
            self.recorded_rare.clear();
        }
    }

    pub fn share_recording(&mut self) {
        let mut text = self.recorded_rare.join("\n");
        for line in &self.recorded_live {
            text.push('\n');
            text.push_str(line);
        }
        self.push(Command::Share(text));
    }

    // ----- New versions of the app -----

    pub fn check_for_update(&mut self) {
        self.update_state = UpdateState::Checking;
        self.push(Command::CheckUpdate { url: self.releases.latest_url() });
    }

    /// The rider asked, so they want to know how it went.
    pub fn check_for_update_now(&mut self) {
        self.update_quietly = false;
        self.check_for_update();
    }

    pub fn set_update_auto(&mut self, on: bool) {
        self.settings.update_auto = on;
        self.save_settings();
    }

    pub fn on_update_reply(&mut self, reply: Result<&str, &str>) {
        let quiet = std::mem::replace(&mut self.update_quietly, false);
        if let (Ok(_), Some(time)) = (reply, self.clock) {
            self.settings.update_checked_on = clock_day(time);
            self.save_settings();
        }
        match reply.map_err(str::to_string).and_then(|json| self.releases.parse_latest(json, env!("CARGO_PKG_VERSION"))) {
            Ok(Some(release)) => {
                self.update = Some(release);
                self.update_state = UpdateState::Available;
            }
            Ok(None) => {
                self.update = None;
                self.update_state = if quiet { UpdateState::Idle } else { UpdateState::UpToDate };
            }
            Err(_) => self.update_state = if quiet { UpdateState::Idle } else { UpdateState::CheckFailed },
        }
    }

    pub fn install_update(&mut self) {
        if let Some(release) = &self.update {
            let command = Command::InstallUpdate { url: release.url.clone(), sha256: release.sha256.clone() };
            self.update_state = UpdateState::Downloading(0);
            self.push(command);
        }
    }

    pub fn on_update_state(&mut self, state: UpdateState) {
        self.update_state = state;
    }

    // ----- The natural voice -----

    pub fn download_voice(&mut self) {
        self.voice_pack = VoicePack::Downloading(0);
        self.push(Command::VoiceDownload);
    }

    pub fn remove_voice(&mut self) {
        self.push(Command::VoiceRemove);
    }

    pub fn set_voice_engine(&mut self, engine: VoiceEngine) {
        self.settings.voice_engine = engine;
        self.save_settings();
        self.push(Command::VoiceEngine(engine));
    }

    pub fn on_voice_pack(&mut self, state: VoicePack) {
        self.voice_pack = state;
        self.push(Command::VoiceEngine(self.settings.voice_engine));
    }

    // ----- Map pictures -----

    /// Asks for a map picture once, and again after a while if it failed.
    pub fn want_tile(&mut self, key: TileKey) {
        let retry = match self.tiles_asked.get(&key) {
            None => true,
            Some(Some(failed_at)) => self.now - failed_at > TILE_RETRY_S,
            Some(None) => false,
        };
        if retry {
            self.tiles_asked.insert(key, None);
            self.push(Command::FetchTile { z: key.0, x: key.1, y: key.2 });
        }
    }

    pub fn on_tile(&mut self, key: TileKey, path: &str) {
        self.tiles_ready.push((key, path.to_string()));
    }

    pub fn on_tile_failed(&mut self, key: TileKey) {
        self.tiles_asked.insert(key, Some(self.now));
    }

    /// The map's pictures have to be asked for again, such as after the light or dark look changes.
    pub fn forget_tiles(&mut self) {
        self.tiles_asked.clear();
        self.tiles_ready.clear();
    }

    /// When the rider should arrive, by the phone's clock.
    pub fn arrival_time(&self) -> Option<String> {
        let clock = self.clock?;
        let nav = self.nav.as_ref()?;
        let minutes = u32::from(clock.hour) * 60 + u32::from(clock.minute) + (nav.remaining_s() / 60.0).ceil() as u32;
        Some(format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60))
    }

    // ----- Battery and consumption -----

    fn on_live(&mut self, live: Live) {
        let capacity: f64 = live.packs.iter().filter(|p| p.fitted).map(|p| f64::from(p.remaining_ah)).sum();
        let fitted = [live.packs[0].fitted, live.packs[1].fitted];
        let riding = self.trip.start_odometer_m.is_some();
        // Taking a battery out isn't energy used, and putting one in or charging isn't energy gained.
        let batteries_changed = riding && (capacity > self.trip.start_capacity_ah + 0.05 || fitted != self.trip.fitted);
        if batteries_changed {
            self.bank_ride();
        }
        if batteries_changed || (!riding && live.switched_on) {
            // A ride starts when the main switch is on.
            self.trip = Trip {
                start_odometer_m: Some(live.odometer_m),
                start_capacity_ah: capacity,
                fitted,
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

    /// Opens the camera on the moped's QR sticker.
    pub fn scan_pairing(&mut self) {
        self.push(Command::ScanQr {
            title: t("Point the camera at the QR sticker on the moped.").to_string(),
            torch: t("Torch").to_string(),
            type_it: t("Type it").to_string(),
            cancel: t("Cancel").to_string(),
        });
    }

    pub fn ask_pairing(&mut self) {
        self.ask_text("pair", t("Pairing code"), t("24 characters from the QR sticker"));
    }

    fn ask_text(&mut self, key: &str, title: &str, hint: &str) {
        self.push(Command::AskText {
            key: key.to_string(),
            title: title.to_string(),
            hint: hint.to_string(),
            dark: self.settings_say_dark().or(self.system_dark).unwrap_or(true),
        });
    }

    /// Light or dark when the rider has picked one, or `None` when it follows the phone.
    pub fn settings_say_dark(&self) -> Option<bool> {
        match self.settings.appearance {
            Appearance::Light => Some(false),
            Appearance::Dark => Some(true),
            Appearance::System => self.system_dark,
        }
    }

    /// A small tick from the phone, so a tap can be felt with gloves on.
    pub fn tick_haptic(&mut self) {
        self.push(Command::Haptic);
    }

    pub fn set_moped_color(&mut self, color: MopedColor) {
        self.settings.moped_color = color;
        self.save_settings();
    }

    /// The brief message waiting to be shown, if any.
    pub fn take_snack(&mut self) -> Option<String> {
        self.snack.take()
    }

    pub fn on_text(&mut self, key: &str, value: &str) {
        match key {
            "search" if !value.trim().is_empty() => {
                self.nav_status = t("Searching...").to_string();
                let url = self.map.search_url(value.trim(), self.gps, locale::current().code());
                self.push(Command::Search { url });
            }
            "bookmark" if !value.trim().is_empty() => {
                if let Some(at) = self.pending_bookmark.take() {
                    self.save_bookmark(BookmarkKind::Other, &Place::new(value.trim(), at));
                }
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
        match reply.map_err(str::to_string).and_then(|json| self.map.parse_search(json)) {
            Ok(places) => {
                self.nav_status = if places.is_empty() { t("Nothing found.").to_string() } else { String::new() };
                self.search_results = places;
            }
            Err(_) => self.nav_status = t("Search failed. Check the phone's internet connection.").to_string(),
        }
    }

    pub fn navigate_to(&mut self, place: &Place) {
        self.search_results.clear();
        remember_recent(&mut self.settings.recents, place, RECENT_PLACES);
        self.save_settings();
        self.destination = Some((place.name.clone(), place.at));
        self.route_roads.clear();
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
        let moving = self.speed_kmh.is_some_and(|s| s > MOVING_KMH);
        let heading = self.heading_deg.filter(|_| moving);
        self.push(Command::FetchRoute {
            url: self.map.route_url(from, at, self.route_nearby, heading, locale::current().code()),
            destination: name,
        });
    }

    pub fn on_route_reply(&mut self, reply: Result<&str, &str>, destination: &str) {
        // The rider may have stopped navigating while the request was on its way.
        if self.destination.is_none() {
            return;
        }
        let rerouting = std::mem::replace(&mut self.rerouting, false);
        match reply.map_err(str::to_string).and_then(|json| self.map.parse_route(json, destination)) {
            Ok(route) => {
                let nav = Navigation::new(route);
                let nearby = t("A moped can't get right to that spot, so this goes to the nearest road.");
                self.nav_status = if self.route_nearby { nearby.to_string() } else { String::new() };
                if self.settings.say_navigation {
                    let mut text = if rerouting {
                        let first = nav.route.maneuvers.first().map_or(String::new(), |m| m.spoken_before.clone());
                        format!("{} {first}", t("New route."))
                    } else if self.route_nearby {
                        format!("{nearby} {}", nav.opening())
                    } else {
                        nav.opening()
                    };
                    // When the rider will get there, from the phone's clock: more useful than minutes on a moped.
                    if let (false, Some(clock)) = (rerouting, self.clock) {
                        let minutes = u32::from(clock.hour) * 60 + u32::from(clock.minute) + (nav.remaining_s() / 60.0).ceil() as u32;
                        let at = format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60);
                        text = format!("{text} {}", tf("You'll be there at {}.", &[&at]));
                    }
                    self.say(text.trim().to_string(), true);
                }
                let url = self.map.route_roads_url(&nav.route);
                self.route_roads.clear();
                if !rerouting {
                    self.halfway_said = false;
                }
                self.nav = Some(nav);
                if self.settings.say_limits || self.settings.warn_speeding {
                    self.push(Command::Fetch { key: "route_roads".to_string(), url });
                }
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
        self.route_roads.clear();
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
        // Close fixes only show the GPS wobbling, not which way the rider is going.
        match self.last_fix {
            Some(last) if nav::distance_m(last, at) < HEADING_MIN_MOVE_M => {}
            Some(last) => {
                self.heading_deg = Some(nav::bearing_deg(last, at));
                self.last_fix = Some(at);
            }
            None => self.last_fix = Some(at),
        }
        self.speed_kmh = speed_mps.map(|s| s * 3.6);
        if self.route_pending {
            self.request_route();
        }
        self.follow_limits(at);
        let Some(nav) = self.nav.as_mut() else {
            return;
        };
        let events = nav.update(at, speed_mps);
        // Halfway on a longer ride: how far is left, so the rider knows all is well.
        let (total, left) = (nav.route.total_m, nav.remaining_m());
        if total > HALFWAY_FROM_M && left < total / 2.0 && !self.halfway_said {
            self.halfway_said = true;
            if self.settings.say_navigation {
                self.say(tf("Halfway there, {} kilometres to go.", &[&format!("{:.0}", left / 1000.0)]), false);
            }
        }
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

    // ----- Power -----

    /// What Pillion needs from the phone right now. See `Power`.
    pub fn wanted_power(&self) -> Power {
        let linking = matches!(self.link, Link::Online | Link::LoggingIn | Link::Connecting | Link::Scanning);
        let riding = self.link == Link::Online && self.switched_on_said != Some(false);
        if self.demo_active || self.nav.is_some() || self.destination.is_some() || riding {
            Power::Ride
        } else if self.screen_on || linking {
            Power::Look
        } else {
            Power::Rest
        }
    }

    fn update_power(&mut self) {
        let wanted = self.wanted_power();
        if self.power != Some(wanted) {
            self.power = Some(wanted);
            self.push(Command::SetPower(wanted));
        }
    }

    /// How often the engine should tick, in milliseconds.
    pub fn pace_ms(&self) -> u64 {
        self.power.unwrap_or(Power::Look).pace_ms()
    }

    pub fn set_screen_on(&mut self, on: bool) {
        self.screen_on = on;
        self.update_power();
    }

    // ----- Speed limits -----

    fn follow_limits(&mut self, at: LatLon) {
        let now = self.now;
        // Navigating with the route's roads known: no need to ask the map as we go.
        if let Some(nav) = &self.nav {
            if let Some(Some(road)) = self.route_roads.get(nav.point_index()).cloned() {
                if let Some(event) = self.limits.on_road(road, now) {
                    self.limit_event(event);
                }
                return self.limit_speed();
            }
        }
        if self.demo_active {
            return self.limit_speed();
        }
        let riding = self.link == Link::Online || self.speed_kmh.is_some_and(|s| s > MOVING_KMH);
        let due = match self.road_asked {
            Some((from, when)) => now - when >= ROAD_EVERY_S && geo::distance_m(from, at) >= ROAD_EVERY_M,
            None => true,
        };
        if riding && due && !self.road_pending && (self.settings.say_limits || self.screen_on) {
            self.road_asked = Some((at, now));
            self.road_pending = true;
            let url = self.map.road_url(at, self.heading_deg);
            self.push(Command::Fetch { key: "road".to_string(), url });
        }
        self.limit_speed();
    }

    fn limit_speed(&mut self) {
        if let Some(event) = self.limits.on_speed(self.speed_kmh, self.now) {
            self.limit_event(event);
        }
    }

    fn limit_event(&mut self, event: LimitEvent) {
        match event {
            LimitEvent::NewLimit { limit, road } if self.settings.say_limits => {
                let text = if road.is_empty() {
                    tf("Speed limit {}.", &[&limit])
                } else {
                    tf("{}: speed limit {}.", &[&road, &limit])
                };
                self.say(text, false);
            }
            LimitEvent::TooFast { limit } if self.settings.warn_speeding => {
                self.say(tf("Slow down: the limit here is {}.", &[&limit]), true);
            }
            _ => {}
        }
    }

    /// The answer to a `Command::Fetch`.
    pub fn on_fetch(&mut self, key: &str, reply: Result<&str, &str>) {
        match key {
            "road" => {
                self.road_pending = false;
                if let Ok(Some(road)) = reply.map_err(str::to_string).and_then(|json| self.map.parse_road(json)) {
                    if let Some(event) = self.limits.on_road(road, self.now) {
                        self.limit_event(event);
                    }
                }
            }
            "route_roads" => {
                let points = self.nav.as_ref().map_or(0, |n| n.route.points.len());
                if let Ok(roads) = reply.map_err(str::to_string).and_then(|json| self.map.parse_route_roads(json, points)) {
                    self.route_roads = roads;
                }
            }
            "reach" => {
                self.reach_pending = false;
                match reply.map_err(str::to_string).and_then(|json| self.map.parse_reach(json)) {
                    Ok(rings) if !rings.is_empty() => self.reach = rings,
                    _ => self.reach.clear(), // the screen draws a plain circle instead
                }
            }
            _ => {}
        }
    }

    // ----- How far the battery reaches -----

    /// Asks for the area reachable on roads with the range left, when the map is on screen. Until it arrives (or
    /// without internet) the screen shows a circle of the range.
    pub fn want_reach(&mut self) {
        let (Some(at), Some(km)) = (self.gps, self.range_km()) else {
            return;
        };
        if self.reach_pending {
            return;
        }
        let fresh = match self.reach_for {
            Some((from, range, when)) => {
                geo::distance_m(from, at) < REACH_AGAIN_KM * 1000.0 && (range - km).abs() < REACH_AGAIN_KM
                    || self.now - when < REACH_EVERY_S
            }
            None => false,
        };
        if fresh {
            return;
        }
        self.reach_for = Some((at, km, self.now));
        self.reach_pending = true;
        let url = self.map.reach_url(at, km);
        self.push(Command::Fetch { key: "reach".to_string(), url });
    }

    /// The range around the rider as a ring, for when the road-reachable area isn't known.
    pub fn range_ring(&self) -> Option<Vec<LatLon>> {
        Some(geo::circle(self.gps?, self.range_km()? * 1000.0, 72))
    }

    // ----- Saved places -----

    pub fn save_bookmark(&mut self, kind: BookmarkKind, place: &Place) {
        self.settings.bookmarks.save(kind, place.clone());
        self.save_settings();
        self.snack = Some(match kind {
            BookmarkKind::Home => t("Saved as Home.").to_string(),
            BookmarkKind::Work => t("Saved as Work.").to_string(),
            BookmarkKind::Other => tf("Saved {}.", &[&place.name]),
        });
    }

    pub fn remove_bookmark(&mut self, index: usize) {
        self.settings.bookmarks.remove(index);
        self.save_settings();
    }

    /// Saves the spot under a name the rider types (from a long press on the map).
    pub fn ask_bookmark_name(&mut self, at: LatLon) {
        self.pending_bookmark = Some(at);
        self.ask_text("bookmark", t("Save this place"), t("Name"));
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
        // The day only counts as checked once GitHub has answered: the phone may have no signal right now.
        let waited = self.update_tried_at.is_none_or(|at| self.now - at >= UPDATE_RETRY_S);
        if self.settings.update_auto && waited && self.update_state == UpdateState::Idle && self.settings.update_checked_on != clock_day(time) {
            self.update_tried_at = Some(self.now);
            self.update_quietly = true;
            self.check_for_update();
        }
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
            self.say_anyway(tf("Voice mode on. {}", &[&text]), true);
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
            Link::Online => {
                parts.push(self.battery_sentence());
                if let Some([first, second]) = self.live.as_ref().map(|l| l.packs).filter(|p| p[0].fitted && p[1].fitted) {
                    parts.push(tf(
                        "Battery 1 at {} percent, battery 2 at {} percent.",
                        &[&first.percent, &second.percent],
                    ));
                }
            }
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
        if !self.settings.voice_mode {
            self.snack = Some(t("Speaking is off. Turn it on to hear this.").to_string());
        }
        self.say_anyway(text, true);
    }

    pub fn repeat_last(&mut self) {
        if let Some(text) = self.spoken.front().cloned() {
            self.say_anyway(text, true);
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
        // A made-up limit, so the demo shows the speed-limit sign (it never asks the map: the ride isn't real).
        self.limits.reset();
        self.limits.on_road(Road { way: 1, name: String::new(), limit: Some(50) }, self.now - 60.0);
        let (vehicle, records) = crate::demo::records();
        for frame in proto::build_records_frames(&vehicle, &records[0], 0).iter().chain(proto::build_records_frames(&vehicle, &records[1], 0).iter()) {
            self.on_rx(frame);
        }
        self.tick(self.now);
    }

    pub fn stop_demo(&mut self) {
        self.demo = None;
        self.demo_active = false;
        self.vehicle = None;
        self.battery_records.clear();
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

    /// Adds what this ride has measured to the running average.
    fn bank_ride(&mut self) {
        self.settings.total_km += self.trip.km;
        self.settings.total_kwh += self.trip.kwh;
        self.save_settings();
    }

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
        self.bank_ride();
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
        self.update_power();
        self.finish_notif_bursts();
        if let Some(mut demo) = self.demo.take() {
            demo.step(self, now);
            self.demo = Some(demo);
            return;
        }
        self.check_ride_end();
        match self.link {
            Link::Scanning if now - self.link_since > SCAN_GIVE_UP_S => {
                self.push(Command::StopScan);
                self.set_link(Link::NotFound);
            }
            // The moped didn't answer: it's probably off. Its address is known, so wait for it without searching.
            Link::Connecting if now - self.link_since > CONNECT_TIMEOUT_S => self.wait_for_moped(),
            Link::LoggingIn if now - self.link_since > CONNECT_TIMEOUT_S => {
                self.push(Command::Disconnect);
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

    /// The next command, leaving out the power level and background map requests, which have their own tests.
    fn next_command(e: &mut Engine) -> Option<Command> {
        loop {
            match e.poll_command() {
                Some(Command::SetPower(_) | Command::Fetch { .. }) => continue,
                other => return other,
            }
        }
    }
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
        while let Some(c) = next_command(e) {
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
        while next_command(&mut e).is_some() {}
        e
    }

    #[test]
    fn a_new_version_is_offered_and_a_quiet_check_stays_quiet_when_it_fails() {
        let mut e = Engine::new();
        e.check_for_update();
        e.update_quietly = true;
        e.on_update_reply(Err("offline"));
        assert_eq!(e.update_state, UpdateState::Idle);
        e.check_for_update_now();
        e.on_update_reply(Err("offline"));
        assert_eq!(e.update_state, UpdateState::CheckFailed);
        let reply = r#"{"tag_name":"v99.0.0","assets":[{"name":"Pillion-99.0.0.apk","browser_download_url":"https://example.org/a.apk"}]}"#;
        e.check_for_update_now();
        e.on_update_reply(Ok(reply));
        assert_eq!(e.update_state, UpdateState::Available);
        assert_eq!(e.update.as_ref().unwrap().version, "99.0.0");
        while next_command(&mut e).is_some() {}
        e.install_update();
        assert!(matches!(next_command(&mut e), Some(Command::InstallUpdate { .. })));
    }

    #[test]
    fn the_app_looks_for_a_new_version_once_a_day() {
        let mut e = Engine::new();
        let day = |day| ClockTime { year: 2026, month: 11, day, hour: 9, minute: 0, second: 0 };
        let checks = |e: &mut Engine| {
            let mut count = 0;
            while let Some(command) = next_command(e) {
                count += usize::from(matches!(command, Command::CheckUpdate { .. }));
            }
            count
        };
        e.on_clock(day(6));
        assert_eq!(checks(&mut e), 1);
        // No signal: it waits a while, then tries again.
        e.on_update_reply(Err("offline"));
        e.on_clock(day(6));
        assert_eq!(checks(&mut e), 0);
        e.now += UPDATE_RETRY_S;
        e.on_clock(day(6));
        assert_eq!(checks(&mut e), 1);
        // An answer settles it for the day.
        e.on_update_reply(Ok(r#"{"tag_name":"v0.0.1","assets":[]}"#));
        assert_eq!(e.update_state, UpdateState::Idle);
        e.now += UPDATE_RETRY_S;
        e.on_clock(day(6));
        assert_eq!(checks(&mut e), 0);
        e.on_clock(day(7));
        assert_eq!(checks(&mut e), 1);
        e.on_update_reply(Ok(r#"{"tag_name":"v0.0.1","assets":[]}"#));
        e.set_update_auto(false);
        e.on_clock(day(8));
        assert_eq!(checks(&mut e), 0);
    }

    #[test]
    fn connection_runs_the_original_sequence() {
        let mut e = Engine::new();
        assert_eq!(e.link, Link::NeedsPairing);
        assert!(e.set_pairing_code(CODE));
        assert_eq!(e.link, Link::Scanning);
        assert_eq!(next_command(&mut e), Some(Command::Scan));
        e.on_ble_found("SomeHeadphones", "00:11");
        assert!(e.found_devices.is_empty());
        e.on_ble_found("YSCCU_ABCDEFGHIJKLMN", "AA:BB");
        assert_eq!(e.link, Link::Connecting);
        assert_eq!(next_command(&mut e), Some(Command::StopScan));
        assert_eq!(next_command(&mut e), Some(Command::Connect("AA:BB".to_string())));
        e.on_ble_ready();
        let Some(Command::Write(first)) = next_command(&mut e) else {
            panic!("expected the login");
        };
        assert_eq!(&first[5..19], b"ABCDEFGHIJKLMN");
        assert_eq!(first[57], 1);
        // Reconnecting in the same run uses the zeroed form.
        e.on_ble_ready();
        let Some(Command::Write(second)) = next_command(&mut e) else {
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
    fn taking_a_battery_out_is_not_counted_as_energy_used() {
        let mut e = online();
        let mut two = live(80, 10_000);
        two.packs[1] = pack(100, 20.0);
        feed(&mut e, &two);
        // 3 km on 1 Ah from the first battery.
        two.odometer_m = 13_000;
        two.packs[0].remaining_ah -= 1.0;
        feed(&mut e, &two);
        assert!((e.trip.kwh - 0.05).abs() < 0.005);
        // The second battery comes out, taking 20 Ah with it.
        let mut one = two.clone();
        one.packs[1] = Pack::default();
        one.odometer_m = 13_100;
        feed(&mut e, &one);
        assert_eq!(e.trip.km, 0.0);
        assert_eq!(e.trip.kwh, 0.0);
        assert!((e.settings.total_km - 3.0).abs() < 0.01);
        assert!((e.settings.total_kwh - 0.05).abs() < 0.005);
        assert!((e.efficiency() - 60.0).abs() < 7.0);
    }

    #[test]
    fn putting_a_battery_in_keeps_the_ride_so_far_in_the_average() {
        let mut e = online();
        feed(&mut e, &live(80, 10_000));
        let mut after = live(70, 13_000);
        after.packs[0].remaining_ah = 14.0;
        feed(&mut e, &after);
        after.packs[1] = pack(100, 20.0);
        feed(&mut e, &after);
        assert_eq!(e.trip.km, 0.0);
        assert!((e.settings.total_km - 3.0).abs() < 0.01);
    }

    #[test]
    fn status_report_names_each_of_two_batteries() {
        let mut e = online();
        let mut two = live(30, 1000);
        two.packs[1] = pack(90, 20.0);
        feed(&mut e, &two);
        let report = e.status_report();
        assert!(report.contains("Battery 60 percent"));
        assert!(report.contains("Battery 1 at 30 percent, battery 2 at 90 percent."));
        feed(&mut e, &live(30, 1000));
        assert!(!e.status_report().contains("battery 2"));
    }

    #[test]
    fn a_low_second_battery_alone_is_still_announced() {
        let mut e = online();
        let mut two = live(90, 1000);
        two.packs[1] = pack(10, 20.0);
        two.packs[1].low = true;
        feed(&mut e, &two);
        assert!(speech(&mut e).iter().any(|s| s == "Battery low."));
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
    fn voice_engine_setting_survives_and_old_switch_migrates() {
        assert_eq!(Settings::from_text("natural_voice=1").voice_engine, VoiceEngine::Auto);
        assert_eq!(Settings::from_text("natural_voice=0").voice_engine, VoiceEngine::System);
        let s = Settings { voice_engine: VoiceEngine::Natural, ..Settings::default() };
        assert_eq!(Settings::from_text(&s.to_text()).voice_engine, VoiceEngine::Natural);
    }

    #[test]
    fn quiet_when_the_moped_is_not_connected() {
        let mut e = online();
        e.on_ble_disconnected();
        assert_eq!(speech(&mut e), vec![t("Moped connection lost.").to_string()]);
        e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        assert!(speech(&mut e).is_empty());
        // Still logged, so Repeat and the log work.
        assert!(e.spoken.iter().any(|s| s.contains("Sam")));
    }

    #[test]
    fn asked_for_status_is_spoken_even_off_the_moped() {
        let mut e = Engine::new();
        e.settings.voice_mode = true;
        e.speak_status();
        assert_eq!(speech(&mut e).len(), 1);
    }

    #[test]
    fn demo_ride_speaks_without_a_moped() {
        let mut e = Engine::new();
        e.settings.voice_mode = true;
        e.start_demo();
        e.on_notification("a.b", "Chat", "Sam", "Hello", false);
        assert!(speech(&mut e).iter().any(|s| s.contains("Sam")));
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
        while next_command(&mut e).is_some() {}
        e.tick(100.0);
        e.tick(100.1);
        let mut flags = None;
        while let Some(command) = next_command(&mut e) {
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
        e.settings.update_auto = false;
        e.on_clock(ClockTime { year: 2026, month: 1, day: 2, hour: 3, minute: 4, second: 5 });
        e.tick(100.0);
        let Some(Command::Write(clock)) = next_command(&mut e) else {
            panic!("expected the clock");
        };
        assert_eq!(&clock[2..4], &[0x05, 0x8A]);
        assert!(next_command(&mut e).is_none());
        e.tick(100.1);
        let Some(Command::Write(flags)) = next_command(&mut e) else {
            panic!("expected the flags");
        };
        assert_eq!(&flags[2..4], &[0x05, 0x8B]);
        e.tick(100.5);
        assert!(next_command(&mut e).is_none());
        e.tick(101.0);
        assert!(matches!(next_command(&mut e), Some(Command::Write(_))));
    }

    #[test]
    fn a_first_search_gives_up_instead_of_scanning_forever() {
        let mut e = Engine::new();
        e.set_pairing_code(CODE);
        while next_command(&mut e).is_some() {}
        e.tick(16.0);
        assert!(next_command(&mut e).is_none());
        assert_eq!(e.link, Link::Scanning);
        e.tick(50.0);
        assert_eq!(next_command(&mut e), Some(Command::StopScan));
        assert_eq!(e.link, Link::NotFound);
        e.tick(500.0);
        assert!(next_command(&mut e).is_none());
    }

    #[test]
    fn a_known_moped_that_doesnt_answer_is_waited_for_quietly() {
        let mut e = Engine::new();
        e.set_pairing_code(CODE);
        e.choose_device("AA:BB");
        while next_command(&mut e).is_some() {}
        e.tick(16.0);
        assert_eq!(next_command(&mut e), Some(Command::WaitFor("AA:BB".to_string())));
        assert_eq!(e.link, Link::Waiting);
        // No more searching or reconnecting, however long it waits.
        for s in 17..600 {
            e.tick(f64::from(s));
        }
        assert!(next_command(&mut e).is_none());
        // The moped comes back: Android reconnects and the engine signs in.
        e.on_ble_ready();
        assert!(matches!(next_command(&mut e), Some(Command::Write(_))));
    }

    #[test]
    fn switching_the_moped_off_waits_instead_of_searching() {
        let mut e = online();
        e.settings.address = "AA:BB".to_string();
        while next_command(&mut e).is_some() {}
        e.on_ble_disconnected();
        assert_eq!(e.link, Link::Waiting);
        let mut seen = Vec::new();
        while let Some(c) = e.poll_command() {
            seen.push(c);
        }
        assert!(seen.contains(&Command::WaitFor("AA:BB".to_string())));
        assert!(!seen.contains(&Command::Scan));
    }

    #[test]
    fn power_follows_what_the_rider_is_doing() {
        let mut e = Engine::new();
        e.settings.address = "AA:BB".to_string();
        e.set_screen_on(false);
        e.tick(1.0);
        assert_eq!(e.wanted_power(), Power::Rest);
        assert_eq!(e.pace_ms(), 1000);
        e.set_screen_on(true);
        assert_eq!(e.wanted_power(), Power::Look);
        let mut e = online();
        e.set_screen_on(false);
        assert_eq!(e.wanted_power(), Power::Ride);
        let mut powers = Vec::new();
        while let Some(c) = e.poll_command() {
            if let Command::SetPower(p) = c {
                powers.push(p);
            }
        }
        assert_eq!(powers.last(), Some(&Power::Ride));
    }

    #[test]
    fn speed_limits_are_asked_while_riding_and_said_once() {
        let mut e = online();
        e.speed_kmh = Some(30.0);
        e.tick(10.0);
        while e.poll_command().is_some() {}
        e.on_gps((44.4949, 11.3426), Some(8.0));
        let mut asked = false;
        while let Some(c) = e.poll_command() {
            asked |= matches!(c, Command::Fetch { ref key, .. } if key == "road");
        }
        assert!(asked);
        let reply = r#"[{"edges":[{"edge_info":{"way_id":32479333,"names":["Via Ugo Bassi"],"speed_limit":30}}]}]"#;
        e.on_fetch("road", Ok(reply));
        e.tick(14.0);
        e.on_gps((44.4949, 11.3426), Some(8.0));
        assert_eq!(e.spoken.front().map(String::as_str), Some("Via Ugo Bassi: speed limit 30."));
        assert_eq!(e.limits.limit(), Some(30));
    }

    #[test]
    fn route_needs_gps_and_waits_for_it() {
        let mut e = online();
        e.navigate_to(&Place::new("Home", (48.1, 2.1)));
        assert_eq!(e.nav_status, "Waiting for GPS...");
        assert!(next_command(&mut e).is_none());
        e.on_gps((48.0, 2.0), Some(5.0));
        assert!(matches!(next_command(&mut e), Some(Command::FetchRoute { .. })));
        assert_eq!(e.speed_kmh, Some(18.0));
    }

    fn short_route() -> Route {
        let points: Vec<LatLon> = (0..=20).map(|i| (48.0 + f64::from(i) * 0.001, 2.0)).collect();
        let m = |kind, begin, text: &str| Maneuver {
            kind,
            text: text.to_string(),
            spoken_before: text.to_string(),
            spoken_at: text.to_string(),
            begin, ..Default::default() };
        Route::new("Home", points, vec![m(1, 0, "Go north"), m(4, 20, "Arrive")], 300.0)
    }

    const NO_PATH: &str = r#"{"error_code":442,"error":"No path could be found for input"}"#;

    fn route_url(e: &mut Engine) -> String {
        match next_command(e) {
            Some(Command::FetchRoute { url, .. }) => url,
            other => panic!("expected a route request, got {other:?}"),
        }
    }

    #[test]
    fn unreachable_destination_retries_once_for_the_nearest_road() {
        let mut e = online();
        e.on_gps((48.0, 2.0), None);
        e.navigate_to(&Place::new("Palace", (48.1, 2.1)));
        assert!(!route_url(&mut e).contains("radius"));
        e.on_route_reply(Ok(NO_PATH), "Palace");
        assert!(route_url(&mut e).contains("radius"));
        // The second refusal is final, and it says so.
        e.on_route_reply(Ok(NO_PATH), "Palace");
        assert!(next_command(&mut e).is_some());
        assert!(e.nav_status.starts_with("No route"));
    }

    #[test]
    fn reply_after_stopping_navigation_is_ignored() {
        let mut e = online();
        e.on_gps((48.0, 2.0), None);
        e.navigate_to(&Place::new("Home", (48.1, 2.1)));
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
            e.navigate_to(&Place::new(&format!("Place {i}"), (48.0, 2.0)));
        }
        e.navigate_to(&Place::new("Place 3", (48.0, 2.0)));
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
    fn test_voice_speaks_even_before_pairing_and_with_speaking_off() {
        let mut e = Engine::new();
        e.test_voice();
        assert_eq!(speech(&mut e).len(), 1);
        assert!(e.take_snack().is_some());
        assert!(e.take_snack().is_none());
        e.settings.voice_mode = false;
        e.test_voice();
        assert_eq!(speech(&mut e).len(), 1);
    }

    #[test]
    fn asking_with_speaking_off_explains_the_silence() {
        let mut e = online();
        e.settings.voice_mode = false;
        e.speak_status();
        assert!(speech(&mut e).is_empty());
        assert!(e.take_snack().is_some());
        e.settings.voice_mode = true;
        e.speak_status();
        assert_eq!(speech(&mut e).len(), 1);
        assert!(e.take_snack().is_none());
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
        s.moped_color = MopedColor::Black;
        s.high_contrast = true;
        s.language = Some(Lang::It);
        s.recents = vec![Place::new("Home; sweet, home", (48.5, 16.25))];
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
        assert_eq!(back.moped_color, MopedColor::Black);
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
