# AURA-PROJECT changes

## Adaptive edge HUD overlay - 0.14.0

- Replaced the 360x540 idle block with a 32x32 standby pill, a 432x52 compact
  match ribbon, and a 520x150 expanded telemetry panel at 100% scale.
- Kept the compact row focused on clock, KDA, the honest team-gold state, and
  Dragon/Baron timers. Combined CS and CS/min into one expanded value.
- Added synchronized native/WebView scale presets at 75%, 90%, and 100% so
  lower-density modes reduce the real always-on-top window footprint instead
  of leaving a large transparent hit/compositor area.
- Added a 40-100% flat-alpha opacity control with a 55% default. Inner cells no
  longer stack the selected alpha, and the gameplay HUD uses no backdrop blur.
- Added a volatile Rust overlay-layout state, typed `overlay:layout-changed`
  event, `get_overlay_layout`, `set_overlay_layout`, and
  `toggle_overlay_interaction` commands.
- Made locked click-through/non-focusable mode the safe default. The dashboard
  and native tray provide the unlock escape hatch; the overlay can be edited
  only after unlock, and hide/match end always re-arm the lock before the next
  automatic show.
- Rejected interaction toggles for absent or hidden overlays and routed tray
  re-show through the full native show path so size, topmost state, lock, and
  layout events are reapplied.
- Kept layout settings in RAM only and preserved lazy WebView teardown. No
  global shortcut is claimed in this release.
- Preserved the metric-integrity boundary: exact team gold, earned GPM, DPM,
  and XP remain unavailable instead of being fabricated for the new layout.

## Live performance telemetry and compact overlay - 0.13.0

- Extended the typed `live:game-tick` contract with current level, CS, CS/min,
  kill participation, observable held value, observable held value/minute,
  exact-earned-GPM compatibility state, XP-progress compatibility state, and
  per-field availability/source metadata.
- Added exact local calculations for CS/min (`CS * 60 / game time`) and kill
  participation (`(kills + assists) / team kills * 100`) whenever every
  required Live Client Data field is present. KP remains unavailable while the
  team has zero kills or its source snapshot is incomplete.
- Added a separately named, explicitly estimated held-value rate from current
  cash plus the listed value of items still in inventory. It is never labeled
  as earned GPM, and disappears when an item price/count or another required
  source value is missing.
- Kept unsupported metrics honest. Exact DPM, exact team-gold differential,
  lifetime earned GPM, and within-level XP progress remain unavailable rather
  than being synthesized from unrelated stats, item value, or level.
- Modernized the Live Game page into a responsive dark Match Command Center
  with a clear player hierarchy, source-aware telemetry cards, objective
  timers, and an unavailable XP state instead of a fabricated progress bar.
- Rebuilt the live overlay as a compact, flat, read-only HUD using the same IPC
  normalizer and view-model rules as the main dashboard. It shows level, KDA,
  CS, CS/min, KP, held-value rate provenance, unsupported-metric states, and
  Dragon/Baron timers in a 360x540 logical-pixel click-through window.
- Preserved lazy overlay creation and teardown: the secondary WebView exists
  only while the overlay is visible, and every native event listener is removed
  when its owning webview unloads.
- Kept one native telemetry supervisor. Gamestats, active-player, and
  player-list requests run concurrently at one-second cadence; event refreshes
  and player updates run every five seconds; active matches skip Windows
  process-table scans; local responses are capped at 2 MiB; and no endpoint is
  fetched twice in a single tick.
- Removed the obsolete `aura-objective-timers` compatibility event. Home,
  dashboard, overlay, and advisor state now share the objective fields from the
  single normalized `live:game-tick` path.
- Sized the non-focusable overlay window to 360x540 logical pixels so all ten
  metric cells and both objective rows remain visible, and set a 760x600
  desktop minimum for the resizable main window.
- Reduced unchanged-frame frontend work by comparing text, attributes, classes,
  progress values, and hidden state before mutating the DOM.
- Added `scripts/BENCH-LIVE-PIPELINE.mjs` for repeatable synthetic frontend
  transform comparisons and `LIVE-METRICS-AND-OVERLAY.md` for metric provenance,
  overlay constraints, and real-match acceptance. Synthetic throughput is not
  presented as proof of zero input lag, zero frame drops, or a RAM target.

## Typed live-client IPC and real-time HUD - 0.12.0

- Added a canonical `src/types/ipc.ts` contract for game status, live ticks,
  objective countdowns, KDA/current gold, and medium-frequency player item
  updates without adding React or a frontend build tool.
- Added a reusable frontend IPC service that validates dynamic command names,
  applies one timeout implementation, normalizes native event payloads, and
  returns an idempotent listener cleanup function.
- Routed advisor, Spotify, overlay, and direct command calls through that
  wrapper while retaining the static command-contract test.
- Added typed Rust payloads and emitters for `game:state-changed`,
  `live:game-tick`, `live:player-update`, and `draft:update`.
- Reused one Live Client Data supervisor instead of creating a second polling
  loop. HUD ticks emit once per second, objective events refresh every five
  seconds, and player inventory updates emit every five seconds.
- Added tolerant current Riot-ID decoding when the local API returns both the
  legacy `summonerName` and current `riotId` fields, avoiding duplicate-field
  decode failures.
- Deduplicated repeated game-state events and require three consecutive
  gamestats failures before ending a match, preventing transient local endpoint
  errors from immediately hiding the overlay.
- Added the Live Game HUD for game time, summoner, champion, KDA, current gold,
  DPM, team gold differential, and Dragon/Baron countdowns with responsive
  desktop/narrow layouts.
- Kept live data honest: Riot's official local endpoint does not expose total
  champion damage or exact team total gold, so availability flags make the UI
  show `Unavailable` instead of presenting fabricated zeroes or item-value
  estimates as measured DPM/gold lead.
- Added direct-source Node tests for payload normalization, command wrappers,
  listener cleanup, safe DOM rendering, and metric availability. Rust tests
  cover wire serialization, phase mapping, dual-name response decoding,
  objective timers, and player updates.
- Added `aura_project_structure.txt` documenting the implemented vanilla
  architecture, channel frequencies, failure isolation, and manifest-owned map
  asset flow, and bumped the application/installer feature version to 0.12.0.

## Structural integrity and Riot failure isolation - 0.11.1

- Restored `get_league_entries` as a dedicated League-V4 command and removed
  rank fetching from `get_summoner_profile`. Level/icon, rank, mastery, and
  recent matches now load independently, so one endpoint timeout, rate limit,
  or decode failure no longer blanks data returned by the others.
- Added a two-minute, identity-scoped, volatile rank cache with in-flight
  request deduplication and invalidation when the Riot key or selected account
  changes. This avoids duplicating rate-limited League-V4 calls.
- Removed the orphaned RAM-only Riot preference store. Its save command wrote
  data that no caller ever loaded, while local League account discovery and
  active frontend state already own the current selection.
- Added a static IPC contract test covering both direct and reviewed dynamic
  advisor, Spotify, and overlay dispatch. Registered handlers and frontend
  calls must now match in CI.
- Replaced hand-maintained minimap filename construction with generated runtime
  metadata derived from a canonical manifest. A shared verifier checks exact
  filenames, sizes, SHA-256 digests, PNG signatures, asset sets, generated
  metadata, and documentation before local cached builds and release CI.
- Added an explicit terrain decode/load failure state. Replay movement and
  pressure remain usable on the neutral coordinate grid instead of silently
  claiming that bundled terrain loaded.
- Kept bundled minimaps intentionally pinned and offline. The live Data Dragon
  catalog version is advisory only and can never be used to construct a local
  asset path that does not exist.
- Added rank-failure browser fixtures and map/IPC regression tests, and bumped
  the application and installer patch version to 0.11.1.

## Multi-map Dynamic Map Control Replay - 0.11.0

- Split positional replay availability from territorial-estimate availability.
  Any map with usable Riot Timeline positions now keeps champion movement,
  synchronized kills/gold/turrets, event banners, seeking, and playback even
  when Aura has no honest pressure model for its geometry.
- Added a separately calibrated Howling Abyss engine. It validates per-match
  early spawn clusters, projects champions onto the bridge axis, computes
  one-dimensional lane pressure, and draws a straight frontier perpendicular
  to the bridge instead of reusing Summoner's Rift's curved field.
- Added split-pressure detection: multiple lane-pressure crossings suppress a
  fabricated single frontier while retaining an integrated percentage and an
  explicit status label.
- Replaced the one-size-fits-all coordinate transform with schema-2 map models:
  Summoner's Rift 0..15000, empirically calibrated Howling Abyss 0..12800, and
  padded observed extents for unknown maps. Separate position/control reasons
  make fallbacks explainable without hiding valid replay data.
- Calibrated map 12 against eight real current queue-450 Match-V5 timelines.
  Only aggregate coordinate ranges and centroids were used; no match IDs,
  PUUIDs, Riot IDs, or API keys were retained.
- Bundled official Data Dragon 16.15.1 minimaps for map 11 and map 12 with
  pinned SHA-256 verification in CI. Terrain loads locally and never adds a
  runtime download or gameplay cache write.
- Added game-version labeling and a visible reference-only warning when the
  match patch family differs from the bundled terrain patch.
- Kept evidence boundaries explicit: the minimap is not a brush/walkability
  mask, Match-V5 does not prove exact recall casts or continuous paths, and all
  control values remain estimates rather than measured fog-of-war.
- Expanded deterministic replay tests, added ARAM and movement-only browser
  fixtures, and raised the Rust suite to 55 passing tests.
- Added Riot's required Legal Jibber Jabber asset notice to Settings and README.
- Bumped the application and installer version to 0.11.0.

## Official rank and champion mastery - 0.10.0

- Added a typed Champion-Mastery-V4 backend command using Riot's platform
  route. Results are filtered, deterministically sorted, deduplicated, capped,
  and held only in a two-minute volatile RAM cache.
- Added top-five Champion Mastery cards to Profile, a top-mastery/rank/account
  summary to Home, and official mastery level, points, progression, and last
  played information to champion detail cards.
- Added official Solo/Duo and Flex win/loss records, win rate, and available
  mini-series, hot-streak, veteran, and new-ranked-entrant status badges.
- Kept mastery independent from profile and match requests: a 403, 429,
  timeout, or decode failure in mastery now produces its own explicit state
  without hiding successfully loaded rank or match history.
- Added generation guards and immediate presentation resets so delayed results
  from a previously selected player cannot overwrite the current profile.
- Added Data Dragon readiness gating, accessible rank groups and mastery live
  status, responsive Home statistics, and click-through mastery cards that open
  the correct champion detail page.
- Added seven deterministic profile-summary tests, five focused Riot payload
  and normalization tests, a real-page browser fixture, strict CI coverage for
  the new frontend module, and desktop/narrow failure-path browser validation.
- Bumped the application and installer version to 0.10.0.

## Dynamic Map Control Replay - 0.9.0

- Added a lazy Match-V5 Timeline command for an expanded post-match report.
  Timeline requests are deduplicated, bounded, decoded directly into compact
  Rust DTOs, and retained in a three-match volatile LRU cache. A streaming
  16 MiB ceiling rejects oversized responses before deserialization.
- Added an HTML5 Canvas replay with interpolated champion movement, blue/red
  portrait borders, a curved territorial-pressure frontier, and a synchronized
  estimated-control percentage bar.
- Added synchronized Kills, Gold, Turrets, and Game Clock header values plus
  timed Dragon, Baron, Void Grub, Herald, Atakhan, turret, inhibitor, and
  best-effort unknown-objective banners.
- Added a full-game Map Control Over Time graph with a moving scrubber,
  play/pause, +/-15-second seeking, and 1x/4x/8x/16x playback speeds.
- Kept the metric honest: Aura labels control as an estimate derived from
  coarse champion positions, levels, and gold. Riot does not provide measured
  fog-of-war polygons or complete ward coordinates through Match-V5 Timeline.
- Added lifecycle guards that cancel rendering when a report closes, Profile
  is replaced, the page changes, or the document is hidden. Canvas DPR is
  capped at 1.5 and active drawing at 24 FPS.
- Added base-aware recall snapping, missing-position carry-forward, accessible
  game-time scrubber text, and one-time objective announcements.
- Added deterministic JavaScript model tests, defensive timeline decoding
  tests, a browser fixture, pipeline documentation, and release-workflow
  validation for the new module.
- Corrected Match-V5 OCE routing to Riot's SEA regional host.
- Bumped the application and installer version to 0.9.0.

## Native dropdown contrast fix — 0.8.2

- Fixed server, match-filter, advisor-role, and Spotify-device option lists that
  could render white text over WebView2's light native dropdown background.
- Declared Aura as a dark color-scheme application and gave every native option
  an explicit dark background, readable foreground, disabled state, and selected
  state so the fix applies consistently to every theme and dropdown.
- Bumped the application and installer version to 0.8.2.

## Spotify protected-player initialization hotfix — 0.8.1

- Fixed `Initialization: Failed to initialize player`. Aura 0.8.0 delegated
  encrypted media only to its own loopback page, which blocked the cross-origin
  protected-media frame created by Spotify's official Web Playback SDK.
- Delegated `encrypted-media` and `autoplay` to Spotify's SDK frames while
  retaining the strict Spotify-only Content Security Policy, no framing rule,
  loopback-only token bridge, and RAM-only session nonce.
- Added session-generation guards so delayed ready, offline, activation, or SDK
  error messages from an old player cannot overwrite a newly started device.
- Added a fresh-session **Open Browser Player** recovery action for a machine
  with a genuine WebView2/EME failure. It hosts Aura's local player in the
  default browser; it does not require Spotify Desktop or open.spotify.com.
- Added mode-specific protected-media diagnostics and regression tests that
  prevent `encrypted-media=(self)` or `autoplay=(self)` from returning.
- Bumped the application and installer version to 0.8.1.

## Automatic identities, linked profiles, and Aura Player — 0.8.0

- Added automatic discovery of the account signed in to LeagueClientUx. Aura
  combines tolerant current-summoner/chat responses with region-locale, never
  requires the retired numeric `id` field, and retries while client login is
  still settling.
- Added replayable RAM state plus an LCU event so profile discovery works both
  when League starts before Aura and when League starts later.
- Added official Account-V1 by-PUUID canonicalization while retaining official
  Summoner-V4, League-V4, and Match-V5 for visible profile data.
- Serialized each Match-V5 participant PUUID and made the player name and
  champion portrait one keyboard-accessible profile link.
- Added **My League Profile** to restore the locally detected account after
  inspecting another player.
- Serialized profile requests and added a generation guard so rapid player
  selection cannot let an older response overwrite a newer profile.
- Added an opt-in **Aura Player (Beta)** based on Spotify's official Web
  Playback SDK. It creates `Aura on this PC` without Spotify Desktop or the
  external Web Player when Premium and WebView2 protected playback are
  available.
- Isolated Spotify's remote SDK in a separate loopback-hosted Aura window with
  no configured Tauri IPC access. Tokens remain RAM-only, are served only after
  a 256-bit session nonce, and are never logged or stored.
- Added the official `streaming`, `user-read-email`, and `user-read-private`
  OAuth scopes. Existing users must reconnect once to consent.
- Kept external Spotify Connect devices and the Web Player as a fallback for
  unsupported WebView2/EME environments.
- Fixed the local publisher script so each version is staged in its own folder
  and only the exact current executable and installer are copied and hashed;
  stale installers can no longer leak into a new release folder.
- Added frontend/configuration validation and strict Clippy to the Windows
  release workflow.
- Bumped the application and installer version to 0.8.0.

## Profile analytics and publishing — 0.7.0

- Replaced thin match cards with expandable, keyboard-accessible reports.
- Added player KDA and ratio, CS/CSM, kill participation, champion damage and
  damage share, damage taken, gold and gold share, vision, wards, objective
  damage, healing, shielding, crowd control, multikills, items, summoner spells,
  full rune selections, ten-player scoreboards, team totals, bans, and
  objectives from the already-fetched Match-V5 response.
- Added ten/twenty-game selection plus queue/result filters and recent-profile
  average KP and champion damage per minute.
- Added a two-minute volatile match cache. Expanding a report performs no
  network request and repeat loads for the same Riot identity reuse RAM.
- Added tolerant Match-V5 decoding because Riot may omit empty/zero fields.
- Added tests for detailed calculations, missing fields, team synthesis, legacy
  duration normalization, and percentage boundaries.
- Added a cached GitHub Actions Windows x64 release workflow, mandatory signing
  gate for public tag releases, private-draft validation before publishing,
  NSIS publishing, and SHA-256 checksums.
- Changed release compilation from full LTO and one code-generation unit to thin
  LTO and eight units, enabled incremental release caching, and limited bundles
  to NSIS for substantially faster repeat publishing builds.
- Added a restrictive Tauri Content Security Policy and explicit current-user
  NSIS installation.
- Cached Tauri's bundler tools inside the project target cache. A verified
  no-change Windows x64 + NSIS repeat build completed in 4.84 seconds on the
  validation machine.
- Corrected match UI edge cases where 1% KP could display as 100%, missing
  metrics could display as zero, numeric bans lacked champion art, and invalid
  timestamps could display `NaN`.
- Replaced raw champion-detail and overlay-status `TypeError` failures with
  explicit recoverable error messages when a backend response is malformed.
- Bumped the application and installer version to 0.7.0.

- Replaced Riot ID file persistence with volatile in-memory session storage.
- Changed memory trimming from every guard tick to once when League starts.
- Retained automatic restoration to normal priority when League exits.
- Added bounded Data Dragon request timeout and a one-idle-connection pool.
- Removed stale references to non-existent project specification files.
- Rewrote project documentation to match the actual implementation.
- Validated frontend JavaScript and Tauri JSON configuration syntax.
- Fixed the Tauri v2 tray callback to resolve `AppHandle` from `TrayIcon`.
- Connected the Spotify previous-track button to a native API command.
- Replaced shell-parsed Spotify OAuth launching with direct Windows URL handling.
- Added a bounded timeout to local LCU REST calls.
- Passed headless Chromium dashboard and overlay smoke tests.

## Windows executable preparation
- Replaced all application, taskbar, title-bar, tray, and installer icon resources with the user-provided Aura icon.
- Generated standard 16/24/32/48/64/128/256-pixel Windows ICO entries plus Tauri PNG resources.
- Disabled the console window in release builds.
- Added size-optimized Rust release settings: `opt-level = "z"`, LTO, one codegen unit, abort-on-panic, and symbol stripping.
- Added `BUILD-WINDOWS.bat`, which produces `dist/Aura.exe` and an NSIS setup `.exe` on Windows.
- Added a Windows GitHub Actions workflow that generates downloadable `.exe` artifacts.
## Windows build fix — 2026-07-29

- Imported `tauri::Manager` in `src-tauri/src/main.rs`.
- Fixes Rust error `E0599`: `app.state(...)` was unavailable because the trait that provides it was not in scope.
- No behavior or UI changes were made by this patch.

## Secure integration configuration — 2026-07-29

- Removed the Spotify Client ID, redirect URI, and scopes from the OAuth
  implementation and centralized them in validated backend runtime config.
- Kept the provided public Spotify Client ID as the backend default, with
  `SPOTIFY_CLIENT_ID`, `SPOTIFY_REDIRECT_URI`, and `SPOTIFY_SCOPES` overrides.
- Kept Riot credentials out of source and added startup-only `RIOT_API_KEY`
  loading into private Rust state.
- Added redacted configuration status to the dashboard; no credential values
  are serialized to frontend JavaScript or printed to logs.
- Added strict loopback redirect validation and proper URL encoding/parsing.
- Bound the Spotify callback listener before launching the browser.
- Added Spotify network timeouts and stopped exposing API response bodies.
- Added configuration and OAuth unit tests plus full setup documentation.
- Added a credential-focused `.gitignore`.
- Changed the overlay to lazy creation so a second WebView does not consume
  memory while League is closed.
- Bumped the application and installer version to 0.2.0.

## Validation results for 0.2.0

- `cargo check`: passed with no warnings.
- `cargo test`: 8 passed, 0 failed.
- Rust formatting check: passed.
- Frontend JavaScript and Tauri JSON syntax checks: passed.
- Optimized Windows release build: passed.
- NSIS installer build: passed.
- Final portable executable stayed responsive during an 8-second startup smoke
  test and used 29.13 MiB working set before the lazy overlay was mounted.
- Credential-content scan: passed; the exposed Riot development key is absent.

## Functional application update — 0.3.0

- Replaced every decorative sidebar link with working Home, Profile,
  Champions, Builds, Runes, Live Game, Overlays, and Settings pages.
- Connected the top search bar to the current Data Dragon champion catalog,
  including suggestions, Enter-to-open, champion lore, stats, and abilities.
- Added RAM-only item build and rune planners using current patch data.
- Filtered temporary champion variants, non-Summoner's-Rift items, and duplicate
  catalog entries while preserving numeric IDs needed by live League events.
- Added Windows Credential Manager storage for Riot API keys. Keys can be
  saved or cleared from Settings and are never returned to the webview.
- Added Spotify playback status, device discovery, playback transfer,
  Web Player launch, automatic device fallback, and visible playback errors.
- Documented that the loopback callback exists only during Connect, Spotify
  Premium is required for Web API playback control, and an active Spotify
  Connect device is required.
- Added manual overlay show/hide/status commands and a dedicated overlay page.
- Bumped the application and installer version to 0.3.0.

## Validation results for 0.3.0

- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 10 passed, 0 failed.
- Rust formatting, JavaScript syntax, static element-ID, and IPC command
  contract checks: passed.
- Real Windows app UI test: all eight pages opened; Yone search/details and
  rune selection were exercised successfully.
- Secret scan: passed; the Riot key previously shared in chat is absent.
- Optimized Windows executable and NSIS installer builds: passed.
- Final 8-second release smoke test: responsive, version 0.3.0, 32.95 MiB
  working set with the metadata cache ready.

## Reliability and recommendations update — 0.4.0

- Fixed the Windows WebView2 deadlock caused by creating the overlay from a
  synchronous Tauri command. Overlay creation now runs from an async command,
  and its WebView memory is released when hidden.
- Added frontend deadlines and clear timeout errors for Riot profile, champion
  details, metadata, Spotify, and overlay requests.
- Added Riot level/rank profile data and parallelized the ten recent match
  detail requests with a bounded concurrency of four.
- Removed the 80-champion display cap and the 180-item display cap. Aura now
  shows the complete current champion roster and current Summoner's Rift shop.
- Added automatic six-item templates for every champion archetype, specific
  champion overrides, and optional enemy-team adaptation from visible
  Champion Select picks.
- Added automatic champion rune pages. Per-rune win and pick rates use the
  user's loaded recent matches and never present invented global statistics.
- Fixed Spotify HTTP 411 by sending an explicit zero-length request body and
  targeting the selected Connect device for play/pause/previous/next.
- Bumped the portable executable and installer to version 0.4.0.

## Validation results for 0.4.0

- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 10 passed, 0 failed.
- Rust formatting and frontend JavaScript syntax checks: passed.
- Optimized Windows executable and NSIS installer builds: passed.
- Real Windows UI test: lazy overlay show, hide, and re-show passed without a
  deadlock or cleanup error; all 173 champions remained available afterward.
- Briar details, automatic six-item build, and automatic rune-page rendering
  were exercised in the release executable.

## Riot profile and rune-page correction — 0.5.0

- Updated profile decoding for Riot's current SUMMONER-V4 response, which no
  longer returns the legacy encrypted summoner `id`.
- Changed ranked lookup to LEAGUE-V4's current PUUID route, fixing the
  `response decode failed: missing field id` error.
- Updates the header as soon as the Riot ID resolves, so a later network error
  cannot leave the top-right identity stuck on `Riot ID not loaded`.
- Rebuilt the rune planner around League's real page rules: one primary path
  with four selections, a different secondary path with two selections from
  distinct rows, and three current stat-shard rows.
- Added current Move Speed, Health, Tenacity, and scaling shard choices and
  removed obsolete armor/resistance shard layouts.
- Added automatic validation when manually selecting a third secondary row.
- Preserved honest personal rune win/pick rates from the loaded recent-match
  sample and the champion-based 9/9 recommendation flow.
- Bumped the portable executable and installer to version 0.5.0.

## Validation results for 0.5.0

- `cargo check --all-targets`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test --all-targets`: 12 passed, 0 failed.
- Rust formatting, frontend JavaScript, Tauri JSON, unique HTML ID, and secret
  scans passed.
- Real Windows UI test loaded `ShadowStriker#1386` on EUNE, level, rank state,
  and ten recent matches without the legacy decoding error; the header updated
  to the resolved Riot ID.
- Briar's recommended rune page completed all 9/9 choices, personal rune rates
  populated from eight recent Briar games, and the two-distinct-secondary-row
  rule was exercised successfully.

## Aura Intelligence advisor — 0.6.0

- Added draft recommendations, visible-state live priorities, and personal
  post-game review through a private Rust backend service.
- Added an optional aggregate data feed configured with `AURA_META_API_URL` and
  `AURA_META_API_TOKEN`. The token stays in backend memory and is never
  serialized, returned to the webview, or printed.
- Restricted feed URLs to HTTPS, with `http://127.0.0.1` permitted only for
  local development. URLs containing user information, queries, or fragments
  are rejected.
- Added an eight-second request deadline, three-second connection deadline,
  redirect blocking, one idle connection per host, and a two-MiB streamed
  response limit.
- Added a 15-minute volatile-RAM cache and bounded startup refresh. No advisor
  data is written to disk.
- Required aggregate providers to disclose source, patch, queue, rank range,
  region, provider-reported sample size, generation time, dataset/schema
  versions, and a source or methodology URL.
- Labels aggregate sample sizes as provider-reported and not independently
  verified. No "millions of matches" claim is generated from a large number.
- Added a deterministic local fallback with `sample_size: 0` and an explicit
  statement that it is not aggregate or mass-match data.
- Draft output ranks a best statistical fit plus alternatives, evidence, and
  tradeoffs. It preserves player choice and never claims a guaranteed result.
- Live output uses only supplied visible Champion Select IDs and current-game
  telemetry. It does not infer hidden opponents, MMR, or ELO.
- Post-game output is limited to the user's supplied completed-match sample.
- Added safe advisor configuration fields to integration status without
  exposing the feed token or authorization header.
- Added Champion Select role auto-detection and preserved the current match
  draft separately from unrelated champion-page selections.
- Rejects live analysis outside an active match or when current-game telemetry
  is missing or more than 15 seconds old.
- Marks provider snapshots older than 72 hours as stale, validates RFC 3339
  generation times, lowers confidence for patch/queue/rank/region mismatches,
  and displays those mismatches as limitations.
- Rejects duplicate aggregate rows and impossible row counts above the
  provider-reported match total.
- Changed the RAM cache to share aggregate snapshots by `Arc` instead of
  cloning the full feed for each advisor request.
- Post-game review now uses trends across every supplied recent match rather
  than raising confidence from records that did not affect the analysis.
- Added the Riot-required non-endorsement notice and documented product
  registration plus the server-side production-key release gate.
- Added provenance, URL-policy, secret-redaction, deterministic-ranking,
  visible-pick exclusion, fallback-disclosure, and sample-label unit tests.
- Bumped the executable and installer version to 0.6.0.

## Validation results for 0.6.0

- `cargo fmt --all -- --check`: passed.
- `cargo check --all-targets`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test --all-targets`: 29 passed, 0 failed.
- Frontend JavaScript syntax, Tauri JSON, 100 unique HTML IDs, and all 22
  frontend-to-Rust command registrations: passed.
- Local UI interaction test: navigation, champion search/details, role
  selection, draft ranking, live priorities, evidence, tradeoffs, source
  provenance, local/provider labels, and post-game empty state: passed.
- Source and release-binary scans found no embedded Riot API key, aggregate
  feed token, or Spotify client secret.
- Optimized Windows executable and NSIS installer builds: passed; both report
  version 0.6.0.
- Eight-second release smoke test: passed with a 32.26 MiB main-process working
  set. The existing sub-30 MiB aspiration remains unproven on this machine.
