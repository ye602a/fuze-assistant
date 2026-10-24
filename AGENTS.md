# FUZE repository guide

FUZE is a Rust application. Read `docs/CODEX_HANDOFF.md` for release identity and
`docs/development/README.md` for build entry points. Preserve unrelated local changes.

- `rust/crates/`: services, capture, OCR, native overlay, update and rollback.
- `rust/ui-lab/`: production Slint/Skia interface; the directory name is historical.
- `scripts/`: active data, build, packaging and offline diagnostic tools.
- `scripts/prepare-rust-data.mjs`: build-time data preparation through the Rust downloader.
- `public/`, `resources/`, `tests/fixtures/`: assets and fixtures still used by Rust.
- `tests/unit/`: focused native-tooling tests (`npm test`).
- `legacy/electron/`: archived v0.6.3 source, tests and build configuration.

Root `package.json` describes development tools, not the application version or runtime.
Use `npm ci --ignore-scripts` for those tools. Rust builds use the existing Cargo
workspaces and `scripts/build-rust-with-evidence.ps1`; do not launch the desktop or
package/release merely to validate a documentation or repository-layout change.
Choose focused checks for the affected code. Keep existing update formats, version
identity, user data and release channels compatible. Do not infer actual augment
claim stages from levels or disappearance. Do not access the game or personal logs
without the user's authorization. Never commit credentials or private runtime data.

The archived Electron guide applies only to that historical code. Reproducing an
old release uses its original tag, not the current native packaging pipeline.
