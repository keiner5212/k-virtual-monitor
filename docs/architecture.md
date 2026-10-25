# Architecture

The flows are drawn in [flows.md](flows.md).

## The pieces

```text
phone (Expo, Android)          desktop (Tauri v2, Linux)
┌───────────────────┐          ┌────────────────────────────┐
│ SessionClient     │          │ server::Server             │
│  @kvm/protocol    │◀────────▶│  kvm-protocol crate        │
│                   │   TCP    │  device store (JSON)       │
│ FrameFeed         │  28252   │  FrameSource               │
└───────────────────┘          │   └─ SyntheticSource (now)  │
                               │   └─ portal capture (next) │
                               └────────────────────────────┘
```

The wire protocol lives in `packages/protocol` and is implemented twice, in
Rust and in TypeScript. The golden vectors in
`packages/protocol/testdata/vectors.json` tie the two implementations
together: a change in one that is not mirrored in the other fails the tests.

## The desktop app

- `apps/desktop/src-tauri` is a Cargo crate. The server is a library, the
  Tauri app is a thin shell around it, and `--headless` runs the server
  without a webview.
- The capture is a `FrameSource`. The server does not know how the picture
  is produced. On a Wayland desktop the source is a virtual screen the XDG
  portal creates, and the synthetic test pattern is the fallback for the
  tests and the headless runs.
- The virtual screen is found by name. The output ids of `kscreen-doctor`
  change when the layout changes, so the screen of the user is never the one
  that gets resized.
- The portal permission is kept in `~/.local/state/kvm/portal.json`, so the
  consent dialog only appears the first time.
- The input path is the mirror image: the server parses the input messages
  and hands them to an injector. Today the injector only logs.

## The phone app

- `apps/mobile` is an Expo app. Metro is configured to watch the workspace,
  so the protocol package is compiled from source.
- The phone owns its id. It is made once and kept in the secure store, so the
  desktop knows the same phone even after it pairs again, and the device list
  does not fill up with copies of it.
- The desktop owns the size of the stream. It picks a size and a frame rate
  for every phone, the phone only says what it can show, and the list of
  sizes is the same on both sides.
- The interface shows the newest frame. A late frame is worth less than the
  one that replaced it, and a queue would turn a slow network into a growing
  lag. The decoder is the exception: it takes every frame, in the order they
  arrive, because a frame that depends on another one cannot be drawn when
  that other one was lost.
- The client acknowledges frames on receipt, before decoding, so the decoder
  never becomes the bottleneck of the server.

## The numbers the design is based on

Measured on the previous implementation, on a 5 GHz link:

- The old protocol waited for an acknowledgement before every frame, so the
  frame rate could not exceed one frame per round trip. At 70 ms that is 14
  frames per second. The old server delivered 7 to 14.
- The capture produced 20 to 27 frames per second, so the capture was never
  the bottleneck.
- A JPEG frame at 1408x792 and quality 50 is about 114 KB, so 30 frames per
  second is 27 Mbps.

From those numbers:

| Parameter | Value | Why |
| --- | --- | --- |
| Window | 8 frames, 2 MB | 30 fps at 70 ms needs 7 frames in flight |
| Acknowledgement | on receipt | the decoder must not throttle the sender |
| Codec | H.264, always | 5 to 20 times less bandwidth, hardware decode on the phone |
| Degradation order | fps, then quality, then resolution | text legibility is worth more than motion |
| Default | 1408x792, 20 fps, quality 70 | 35 Mbps, inside the usable band of a home WiFi |

The phone is the real limit at high resolutions: a software JPEG decode of
1920x1200 takes about 45 ms, so 22 frames per second is the ceiling at that
size. With a hardware decoder the limit moves to the network.

## The rate of the pictures

The compositor only paints the screen when something on it changes, so the
capture is at the rate of the changes and not at a fixed rate. The measured
delay between a change on the screen and the picture that is sent is about ten
milliseconds.

The encoder gets the last picture again when the screen does not change, so
the stream keeps the rate it was asked for. The pictures that repeat cost
almost nothing in H.264, so the phone always has a picture and the link is
quiet when the screen is quiet.

The repetition happens in the child that encodes, before the encoder, and
never after it: a picture that is already encoded cannot be sent twice,
because the decoder of the phone would apply the second one twice. The child
keeps an absolute schedule, so the distance between two pictures is the same
every time and the rate of the stream does not depend on how late the timer
woke up.

The capture cannot raise the rate on its own. The portal sends a stream with
a variable rate, and `videorate` without `drop-only` tries to catch up with
it, which takes minutes when the screen was quiet for a while. The `drop-only`
mode passes the pictures of the compositor as they are, and the child repeats
the last one.

## Milestones

1. Protocol, both implementations, golden vectors. Done.
2. Desktop server: handshake, frame window, synthetic source. Done.
3. Desktop app: status screen, pairing, settings, window buttons. Done.
4. Phone app: pairing by QR code, session, frame display, settings. Done.
5. Desktop server: XDG portal capture. Done. Input injection is next.
6. Phone: touch, mouse and keyboard input.
7. Codecs: H.264 with hardware encode and decode. Done.
8. Packaging: the desktop bundle and the APK.

## Known gaps

- The input messages are parsed and the capability is enforced, but the
  events are not injected into the desktop yet.
- There is no discovery. The address comes from the pairing code.
- The phone sends the input events it captures, but the touch surface that
  maps the finger to the desktop is not built yet.
