# k-virtual-monitor, phone app

The Android client. It pairs with a desktop running the k-virtual-monitor
server, receives the screen and sends back touch, mouse and keyboard input.

Built with Expo. The source lives in this folder, the protocol client is in
`src/lib`, and the shared wire protocol is the `@kvm/protocol` package at
`packages/protocol/ts`.

## Download

Download: **TODO(keiner): add the link to the APK here once the release build
is generated with Expo.**

The APK is produced with Expo Application Services:

```bash
cd apps/mobile
pnpm exec eas build --platform android --profile preview
```

The `preview` profile builds an APK that can be installed by hand. The
`production` profile builds an app bundle for the store.

## Running from the sources

```bash
pnpm install
cd apps/mobile
pnpm start          # Expo dev server, scan the code with Expo Go
pnpm android        # build and run on a connected device
```

An Android device on the same network as the desktop is required. The phone
and the desktop have to be able to reach each other, and the desktop has to
allow the connection through the firewall.
