# Project rules

## Builds and temporary output

- Every build, test run, or tool that writes generated or temporary files writes inside the workspace at `./.tmp`.
- Never write to `/tmp` or any other system temp dir. Point `TMPDIR`, `CARGO_TARGET_DIR`, `--out-dir`, and cache paths at `./.tmp`.
- Cargo already resolves this in `.cargo/config.toml` (`[build] target-dir = ".tmp/target"`). Reuse it, do not add a second target dir.
- The Rust tests use the workspace scratch dir (`kvm_desktop_lib::scratch_dir`, or `KVM_TMP`).
- `.tmp` is gitignored. Never commit its contents.

## Commits

- Commit recurrently while the work progresses, one logical change per commit, with a message that explains the change.
- Never push. The user pushes.

## CI

- Keep `.github/workflows/ci.yml` green. Run the local equivalent before committing:

  - `node scripts/changelog.mjs` (needs a `CHANGELOG.md` section for the current version)
  - `pnpm -r typecheck`
  - `pnpm -r lint`
  - `pnpm -r test`
  - `pnpm --filter @kvm/desktop build`
  - `pnpm audit --audit-level high`
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `pnpm --filter @kvm/protocol test`
  - `cargo audit`

- Fix the cause of a failing check in the same change. Never weaken, skip, or ignore a check to make it pass.
