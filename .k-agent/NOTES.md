# Workspace notes

- Tauri command responses are camelCase: the response structs in `apps/desktop/src-tauri/src/lib.rs` carry `#[serde(rename_all = "camelCase")]`. `ServerConfig` stays snake_case because it is also the TOML file format.
- `pnpm format:check` fails on the baseline: there is no prettier config here, so prettier defaults (2 spaces, 80 columns) disagree with the code (4 spaces, 100 columns). Do not run `pnpm format`; match the surrounding style instead.
- The device store file `~/.local/state/kvm/devices.json` is the source of truth. The server reloads it at handshake time, so a pairing code created by `kvm-desktop --pair` is accepted by a server that is already running.
- The git remote is https but there are no credentials for it. Push with the SSH alias: `git push git@github-com-personal:keiner5212/k-virtual-monitor.git main`.
- Keep the lockfile in the same commit as the `package.json` that changes it, so every commit installs with `--locked`.
