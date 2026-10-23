# Architecture

## The pieces

```text
phone (Expo, Android)          desktop (Tauri v2, Linux)
┌───────────────────┐          ┌────────────────────────────┐
│ SessionClient     │          │ server::Server             │
│  @kvm/protocol    │◀────────▶│  kvm-protocol crate        │
│                   │   TCP    │  device store (JSON)       │
│ LatestFrame       │  28252   │  FrameSource               │
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
  is produced. Today the source is a synthetic test pattern; the XDG portal
  capture replaces it without touching the protocol or the session logic.
- The input path is the mirror image: the server parses the input messages
  and hands them to an injector. Today the injector only logs.

## The phone app

- `apps/mobile` is an Expo app. Metro is configured to watch the workspace,
  so the protocol package is compiled from source.
- The client keeps only the newest frame. A late frame is worth less than
  the one that replaced it, and a queue would turn a slow network into a
  growing lag.
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
| Codec | H.264 first, JPEG as fallback | 5 to 20 times less bandwidth, hardware decode on the phone |
| Degradation order | fps, then quality, then resolution | text legibility is worth more than motion |
| Default | 1408x792, 20 fps, quality 70 | 35 Mbps, inside the usable band of a home WiFi |

The phone is the real limit at high resolutions: a software JPEG decode of
1920x1200 takes about 45 ms, so 22 frames per second is the ceiling at that
size. With a hardware decoder the limit moves to the network.

## Milestones

1. Protocol, both implementations, golden vectors. Done.
2. Desktop server: handshake, frame window, synthetic source. Done.
3. Desktop server: XDG portal capture and input injection.
4. Phone: pairing by QR code, session, frame display.
5. Phone: touch, mouse and keyboard input.
6. Codecs: H.264 with hardware encode and decode, JPEG as the fallback.
7. Packaging: the desktop bundle, the APK, and the release workflow.

## Known gaps

- The client renders frames as base64 data URIs. That copies every frame
  twice and will not hold 30 fps. The next step is a native view that takes
  the bytes without the round trip through the bridge.
- The synthetic source is a placeholder for the real capture.
- The input messages are parsed but not injected anywhere yet.
- There is no discovery. The address comes from the pairing code.
