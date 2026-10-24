# Changelog

Each app version needs a section in this file. The version is the one in
`apps/desktop/package.json`. CI rejects a bump that does not add the matching
section. The release draft uses that section as its body.

## [0.1.4] - 2026-10-24

### Fixed

- The pictures arrive. The capture threw every frame away: the portal hands them over in dma-bufs, whose size is not where the code looked for it.
- The picture of a screen that does not change is repeated, so the phone has a picture instead of waiting for one that never comes.
- The desktop adds a new virtual monitor. It no longer shares the existing screen.
- The desktop leaves the screen of the user alone. The new virtual screen is found by name, so the resolution of the main display is never changed.
- The desktop asks for the screen permission once. The permission is kept, so the dialog only appears the first time.
- The size the desktop picks for a phone is the size the phone gets. Before, the phone asked for a size and the desktop ignored its own setting.
- The phone is one entry on the desktop. The phone owns its id now, so pairing again does not leave a second copy of the same phone behind.
- The phone pairs again once after this update, because the id it had was made by the desktop.
- The picture of a frame is no longer copied for every frame that is sent.
- The display is only reconfigured when the size really changes.
- Pairing opens the phone camera. The code scanner no longer fails to open.
- The phone scanner reads the code. The camera patch is part of the app now, and the reader is attached whatever order the camera receives its settings in.
- The phone reads the code with the scanner of the system first. The camera in the app is the fallback.
- The scanner of the system is no longer stripped from the release build.
- A code the camera cannot read says so, instead of leaving the scanner open with nothing happening.
- Closing the desktop app ends the portal session, removes the virtual screen and stops the encoder.
- A frame the capture cannot read no longer takes the whole app down.
- The desktop says when the virtual screen is waiting for its first picture.
- The phone sends the model of the device, so the desktop tells the phones apart.
- The desktop shows every phone with a live session, not only the first one.

### Added

- The phone app has a development build. `pnpm --filter @kvm/mobile build:dev:android` builds a dev client that pulls the code from the local server, and `pnpm --filter @kvm/mobile start` starts the server for it.

### Changed

- The window title bar drags the window again.
- The resolution and the frame rate are dropdowns on both sides, and they come from one list.
- The desktop owns the size of the stream. The phone shows the size in effect instead of picking one of its own.
- The phone has no retry settings any more. The phone comes back on its own.

## [0.1.2] - 2026-10-23

### Added

- The phone opens on a list of saved computers. Connecting tells you to open the PC app when that app is not running.
- Pairing scans the desktop code with the system code scanner.

### Fixed

- The desktop accepts the virtual-screen session path KDE returns as a string, so the screen can be created.
- A closed PC app no longer makes the phone retry the connection forever.

### Changed

- Release drafts take their tag and title from the desktop app version, and their notes from this file.
