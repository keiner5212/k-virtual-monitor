# k-virtual-monitor

Turn a desktop into a virtual monitor you can see and control from a phone.

The desktop app captures the screen, encodes it and streams it over the local
network. The phone app receives the stream, decodes it and sends back touch,
mouse and keyboard input. Both ends are our own software, so the wire
protocol, the reconnect behaviour and the latency are under our control.

This is a clean room implementation. It does not speak the spacedesk wire
format and it does not reuse any spacedesk code or assets.

## Layout

```text
apps/
  desktop/         Tauri v2 app: the server. Rust backend, React UI.
  mobile/          Expo app: the phone client. Android first.
packages/
  protocol/        The wire protocol, implemented in Rust and TypeScript.
docs/              Architecture, protocol, security, decisions.
```

The protocol is the contract between the two apps. It lives in
`packages/protocol` and exists twice, in Rust and in TypeScript, checked
against the same golden vectors so the two ends cannot drift apart.

## Phone app (Android)

The phone client is an Expo app.

Download: **TODO(keiner): add the link to the APK here once the release build
is generated with Expo.**

The app is also published from this repository:

- `apps/mobile` holds the source.
- A release build is produced with `eas build` and published as an APK.

## Desktop app

The desktop app is a Tauri v2 app. The server part is a plain Rust library, so
it can run headless and be tested without a webview.

```bash
pnpm install
pnpm dev:desktop          # run the app in development
pnpm build:desktop        # build the bundle
```

The headless server, useful for a machine without a desktop session:

```bash
cargo run -p kvm-desktop -- --pair       # print a pairing code
cargo run -p kvm-desktop -- --headless   # serve without a window
```

Every push to `main` builds the Linux bundle and leaves it in a draft
release. The workflow is `.github/workflows/release.yml`.

## Protocol

Read `docs/protocol-v1.md` for the wire format, the handshake and the flow
control rules, and `docs/flows.md` for the diagrams. The short version:

- TCP, one session per connection.
- Pairing once per phone, by QR code, with a six digit safety code both sides
  show so the user can confirm the pairing.
- Every message after the handshake is sealed with ChaCha20-Poly1305, with
  the header as associated data. The tag is verified before the payload is
  parsed.
- A window of frames in flight, so the frame rate is not bounded by the round
  trip time. This is the difference that made the previous server stop at
  12 fps on a 70 ms link.
- The codec is negotiated: the phone asks for what it can decode, the server
  answers with the most compressed codec both ends support.

## Development

```bash
pnpm test             # TypeScript tests
pnpm lint             # eslint
pnpm typecheck        # tsc
cargo test            # Rust tests
cargo clippy --workspace --all-targets -- -D warnings
```

The two implementations of the protocol are checked against
`packages/protocol/testdata/vectors.json`, and the interoperability test
starts the real server and connects to it with the real client. Regenerate
the vectors with:

```bash
cd packages/protocol/rust && cargo run --example gen_vectors > ../testdata/vectors.json
```

## License

MIT. See `LICENSE`.
