# Changelog

## [0.5.1] - 2026-11-09

- The suite's new shared look (0.5.0 didn't build: its licence notices were out of date).

## [0.6.0] - 2026-11-09

- Pick the voice on the Voice tab: "Best" (the phone's own voice when it's a good one, such as Google's, otherwise the
  natural voice), "Phone's" or "Natural". Best is the new default. If one voice fails on a sentence, the other says it.
- Pillion only talks while the moped is connected (or on the demo ride). Off the moped it stays quiet, apart from
  "connection lost", Bluetooth problems and what you ask for with the earpiece button. Everything still goes in "Last said".

## [0.5.0] - 2026-11-09

- The suite's new shared look: neutral greys instead of blue-tinted ones, with indigo kept as the one accent colour.

## [0.4.0] - 2026-11-09

- The Route tab is a full-screen map, and everything over it is liquid glass: the title bar, the tabs, the search bar
  or the next turn, the map's buttons and a sheet with your places along the bottom. The map shows through all of
  them, blurred, and they're more see-through than before. Tap the sheet's heading to open or fold it; it opens by
  itself for search results and a picked spot, and Stop is always one tap away while navigating.
- Words fit the screen in every language: titles, tab names, buttons and hints shrink a little or end with "…"
  instead of running off the edge or under other things (checked in Italian and Russian on narrow phones).

## [0.3.1] - 2026-11-09

- Liquid glass done properly, the Apple way: backgrounds are plain and cards solid again (the colour wash is gone),
  and the map's buttons are glass. What shows through them is the real map underneath, blurred, with a bright rim
  and a soft shadow.

## [0.3.0] - 2026-11-08

- When the moped is switched off, Pillion stops looking for it. It leaves one low-power request with the phone's
  Bluetooth, so it reconnects by itself as soon as the moped is on again. The first search gives up after 45 seconds
  instead of running forever.
- Much lighter on the phone's battery when not riding: no GPS with the screen off, a slow GPS fix with it on, and
  Pillion checks for work once a second instead of twenty times.
- Speed limits: riding onto a road with a different limit, the voice says it ("Via Zamboni: speed limit 30"), and a
  road sign shows on the Ride and Route screens. It warns once per road when you stay over the limit. The limits come
  from OpenStreetMap, so check the real signs too; where the map has none, Pillion says nothing. Both can be switched
  off on the Voice tab.
- A new map: drag it, pinch or use the buttons to zoom, and the target button brings it back to you. Press and hold
  a spot to go there or save it. It shows how far the battery reaches, on real roads when online, as a circle
  otherwise.
- Saved places: Home, Work and any place you star, on the Route tab and on the map. Search is now typo-tolerant and
  puts places near you first.
- Better spoken directions: the route's start says when you'll arrive, longer rides get a "halfway there", and after
  going off the route you hear "New route".
- A glass look over Material 3, with brighter colours, like the rest of the suite. Settings turns it off; High
  contrast stays solid.

## [0.2.1] - 2026-11-06

- Pillion is now free software under the GNU GPL version 3. The app says so in Settings and carries the licence and the notices for
  everything it's built with.
- Each release now includes the source code of the speech engine.

## [0.2.0] - 2026-11-06

- Updates from GitHub: Pillion looks for a new version once a day, and installs it when you say so.
- Pair by scanning the moped's QR sticker with the camera. Typing the code is still there.
- A 3D model of the moped on the Moped tab, traced from photos of the real one: drag to turn it, pinch to zoom, and tap a part to see its
  figures. The seat opens to show the batteries, and their charge lights follow the real charge. Teal, white or black.
- More from the moped: each battery's charge count and cell voltages, the current limit, the throttle, the frame number and software, and a
  way to record what it sends. It can't be controlled from the phone: the moped has no such messages.
- Navigation: a street map that turns with you, or shows the whole route; warnings timed to your speed; joined turns told together; roundabout
  exits; the right arrows on ramps and forks; arrival time; nearby places first in search; no U-turn at the start of a new route.
- A natural voice that runs on the phone with no internet, downloaded when you ask, in English, Italian or Russian. Without it, Android's best
  voice is used.
- Screens redone in Material 3, with a guided start, a Tell me button on the Ride tab, brief confirmations and a tick you can feel on each tap.
- Riding with two batteries: taking one out or putting one in no longer upsets the ride's figures, and each has its own bar.

## [0.1.0] - 2026-11-05

The first test build, for trying on a real phone and moped.

- Battery, range and consumption from the moped over Bluetooth, for one or two batteries.
- Spoken directions on roads a moped can use, with free OpenStreetMap services.
- A voice mode for riding with an earpiece: turns, battery, consumption, warnings, calls and messages. The earpiece's button
  asks for a status report.
- Call and app lights on the moped's dash. The dash can only light up, so message text is spoken instead.
- The suite's look, in English, Italian and Russian, light, dark and high contrast.

Not yet tried on a real moped. The Bluetooth connection is written from the original app's code and is the first thing to test.
