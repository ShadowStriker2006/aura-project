# AURA-PROJECT

Ultra-light League of Legends companion and headless Spotify controller built
with Tauri v2, Rust, and vanilla HTML/CSS/JS.

Aura was created under Riot Games' "Legal Jibber Jabber" policy using assets
owned by Riot Games. Riot Games does not endorse or sponsor this project.

## Architecture

- Native Tauri webviews; no Electron or frontend framework.
- League Client integration through local LCU REST and WebSocket APIs.
- Typed live-client events for game state, one-second player HUD ticks,
  five-second inventory updates, and Champion Select changes.
- A polished dark Match Command Center and adaptive edge overlay show current
  gold, KDA, level, merged CS/CS-min, kill participation, objective timers,
  and a clearly labeled held-value/minute estimate from local game data. The
  overlay has 32px standby, compact ribbon, and expanded telemetry modes.
- Metric provenance stays visible: exact DPM, exact team-gold differential,
  earned GPM, and XP progress remain `Unavailable` because Riot's local live
  endpoint does not expose the required source data.
- Spotify Authorization Code with PKCE and RAM-only tokens, plus an optional
  isolated Aura Player using Spotify's official Web Playback SDK.
- Data Dragon metadata cached only in RAM.
- Aura Intelligence draft, live, and post-game recommendations with explicit
  data provenance, deterministic local fallback, and an optional aggregate
  HTTPS feed cached only in RAM.
- Riot API keys stored in Windows Credential Manager; values never enter the webview.
- Automatic signed-in Riot identity discovery from the local League Client;
  the selected public-API identity is cached only for the current process lifetime.
- Functional page navigation, all-champion search/details, automatic
  champion-based builds, a League-valid primary/secondary/stat-shard rune
  editor, enemy-team build adaptation, Spotify Connect device selection, and
  manual overlay controls.
- Expandable OP.GG-style recent-match reports with KDA, CSM, kill participation,
  damage, gold, vision, wards, items, runes, summoner spells, ten-player
  scoreboards, bans, and team objectives.
- Dynamic Map Control Replay for Summoner's Rift and Howling Abyss post-match
  reports: bundled official terrain, interpolated champion movement, separate
  two-dimensional map-control and one-dimensional ARAM lane-control models,
  synchronized kills/gold/turrets/objective banners, and scrub-enabled graphs.
  Unsupported maps retain movement/stats/events without fabricated pressure. See
  [DYNAMIC-MAP-CONTROL-REPLAY.md](DYNAMIC-MAP-CONTROL-REPLAY.md).
- Player names and champion portraits in a match scoreboard open that player's
  profile; **My League Profile** restores the locally detected account.
- Official Champion-Mastery-V4 data appears on Profile, Home, and each champion
  detail card. Official Solo/Flex records include LP, win rate, and Riot's
  available hot-streak, veteran, new-entrant, and mini-series status fields.
- Level/icon, rank, mastery, and match-history requests are independently
  guarded, so one Riot endpoint failure cannot erase successful data from the
  others and a late response from an old player cannot overwrite the currently
  selected account.
- Ten/twenty-game history selection plus queue and result filters.
- Windows self-priority reduction and one working-set trim when League starts.

## Secure configuration

1. Revoke or replace the Riot development key previously posted in chat.
2. Save the fresh key from Aura's Settings page; Windows Credential Manager
   protects it. `RIOT_API_KEY` remains an optional managed override.
3. Register `http://127.0.0.1:8888/callback` exactly in the Spotify dashboard.
4. Reconnect Spotify once in Aura 0.8 or later to grant `streaming`, `user-read-email`,
   and `user-read-private` for Aura Player. Do not add a Spotify client secret;
   desktop PKCE does not use one. Spotify Premium is required for playback.
5. Optional aggregate advisor data is configured only through
   `AURA_META_API_URL` and `AURA_META_API_TOKEN`; the token never enters the
   webview or logs.
6. Before any public release, register the product with Riot and complete the
   required review. Aggregate Match-V5 collection belongs on a separate approved
   server with a production key; never place that key in the desktop binary.

The provided Spotify Client ID is configured as a backend default. It can be
overridden with `SPOTIFY_CLIENT_ID`; the loopback redirect and scopes can be
overridden with `SPOTIFY_REDIRECT_URI` and `SPOTIFY_SCOPES`.

Aura's local League integration reads only `/lol-summoner/v1/current-summoner`,
`/lol-chat/v1/me`, `/riotclient/region-locale`, gameflow, and Champion Select
from the signed-in client. Public profile, rank, and match data still use Riot's
official web APIs. LCU is unsupported by Riot and may change, so these endpoints
must be disclosed when registering the product.

See [CONFIGURATION.md](CONFIGURATION.md) for complete Windows setup commands,
defaults, validation rules, and the development-key expiration warning.

## Publish and build

Players do **not** receive a command file or need Rust. A public release consists
of the NSIS file named `Aura_<version>_x64-setup.exe`.

The included GitHub Actions workflow validates, caches, builds, optionally
Authenticode-signs, hashes, and publishes that installer whenever a matching
`v<version>` tag is pushed. See [PUBLISHING.md](PUBLISHING.md) for the one-time
repository, signing, Riot production-access, and release setup.

Install stable Rust, Microsoft C++ Build Tools, and WebView2, then double-click
`BUILD-WINDOWS.bat` only when making a local publisher build. It reuses Cargo's
incremental cache and creates the portable executable, NSIS installer, and
checksums in `dist/release/<version>`. An input fingerprint safely skips the
Tauri relink/bundle step when source, configuration, dependencies, icons, Rust,
and the Tauri CLI are unchanged; the most recent validated cached publisher
path is recorded in the versioned test report.

These local builds are not Authenticode-signed. A signed public release requires
a Windows code-signing certificate and a protected release signing workflow.

Developer commands:

```powershell
cargo check --manifest-path .\src-tauri\Cargo.toml
cargo test --manifest-path .\src-tauri\Cargo.toml
node --test .\tests\*.test.mjs
node .\scripts\BENCH-LIVE-PIPELINE.mjs
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\VERIFY-MAP-ASSETS.ps1
cargo tauri dev
```

The live-pipeline benchmark is synthetic and frontend-only. It compares payload
normalization and view-model construction between revisions; it does not prove
League input latency, WebView frame pacing, or RAM usage on player hardware.

## Performance rules

- No SQLite, IndexedDB, local JSON cache, or match-time file logging.
- Runtime credentials and OAuth tokens remain in volatile backend memory.
- External metadata uses bounded timeouts and small connection pools.
- UI updates are event-driven where possible.
- One native supervisor owns port-2999 telemetry. It issues the independent
  gamestats, active-player, and player-list requests concurrently once per
  second, refreshes events/player updates every five seconds, and never requests
  the same endpoint twice in one tick. Local response bodies are capped at
  2 MiB before deserialization.
- Active matches do not incur one process-table scan per second. Idle discovery
  uses a low-frequency scan, while three consecutive gamestats failures guard
  match-end detection.
- Live renderers compare text, classes, attributes, and progress state before
  touching the DOM, and both the dashboard and overlay tear down their native
  event listeners on unload.
- Match timelines are fetched only for an expanded report, normalized to a
  compact DTO, and cached for at most three matches in RAM.
- The overlay is transparent and locked/click-through by default. Its WebView
  is created only while visible and released again when hidden. The native
  window follows the active mode: 32x32 standby, 432x52 compact, or 520x150
  expanded at 100% scale, with 75%, 90%, and 100% density presets. Opacity is
  session-only and uses a flat alpha surface without a continuous blur filter.
- A locked HUD cannot receive its own unlock click. Unlock it from Aura's
  Overlay Controls page or the native tray menu, make changes, then use
  **Lock HUD**. Hiding or ending the match re-arms click-through before the next
  automatic show. Aura does not advertise an overlay hotkey because no global
  shortcut is registered in this build.
- Aura Player is opt-in and creates its isolated WebView and loopback server
  only while running. The main dashboard never receives its Spotify bearer token.
  Aura delegates encrypted media only because Spotify's protected SDK frame
  requires it; the Content Security Policy still restricts frames and network
  access to Spotify. A fresh-session browser player is offered only if the
  embedded runtime reports a genuine initialization failure.

See [LIVE-METRICS-AND-OVERLAY.md](LIVE-METRICS-AND-OVERLAY.md) for the complete
wire contract, formulas, unavailable-data boundary, overlay lifecycle, and
real-match performance acceptance checklist. The polling design minimizes
wakeups and duplicate work, but Aura does not claim universal zero input lag,
zero frame drops, or a fixed RAM result without measurements on the target PC.
