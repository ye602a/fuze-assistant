# Rust migration status - 2026-10-15

This is the current checklist. Earlier milestone documents are historical evidence, not a second list of current blockers. The stable application remains Electron/Vue. Rust is an isolated development preview and the repository remains private.

| Area | Implemented | Remaining acceptance or development |
| --- | --- | --- |
| Capture / OCR / three-card overlay | Native WGC host, persistent worker, exact title reuse, CPU/DirectML recovery, partial slots, public statistics and stale-result rejection | Popup/shard classifier, reroll retention, loading-phase prewarm; real Tencent sessions and target GPU timing |
| Client integration | Process-first discovery, command-line fallback, WebSocket/reconnect, phase/hero changes and automatic language | Real client reconnect and region-specific acceptance |
| Scheduling | M11 connects the actual local level endpoint; expiry, cancellation generations and ordinary fallback | Gameplay timing; levels never prove actual claims |
| Main UI / tray / settings | Three languages, pause/exit, tray, safe defaults, legacy import, directory fallback, OCR/backend and module status | Final player interaction review and optional legacy controls |
| Champion view | Five-player team, ranked available heroes, details, items and bounded public icon cache | Complete rune-page source and optional write actions are absent |
| Window continuity | Main, champion and augment geometry; monitor clamping and health handshake | Physical multi-monitor/DPI/fullscreen acceptance |
| Data / cleanup | Local-first locale isolation, explicit pause-only transactional update, bounded retirement preserving current/previous/unknown files | Background refresh parity and long-term usage; interrupted staging is preserved |
| History / records | Account isolation, cached failure fallback, match detail/icons, selective old record import, explicit reduced JSON / local PNG export | External SGP with unambiguous region; player acceptance; observation is not a claim |
| Updates | Private credentials, differential reconstruction/full fallback, managed version switch and rollback | Publisher signing and production channel policy |
| Local installation | M15 per-user Windows Apps registration and owned maintenance refresh; M14 shortcuts / conservative uninstall; M12 offline installation | Signed installer/bootstrap, runtime prerequisites and end-to-end player acceptance |
| Stage winrate | M13 native manual reference: All/1-4, public source/patch/sample labels, isolated cache and explicit refresh; automatic ranking stays overall | Provider/live interaction acceptance and reliable actual-claim source for automatic ranking; no inference from levels/shards/forges/rerolls |
| Public release | Private branch/PR workflow, sanitized CI artifacts | Full history, Actions, release-asset and redistribution-license audit after migration |
| Secondary augment panel | M14 explicit tray entry, active supported champion, local overall rates and bounded icon cache; hidden on pause/disconnect | Actual gameplay/fullscreen acceptance; no inferred acquisition stage |

## Latest changes

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
