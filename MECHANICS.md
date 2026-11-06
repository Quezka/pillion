# Pillion: how the moped talks, fact sheet

Read from MyRide 6.2.9 by decompiling its Android code. Our code is written from this sheet; nothing
is pasted from the original. `estimated` marks values the code does not settle.

## How this was read

The app's logic is Kotlin and Java bytecode, which has no native addresses. On the app itself,
`native_strings_find` and `native_functions_list` both refuse: "An Android app is not native machine code"
(the Ghidra run on the APK also failed). Its six native libraries (`librealm-jni`, `libgmm-jni`,
`libbarhopper_v3`, `libimage_processing_util_jni`, `libdatastore_shared_counter`, `libsurface_util_jni`)
hold no moped code: a byte search of all six for `6e400001`, `YSCCU`, `YCCU`, `RSOC`, `CCU-ID`, `passkey`,
`bluetooth` and `yamaha` finds nothing, and `native_strings_find` on `librealm-jni` agrees
(ev_83d8d088...). So no native function had anything to read. `native_function_pseudocode` on the app gives the same refusal.

The real code was read with JADX and with `android_method_show`, so an address here is a DEX method.
Checked instruction by instruction:

| Method | DEX | Evidence |
|---|---|---|
| `Lcom/yamaha/myride/helper/b2;->a(FIFII)D` range | classes4 | ev_5d528907... |
| `Lcom/yamaha/myride/helper/b2;->b(IIIF)F` charge time, `div-int` at 0005 | classes4 | ev_742dd660... |
| `ev/MessageId0x56_CAN;->getBatteryRemain1()I`, mask at 000e | classes3 | ev_1c70be13... |
| `ev/MessageId0x56_CAN;->getODO()I`, times 100 at 000a | classes3 | ev_42def6c8... |
| `ev/MessageId0x56_CAN;->getOutputLimit()I`, bit 1 at 0006 | classes3 | ev_0c99c165... |
| `ev/MessageId0x55_Local0x0028;->getTotalDischargeCapacity()I`, times 5 at 000a | classes3 | ev_8450ffb8... |
| `ev/MessageId0x56_CAN;->getAPSOpening()I`, raw value at 000a | classes3 | ev_d5e82bef... |
| `Ldg/e;-><clinit>()V` QR table | classes3 | ev_6a468804... |
| `Lug/e;->o(Ljava/util/List;I)Ljava/lang/String;` fault filter | classes3 | ev_f3741190... |
| `Lag/h;->a(I)V` call state, change test at 000b | classes3 | ev_4e523c8a... |

## Finding the moped

| What | Value | Source |
|---|---|---|
| Bike family used by electric models | `EV` (`BIKE_09_X`) | `data.models.a`, `YSCCUService` line 650 |
| Bluetooth names to look for | contains `YSCCU` or `YCCU` | `dg.g.b` |
| Device name format | `YSCCU_<CCU-ID>` | `kg.j.b` |
| Service UUID | `6e400001-b5a3-f393-e0a9-e50e24dcca9e` | `kg.j.a` |
| Phone writes to | `6e400002-b5a3-f393-e0a9-e50e24dcca9e` | `kg.j.b`, `kg.t1.h` |
| Moped notifies on | `6e400003-b5a3-f393-e0a9-e50e24dcca9e` | `kg.j.c`, `kg.t1.j` |
| Notification switch | descriptor `00002902-...` | `kg.j.d` |
| Connection | bond, then GATT over LE with auto-connect | `YSCCUService.v0` |
| MTU asked for | 512 | `YSCCUService.d` |
| Gap between writes | 100 ms | `YSCCUService.a1` |
| Connection timeout | 15 s | `YSCCUService.p1` |

## Pairing code

The sticker's QR code holds 24 characters in scrambled order. Read position `table[i]` for each
`i` from 0 to 23. Characters 0 to 13 of the result are the CCU-ID, 14 to 19 are the passkey.
Table: `7 14 20 0 12 17 6 22 2 13 10 1 16 19 8 21 23 5 15 3 9 18 4 11` (`dg.e`).

The other route in the original asks Yamaha's server for the same two values by VIN
(`v3.00/glocaliot/pairing/{vinNumber}`, `ug.u`, `PairMyBikeVinInputActivity`). We don't use that route.

## Message format

`[start byte][number of groups]`, then for each group `[id high][id low][length][data...]`, then
`[counter][checksum]`. Numbers are big-endian. Checksum: `256 - (sum of all earlier bytes & 0xFF)`
(`ev.Message.MessageBuilder`). The original does not reject messages with a bad checksum (`kg.t1.j`),
so neither do we. The real length byte can be shorter than the normal layout, and fields past it read as 0
(`ev.Message.getByteData`).

| Start byte | Direction | Meaning |
|---|---|---|
| `0xAA` group `7F00` | phone to moped | login |
| `0x5A` group `7F01` | moped to phone | login result, 1 means accepted |
| `0x56` 19 groups | moped to phone | live data (`MessageId0x56_CAN`) |
| `0x55` group `0028` | moped to phone | battery history |
| `0x5B` 6 groups | moped to phone | VIN, software, energy totals |
| `0xA6` group `058A` | phone to moped | clock and phone battery |
| `0xA6` group `058B` | phone to moped | dash notification flags |

### Login, group `7F00` (`LocalId0x7F00`, `kg.t1.m`, `YSCCUService.onMtuChanged`)

CCU-ID 14 ASCII, passkey 6 ASCII, phone UUID 32 ASCII (a random UUID without dashes, new for each
service start), bonding flag 1 byte. First connection of a run: real values, flag 1. Reconnection in the same run:
`00000000000000`, `000000`, flag 0. Sent after the MTU change.

### Live data, start byte `0x56`

| Group | Offset:length | Field | Rule | Source method |
|---|---|---|---|---|
| 0340 | 0:1, 1:1 | Battery status 1, 2 | bit 5 set means the battery is fitted | `getBatteryExist1/2` |
| 0450, 0451 | 0:1 | Segment | raw | `getBattery1SegmentInfo` |
| 0450, 0451 | 2:1, 3:1 | Temperature max, min | `(raw - 40) * 0.5` deg C | `getBattery1TemperatureMax` |
| 0450, 0451 | 4:1 | Pack voltage | `raw * 0.5` V | `getTotalBattery1Voltage` |
| 0450, 0451 | 5:2 signed | Current | `raw * 10` mA | `getCurrent1` |
| 0450, 0451 | 7:1 | Charge | `raw & 127` %, 0 if not fitted | `getBatteryRemain1` |
| 0450, 0451 | 7:1 | Low flag | bit 7 set and charge below 50 | `getBatteryLowFlg1` |
| 0470, 0471 | 0:1, 1:1 | Remaining, full capacity | `raw * 0.1` Ah | `getRemainingCapacity1`, `getCurrentFCC1` |
| 0470, 0471 | 4:1 | Health | raw % | `getSOH1` |
| 0230 | 0:3 | Odometer | `raw * 100` m (unit estimated) | `getODO` |
| 0230 | 4:1 | Main switch | 2 means on | `service.g.K2` |
| 0230 | 5:1 | Model number | raw | `getModelNumber` |
| 023C | 0:2 | Motor speed | `raw - 32768` rpm | `getMotorSpeed` |
| 0243 | 0:2 | Motor temperature | `raw * 0.1 - 20` deg C | `getMotorTemperature` |
| 0248 | 3:2 | Controller temperature | `raw * 0.1 - 20` deg C | `getMCUTemperature` |
| 0322 | 0:1 | 12 V battery | `raw * 0.1` V | `getPbBatteryVoltage` |
| 0322 | 1:1 | System warning | `(raw >> 6) & 3` | `getSystemWarningIndicator` |
| 0322 | 2:1 | Riding mode | `(raw >> 4) & 15`, names unknown | `getEVMode` |
| 0323 | 0:8 | Vehicle error codes | four 2-byte values | `getVehicleErrorList` |
| 0252 | 0:8 | Controller error codes | four 2-byte values | `getMCUErrorList` |
| 0329 | 0:1 | Speed pulse | raw, conversion unknown | `getEVSpeedPulse` |

### Formulas

- Remaining range, each fitted battery, added together (`helper.b2.a`, `service.g.l1`):
  `km_per_kwh * charge% * 0.01 * full_Ah * avg_voltage` gives metres.
  The stored records are named `EvDistanceToEmptyEntity` (`of.t`) and `EvEstimatedChargingTimeEntity`
  (`of.u`), which confirms what these two formulas mean.
- A battery counts toward range only if fitted and above 0% (`ug.e` screen update, called as `B`).
- Range is withheld (`ug.e`, same method) if output is limited, or with one battery at 15% or less, or
  with two batteries at a combined 30% or less, or with none fitted. We show "too low to estimate".
- Full-charge range (`ug.e`, same method): as above with 100%, a fixed 20.0 Ah per fitted battery, divided by 1000.
- Average efficiency defaults to 30 km/kWh, and is ignored if below 5 (`ug.e`; `service.g` lines 1383, 1465).
  The original's average is `averageElectricityCost` from Yamaha's server (`ElectricityCostWorker`, table
  `electricity_cost`: total distance, total consumption, average). We keep the same kind of total on the phone:
  all finished rides plus this one, used once it covers 2 km (the 2 km is estimated).
- Charging time in hours (`helper.b2.b`, base 8 and factor 1 from `ug.e.<init>`):
  `(8 * (100 - charge%)) / 100` as a whole-number division, `* health% * soh_factor / 100`. We follow it exactly,
  so a nearly full battery reads 0 h, as it does in the original.
- Model data (`EVConfig.json`, public file the app downloads): average voltage 50 V; `soh_factor` 1.125 for all
  listed models (BFM4, BFM7, BFMG, BUS1, BUX1, D341, D342, D343) and 1 for the fallback `00`.
- Fallbacks if the config has no value (`of.p.a`): 50 V, factor 1.0, disconnect 5, bands 20, 30, 40, 50. The config is
  looked up by a model code from Yamaha's server, then falls back to the `00` row (`service.g` lines 1395-1460).
  We can't ask that server, so we use the BFM value 1.125 (estimated for this moped, whose code isn't confirmed).
- The `disconnect` value and efficiency bands (`revsDb.electricityCost1..4`) are stored in table `ev_config`, but no
  code in the app reads them for a decision, so the build doesn't use them.

### Two batteries

Each battery has its own groups: `0450` and `0470` for the first, `0451` and `0471` for the second, and one byte each in `0340`
for whether it's fitted (`MessageId0x56_CAN`). Range adds the two, and the withheld-range rule above changes with how many are
fitted. The original shows each battery on its own; the combined percent on our Ride screen and in the voice is ours, weighted by each
battery's full capacity (estimated). Which battery the moped drains first is decided on the moped and isn't in the app. A battery
going in or out during a ride starts a new ride (ours), so it never counts as energy used or gained.

### Rides (`service.g.w1`, `service.g.h`, `service.g.K2`)

- A ride starts when the moped sends group `0028` (`service.g.h`). It marks the main switch on. We start one when the live
  data shows the switch on, which is the same signal. We also restart after a charge or battery swap (ours).
- Checked once a second: a ride ends once the main switch is off or the link is down for 5 seconds running.
  A shorter dropout carries on the same ride.

### Faults (`ug.e.o`, list from `ug.e.<init>`)

Fault codes count only while the system warning value is not 0. The codes `C111`, `C110`, `0B25`, `0D22` and `0000`
are never reported. The original also stops after its first report; we speak each new code once.

### Fields the app doesn't interpret

Speed pulse, riding mode, output limit, sub mode and most others go only into a stored record (`of.c.a`) for upload to
Yamaha, with no conversion or names anywhere in the app, except that output limit also withholds the range. So speed
comes from GPS, and riding mode has no name to show.

### Clock and dash flags (timer every 1 s: `058A`, then `058B` 100 ms later; `YSCCUService.v`, `v1`)

`058A`: year 2 bytes, month, day, hour, minute, second, then `(phone battery % << 1) | 1`.

`058B` is one 16-bit value:

| Bit | Meaning | Bit | Meaning |
|---|---|---|---|
| 15 | incoming call off | 14 | incoming call on |
| 13 | missed call off | 12 | missed call on |
| 11, 10 | app 1 off, on | 9, 8 | app 2 off, on |
| 7, 6 | app 3 off, on | 5, 4 | app 4 off, on |
| 3, 2 | app 5 off, on | 1, 0 | app 6 off, on |

Rule (`MessageId0xA6_Local0x058B.createBytes`, `ag.h.a`): flags are sent once, when something changes. A change means
the ringing state flipped (`ag.h.a`: `z = state == ringing`, offset 000b), so an outgoing call changes nothing. Ringing gives
"incoming on". Ringing that stops gives "missed on", whether or not it was answered: the suppress flag (`ag.h.I`) starts false
and nothing sets it, and "incoming off" is never sent, so "missed" may be what clears the ringing light. We follow this,
and only the voice tells them apart.
Call lights need the `incomingcall` setting (`ag.h.J`, `ag.l.b`), which the original leaves off until the rider sets it. We
have the same switch, on by default (ours), because the rider asked for notifications on the dash.
An app's light goes on when it notifies and off when that notification is removed (`ag.h.Q`, `ag.h.R`, `ag.h.X`). Lights follow
the six apps picked in the original's settings; we hand them out as apps first notify, with a mute switch (ours). WhatsApp calls
count as calls, not as an app notification (`ag.h.L`); we treat every notification in Android's call category that way.
Apps: each changed slot gets its on or off bit. App 1 is then also set to "on" if any changed slot went on, otherwise
"off", on every message.

**The dash cannot show text.** The message has only these flags, and the electric message set has
no navigation group at all (compare the `ice2` family, which does). So message text and turn directions
can't appear on this moped's dash; the app speaks them and shows them on the phone.

## Not from the original

The original navigates with Google's paid Navigation SDK and its own key, which can't be reused.
We use free OpenStreetMap services: Valhalla (`motor_scooter` costing, top speed 45 km/h, no motorways)
for routes and spoken directions, Nominatim for address search. Announcement distances, voice phrasing
and off-route rules are ours and marked `estimated` in `mechanics.json`. Places a scooter can't enter,
such as palace grounds, come back as "no path"; the app then asks again, accepting roads within 300 m.

## The 3D moped

`src/model3d.rs` models the rider's moped in code and draws it with its own small renderer, so the app needs no model file or
graphics library. MyRide has no 3D model. The side outlines are traced from the rider's side photo at 470 pixels to the metre,
which agrees with the 13 inch wheels and a seat about 0.79 m high. Widths are judged from their front and three-quarter photos, so
they are less exact. It leaves out badges and lettering. `examples/model_shot.rs` draws it to a picture for comparing with photos.
The batteries' place and size under the seat are estimated: no photo shows the seat open.

## Other things the moped sends (`MessageId0x5B_CommonRecords`, `MessageId0x55_Local0x0028`, `MessageId0x56_CAN`)

The phone only ever sends login, clock and dash flags: there's no message for horn, lock, lights or riding mode anywhere in the
electric message set, so the app can't control the moped. The moped sends all the rest by itself (`lg.d.h`, `lg.d.k`).

| Start, group | Offset:length | Field | Rule | Source method |
|---|---|---|---|---|
| 0x5B F190 | 0:17 | Frame number | text | `getVehicleVin` |
| 0x5B F194 | 0:4 | Software number | raw | `getECUSoftwareVersion` |
| 0x5B F1A1 | 0:4 | Total powered time | raw, unit unknown, not shown | `getECUElectrifyTotalTime` |
| 0x5B F1A2 | 0:2 | Times switched on | raw | `getECUIgnitionTotalCount` |
| 0x5B F701 | 0:16 | Four energy totals | raw, unit unknown, not shown | `getAccumulatePowerConsume1` |
| 0x55 0028 | 8:2 | Total discharged | `raw * 5`, read but not shown as its unit is unknown | `getTotalDischargeCapacity` |
| 0x55 0028 | 12:2 | Charges | raw | `getTotalChargeCount` |
| 0x55 0028 | 14:2, 16:2, 18:2 | Fast, normal, slow charges | raw | `getFastChargeCount` |
| 0x55 0028 | 38:2 | Battery id | raw | `getBatteryIdentifyID` |
| 0x55 0028 | 40:3 | First use | hex digits; reading them as YY MM DD is estimated | `getBatteryStartDate` |
| 0x56 0459, 045A | 0:2, 2:2 | Highest, lowest cell | see below | `getCellVoltageMax1` |
| 0x56 0459, 045A | 4:1, 5:1 | Discharge limit, charge limit | raw A, `raw * 0.5` A | `getCurrentLimitDischarge1` |
| 0x56 0480, 0481 | 2:2 | Battery fault code | raw, read but not used yet | `getAbnormalCode1` |
| 0x56 0220 | 7:1 | Throttle | raw % | `getAPSOpening` |

The original never converts the cell readings. Its long-term record scales the single-byte form by 5/256, so we try 5/65536 for
two bytes, then millivolts, and show nothing unless the result looks like a cell (2.0 to 4.6 V). This scale is estimated. The Moped
tab can record what the moped sends and share it as text, to settle these on real data. A recording holds the frame number.

## How the navigation speaks and draws

Valhalla's route answers are used for the wording. Turn warnings go by time, 24 s and 5 s ahead, within 150 to 350 m and 30 to 70 m
(all estimated), or 300 m and 60 m with no speed known. A joined turn (`verbal_multi_cue`) is told together with the one before it. After a
turn with over 1.5 km to the next, its `verbal_post_transition_instruction` is spoken. A new route keeps the rider's direction with
`heading` when moving above 7 km/h. Search prefers places within about 0.3 by 0.2 degrees of the rider, but isn't limited to them.
The map's pictures come from tile.openstreetmap.org and are kept for 7 days, as its usage policy asks, with the credit on the map.

## Updates (not from the original)

Once a day, or when asked, the app reads `api.github.com/repos/Quezka/pillion/releases/latest` without logging in, so **the repository and its releases
have to be public** for this to work. A newer `vX.Y.Z` tag with an `.apk` is offered, never installed unasked, and not mentioned on the Ride tab while riding.
The file is checked against GitHub's SHA-256 when it gives one, must be this package with a higher version code, and goes to Android's own installer. Android
asks the rider to confirm, and refuses any file not signed with the installed app's key. So a release must be built with `android/pillion.jks`.

## The natural voice

The voices are Piper models run offline by sherpa-onnx 1.13.8. Version 2 removed espeak-ng, which turns these voices' words into sounds.
They're downloaded only when the rider asks, from `csukuangfj/vits-piper-*` on Hugging Face, and the model is checked against its SHA-256.
Voices: `en_GB-alba-medium` (CC BY 4.0), `it_IT-paola-medium` (licence: see its dataset's page), `ru_RU-irina-medium` (its model card says
unknown). **espeak-ng is GPL 3**, and it's inside the engine library that ships in the APK, so anyone given the APK is owed its source under those terms.
Until a voice is there, the app uses Android's speech and picks its best installed voice for the language.

## The suite's look

Pillion belongs to the Quezka suite, so its look isn't taken from MyRide. Colours are suite-kit's `quezka_ui/theme.py` token
for token (light, dark, and both high contrast themes), the font is its Manrope, icons are Material Symbols fetched the same
way (`tools/make_symbols.py`), outlined and filled when selected. English, Italian and Russian as every app has, with a test
for completeness. Package `io.github.quezka.pillion`, like Quire and Vesper.
