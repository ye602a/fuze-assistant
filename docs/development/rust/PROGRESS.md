# Rust migration status - 2026-10-16

Current explicit differences: [preview vs complete migration](../../RUST_PREVIEW_DIFFERENCES.md), updated 2026-10-17. Chinese-only scope and persistent logs supersede older open-item lists. The exact [v0.6.3 parity audit](V063_PARITY.md) governs completion; full parity is NOT accepted. This is the current checklist. Earlier milestone documents are historical evidence, not a second list of current blockers. The stable application remains Electron/Vue. Rust is an isolated development preview and the repository remains private.

| Area | Implemented | Remaining acceptance or development |
| --- | --- | --- |
| Capture / OCR / three-card overlay | Native WGC host, persistent worker, exact title reuse, CPU/DirectML recovery, narrow three-title shard classification, bounded reroll retention, loading prewarm, partial slots, public statistics and stale-result rejection | Full popup geometry/absence proof, real shard/forge fixtures, Tencent sessions and target GPU timing |
| Client integration | Process-first discovery, command-line fallback, WebSocket/reconnect, phase/hero changes and automatic language | Real client reconnect and region-specific acceptance |
| Scheduling | M11 connects the actual local level endpoint; expiry, cancellation generations and ordinary fallback | Gameplay timing; levels never prove actual claims |
| Main UI / tray / settings | Restored Chinese sidebar/status/four-module/three-card structure; pause/exit, tray, safe defaults, legacy import, directory fallback, OCR/backend and module status; Simplified Chinese only (multilingual UI is explicitly out of scope) | Individual control audit: see ../../RUST_CONTROL_ACCEPTANCE.md. OS dialogs and actual client actions remain unverified; missing legacy functions are not automatically removed. |
| Champion view | Restored vertical team/bench/hero/statistics/two-tab popup; details, items and bounded public icon cache, scrolled hit coordinates and compact layout | Original rarity filters, complete ranked augment pool, descriptions, guide links and route selection are now implemented; explicit item-set transactions and skill grids are now connected; final old/new interaction acceptance remains open |
| Window continuity | Main, champion and augment geometry; monitor clamping and health handshake | Physical multi-monitor/DPI/fullscreen acceptance |
| Data / cleanup | Local-first locale isolation, explicit pause-only transactional update, bounded retirement preserving current/previous/unknown files | Idle background refresh with cancellation/version comparison implemented; manual classified storage preview/cleanup is implemented; marked stale transaction cleanup implemented while retaining recovery backups; long-term usage remains open |
| History / records | Account isolation, cached failure fallback, match detail/icons, selective old record import, explicit reduced JSON / local PNG export | Fixed-route SGP, cached lists/filter/paging, anonymous clipboard, post-game sync and verified runtime observation bindings implemented; timestamped observation timelines and recognition summaries implemented; player acceptance remains open |
| Updates | Private credentials, differential reconstruction/full fallback, managed version switch and rollback | Publisher signing and production channel policy |
| Local installation | M15 per-user Windows Apps registration and owned maintenance refresh; M14 shortcuts / conservative uninstall; M12 offline installation | Signed installer/bootstrap, runtime prerequisites and end-to-end player acceptance |
| Stage winrate | M13 native manual reference: All/1-4, public source/patch/sample labels, isolated cache and explicit refresh; automatic ranking stays overall | Provider/live interaction acceptance and reliable actual-claim source for automatic ranking; no inference from levels/shards/forges/rerolls |
| Public release | Private branch/PR workflow, sanitized CI artifacts | Full history, Actions, release-asset and redistribution-license audit after migration |
| Secondary augment panel | M14 explicit tray entry, active supported champion, local overall rates and bounded icon cache; hidden on pause/disconnect | Actual gameplay/fullscreen acceptance; no inferred acquisition stage |

## Latest changes

- History display parity and redundant-entry cleanup: Windows local-time conversion, ten rows per page, removed unused UI-language binding, and consistent Slint packaging default. Retained active legacy compatibility and CI probes; no new package or live-game acceptance. See the current difference report for the remaining release boundaries.

- Exact 0.6.3 comparison: full augment lists and authoritative ranking, rarity/description/guide controls, shared side-panel rendering and automatic side-panel preference, viewport-prioritized artwork and idle data refresh. See the parity audit for mandatory remaining features.

- [Chinese workbench restoration](../../../rust/docs/WORKBENCH_RESTORATION.md) and [champion / in-game popup restoration](../../../rust/docs/POPUP_RESTORATION.md): actual hidden Win32 renders compared with retained Vue components, existing public artwork, bounded nonblocking three-card image delivery. PR #46's CI-only WGC failure is not declared resolved by local visual checks.

- [M17: title classification, reroll retention and loading prewarm](M17_SELECTION_PREWARM.md)

- [M16: complete flow audit against Electron and scheduling / inference fixes](M16_FLOW_AUDIT.md)

- [M15: Windows Apps registration, share icons and test-package readiness](M15.md)

- [M14: shortcuts, uninstall, secondary reference and local image export](M14.md)
- [M13: native manual stage reference](M13.md)
- [M12: independent local installation](M12.md)
- [M11: live level scheduling, native preferences and reduced export](M11.md)
- [M10: runtime continuity, health and bounded cleanup](M10.md)
- [M9: match details and selective observations](M9.md)
- [M8: account history](M8.md)
- [M7c: desktop continuity](M7C.md)

PRs remain stacked in dependency order. CI/self-owned-window checks establish isolated behavior, not live Tencent success. No production installer or stable-entry replacement is claimed by this checklist.
