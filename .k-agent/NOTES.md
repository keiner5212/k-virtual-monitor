# Workspace notes

- Tauri command responses are camelCase: the response structs in `apps/desktop/src-tauri/src/lib.rs` carry `#[serde(rename_all = "camelCase")]`. `ServerConfig` stays snake_case because it is also the TOML file format.
- `pnpm format:check` fails on the baseline: there is no prettier config here, so prettier defaults (2 spaces, 80 columns) disagree with the code (4 spaces, 100 columns). Do not run `pnpm format`; match the surrounding style instead.
- The device store file `~/.local/state/kvm/devices.json` is the source of truth. The server reloads it at handshake time, so a pairing code created by `kvm-desktop --pair` is accepted by a server that is already running.
- The git remote is https but there are no credentials for it. Push with the SSH alias: `git push git@github-com-personal:keiner5212/k-virtual-monitor.git main`.
- Keep the lockfile in the same commit as the `package.json` that changes it, so every commit installs with `--locked`.
- All Flatpak apps are pinned to the Intel iGPU by `flatpak override --user`: `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/intel_icd.x86_64.json`, `__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json`, `MESA_LOADER_DRIVER_OVERRIDE=iris`. Without it they pick the RTX 3050 through the NVIDIA Vulkan ICD. Override only affects new processes, so restart the app after changing it.
- Expo SDK 58 consumes expo modules as prebuilt Android artifacts, so a patched native source is ignored. `apps/mobile/package.json` sets `expo.autolinking.buildFromSource: ["expo-camera"]` so the patch in `patches/` reaches the APK. Without it the phone camera keeps the stock code.
- The desktop capture finds the virtual screen by name, never by the `kscreen-doctor` output id: the ids shift when the layout changes, and the built-in screen of the user must never be resized.
- The portal permission is in `~/.local/state/kvm/portal.json` (restore token, owner only). With it the consent dialog only appears the first time.
- The protocol interop test (`packages/protocol/ts`) needs the synthetic source, so it only passes with no `WAYLAND_DISPLAY` or with `KVM_SYNTHETIC=1`. In a Wayland shell the server uses the portal capture and the test times out. CI has no Wayland session, so CI is fine.
