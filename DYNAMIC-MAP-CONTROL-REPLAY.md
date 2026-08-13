# Dynamic Map Control Replay

Aura's post-match replay is a lazy, RAM-only visualization built from Riot's
Match-V5 Timeline endpoint. It uses the HTML5 Canvas API and bundled map assets;
it adds no JavaScript framework, D3 dependency, runtime terrain download, or
gameplay disk cache.

## Availability is split by capability

Replay playback and control estimation are deliberately independent:

```text
usable Timeline positions?
  no  -> movement replay unavailable; the match report still works
  yes -> always render champions, stats, events, seeking, and playback
           |
           +-- calibrated control model passes its guards?
                 no  -> movement-only replay with an explicit reason
                 yes -> estimated control percentage, frontier, and graph
```

This means Howling Abyss and future maps no longer disappear merely because a
Summoner's Rift pressure model is inappropriate. An unsupported map still gets
its positional replay on a neutral observed-coordinate view. Aura never applies
Summoner's Rift lanes, bases, or pressure geometry to another map.

## Data flow

```text
Profile recent matches
  -> user expands one complete match report
  -> existing match detail is read from Aura's RAM cache
  -> replay shell requests get_match_timeline(matchId)
  -> authenticated GET /lol/match/v5/matches/{matchId}/timeline
  -> Rust validates and normalizes the open Timeline DTO
  -> map-specific coordinate and control guards run
  -> compact MatchTimelineReplay is cached in RAM (maximum three matches)
  -> Canvas module precomputes only the enabled control model
```

The timeline is never fetched for every recent match. This avoids doubling the
Profile request count and keeps the scoreboard usable when Riot rejects or
rate-limits the optional Timeline request.

## Compact JSON contract (schema 2)

```json
{
  "schema_version": 2,
  "match_id": "EUN1_1234567890",
  "map_id": 12,
  "game_version": "16.15.1.1234",
  "frame_interval_ms": 60000,
  "duration_ms": 1835000,
  "coordinates": {
    "model_id": "howling_abyss_rect_v1",
    "min_x": 0,
    "max_x": 12800,
    "min_y": 0,
    "max_y": 12800,
    "invert_x_for_canvas": false,
    "invert_y_for_canvas": true,
    "swap_axes_for_canvas": false,
    "provenance": "aura_match_v5_calibration_16_15_v1"
  },
  "control_model": {
    "id": "howling_abyss_linear_v1",
    "topology": "linear_lane",
    "blue_base": { "x": 1041, "y": 985 },
    "red_base": { "x": 11965, "y": 11521 },
    "anchor_provenance": "timeline_spawn_clusters"
  },
  "participants": [
    {
      "participant_id": 1,
      "team_id": 100,
      "champion_id": 103,
      "champion_name": "Ahri"
    }
  ],
  "frames": [
    {
      "timestamp_ms": 60000,
      "players": [
        {
          "participant_id": 1,
          "x": 4200,
          "y": 3900,
          "level": 5,
          "total_gold": 2800,
          "cs": 42
        }
      ],
      "teams": [
        { "team_id": 100, "kills": 2, "gold": 13200, "turrets": 0 },
        { "team_id": 200, "kills": 1, "gold": 12750, "turrets": 0 }
      ]
    }
  ],
  "events": [
    {
      "id": "e12",
      "timestamp_ms": 412233,
      "kind": "turret",
      "raw_type": "BUILDING_KILL",
      "team_id": 100,
      "killer_participant_id": 3,
      "x": 8040,
      "y": 7600,
      "detail": "OUTER_TURRET"
    }
  ],
  "availability": {
    "positions": true,
    "positions_reason": null,
    "control_estimate": true,
    "control_reason": null
  }
}
```

The frontend remains tolerant of schema-1 `availability.reason`, but new Rust
responses separate `positions_reason` from `control_reason`. A missing control
model can no longer suppress valid movement, score, or event data.

## Backend normalization and safety

The Match-V5 detail response supplies map ID, match game version, participant
teams/champions, and the final scoreboard. Timeline supplies sampled positions,
gold, level, CS, and supported events.

`get_match_timeline`:

1. validates the match identifier;
2. requires the match in the current identity's RAM cache;
3. uses the correct Match-V5 regional route;
4. performs one authenticated Rust request with a streaming 16 MiB body cap;
5. tolerates missing or extra fields on supported event types;
6. joins participant IDs to the cached teams and champions;
7. selects a map-specific projection or a padded observed extent;
8. fails control estimation closed when the calibration guards do not pass;
9. caches only the compact result in a three-entry volatile LRU.

Unknown event types are not copied into the compact event stream. Known major
events include champion kills, Dragons, Baron, Void Grubs, Rift Herald,
Atakhan, turrets, and inhibitors when Riot supplies their fields.

API credentials remain in Rust configuration or Windows Credential Manager.
They are never serialized into replay JSON, frontend code, assets, ZIPs, or
logs.

## Map registry and terrain

Aura explicitly registers map IDs; it never infers geometry from queue ID:

- map 11: Summoner's Rift, `summoners_rift_rect_v1`, 0..15000 projection;
- map 12: Howling Abyss, `howling_abyss_rect_v1`, empirical 0..12800 projection;
- every other map: padded `observed_extent_v1`, movement only.

The app bundles pinned official Data Dragon terrain for map 11 and map 12. The
canonical pinned version, filenames, source URLs, byte sizes, and SHA-256
hashes live in `src/assets/maps/manifest.json`; renderer metadata and
`src/assets/maps/README.txt` are generated from that manifest. The shared map
asset verifier runs before local publisher builds and in the Windows release
workflow. The images are loaded from the packaged frontend and require no
runtime network or disk write. A missing or invalid image produces an explicit
terrain error and the replay continues on its neutral coordinate grid.

The match's `game_version` is included in schema 2. If its major/minor patch
family differs from the bundled terrain version, the UI visibly calls the map
a reference instead of implying a patch-perfect terrain match.

Data Dragon minimaps are visual references. Riot does not publish brush,
walkability, fog-of-war, or vision polygons through Match-V5. Aura therefore
does not claim that an icon is exactly inside a bush.

## Summoner's Rift estimate

`summoners_rift_radial_v1` retains the existing two-dimensional pressure field.
Each champion contributes bounded team-signed radial influence weighted by
level and gold. Static base anchors stabilize early frames. Aura integrates a
small grid to produce Blue/Red percentages and draws the smoothed zero-pressure
frontier. The cached per-frame samples drive both the map and the graph.

This remains **Estimated Map Control**, not measured vision control.

## Howling Abyss estimate

`howling_abyss_linear_v1` is a separate one-dimensional model. It is never the
Summoner's Rift field with a different gate.

The backend first looks for an early frame no later than 30 seconds with at
least three valid positions for each team. Team medians must form compact,
well-separated clusters near opposite ends of the calibrated map. If any guard
fails, Aura keeps movement playback and reports why lane control is unavailable.

When anchors pass:

1. raw 2D positions still place champion icons on the official minimap;
2. each position is projected onto the blue-base-to-red-base bridge axis;
3. bounded level/gold-weighted kernels form a signed pressure curve along the
   bridge;
4. a single zero crossing becomes a straight frontier perpendicular to the
   bridge;
5. its lane position produces the Blue/Red lane-territory percentage;
6. multiple crossings are labeled **Split pressure** and suppress the false
   single frontier; the percentage falls back to integrated pressure.

The UI calls this **Estimated Lane Control**. It does not imply two-dimensional
territory, pathing truth, vision coverage, or exact brush occupancy.

## Empirical calibration evidence

Riot's formal Timeline DTO documents integer `x` and `y` fields but no bounds,
origin, minimap transform, or base coordinates. Aura therefore labels the map-12
projection as its own empirical calibration, not a Riot guarantee.

The v0.11 calibration check used eight real current queue-450 Match-V5
timelines through a valid runtime credential. No match IDs, PUUIDs, Riot IDs,
or API keys were retained. Across those samples:

- observed champion positions ranged from x=846..12321 and y=860..11716;
- repeated early blue centroids were approximately (1041,985);
- repeated early red centroids were approximately (11965,11521);
- a second observed variant remained close at blue (1088,977) and red
  (12002,11396).

Those points align with the termini of the official 512x512 map-12 minimap when
projected on 0..12800. The previous global 0..15000 transform visibly compressed
ARAM movement toward the center.

## Playback behavior and limits

- Snapshot-to-snapshot movement is interpolated for display, not claimed as a
  continuous Riot path.
- Large transitions to a calibrated base area snap instead of drawing an
  impossible flight. On Howling Abyss this can represent transport, death, or
  another base return; Match-V5 does not prove a recall cast or exact cause.
- Temporarily missing coordinates carry the last valid position into control
  preprocessing only after both teams have been observed.
- Gold interpolates; kills, turrets, and objective banners advance at their
  recorded event timestamps.
- Rendering is device-pixel-ratio aware, capped at 1.5x, and limited to 24 FPS
  while visible and playing.
- Pausing, collapsing the report, changing pages, hiding the document, or
  destroying the match panel cancels animation work.

## Official references

- <https://developer.riotgames.com/apis#match-v5/GET_getTimeline>
- <https://developer.riotgames.com/apis#match-v5/GET_getMatch>
- <https://developer.riotgames.com/docs/lol>
- <https://static.developer.riotgames.com/docs/lol/maps.json>
- <https://ddragon.leagueoflegends.com/api/versions.json>
- <https://www.leagueoflegends.com/en-us/news/game-updates/aram-2023-preview/>
