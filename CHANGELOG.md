# Changelog

Each app version needs a section in this file. The version is the one in
`apps/desktop/package.json`. CI rejects a bump that does not add the matching
section. The release draft uses that section as its body.

## [0.1.3] - 2026-10-23

### Fixed

- The desktop adds a new virtual monitor. It no longer shares the existing screen.
- The desktop leaves the screen of the user alone. The new virtual screen is found by name, so the resolution of the main display is never changed.
- The desktop asks for the screen permission once. The permission is kept, so the dialog only appears the first time.
- Pairing opens the phone camera. The code scanner no longer fails to open.
- The phone scanner reads the code. The camera patch is part of the app now, and the reader is attached whatever order the camera receives its settings in.
- A code the camera cannot read says so, instead of leaving the scanner open with nothing happening.

### Changed

- The window title bar drags the window again.

## [0.1.2] - 2026-10-23

### Added

- The phone opens on a list of saved computers. Connecting tells you to open the PC app when that app is not running.
- Pairing scans the desktop code with the system code scanner.

### Fixed

- The desktop accepts the virtual-screen session path KDE returns as a string, so the screen can be created.
- A closed PC app no longer makes the phone retry the connection forever.

### Changed

- Release drafts take their tag and title from the desktop app version, and their notes from this file.
