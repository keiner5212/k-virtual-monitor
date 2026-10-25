# Changelog

Each app version needs a section in this file. The version is the one in
`apps/desktop/package.json`. CI rejects a bump that does not add the matching
section. The release draft uses that section as its body.

## [0.1.7] - 2026-10-25

### Fixed

- The stream keeps the rate it was asked for. The timer of the child that encodes the picture was taken away after its first call, so the repetition of the last picture only happened when the compositor painted a new one: the phone got one picture every few seconds on a quiet screen, and about half of the rate on a busy one.
- The phone comes back when the desktop goes away. It gave up on the first failure and the greeting had no timeout, so a desktop that was not up yet looked like a server that cannot be found, and a connection that was never answered left the phone waiting forever.
- The decoder of the phone takes every picture, in the order they arrive. The state of the interface kept only the newest one and lost the pictures that arrived together, and a picture that depends on another one cannot be drawn when that other one was lost.
- The decoder of the phone accepts the configuration of the codec. The parameter sets go to it without the start code, which many decoders refuse, and the pictures are decoded by a thread of their own instead of the thread of the interface.
- The reason of a failure of the decoder reaches the screen, over the picture, and the decoder tries again with the next key frame.
- The server stops when it is told to. The reader of the pictures waited for a picture that never came, so the headless server stayed alive after a signal, with its port and its encoder.
- The decoder of the phone builds again. The lock of the decoder was a plain object and the wait of the thread used methods that Kotlin does not have, so the app did not compile.

### Added

- The check of the session of the phone. It runs the client of the phone against the real server: it measures the rate of the stream with a screen that does not change, checks that every key frame carries the configuration the decoder needs, decodes the stream and checks that the phone comes back when the desktop goes away and comes back.

## [0.1.6] - 2026-10-24

### Changed

- Every picture of the stream is compressed with H.264. There is no JPEG any more: a picture that travels alone costs far more than the link can carry. The desktop asks for the plugin of the encoder and says so when it is missing.
- The phone asks for H.264 and says when the build has no decoder. It never falls back to another codec on its own.
- The picture goes to the phone as it is, without the detour of the base64 text. The phone spends its processor on the picture and not on the text around it.
- The encoder repeats the last picture of the screen when nothing changes, so the stream keeps the rate it was asked for and the phone always has a picture.

### Fixed

- The picture of the virtual screen reaches the phone as soon as the compositor paints it. The capture takes about ten milliseconds from the change on the screen to the picture that is sent.
- The list of the paired devices does not fill with copies of the same phone. The entry of the model is the entry of the phone, even when the app is installed again and the phone comes back with another id.
- A panic does not take the server down any more, and a lock that was left by a panic does not stop the phones from connecting.

## [0.1.5] - 2026-10-24

### Fixed

- The virtual screen picture now comes from GStreamer (`pipewiresrc`, scale, JPEG or H.264). Reading the PipeWire buffers by hand produced a scrambled frame or none.
- Only the configured frame rate is compressed and sent. The rate can be set up to 120, and the virtual screen runs at that rate. Extra compositor frames are dropped before the encoder.
- The phone keeps the latest keyframe until the view is ready. Before, that frame was dropped and a still screen never sent another one the decoder could start from.

## [0.1.4] - 2026-10-24

### Fixed

- The pictures arrive. The capture threw every frame away: the portal hands them over in dma-bufs, whose size is not where the code looked for it.
- The picture of a screen that does not change is repeated, so the phone has a picture instead of waiting for one that never comes.
- The size the desktop picks for a phone is the size the phone gets. Before, the phone asked for a size and the desktop ignored its own setting.
- The phone is one entry on the desktop. The phone owns its id now, so pairing again does not leave a second copy of the same phone behind.
- The phone pairs again once after this update, because the id it had was made by the desktop.
- The picture of a frame is no longer copied for every frame that is sent.
- The display is only reconfigured when the size really changes.
- The phone reads the code with the scanner of the system first. The camera in the app is the fallback.
- The scanner of the system is no longer stripped from the release build. The switch that turns the shrinking off was never reaching the build.
- The phone says when the camera reads nothing, instead of leaving the user in front of a preview that does nothing.
- Closing the desktop app ends the portal session, removes the virtual screen and stops the encoder. A server that is asked to stop does the same.
- A frame the capture cannot read no longer takes the whole app down.
- The desktop says when the virtual screen is waiting for its first picture.
- The phone sends the model of the device, so the desktop tells the phones apart.
- The desktop shows every phone with a live session, not only the first one.

### Added

- The phone app has a development build. `pnpm --filter @kvm/mobile build:dev:android` builds a dev client that pulls the code from the local server, and `pnpm --filter @kvm/mobile start` starts the server for it.

### Changed

- The resolution and the frame rate are dropdowns on both sides, and they come from one list.
- The desktop owns the size of the stream. The phone shows the size in effect instead of picking one of its own.
- The phone has no retry settings any more. The phone comes back on its own.

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
