//! The bridge to the Android side. Java sends events and polls for commands, both as strings with
//! a separator between fields, so there's one small door between the two languages.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jstring};
use jni::JNIEnv;
use winit::platform::android::activity::AndroidApp;

use crate::engine::{BluetoothState, CallState, Command, Engine, Power, UpdateState, VoicePack};
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
                // Quick while riding or looking at the screen, slow at rest: this loop is part of the battery cost.
                let mut pace = TICK;
                if let Ok(mut e) = ticking.lock() {
                    e.tick(start.elapsed().as_secs_f64());
                    pace = Duration::from_millis(e.pace_ms());
                }
                std::thread::sleep(pace);
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
        Command::Fetch { key, url } => vec!["fetch".into(), key, url],
        Command::SetPower(power) => vec!["power".into(), match power {
            Power::Ride => "ride",
            Power::Look => "look",
            Power::Rest => "rest",
        }.into()],
        Command::WaitFor(address) => vec!["wait_for".into(), address],
        Command::AskText { key, title, hint, dark } => vec!["ask".into(), key, title, hint, u8::from(dark).to_string()],
        Command::Haptic => vec!["haptic".into()],
        Command::ScanQr { title, torch, type_it, cancel } => vec!["scan_qr".into(), title, torch, type_it, cancel],
        Command::CheckUpdate { url } => vec!["update_check".into(), url],
        Command::InstallUpdate { url, sha256 } => vec!["update_install".into(), url, sha256],
        Command::VoiceDownload => vec!["voice_download".into()],
        Command::VoiceRemove => vec!["voice_remove".into()],
        Command::VoiceNatural(on) => vec!["voice_natural".into(), u8::from(on).to_string()],
        Command::FetchTile { z, x, y } => vec!["tile".into(), z.to_string(), x.to_string(), y.to_string()],
        Command::Share(text) => vec!["share".into(), text],
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
        ["fetch_ok", key, json] => e.on_fetch(key, Ok(json)),
        ["fetch_err", key, error] => e.on_fetch(key, Err(error)),
        ["screen", on] => e.set_screen_on(*on == "1"),
        ["search_err", error] => e.on_search_reply(Err(error)),
        ["text", key, value] => e.on_text(key, value),
        ["button"] => e.speak_status(),
        ["voice_missing"] => e.voice_missing = true,
        ["voice_pack", state, percent] => e.on_voice_pack(match *state {
            "ready" => VoicePack::Ready,
            "downloading" => VoicePack::Downloading(percent.parse().unwrap_or(0)),
            "failed" => VoicePack::Failed,
            _ => VoicePack::None,
        }),
        ["update_ok", json] => e.on_update_reply(Ok(json)),
        ["update_err", error] => e.on_update_reply(Err(error)),
        ["update_state", state, percent] => e.on_update_state(match *state {
            "downloading" => UpdateState::Downloading(percent.parse().unwrap_or(0)),
            "permission" => UpdateState::NeedsPermission,
            "installing" => UpdateState::Installing,
            _ => UpdateState::InstallFailed,
        }),
        ["qr_type"] => e.ask_pairing(),
        ["tile", z, x, y, path] => {
            if let (Ok(z), Ok(x), Ok(y)) = (z.parse(), x.parse(), y.parse()) {
                e.on_tile((z, x, y), path);
            }
        }
        ["tile_failed", z, x, y] => {
            if let (Ok(z), Ok(x), Ok(y)) = (z.parse(), x.parse(), y.parse()) {
                e.on_tile_failed((z, x, y));
            }
        }
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
pub extern "system" fn Java_io_github_quezka_pillion_Native_decodeQr(
    env: JNIEnv,
    _class: JClass,
    brightness: JByteArray,
    width: jint,
    height: jint,
) -> jstring {
    let Ok(pixels) = env.convert_byte_array(&brightness) else {
        return std::ptr::null_mut();
    };
    match crate::qr::read_pairing_code(&pixels, width as usize, height as usize) {
        Some(code) => env.new_string(code).map_or(std::ptr::null_mut(), JString::into_raw),
        None => std::ptr::null_mut(),
    }
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
