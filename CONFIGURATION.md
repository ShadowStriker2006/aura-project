# Aura integration configuration

Aura handles integration settings in the Rust backend. The frontend receives
only redacted status fields. Riot keys never enter a webview. Spotify tokens are
never logged or persisted; when the optional Aura Player is running, its
isolated player window receives a just-in-time token through a random,
nonce-protected loopback endpoint required by Spotify's official SDK. The main
dashboard never receives that token.

## Riot Games API

The Riot development key shared in the previous conversation is exposed and
must be revoked or allowed to expire. Generate a fresh key before testing.
Development keys are temporary, so never compile one into Aura or distribute it
inside a ZIP, executable, installer, frontend file, or build script.

Open **Settings → Riot API Credential**, paste the fresh key, and choose
**Save Securely**. Aura stores it as `Aura/RiotApiKey` in Windows Credential
Manager and enables match history immediately. The key is not added to Aura's
source, HTML, executable, installer, or ZIP.

For managed deployments, `RIOT_API_KEY` remains available as an override:

```powershell
$env:RIOT_API_KEY = "RGAPI-your-new-key"
.\dist\Aura.exe
```

That PowerShell form applies only to the process started from the same terminal
and takes priority over the saved Windows credential. Replace development keys
whenever they expire.

### Automatic signed-in account

When LeagueClientUx starts and the user is signed in, Aura reads the current
PUUID and region from the local League Client, prefills the Riot ID form, and
loads that profile automatically when a valid Riot API key is available. The
decoder intentionally does not require Riot's retired numeric `id` field. Aura
keeps the detected local account separate from profiles opened from a match;
choose **My League Profile** to return to it.

The local endpoints are `/lol-summoner/v1/current-summoner`, `/lol-chat/v1/me`,
and `/riotclient/region-locale`. They are accessed only over authenticated
loopback using the current lockfile. Lockfile passwords, PUUIDs, and Riot IDs
are not logged. Riot describes LCU as unsupported and subject to change; list
these endpoints in the product registration. Canonical Riot IDs, rank, and
match data still come from Account-V1, Summoner-V4, League-V4, and Match-V5.

### Public release gate

Do not distribute Aura publicly as a Riot-powered product until it is registered
and reviewed through the Riot Developer Portal. A temporary development key is
for local development only and is not a production data pipeline.

Any service that collects Match-V5 data at aggregate scale must run separately
from the desktop app with an approved production key held only on that server.
The Aura executable must receive only a precomputed, licensed aggregate snapshot;
it must never contain the collector's Riot key. Confirm the provider's collection,
redistribution, retention, and commercial-use permissions before enabling its
feed in a release.

## Spotify OAuth with PKCE

Aura uses Authorization Code with PKCE. A client secret is neither required nor
supported in the desktop app. Never add a Spotify client secret to this project.

The provided public Spotify Client ID is the backend default:

```text
681dfe3599314fd2adde1cd53ab731a8
```

In the Spotify developer dashboard, register this redirect URI exactly:

```text
http://127.0.0.1:8888/callback
```

The callback is not a regular website. Aura opens a temporary listener only
after **Connect Spotify** is pressed, and closes it when login succeeds or times
out. Opening the callback directly at any other time will correctly show
"connection refused."

The default scopes are:

```text
streaming user-read-email user-read-private user-read-playback-state user-modify-playback-state
```

All three public OAuth settings can be overridden before launch:

```powershell
$env:SPOTIFY_CLIENT_ID = "your-32-character-client-id"
$env:SPOTIFY_REDIRECT_URI = "http://127.0.0.1:8888/callback"
$env:SPOTIFY_SCOPES = "streaming user-read-email user-read-private user-read-playback-state user-modify-playback-state"
.\dist\Aura.exe
```

Security validation requires an `http://127.0.0.1` loopback redirect with an
explicit unprivileged port and callback path. All five scopes above are required
for Aura Player plus remote controls; additional valid Spotify scopes may be
appended. Existing users must press **Connect Spotify** once after upgrading to
0.8 or later so Spotify can grant the new scopes. If the Spotify app is in Development
mode, the login account must be permitted by that app's dashboard settings.

**Start Aura Player** creates a real Spotify Connect device named
`Aura on this PC` through Spotify's official Web Playback SDK. It runs in a
small isolated Aura window, uses a random loopback session nonce, and exists
only in volatile memory. Press **Activate Aura Playback** once in that window;
then the main Aura controls can play, pause, skip, and transfer without Spotify
Desktop or the external Web Player. Spotify Premium is mandatory.

The Web Playback SDK depends on protected-media/EME support. Aura 0.8.1 grants
Spotify's cross-origin SDK frame the required `encrypted-media` and `autoplay`
permissions while its CSP still restricts frames and connections to Spotify.
If the isolated window still reports an initialization error, Aura reveals
**Open Browser Player**. That action starts a fresh loopback session in the
default browser, so no failing WebView and browser tab share a device. Keep the
small Aura Player tab open while listening. Spotify Desktop and the external
Spotify Web Player are not required. If Edge or Chrome also fails, update the
browser and enable protected content/Widevine. Aura does not use unofficial
Spotify clients or store decoded audio. Before a public or commercial release,
review Spotify's current Developer Policy and obtain any required written
approval for streaming integrations.

## Environment variable summary

| Variable | Required | Default |
| --- | --- | --- |
| `RIOT_API_KEY` | For Riot match history | None |
| `SPOTIFY_CLIENT_ID` | No | Provided public Client ID |
| `SPOTIFY_REDIRECT_URI` | No | `http://127.0.0.1:8888/callback` |
| `SPOTIFY_SCOPES` | No | Web Playback plus playback read/modify scopes |
| `AURA_META_API_URL` | No | Local deterministic advisor |
| `AURA_META_API_TOKEN` | No | None |

## Aura Intelligence aggregate feed

Aura Intelligence always works with a deterministic local fallback. That
fallback is labeled as a local heuristic, reports `sample_size: 0`, and does
not claim to represent aggregate or mass-match data.

An aggregate provider can be enabled before Aura starts:

```powershell
$env:AURA_META_API_URL = "https://meta.example.com/aura/feed"
$env:AURA_META_API_TOKEN = "your-private-provider-token"
.\dist\Aura.exe
```

`AURA_META_API_URL` must use HTTPS. The only permitted plain-HTTP endpoint is
`http://127.0.0.1` for local development. User information, query parameters,
and fragments are rejected. Redirects are not followed. The optional token is
held only in private Rust memory, sent as a Bearer authorization header, and is
never returned to JavaScript or written to a log.

The feed request has an eight-second total timeout, a three-second connection
timeout, a one-connection idle pool, and a maximum decoded response size of
2 MiB. A successful response is cached in volatile RAM for 15 minutes.
`generated_at` must be an RFC 3339 timestamp. Aura marks a provider snapshot
older than 72 hours as stale and rejects timestamps more than 24 hours in the
future; transport cache age and dataset age are both surfaced as limitations.

### Required aggregate response

The endpoint must return `application/json` with this shape. `matchups` and
`synergies` may be empty, but at least three champion/role records are required.
Rates are ratios from `0.0` to `1.0`; `provider_score`, when supplied, is from
`0` to `100`.

```json
{
  "provenance": {
    "source": "Provider display name",
    "patch": "16.15",
    "queue": "Ranked Solo",
    "rank_range": "Iron-Challenger",
    "region": "global",
    "sample_size": 50000,
    "generated_at": "2026-07-29T12:00:00Z",
    "methodology": "Aggregate champion, matchup, and synergy weighting.",
    "methodology_url": "https://provider.example/methodology",
    "source_url": "https://provider.example/dataset",
    "dataset_version": "2026-07-29",
    "schema_version": "1"
  },
  "champions": [
    {
      "champion_id": 1,
      "name": "Example",
      "role": "mid",
      "games": 1000,
      "win_rate": 0.51,
      "pick_rate": 0.08,
      "ban_rate": 0.02,
      "provider_score": 63.5,
      "strengths": ["Provider evidence statement"],
      "tradeoffs": ["Provider tradeoff statement"]
    }
  ],
  "matchups": [
    {
      "champion_id": 1,
      "enemy_champion_id": 2,
      "role": "mid",
      "games": 500,
      "win_rate": 0.54
    }
  ],
  "synergies": [
    {
      "champion_id": 1,
      "ally_champion_id": 3,
      "role": "mid",
      "games": 400,
      "win_rate": 0.53
    }
  ]
}
```

`sample_size` is the provider-reported total number of matches represented by
the snapshot. No champion, matchup, or synergy row may report more games than
that total. Aura displays the value as reported and does not independently
verify it.
`methodology` and either `source_url` or `methodology_url` are required, and all returned
recommendations carry the complete provenance object.
Patch, queue, rank, and region mismatches are shown explicitly and lower
recommendation confidence instead of being presented as matched evidence.

### Tauri command contract

The frontend command names are:

- `advisor_status` — no arguments.
- `advisor_refresh` — no arguments; refreshes the configured feed.
- `advisor_draft_mandate` — `{ "request": DraftRequest }`.
- `advisor_live_orders` — `{ "request": LiveRequest }`.
- `advisor_post_game` — `{ "request": PostGameRequest }`.

The compatibility command names retain the words `mandate` and `orders`, but
their output is advisory: it ranks choices, includes alternatives and
tradeoffs, and never dictates a guaranteed decision.

All three request objects accept these common snake-case fields:

```text
role, patch, region, queue_id, queue, rank_range, gameflow_phase,
selected_champion_id, ally_champion_ids, enemy_champion_ids,
context_captured_at
```

Draft additionally accepts:

```text
champion_catalog: [{ id, name, image_id }]
```

Live additionally accepts both a nested telemetry object and flat values:

```text
telemetry: { game_time, dragon_respawn_at, baron_respawn_at,
             received_at_ms, age_ms }
game_time, dragon_respawn_at, baron_respawn_at,
telemetry_received_at_ms, telemetry_age_ms,
kills, deaths, assists, cs, vision_score, current_gold
```

Post-game additionally accepts:

```text
latest_match
recent_matches
```

Each match record accepts:

```text
match_id, queue_id, game_mode, game_creation_ms, game_duration_secs,
champion_name, win, kills, deaths, assists, cs, gold, vision_score, items
```

Advisor responses contain:

```text
phase, advisory_label, headline, mandate, recommended_champion,
recommended_champion_id, confidence, reasoning, actions, warnings,
alternatives, provenance, used_fallback, policy_notice
```

Each alternative contains:

```text
rank, champion_id, champion, title, reason, tradeoff, confidence,
score, win_rate, sample_size
```

Status contains only safe fields:

```text
configured, ready, stale, refreshing, mode, source, message, last_error,
cache_age_seconds, cache_ttl_seconds, max_response_bytes, provenance,
policy_notice
```

The integration status adds `advisor_configured`, `advisor_mode`, and
`advisor_error`. It never contains the configured URL token, Bearer header, or
other credential material.
