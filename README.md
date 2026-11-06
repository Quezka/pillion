# Pillion

*The pillion is the seat behind the rider.* A riding companion for an electric moped, in the style of the Quezka suite: battery,
range, consumption, spoken directions and lights on the dash, with a voice mode for riding with an earpiece.

## Install on a phone

Open the latest release on the phone, download `Pillion-x.y.z.apk` and open it. Android asks to allow installing from the browser
the first time. Then pair the moped with the 24 characters of its QR sticker, or try the demo ride without one.

Notifications need one more permission: Pillion's Alerts tab opens the setting. Android calls it a restricted setting for an app
installed this way; if so, open Pillion's App info, tap the three dots, then Allow restricted settings.

Not yet tried on a real moped.

## Build

- Tests: `cargo test --lib`
- Desktop preview, which always plays the demo ride: `cargo run` (`--screenshot file.png` saves a picture and exits)
- Android: `bash android/build_android.sh` makes `Pillion.apk`. It needs the Android SDK and NDK (`ANDROID_HOME`, default
  `~/Android/Sdk`), and the Rust targets `aarch64-linux-android` and `x86_64-linux-android`.

The logic is Rust (`src/`), the same on the desktop and the phone, and the Android side (`android/java/`) carries out its commands.
`MECHANICS.md` holds what's known about how the moped talks and where each fact was read.

## Licence

Pillion is free software under the GNU General Public License version 3 only, with no warranty: see `LICENSE`. Copyright (C) 2026
Quezka. The parts it's built with keep their own licences, listed in `THIRD_PARTY_NOTICES.txt`; after changing dependencies, run
`python3 tools/make_notices.py`, or the release build stops. Pillion isn't affiliated with or endorsed by Yamaha.

## Release

Bump `version` in `Cargo.toml`, add a `CHANGELOG.md` section, then push a tag `vX.Y.Z`. CI tests, builds and publishes the signed APK.

Phones only accept updates signed with the same key, so never lose it or regenerate it. It isn't in this repository. CI holds it as
the secrets `PILLION_KEYSTORE_B64` and `PILLION_KEYSTORE_PASSWORD`, and there's a copy in the private `quezka/pillion-signing` on
git.lan. A local build reads it from `android/pillion.jks` and `android/keystore.properties`, both git-ignored.

`tools/publish.sh` does the first publish: the git.lan backups, then the GitHub repo and its secrets, then the tag.
