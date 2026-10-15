# M16 — Complete flow audit against Electron 0.6.3

Baseline: PR #43 head `56eb125e5995aad71bf24e6e00ab1a6af480f88d` (Windows CI successful). The comparison uses the maintained Electron source (`package.json` 0.6.3), not an assumed upstream ARAMGG version or the player's installed process. No stable installation, game process, mouse, account, credentials or private log is accessed by this audit.

## Changes from this audit

1. **Unknown OCR incorrectly enabled fast capture.** The pipeline previously passed `Unknown` for every result and the level gate treated it as a visible selection. Empty images could thus turn the 8-second saving / 1.5-second fallback schedule into 250-ms work. Unknown frames now cannot open that window. Matched titles supply a separate `Candidate` signal which cannot consume a formal reward window. An unknown run gets a six-second grace period after candidates, then returns to the level/fallback schedule. Pending levels still intentionally remain conservative; there is no popup-type proof.
2. **Identical title pixels repeated inference.** A worker now keeps one bounded title atlas and its three-slot result. Exact byte equality within the same generation and geometry reuses the result under the new request identity. A content, size or generation change requires inference. No hash, encoding, disk image or network operation is introduced. The cache moves the already-owned request buffer rather than copying it again; maximum retention is the wire atlas limit (about 4 MiB) and ends with the worker. `ocr_ms=0` means reused inference, not zero end-to-end latency.
3. **Control-loop wakeups ignored the scheduler.** Waiting now follows the next gate/capture deadline, bounded to one second so phase/level changes remain observable. Capture suspension/recovery deadlines participate in this calculation. This reduces idle wakeups rather than claiming a measured game FPS improvement.
4. **Successful jobs did not reset the session failure streak.** Sporadic failures could accumulate into a stopped session despite successful work between them. A valid accepted result now clears the streak; three consecutive failures still stop the session and stale results never reset it.

## Source-to-flow comparison

| Flow | Maintained Electron | Rust at this milestone | Evidence / remaining boundary |
| --- | --- | --- | --- |
| Install and startup | NSIS installer / Electron entry | Separate user installation, shortcuts, Apps registration, paused native desktop | Temporary-install lifecycle and native readiness; unsigned preview requires x64 VC++ runtime. Executable/receipt replacement is not one transaction. |
| Discovery / connection | Process-first with several path/auth fallbacks | Process metadata, adjacent lockfile, PID-scoped command line, manual directory fallback | Parser, loopback WebSocket and reconnect tests. Rust does not migrate every historical client-log discovery fallback. No live Tencent verification here. |
| Phase and hero changes | Game-session service | Monitor publishes reduced context, generation cancellation, WS events plus refresh | Synthetic phase/hero/game changes reject late windows. Game process presence alone is never connection proof. |
| Capture | Electron capture service and raw-frame path with fallback | WGC stream, GPU ROI readback, raw title atlas to worker | Generated GPU input locally; self-owned-window WGC in Windows CI. No game screenshot. |
| OCR / backend | PaddleOCR Node, GPU path and CPU fallback | Persistent Rust worker, CPU / DirectML, bounded one-way fallback | Committed fixtures plus crash/timeout/reap tests; actual target-GPU advantage is not established by CI. |
| Warmup | Existing service initialization / preparation | Worker warms before Ready, but production construction begins in InProgress | Loading-phase prewarm remains incomplete; first-match startup cost is not an OCR-per-card benchmark. |
| Duplicate work | Title cache, staged gate, notification dedup | Exact title reuse added here; sequence/stale rejection already present | Repeated fixture requests must reuse results; changed pixels/context must miss. Raw capture/IPC still occur. |
| Selection type / shards / forges | Geometry/gate and narrow shard-title classifier | Candidate versus unknown, no complete popup/shard classifier | Not equivalent. No formal-stage inference, and pending-level probes remain conservative. |
| Partial cards / refresh | Position preservation and transient retention | Fixed left/center/right, unreadable placeholders, immediate new snapshot | Partial delivery tested. Equivalent reroll-animation retention is not yet implemented. |
| Recommendation | Local versioned overall statistics; manual stage reference | Same overall scope and independent manual stage reference | Source/locale/version/sample handling is checked. Actual acquisition remains unknown. |
| Champion selection | Five teammates + ranked available heroes, details | Native read-only overview, topmost option, details/items/icon cache | Three-locale synthetic UI and context change tests. Full rune-page data / write actions remain absent. |
| Data updates | Local-first with background refresh | Local-first, explicit pause-only refresh; validate then activate | Corrupt/incomplete/incorrect-locale updates preserve prior data. Background refresh behavior is not yet equivalent. |
| History / records | Online history + cache and observation journal | Manual account/region-scoped history, selective import, anonymous local observations, JSON/PNG export | Synthetic account switching/failure/cache tests. Live provider access is unverified; unknown region is not guessed. |
| Pause / exit / next game | Stop background pipeline and clear prior results | Cancel generation, hide window, release capture/worker, bounded records | Worker crashes, stale results, pause/exit and game changes covered. No long multi-match endurance claim. |
| App updates | Electron full/differential update channel | Separate Rust archive/blockmap feed, credential store, full fallback and managed rollback | Reconstruction/HTTP fixtures and real isolated child installation/start failure. No production release feed or credential download was exercised. |
| Locale / window placement | Vue three locales and persisted placement | Native three locales and monitor-clamped placement | Synthetic renders and geometry tests; physical monitors/DPI/fullscreen remain player acceptance. |

## Verification and delivery

- Electron baseline: 89 test files / 480 tests passed, lint, type checks and production build passed. These are source checks, not installed Electron gameplay validation.
- Rust: one milestone-wide workspace test/build, worker/fixture integration, same-input OCR comparison and final Windows CI. Follow-up testing is focused on discovered changes; final CI results are recorded on the PR.
- Local test executables use isolated data, fixed synthetic inputs and hidden self-owned windows. Visible WGC fixtures are restricted to GitHub's runner. No game discovery is invoked by the harness.
- No changes to stable Electron sources, runtime settings or release channel. No repository visibility change. Private data is not included in diagnostic reports or uploaded artifacts.

The main native chain has an executable isolated acceptance path. It is not yet a fully equivalent replacement: selection-type/refresh behavior, loading-phase prewarm, background data refresh and real-game acceptance are the most consequential remaining gaps. Keep stable Electron available while resolving these before calling the Rust package complete.
