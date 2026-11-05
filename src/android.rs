//! The bridge to the Android side. Java sends events and polls for commands, both as strings with
//! a separator between fields, so there's one small door between the two languages.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use winit::platform::android::activity::AndroidApp;

use crate::engine::{BluetoothState, CallState, Command, Engine};
use crate::locale::Lang;
use crate::protocol::{from_hex, to_hex, ClockTime};
use crate::ui::CompanionApp;

const SEPARATOR: char = '\u{1f}';
const TICK: Duration = Duration::from_millis(50);

static ENGINE: OnceLock<Arc<Mutex<Engine>>> = OnceLock::new();

/// The screen and the background service share one engine, which outlives the screen.
fn engine() -> &'static Arc<Mutex<Engine>> {
    ENGINE.get_or_init(|| {
        let engine = Arc::new(Mutex::new(Engine::new()));
        let ticking = Arc::clone(&engine);
        std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                if let Ok(mut e) = ticking.lock() {
                    e.tick(start.elapsed().as_secs_f64());
                }
                std::thread::sleep(TICK);
            }
        });
        engine
    })
}

fn encode(command: Command) -> String {
    let fields: Vec<String> = match command {
        Command::Scan => vec!["scan".into()],
        Command::StopScan => vec!["stop_scan".into()],
        Command::Connect(address) => vec!["connect".into(), address],
        Command::Disconnect => vec!["disconnect".into()],
        Command::Write(bytes) => vec!["write".into(), to_hex(&bytes)],
        Command::Speak { text, urgent } => vec!["speak".into(), u8::from(urgent).to_string(), text],
        Command::SetSpeechRate(rate) => vec!["rate".into(), rate.to_string()],
        Command::FetchRoute { url, destination } => vec!["route".into(), url, destination],
        Command::Search { url } => vec!["search".into(), url],
        Command::AskText { key, title, hint } => vec!["ask".into(), key, title, hint],
        Command::OpenNotificationAccess => vec!["notif_access".into()],
        Command::SetLanguage(code) => vec!["lang".into(), code.into()],
        Command::EnableBluetooth => vec!["bt_enable".into()],
        Command::OpenLocationSettings => vec!["location_settings".into()],
        Command::OpenAppSettings => vec!["app_settings".into()],
    };
    fields.join(&SEPARATOR.to_string())
}

fn handle_event(e: &mut Engine, message: &str) {
    let fields: Vec<&str> = message.split(SEPARATOR).collect();
    let number = |i: usize| fields.get(i).and_then(|f| f.parse::<f64>().ok());
    match fields.as_slice() {
        ["dir", path] => e.load_settings(PathBuf::from(path).join("settings.txt")),
        ["found", name, address] => e.on_ble_found(name, address),
        ["ready"] => e.on_ble_ready(),
        ["gone"] => e.on_ble_disconnected(),
        ["rx", hex] => {
            if let Some(bytes) = from_hex(hex) {
                e.on_rx(&bytes);
            }
        }
        ["gps", ..] => {
            if let (Some(lat), Some(lon), Some(speed)) = (number(1), number(2), number(3)) {
                e.on_gps((lat, lon), (speed >= 0.0).then_some(speed));
            }
        }
        ["notif", package, label, title, text, is_call] => e.on_notification(package, label, title, text, *is_call == "1"),
        ["notif_gone", package] => e.on_notification_removed(package),
        ["access", allowed] => e.set_notification_access(*allowed == "1"),
        ["call", state] => e.on_call(match *state {
            "ring" => CallState::Ringing,
            "offhook" => CallState::OffHook,
            _ => CallState::Idle,
        }),
        ["phone_batt", _] => {
            if let Some(percent) = number(1) {
                e.on_phone_battery(percent as u8);
            }
        }
        ["time", ..] => {
            if let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
                (number(1), number(2), number(3), number(4), number(5), number(6))
            {
                e.on_clock(ClockTime {
                    year: year as u16,
                    month: month as u8,
                    day: day as u8,
                    hour: hour as u8,
                    minute: minute as u8,
                    second: second as u8,
                });
            }
        }
        ["route_ok", destination, json] => e.on_route_reply(Ok(json), destination),
        ["route_err", destination, error] => e.on_route_reply(Err(error), destination),
        ["search_ok", json] => e.on_search_reply(Ok(json)),
        ["search_err", error] => e.on_search_reply(Err(error)),
        ["text", key, value] => e.on_text(key, value),
        ["button"] => e.speak_status(),
        ["voice_missing"] => e.voice_missing = true,
        ["system", language, dark] => e.on_system(Lang::from_code(language), *dark == "1"),
        ["bt", state] => e.on_bluetooth(match *state {
            "off" => BluetoothState::Off,
            "denied" => BluetoothState::Denied,
            _ => BluetoothState::On,
        }),
        ["location", on] => e.location_on = *on == "1",
        ["insets", ..] => {
            if let (Some(top), Some(bottom)) = (number(1), number(2)) {
                e.system_bars_px = (top as f32, bottom as f32);
            }
        }
        _ => {}
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_quezka_pillion_Native_send(mut env: JNIEnv, _class: JClass, message: JString) {
    let Ok(message) = env.get_string(&message) else {
        return;
    };
    let message: String = message.into();
    if let Ok(mut e) = engine().lock() {
        handle_event(&mut e, &message);
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_quezka_pillion_Native_poll(env: JNIEnv, _class: JClass) -> jstring {
    let command = engine().lock().ok().and_then(|mut e| e.poll_command());
    let Some(command) = command else {
        return std::ptr::null_mut();
    };
    env.new_string(encode(command)).map_or(std::ptr::null_mut(), JString::into_raw)
}

#[no_mangle]
pub fn android_main(app: AndroidApp) {
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    let shared = Arc::clone(engine());
    // Without a window there's nothing to show the error on; Android restarts the screen when reopened.
    let _ = eframe::run_native(
        "Pillion",
        options,
        Box::new(move |_cc| Ok(Box::new(CompanionApp::new(shared, None)))),
    );
}
