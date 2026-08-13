# Aura live metrics and overlay

This document defines the runtime data boundary for Aura's Live Game dashboard
and click-through overlay. It is the source of truth for metric names,
availability, provenance, cadence, rendering, and release acceptance.

Aura reads Riot's loopback Live Client Data API only while a match is active.
This local feed does not use a Riot developer key. Its documented fields are
useful but deliberately limited; an attractive HUD is not permission to turn a
missing value into a guess.

Riot reference: [Live Client Data API](https://developer.riotgames.com/docs/lol#game-client-api_live-client-data-api)

## 1. Runtime pipeline

```text
League game process
  -> HTTPS loopback API at 127.0.0.1:2999
  -> one Rust telemetry supervisor
  -> small typed camelCase Tauri events
  -> src/services/ipc.js validation and normalization
  -> shared pure live-game view model
  -> main Match Command Center and optional overlay WebView
```

The Rust backend owns network access and calculation. The webviews receive no
LCU credential, Riot API key, Spotify token, raw response body, or arbitrary
endpoint URL. Live events are volatile and are not written to a match-time disk
cache.

## 2. Event cadence and ownership

| Channel | Owner | Cadence | Purpose |
| --- | --- | --- | --- |
| `game:state-changed` | LCU watcher, with telemetry fallback | Transitions only | Lobby, Champion Select, in-game, and ended routing |
| `live:game-tick` | Telemetry supervisor | Once per second in game | Active-player metrics and objective countdowns |
| `live:player-update` | Same supervisor | Every five seconds in game | Per-player level, CS, and item IDs |
| `draft:update` | LCU Champion Select watcher | Pick/ban changes | Draft assistant state |

For each active tick, the backend concurrently requests `gamestats`,
`activeplayer`, and `playerlist`. It does not request any of those endpoints a
second time in the same tick. `eventdata` refreshes every five seconds, not once
per second. Player-update events reuse the already fetched player list and are
also emitted every five seconds.

While a match is active, the supervisor does not scan the Windows process table.
Three consecutive `gamestats` failures provide a bounded match-end fallback.
When no match is active, a ten-second idle process check prevents repeated
requests to a closed port.

## 3. Typed tick contract

The canonical TypeScript interfaces live in `src/types/ipc.ts`; the stable Rust
wire structures and emitters live in `src-tauri/src/live_client/mod.rs`. Rust's
`serde(rename_all = "camelCase")` keeps both sides on one wire naming scheme.

`LiveGameTickEventPayload` extends the required `LiveGameTickPayload` with two
integrity objects:

- `metricAvailability`: whether Aura has every required source field for a
  metric in this tick;
- `metricSources`: the approved provenance enum for explicitly estimated
  fields.

Compatibility numbers such as zero are never sufficient evidence of
availability. Every consumer must consult the corresponding availability flag.

The current extension fields under `activePlayer` are:

```text
level
creepScore
creepScorePerMinute
killParticipationPercent
observableHeldValue
observableValuePerMinute
earnedGoldPerMinute
xpProgressPercent
```

The approved estimate source is:

```text
CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE
```

If the source is missing or unknown, the frontend treats the estimate as
unavailable even if a numeric value is present.

## 4. Metric provenance

| Display | Wire field | Classification | Source and rule |
| --- | --- | --- | --- |
| Game clock | `gameTime` | Exact local value | `gamestats.gameTime` |
| Current gold | `activePlayer.currentGold` | Exact local value | `activeplayer.currentGold` |
| KDA | `activePlayer.kda` | Exact local values | Active player's `playerlist.scores` |
| Level | `activePlayer.level` | Exact local value | Active player's `playerlist.level` |
| CS | `activePlayer.creepScore` | Exact local value | Active player's `scores.creepScore` |
| CS/min | `activePlayer.creepScorePerMinute` | Exact derived rate | Current CS and game time |
| KP | `activePlayer.killParticipationPercent` | Exact derived ratio | Active K+A divided by the current sum of teammate kills |
| Held value | `activePlayer.observableHeldValue` | Explicit estimate | Current cash plus listed value/count of currently held items |
| Held value/min | `activePlayer.observableValuePerMinute` | Explicit estimate | Held value divided by elapsed minutes |
| Earned GPM | `activePlayer.earnedGoldPerMinute` | Unavailable | Lifetime earned gold is not exposed locally |
| DPM | `activePlayer.dpm` | Unavailable | Live damage to champions is not exposed locally |
| Team gold lead | `teamGoldDelta` | Unavailable | Exact team total gold is not exposed locally |
| XP progress | `activePlayer.xpProgressPercent` | Unavailable | Progress within the current level is not exposed locally |

"Exact derived" means deterministic arithmetic over exact fields in the current
local snapshot. It does not mean prediction, historical reconstruction, or a
claim that an undocumented Riot response can never change.

### CS per minute

```text
CS/min = creepScore * 60 / gameTimeSeconds
```

Aura reports the value only when CS is present and game time is finite and
greater than zero. It describes current cumulative pace, not projected final CS.

### Kill participation

```text
teamKills = sum(kills for players on the active player's team)
KP% = (activePlayer.kills + activePlayer.assists) / teamKills * 100
```

Aura clamps the final ratio to 0..100. KP is unavailable if identity/team
matching fails, a required score is missing, or `teamKills` is zero. A zero-kill
team has no meaningful participation denominator; Aura does not display a
misleading `0%` as measured data.

### Observable held value and held value/minute

```text
heldValue = currentGold + sum(item.price * item.count)
heldValuePerMinute = heldValue * 60 / gameTimeSeconds
```

Both fields require finite non-negative current gold, a present item list,
finite non-negative price and count for every non-zero item, and positive game
time for the rate. If any input is missing, the availability flag is false and
the provenance value is null.

This is a stock divided by elapsed time, not an income flow. It can move down
when an item is consumed or sold and cannot recover value that is no longer in
the inventory. It also cannot reconstruct passive gold, objective income,
previous purchases, sale losses, free transformations, or other lifetime
economy events. The UI therefore calls it **Held Value / Minute - Est.** and
never abbreviates it to GPM.

### Unsupported exact values

`dpm`, `teamGoldDelta`, and `earnedGoldPerMinute` retain numeric compatibility
slots so a future truthful data provider can be introduced without breaking the
event shape. Their availability flags are currently false. `xpProgressPercent`
is nullable and unavailable. The main dashboard and overlay render explanatory
unavailable text rather than the sentinel values.

## 5. Dashboard rendering

The Live Game view is a framework-free dark Match Command Center with:

- status, game clock, champion, summoner, and level hierarchy;
- KDA, CS, CS/min, KP, current gold, held-value and held-value/minute
  estimates, DPM, earned GPM, and team-gold cards;
- an explicit unavailable XP state with no fake progress fill;
- Dragon and Baron countdowns; and
- a persistent integrity note explaining estimated and unavailable metrics.

`src/services/ipc.js` normalizes every native payload before it reaches state.
`src/components/analytics/live-game-metrics.js` builds display-only strings and
renders with `textContent`; live data is never interpreted as HTML. The renderer
compares text, classes, attributes, progress values, and hidden state before
writing, reducing style/layout work for unchanged values.

`src/main.js` owns one subscription bundle and disposes it during unload. A
partial listener-registration failure tears down listeners that were already
created.

## 6. Overlay rendering and limits

The overlay is a borderless, transparent, non-resizable, always-on-top Tauri
window near the current monitor's top-right edge. Native size and frontend
geometry change together; scaling only the CSS would leave an unnecessarily
large transparent window behind.

| Mode | 100% native size | Purpose |
| --- | ---: | --- |
| Standby | 32x32 | A centered 24px Aura pill when the overlay is shown manually without a match. Normal match end closes the WebView to the tray instead. |
| Compact | 432x52 | One edge ribbon with clock, KDA, team-gold state, Dragon, Baron, and the expand affordance. |
| Expanded | 520x150 | The ribbon plus identity, merged `CS (CS/min)`, KP, current gold, held-value/minute estimate, earned GPM, DPM, XP, and edit controls. |

Scale presets resize both layers to 75%, 90%, or 100%. Examples include a
324x39 compact ribbon at 75% and 468x135 expanded panel at 90%. The dimensions
are logical pixels, so Windows still applies the monitor's DPI scale.

The panel opacity range is 40% through 100%, with 55% as the default. One flat
alpha surface owns the translucency; inner cells add only a small fixed tint.
Aura deliberately does not use `backdrop-filter` blur in this gameplay WebView.
That keeps the terrain visible without adding a continuously sampled blur pass.

`src/components/overlay/live-overlay.js` delegates metric semantics to the same
main-dashboard view model. Compact mode preserves the requested gold-lead slot,
but it displays `Unavailable` until a truthful team-gold source exists. Expanded
mode likewise keeps exact earned GPM, DPM, and XP visibly unavailable. The
separate held-value/minute approximation stays labeled `est.` and CS plus
CS/min are combined into one value.

### Interaction lock

The native default is locked: `set_ignore_cursor_events(true)` and
`set_focusable(false)` pass clicks to League. An in-window button cannot unlock
a pass-through WebView, so the escape hatch is the main Aura Overlay Controls
page or the native tray item **Toggle HUD Interaction Lock**. Unlocking reverses
both native properties. The expanded panel can then change mode, opacity, and
scale, and **Lock HUD** restores pass-through.

The tray toggle rejects an absent or hidden overlay. Hiding/closing the overlay
always resets `locked` to true, preventing a previously edited HUD from stealing
the first gameplay click when the next match starts. This build does not
register a global keyboard shortcut and must not advertise one.

Layout settings live in a Rust `Mutex` for the current Aura process only. They
do not create gameplay disk writes. The overlay WebView is created lazily when
shown and closed when hidden; closing releases the secondary WebView and
triggers idempotent native-listener cleanup.

Always-on-top and click-through do not guarantee visibility over every
exclusive-fullscreen presentation path. Borderless/windowed and the player's
intended League display mode must be checked on the target Windows build.

## 7. Performance design, not a performance promise

The live path limits work through:

- one Rust owner for all port-2999 requests;
- one shared `reqwest::Client` with a 500 ms request timeout;
- three independent high-frequency requests issued concurrently;
- a 2 MiB streamed response-body ceiling before deserialization;
- event and player-update work reduced to five-second cadence;
- no active-match process-table polling;
- compact serialized payloads rather than raw Riot responses;
- transition-only game-state emission;
- changed-value-only DOM writes; and
- lazy overlay WebView lifetime.

These choices reduce overhead. They cannot prove "zero input lag," "zero frame
drops," or a universal CPU/RAM result. League, WebView2, Windows scheduling,
display mode, GPU drivers, overlays from other software, and hardware all affect
the measured outcome.

## 8. Synthetic benchmark

Run the deterministic frontend transform benchmark from the project root:

```powershell
node .\scripts\BENCH-LIVE-PIPELINE.mjs
```

It repeatedly normalizes one representative tick and builds the dashboard view
model, then prints elapsed time, average microseconds, operations per second,
and process heap delta. The checksum prevents the result from being completely
unused. `node --expose-gc` may reduce heap noise between local comparisons.

This script does not start Tauri, WebView2, League, native networking, event
serialization, or DOM rendering. It is useful for catching large transform
regressions only. Never quote its throughput as end-to-end latency or game FPS.

## 9. Automated verification

Use the source modules directly with Node's built-in runner:

```powershell
node --test .\tests\*.test.mjs
```

Rust unit tests cover serialization, response decoding, metric calculations,
missing-source behavior, team mapping, cadence decisions, and objective timers:

```powershell
cargo test --manifest-path .\src-tauri\Cargo.toml --all-targets --locked
```

Do not record final pass counts until these commands have completed against the
exact packaged revision.

## 10. Real-match release acceptance

Automated tests and the synthetic benchmark are necessary but not sufficient.
Before publishing, exercise the exact release executable and installer on the
low-spec target PC.

### Record the environment

- CPU, RAM, GPU, storage type, monitor count, resolution, and DPI scale;
- Windows build, WebView2 runtime, GPU driver, League patch, and Aura version;
- League display mode, graphics preset, frame cap, and background applications;
- measurement tool, sampling interval, run length, and scenario.

### Establish repeatable baselines

Use the same Practice Tool or replay route for at least three runs in each state:

1. League without Aura running.
2. Aura dashboard running with overlay hidden.
3. Aura dashboard running with overlay visible.

Compare League frame-time median, 95th percentile, and 99th percentile rather
than relying only on an FPS counter. Record Aura CPU, private/working-set memory,
network cadence, handle count, and any growth over the run. Investigate a
repeatable regression; do not round it away as "zero."

### Validate lifecycle and behavior

- Start Aura before League, after League, and during a match.
- Enter Lobby, Champion Select, loading, active game, post-game, and client exit.
- Confirm one tick per second and five-second event/player refreshes without
  duplicate endpoint requests.
- Interrupt port 2999 briefly and confirm one transient miss does not end the
  match; confirm sustained failure closes the overlay after the threshold.
- Show, hide, and recreate the overlay repeatedly; verify its WebView is gone
  while hidden, the interaction lock returns to `true`, and listener counts do
  not grow.
- Exercise standby, compact, and expanded modes at 75%, 90%, and 100% scale.
  Measure the native logical size and confirm the page has no clipping or
  scrollbars at every combination.
- Unlock from the dashboard and tray, operate every expanded control, then lock
  again and confirm clicks reach League. Confirm the tray rejects an unlock when
  the overlay is absent/hidden and no later auto-show opens interactively.
- Check 40%, 55%, and 100% opacity over bright terrain and moving units. Verify
  inner telemetry cells do not stack the selected alpha into an opaque block.
- Confirm level, CS, CS/min, and KP against the in-game scoreboard at multiple
  times. Confirm KP is unavailable at zero team kills.
- Confirm held-value/minute is always marked estimated and exact earned GPM,
  DPM, team gold, and XP progress never display a sentinel as a real value.
- Verify click-through, native placement, and complete layout at 100%, 125%,
  150%, and 200% DPI in the intended League display modes, including monitors
  with different scale factors and negative desktop coordinates.
- Exercise alt-tab, sleep/resume, League restart, monitor changes, and Aura exit.

Publish measured results with the environment and method. A result on one PC is
evidence for that PC and scenario, not a guarantee for every player.

## 11. Future truthful sources

If a future approved source provides exact DPM, team gold, lifetime earned gold,
or XP progress, add it behind the existing availability flags and document its
endpoint, cadence, permission model, failure behavior, and provenance. Do not
infer any of these exact metrics from attack damage, champion level, item list
price, or unrelated Match-V5 history.
