# FUZE

FUZE is a Chinese-language Windows companion for ARAM Mayhem. The maintained player documentation is in [简体中文](README.md).

- [Download FUZE v0.7.0 for Windows x64](https://github.com/ye602a/fuze-assistant/releases/tag/rust-v0.7.0)
- [Player guide (Chinese)](docs/PLAYER_GUIDE.md)
- [Acknowledgements](THANKS.md)

Built with Rust, with low memory use and CPU overhead as development priorities. Public downloads do not require an access token. The portable release is currently unsigned; see the release page for runtime requirements and validation limits. Existing Rust installations continue to use the compatible `rust-preview` update feed.

Download `FUZE-Setup-0.7.0-windows-x64.exe` from the release page. This unsigned installer includes the required app-local VC++ DLLs. The original portable ZIP is also retained.

## Repository layout

The current application lives in `rust/crates/` and `rust/ui-lab/`. Root Node dependencies are development tools only. `scripts/`, `public/`, `resources/` and `tests/fixtures/` retain native build inputs. Run tooling checks with `npm test`. The former Electron application, its tests and build configuration are preserved in [legacy/electron](legacy/electron/README.md); they are not part of the current runtime.
