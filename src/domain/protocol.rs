//! The moped's Bluetooth messages. Every number here comes from MECHANICS.md.

pub const SERVICE_UUID: &str = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
pub const WRITE_UUID: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";
pub const NOTIFY_UUID: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";
pub const DEVICE_NAMES: [&str; 2] = ["YSCCU", "YCCU"];

pub const START_LOGIN: u8 = 0xAA;
pub const START_LOGIN_RESULT: u8 = 0x5A;
pub const START_LIVE: u8 = 0x56;
pub const START_BATTERY_RECORD: u8 = 0x55;
pub const START_COMMON_RECORDS: u8 = 0x5B;
pub const START_TO_DASH: u8 = 0xA6;
pub const GROUP_LOGIN: u16 = 0x7F00;
pub const GROUP_LOGIN_RESULT: u16 = 0x7F01;
pub const GROUP_CLOCK: u16 = 0x058A;
pub const GROUP_FLAGS: u16 = 0x058B;

const QR_ORDER: [usize; 24] = [
    7, 14, 20, 0, 12, 17, 6, 22, 2, 13, 10, 1, 16, 19, 8, 21, 23, 5, 15, 3, 9, 18, 4, 11,
];
const CCU_ID_LEN: usize = 14;
const PASSKEY_LEN: usize = 6;
const PHONE_UUID_LEN: usize = 32;

/// Turns what the person typed or scanned into (CCU-ID, passkey). Takes the sticker's 24 character
/// code, or the 20 characters of an already unscrambled CCU-ID and passkey.
pub fn decode_pairing_code(code: &str) -> Option<(String, String)> {
    let code: Vec<char> = code.chars().filter(|c| !c.is_whitespace()).collect();
    if !code.iter().all(char::is_ascii) {
        return None;
    }
    let plain: String = match code.len() {
        24 => QR_ORDER.iter().map(|&i| code[i]).collect(),
        20 => code.iter().collect(),
        _ => return None,
    };
    Some((
        plain[..CCU_ID_LEN].to_string(),
        plain[CCU_ID_LEN..CCU_ID_LEN + PASSKEY_LEN].to_string(),
    ))
}

pub fn checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|&b| u32::from(b)).sum();
    (256 - (sum & 0xFF)) as u8
}

pub fn build_frame(start: u8, groups: &[(u16, Vec<u8>)], counter: u8) -> Vec<u8> {
    let mut out = vec![start, groups.len() as u8];
    for (id, data) in groups {
        out.extend_from_slice(&id.to_be_bytes());
        out.push(data.len() as u8);
        out.extend_from_slice(data);
    }
    out.push(counter);
    out.push(checksum(&out));
    out
}

pub struct Frame<'a> {
    pub start: u8,
    pub groups: Vec<(u16, &'a [u8])>,
}

impl<'a> Frame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        let start = *bytes.first()?;
        let count = *bytes.get(1)? as usize;
        let mut groups = Vec::with_capacity(count);
        let mut at = 2;
        for _ in 0..count {
            let id = u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]);
            let len = *bytes.get(at + 2)? as usize;
            let data = bytes.get(at + 3..at + 3 + len)?;
            groups.push((id, data));
            at += 3 + len;
        }
        // Counter and checksum follow the groups.
        bytes.get(at + 1)?;
        Some(Self { start, groups })
    }

    fn group(&self, id: u16) -> Option<&'a [u8]> {
        self.groups.iter().find(|(g, _)| *g == id).map(|(_, d)| *d)
    }

    /// An unsigned big-endian field, or 0 when the moped sent a shorter group than usual.
    fn uint(&self, id: u16, offset: usize, len: usize) -> u32 {
        self.group(id)
            .and_then(|d| d.get(offset..offset + len))
            .map_or(0, |b| b.iter().fold(0, |acc, &x| (acc << 8) | u32::from(x)))
    }
}

pub fn login_frame(ccu_id: &str, passkey: &str, phone_uuid: &str, reconnecting: bool, counter: u8) -> Vec<u8> {
    let mut data = vec![0u8; CCU_ID_LEN + PASSKEY_LEN + PHONE_UUID_LEN + 1];
    let (ccu, key, flag) = if reconnecting {
        ("00000000000000", "000000", 0)
    } else {
        (ccu_id, passkey, 1)
    };
    data[..CCU_ID_LEN].copy_from_slice(&ccu.as_bytes()[..CCU_ID_LEN]);
    data[CCU_ID_LEN..CCU_ID_LEN + PASSKEY_LEN].copy_from_slice(&key.as_bytes()[..PASSKEY_LEN]);
    data[CCU_ID_LEN + PASSKEY_LEN..CCU_ID_LEN + PASSKEY_LEN + PHONE_UUID_LEN]
        .copy_from_slice(&phone_uuid.as_bytes()[..PHONE_UUID_LEN]);
    data[CCU_ID_LEN + PASSKEY_LEN + PHONE_UUID_LEN] = flag;
    build_frame(START_LOGIN, &[(GROUP_LOGIN, data)], counter)
}

/// `Some(true)` when the moped accepted the login, `Some(false)` when it refused.
pub fn login_result(frame: &Frame) -> Option<bool> {
    if frame.start != START_LOGIN_RESULT {
        return None;
    }
    frame.group(GROUP_LOGIN_RESULT)?;
    Some(frame.uint(GROUP_LOGIN_RESULT, 0, 1) == 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

pub fn clock_frame(t: ClockTime, phone_battery: u8, counter: u8) -> Vec<u8> {
    let year = t.year.to_be_bytes();
    let data = vec![
        year[0],
        year[1],
        t.month,
        t.day,
        t.hour,
        t.minute,
        t.second,
        (phone_battery << 1) | 1,
    ];
    build_frame(START_TO_DASH, &[(GROUP_CLOCK, data)], counter)
}

pub const INCOMING_ON: u16 = 1 << 14;
pub const MISSED_ON: u16 = 1 << 12;
pub const APP_SLOTS: usize = 6;

const fn app_on_bit(slot: usize) -> u16 {
    1 << (10 - 2 * slot)
}
const fn app_off_bit(slot: usize) -> u16 {
    1 << (11 - 2 * slot)
}

/// What the dash should be told, one-shot, as the original tracks it.
#[derive(Clone, Debug, Default)]
pub struct DashFlags {
    call_changed: bool,
    ringing: bool,
    slot_changed: [bool; APP_SLOTS],
    slot_on: [bool; APP_SLOTS],
}

impl DashFlags {
    pub fn call_ringing(&mut self) {
        self.ringing = true;
        self.call_changed = true;
    }

    /// Ringing stopped, answered or not. The original sends "missed" for both and never sends
    /// "incoming off", so this may be what clears the ringing light on the dash.
    pub fn call_stopped_ringing(&mut self) {
        self.ringing = false;
        self.call_changed = true;
    }

    pub fn set_app(&mut self, slot: usize, on: bool) {
        if slot < APP_SLOTS {
            self.slot_changed[slot] = true;
            self.slot_on[slot] = on;
        }
    }

    pub fn app_on(&self, slot: usize) -> bool {
        self.slot_on.get(slot).copied().unwrap_or(false)
    }

    pub fn ringing(&self) -> bool {
        self.ringing
    }

    /// The next 16-bit flag value. Clears what it reports.
    pub fn next_bits(&mut self, show_calls: bool) -> u16 {
        let mut bits = 0;
        if self.call_changed {
            self.call_changed = false;
            if show_calls {
                bits |= if self.ringing { INCOMING_ON } else { MISSED_ON };
            }
        }
        let mut any_on = false;
        for slot in 0..APP_SLOTS {
            if self.slot_changed[slot] {
                self.slot_changed[slot] = false;
                any_on |= self.slot_on[slot];
                bits |= if self.slot_on[slot] { app_on_bit(slot) } else { app_off_bit(slot) };
            }
        }
        bits |= if any_on { app_on_bit(0) } else { app_off_bit(0) };
        bits
    }
}

pub fn flags_frame(bits: u16, counter: u8) -> Vec<u8> {
    build_frame(START_TO_DASH, &[(GROUP_FLAGS, bits.to_be_bytes().to_vec())], counter)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pack {
    pub fitted: bool,
    pub percent: u8,
    pub low: bool,
    pub voltage: f32,
    pub current_ma: i32,
    pub temp_max: f32,
    pub temp_min: f32,
    pub remaining_ah: f32,
    pub full_ah: f32,
    pub health: u8,
    /// Its highest and lowest cell, when the reading makes sense as a voltage.
    pub cell_max_v: Option<f32>,
    pub cell_min_v: Option<f32>,
    pub limit_discharge_a: u8,
    pub limit_charge_a: f32,
    pub abnormal_code: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub packs: [Pack; 2],
    pub odometer_m: u32,
    pub switched_on: bool,
    pub model_number: u8,
    pub motor_rpm: i32,
    pub motor_temp: f32,
    pub controller_temp: f32,
    pub aux_voltage: f32,
    pub warning: u8,
    pub riding_mode: u8,
    pub output_limited: bool,
    pub vehicle_errors: Vec<u16>,
    pub controller_errors: Vec<u16>,
    pub speed_pulse: u8,
    /// How far the throttle is open, in percent.
    pub throttle: u8,
}

/// A battery's own long-term record, which the moped sends by itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatteryRecord {
    pub id: u16,
    pub charges: u16,
    pub fast_charges: u16,
    pub normal_charges: u16,
    pub slow_charges: u16,
    pub discharged_ah: u32,
    pub started: String,
}

/// What the moped says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vehicle {
    pub vin: String,
    pub software: u32,
    pub times_on: u16,
}

const LIVE_PACK_GROUPS: [(u16, u16); 2] = [(0x0450, 0x0470), (0x0451, 0x0471)];
/// Each battery's cell voltages and limits, then its manager's state.
const LIVE_CELL_GROUPS: [(u16, u16); 2] = [(0x0459, 0x0480), (0x045A, 0x0481)];
const MAIN_SWITCH_ON: u32 = 2;
/// Fault codes the original deliberately doesn't report.
const IGNORED_FAULT_CODES: [u16; 5] = [0xC111, 0xC110, 0x0B25, 0x0D22, 0x0000];
const LOW_FLAG_BELOW: u8 = 50;

/// The original never converts these two bytes. Its long-term record scales one byte of the same reading by 5/256, so this
/// tries the same scale over two bytes, then millivolts, and otherwise admits it doesn't know.
fn cell_volts(raw: u32) -> Option<f32> {
    const CELL_RANGE: std::ops::RangeInclusive<f32> = 2.0..=4.6;
    let scaled = raw as f32 * 5.0 / 65536.0;
    let millivolts = raw as f32 / 1000.0;
    [scaled, millivolts].into_iter().find(|v| CELL_RANGE.contains(v))
}

fn error_codes(frame: &Frame, id: u16) -> Vec<u16> {
    (0..4)
        .map(|i| frame.uint(id, i * 2, 2) as u16)
        .filter(|&c| c != 0)
        .collect()
}

impl Live {
    /// Faults worth telling the rider about: only while the warning light is on, as in the original.
    pub fn active_faults(&self) -> Vec<u16> {
        if self.warning == 0 {
            return Vec::new();
        }
        self.vehicle_errors
            .iter()
            .chain(&self.controller_errors)
            .filter(|code| !IGNORED_FAULT_CODES.contains(code))
            .copied()
            .collect()
    }
}

pub fn parse_live(frame: &Frame) -> Option<Live> {
    if frame.start != START_LIVE || frame.group(0x0450).is_none() {
        return None;
    }
    let mut packs = [Pack::default(); 2];
    for (i, (main, capacity)) in LIVE_PACK_GROUPS.into_iter().enumerate() {
        let fitted = (frame.uint(0x0340, i, 1) >> 5) & 1 == 1;
        let raw = frame.uint(main, 7, 1) as u8;
        let percent = if fitted { raw & 127 } else { 0 };
        packs[i] = Pack {
            fitted,
            percent,
            low: percent < LOW_FLAG_BELOW && raw >> 7 == 1,
            voltage: frame.uint(main, 4, 1) as f32 * 0.5,
            current_ma: i32::from(frame.uint(main, 5, 2) as u16 as i16) * 10,
            temp_max: (frame.uint(main, 2, 1) as f32 - 40.0) * 0.5,
            temp_min: (frame.uint(main, 3, 1) as f32 - 40.0) * 0.5,
            remaining_ah: frame.uint(capacity, 0, 1) as f32 * 0.1,
            full_ah: frame.uint(capacity, 1, 1) as f32 * 0.1,
            health: frame.uint(capacity, 4, 1) as u8,
            ..Pack::default()
        };
        let (cells, manager) = LIVE_CELL_GROUPS[i];
        packs[i].cell_max_v = cell_volts(frame.uint(cells, 0, 2));
        packs[i].cell_min_v = cell_volts(frame.uint(cells, 2, 2));
        packs[i].limit_discharge_a = frame.uint(cells, 4, 1) as u8;
        packs[i].limit_charge_a = frame.uint(cells, 5, 1) as f32 * 0.5;
        packs[i].abnormal_code = frame.uint(manager, 2, 2) as u16;
    }
    Some(Live {
        packs,
        odometer_m: frame.uint(0x0230, 0, 3) * 100,
        switched_on: frame.uint(0x0230, 4, 1) == MAIN_SWITCH_ON,
        model_number: frame.uint(0x0230, 5, 1) as u8,
        motor_rpm: frame.uint(0x023C, 0, 2) as i32 - 32768,
        motor_temp: frame.uint(0x0243, 0, 2) as f32 * 0.1 - 20.0,
        controller_temp: frame.uint(0x0248, 3, 2) as f32 * 0.1 - 20.0,
        aux_voltage: frame.uint(0x0322, 0, 1) as f32 * 0.1,
        warning: ((frame.uint(0x0322, 1, 1) >> 6) & 3) as u8,
        riding_mode: ((frame.uint(0x0322, 2, 1) >> 4) & 15) as u8,
        output_limited: (frame.uint(0x0322, 2, 1) >> 1) & 1 == 1,
        vehicle_errors: error_codes(frame, 0x0323),
        controller_errors: error_codes(frame, 0x0252),
        speed_pulse: frame.uint(0x0329, 0, 1) as u8,
        throttle: frame.uint(0x0220, 7, 1) as u8,
    })
}

const GROUP_BATTERY_RECORD: u16 = 0x0028;

pub fn parse_battery_record(frame: &Frame) -> Option<BatteryRecord> {
    if frame.start != START_BATTERY_RECORD {
        return None;
    }
    let data = frame.group(GROUP_BATTERY_RECORD)?;
    let field = |offset: usize| frame.uint(GROUP_BATTERY_RECORD, offset, 2);
    let started = data.get(40..43).map_or(String::new(), |d| format!("20{:02X}-{:02X}-{:02X}", d[0], d[1], d[2]));
    Some(BatteryRecord {
        id: field(38) as u16,
        charges: field(12) as u16,
        fast_charges: field(14) as u16,
        normal_charges: field(16) as u16,
        slow_charges: field(18) as u16,
        discharged_ah: field(8) * 5,
        started,
    })
}

pub fn parse_vehicle(frame: &Frame) -> Option<Vehicle> {
    if frame.start != START_COMMON_RECORDS {
        return None;
    }
    let vin = String::from_utf8_lossy(frame.group(0xF190)?);
    Some(Vehicle {
        vin: vin.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string(),
        software: frame.uint(0xF194, 0, 4),
        times_on: frame.uint(0xF1A2, 0, 2) as u16,
    })
}

/// The moped's own records, as it would send them. For the demo ride and the tests.
pub fn build_records_frames(vehicle: &Vehicle, record: &BatteryRecord, counter: u8) -> [Vec<u8>; 2] {
    let mut common = vec![(0xF190u16, vehicle.vin.as_bytes().to_vec())];
    common.push((0xF194, vehicle.software.to_be_bytes().to_vec()));
    common.push((0xF1A2, vehicle.times_on.to_be_bytes().to_vec()));
    let mut data = vec![0u8; 50];
    let mut put = |offset: usize, value: u16| data[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    put(8, (record.discharged_ah / 5) as u16);
    put(12, record.charges);
    put(14, record.fast_charges);
    put(16, record.normal_charges);
    put(18, record.slow_charges);
    put(38, record.id);
    let date: Vec<u8> = record.started.trim_start_matches("20").split('-').filter_map(|p| u8::from_str_radix(p, 16).ok()).collect();
    if date.len() == 3 {
        data[40..43].copy_from_slice(&date);
    }
    [build_frame(START_COMMON_RECORDS, &common, counter), build_frame(START_BATTERY_RECORD, &[(GROUP_BATTERY_RECORD, data)], counter)]
}

/// Encodes live data the way the moped does. The demo ride goes through the same parser as a real
/// ride this way, and the tests can check the parser against the sheet.
pub fn build_live_frame(live: &Live, counter: u8) -> Vec<u8> {
    let scaled = |v: f32, scale: f32| (v / scale).round() as u8;
    let mut groups: Vec<(u16, Vec<u8>)> = Vec::new();
    for (i, (main, capacity)) in LIVE_PACK_GROUPS.into_iter().enumerate() {
        let p = &live.packs[i];
        let current = ((p.current_ma / 10) as i16).to_be_bytes();
        let low = if p.low { 0x80 } else { 0 };
        groups.push((
            main,
            vec![
                0,
                0,
                (p.temp_max / 0.5 + 40.0).round() as u8,
                (p.temp_min / 0.5 + 40.0).round() as u8,
                scaled(p.voltage, 0.5),
                current[0],
                current[1],
                p.percent | low,
            ],
        ));
        groups.push((capacity, vec![scaled(p.remaining_ah, 0.1), scaled(p.full_ah, 0.1), 0, 0, p.health, 0]));
    }
    for (i, (cells, manager)) in LIVE_CELL_GROUPS.into_iter().enumerate() {
        let p = &live.packs[i];
        let raw = |v: Option<f32>| (v.unwrap_or(0.0) * 65536.0 / 5.0).round() as u16;
        let (high, low) = (raw(p.cell_max_v).to_be_bytes(), raw(p.cell_min_v).to_be_bytes());
        groups.push((cells, vec![high[0], high[1], low[0], low[1], p.limit_discharge_a, scaled(p.limit_charge_a, 0.5), 0, 0]));
        let code = p.abnormal_code.to_be_bytes();
        groups.push((manager, vec![0, 0, code[0], code[1], 0, 0, 0, 0]));
    }
    groups.push((0x0220, vec![0, 0, 0, 0, 0, 0, 0, live.throttle]));
    let odo = (live.odometer_m / 100).to_be_bytes();
    let switch = if live.switched_on { MAIN_SWITCH_ON as u8 } else { 0 };
    groups.push((0x0230, vec![odo[1], odo[2], odo[3], 0, switch, live.model_number, 0, 0]));
    let rpm = ((live.motor_rpm + 32768) as u16).to_be_bytes();
    groups.push((0x023C, vec![rpm[0], rpm[1], 0, 0]));
    let motor = (((live.motor_temp + 20.0) / 0.1).round() as u16).to_be_bytes();
    groups.push((0x0243, vec![motor[0], motor[1], 0]));
    let mcu = (((live.controller_temp + 20.0) / 0.1).round() as u16).to_be_bytes();
    groups.push((0x0248, vec![0, 0, 0, mcu[0], mcu[1]]));
    groups.push((
        0x0322,
        vec![
            scaled(live.aux_voltage, 0.1),
            live.warning << 6,
            (live.riding_mode << 4) | (u8::from(live.output_limited) << 1),
            0,
        ],
    ));
    let errors = |codes: &[u16]| {
        let mut data = vec![0u8; 8];
        for (i, code) in codes.iter().take(4).enumerate() {
            data[i * 2..i * 2 + 2].copy_from_slice(&code.to_be_bytes());
        }
        data
    };
    groups.push((0x0323, errors(&live.vehicle_errors)));
    groups.push((0x0252, errors(&live.controller_errors)));
    groups.push((0x0329, vec![live.speed_pulse, 0]));
    let exists = |p: &Pack| if p.fitted { 1 << 5 } else { 0 };
    groups.push((0x0340, vec![exists(&live.packs[0]), exists(&live.packs[1])]));
    build_frame(START_LIVE, &groups, counter)
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_code_unscrambles_with_the_table() {
        // Each position of this code holds the letter of the slot it lands in.
        let mut scrambled = ['?'; 24];
        let plain: Vec<char> = "ABCDEFGHIJKLMN123456WXYZ".chars().collect();
        for (i, &from) in QR_ORDER.iter().enumerate() {
            scrambled[from] = plain[i];
        }
        let code: String = scrambled.iter().collect();
        let (ccu, key) = decode_pairing_code(&code).unwrap();
        assert_eq!(ccu, "ABCDEFGHIJKLMN");
        assert_eq!(key, "123456");
    }

    #[test]
    fn plain_pairing_values_are_accepted_and_junk_refused() {
        let (ccu, key) = decode_pairing_code("ABCDEFGHIJKLMN 123456").unwrap();
        assert_eq!((ccu.as_str(), key.as_str()), ("ABCDEFGHIJKLMN", "123456"));
        assert!(decode_pairing_code("too short").is_none());
        assert!(decode_pairing_code("ÄBCDEFGHIJKLMN123456").is_none());
    }

    #[test]
    fn checksum_makes_the_frame_sum_to_zero() {
        let frame = build_frame(START_TO_DASH, &[(GROUP_FLAGS, vec![0x08, 0x00])], 7);
        assert_eq!(frame.iter().map(|&b| u32::from(b)).sum::<u32>() & 0xFF, 0);
    }

    #[test]
    fn checksum_of_a_zero_sum_stays_in_a_byte() {
        assert_eq!(checksum(&[0x80, 0x80]), 0);
    }

    #[test]
    fn login_layout_matches_the_sheet() {
        let uuid = "0123456789abcdef0123456789abcdef";
        let f = login_frame("ABCDEFGHIJKLMN", "123456", uuid, false, 3);
        assert_eq!(&f[..5], &[0xAA, 1, 0x7F, 0x00, 53]);
        assert_eq!(&f[5..19], b"ABCDEFGHIJKLMN");
        assert_eq!(&f[19..25], b"123456");
        assert_eq!(&f[25..57], uuid.as_bytes());
        assert_eq!(f[57], 1);
        assert_eq!(f[58], 3);
        assert_eq!(f.len(), 60);
        let again = login_frame("ABCDEFGHIJKLMN", "123456", uuid, true, 4);
        assert_eq!(&again[5..19], b"00000000000000");
        assert_eq!(&again[19..25], b"000000");
        assert_eq!(again[57], 0);
    }

    #[test]
    fn login_result_reads_the_flag() {
        for (flag, expected) in [(1, true), (0, false)] {
            let bytes = build_frame(START_LOGIN_RESULT, &[(GROUP_LOGIN_RESULT, vec![flag])], 0);
            assert_eq!(login_result(&Frame::parse(&bytes).unwrap()), Some(expected));
        }
    }

    #[test]
    fn clock_layout_matches_the_sheet() {
        let t = ClockTime { year: 2026, month: 11, day: 4, hour: 17, minute: 30, second: 5 };
        let f = clock_frame(t, 80, 0);
        assert_eq!(&f[..5], &[0xA6, 1, 0x05, 0x8A, 8]);
        assert_eq!(&f[5..13], &[0x07, 0xEA, 11, 4, 17, 30, 5, 161]);
    }

    #[test]
    fn flags_follow_the_original_rules() {
        let mut d = DashFlags::default();
        let idle = 1 << 11;
        assert_eq!(d.next_bits(true), idle);
        d.call_ringing();
        assert_eq!(d.next_bits(true), INCOMING_ON | idle);
        assert_eq!(d.next_bits(true), idle);
        d.call_stopped_ringing();
        assert_eq!(d.next_bits(true), MISSED_ON | idle);
        d.set_app(2, true);
        assert_eq!(d.next_bits(true), (1 << 6) | (1 << 10));
        d.set_app(2, false);
        assert_eq!(d.next_bits(true), (1 << 7) | idle);
        d.set_app(0, true);
        assert_eq!(d.next_bits(true), 1 << 10);
    }

    #[test]
    fn calls_stay_off_the_dash_when_the_setting_is_off_and_are_not_saved_up() {
        let mut d = DashFlags::default();
        d.call_ringing();
        assert_eq!(d.next_bits(false) & INCOMING_ON, 0);
        assert_eq!(d.next_bits(true) & INCOMING_ON, 0);
    }

    #[test]
    fn live_data_survives_a_round_trip() {
        let mut live = Live {
            odometer_m: 1_234_500,
            switched_on: true,
            model_number: 4,
            motor_rpm: 2100,
            motor_temp: 41.5,
            controller_temp: 38.0,
            aux_voltage: 12.6,
            warning: 1,
            riding_mode: 2,
            vehicle_errors: vec![0x0123],
            controller_errors: vec![],
            speed_pulse: 31,
            ..Live::default()
        };
        live.packs[0] = Pack {
            fitted: true,
            percent: 42,
            low: true,
            voltage: 51.5,
            current_ma: -12340,
            temp_max: 24.5,
            temp_min: 21.0,
            remaining_ah: 9.7,
            full_ah: 23.1,
            health: 96,
            ..Pack::default()
        };
        let bytes = build_live_frame(&live, 9);
        let frame = Frame::parse(&bytes).unwrap();
        assert_eq!(frame.groups.len(), 18);
        let parsed = parse_live(&frame).unwrap();
        assert!((parsed.motor_temp - live.motor_temp).abs() < 0.05);
        assert!((parsed.controller_temp - live.controller_temp).abs() < 0.05);
        assert!((parsed.aux_voltage - live.aux_voltage).abs() < 0.05);
        assert!((parsed.packs[0].remaining_ah - 9.7).abs() < 0.05);
        assert_eq!(parsed.packs[0].percent, 42);
        assert!(parsed.packs[0].low);
        assert_eq!(parsed.packs[0].current_ma, -12340);
        assert!(!parsed.packs[1].fitted);
        assert_eq!(parsed.odometer_m, 1_234_500);
        assert!(parsed.switched_on);
        assert_eq!(parsed.motor_rpm, 2100);
        assert_eq!(parsed.warning, 1);
        assert_eq!(parsed.riding_mode, 2);
        assert_eq!(parsed.vehicle_errors, vec![0x0123]);
    }

    #[test]
    fn only_real_faults_count_and_only_with_the_warning_light_on() {
        let mut live = Live { vehicle_errors: vec![0x0A12, 0xC111], controller_errors: vec![0x0D22, 0x0B26], ..Live::default() };
        assert!(live.active_faults().is_empty());
        live.warning = 1;
        assert_eq!(live.active_faults(), vec![0x0A12, 0x0B26]);
    }

    #[test]
    fn second_battery_is_read_from_its_own_groups() {
        let mut live = Live::default();
        live.packs[1] = Pack {
            fitted: true,
            percent: 77,
            low: false,
            voltage: 52.0,
            current_ma: 500,
            temp_max: 20.0,
            temp_min: 18.0,
            remaining_ah: 17.3,
            full_ah: 22.0,
            health: 90,
            ..Pack::default()
        };
        let parsed = parse_live(&Frame::parse(&build_live_frame(&live, 0)).unwrap()).unwrap();
        assert!(!parsed.packs[0].fitted);
        assert_eq!(parsed.packs[0].percent, 0);
        assert!(parsed.packs[1].fitted);
        assert_eq!(parsed.packs[1].percent, 77);
        assert_eq!(parsed.packs[1].voltage, 52.0);
        assert_eq!(parsed.packs[1].current_ma, 500);
        assert!((parsed.packs[1].remaining_ah - 17.3).abs() < 0.05);
        assert!((parsed.packs[1].full_ah - 22.0).abs() < 0.05);
        assert_eq!(parsed.packs[1].health, 90);
    }

    #[test]
    fn output_limit_bit_is_read() {
        let live = Live { output_limited: true, riding_mode: 3, ..Live::default() };
        let parsed = parse_live(&Frame::parse(&build_live_frame(&live, 0)).unwrap()).unwrap();
        assert!(parsed.output_limited);
        assert_eq!(parsed.riding_mode, 3);
    }

    #[test]
    fn missing_battery_reads_as_empty_and_low_needs_under_50() {
        let mut live = Live::default();
        live.packs[0] = Pack { fitted: false, percent: 90, ..Pack::default() };
        live.packs[1] = Pack { fitted: true, percent: 60, low: true, ..Pack::default() };
        let bytes = build_live_frame(&live, 0);
        let parsed = parse_live(&Frame::parse(&bytes).unwrap()).unwrap();
        assert_eq!(parsed.packs[0].percent, 0);
        assert_eq!(parsed.packs[1].percent, 60);
        assert!(!parsed.packs[1].low);
    }

    #[test]
    fn short_group_and_truncated_frame_are_safe() {
        let bytes = build_frame(START_LIVE, &[(0x0450, vec![1, 2])], 0);
        let parsed = parse_live(&Frame::parse(&bytes).unwrap()).unwrap();
        assert_eq!(parsed.packs[0].percent, 0);
        assert!(Frame::parse(&bytes[..4]).is_none());
        assert!(Frame::parse(&[]).is_none());
    }

    #[test]
    fn hex_round_trip_and_bad_input() {
        assert_eq!(from_hex(&to_hex(&[0, 171, 255])).unwrap(), vec![0, 171, 255]);
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }

    #[test]
    fn reads_the_moped_and_battery_records() {
        let vehicle = Vehicle { vin: "VIN1234567890ABCD".to_string(), software: 0x0102_0304, times_on: 812 };
        let record = BatteryRecord {
            id: 7,
            charges: 143,
            fast_charges: 0,
            normal_charges: 140,
            slow_charges: 3,
            discharged_ah: 2500,
            started: "2024-03-19".to_string(),
        };
        let [common, battery] = build_records_frames(&vehicle, &record, 1);
        assert_eq!(parse_vehicle(&Frame::parse(&common).unwrap()), Some(vehicle));
        assert_eq!(parse_battery_record(&Frame::parse(&battery).unwrap()), Some(record));
        assert_eq!(parse_vehicle(&Frame::parse(&battery).unwrap()), None);
        assert_eq!(parse_battery_record(&Frame::parse(&common).unwrap()), None);
    }

    #[test]
    fn a_cell_reading_that_makes_no_sense_stays_unknown() {
        assert_eq!(cell_volts(0), None);
        assert_eq!(cell_volts(3650), Some(3.65));
        assert!((cell_volts(47841).unwrap() - 3.65).abs() < 0.001);
        assert_eq!(cell_volts(100), None);
    }
}
