import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const sourceUrl = new URL('../src/map-control-replay.js', import.meta.url);
const source = await readFile(sourceUrl, 'utf8');
const replayModule = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const {
  computeControlSample,
  interpolateReplayState,
  mapControlReplayTestHooks,
  prepareControlFrames,
  replayMode,
} = replayModule;
const { mapAsset, terrainPresentation } = mapControlReplayTestHooks;

const participants = [
  { participant_id: 1, team_id: 100, champion_name: 'Ahri' },
  { participant_id: 2, team_id: 100, champion_name: 'LeeSin' },
  { participant_id: 6, team_id: 200, champion_name: 'Zed' },
  { participant_id: 7, team_id: 200, champion_name: 'Vi' },
];
const baseReplay = {
  map_id: 11,
  coordinates: { min_x: 0, max_x: 15000, min_y: 0, max_y: 15000, invert_y_for_canvas: true },
  participants,
  availability: { positions: true, control_estimate: true },
};

const aramReplay = {
  map_id: 12,
  coordinates: {
    model_id: 'howling_abyss_rect_v1',
    min_x: 0,
    max_x: 12800,
    min_y: 0,
    max_y: 12800,
    invert_y_for_canvas: true,
  },
  participants,
  availability: { positions: true, control_estimate: true },
  control_model: {
    id: 'howling_abyss_linear_v1',
    topology: 'linear_lane',
    blue_base: { x: 1065, y: 981 },
    red_base: { x: 11984, y: 11459 },
  },
};

test('pinned map metadata resolves versioned filenames from one generated manifest', () => {
  assert.deepEqual(mapAsset(11), {
    name: "Summoner's Rift",
    filename: 'map11-16.15.1.png',
  });
  assert.equal(mapAsset(12)?.filename, 'map12-16.15.1.png');
  assert.equal(mapAsset(14), null);
});

test('terrain load failure is explicit and retains the neutral coordinate replay', () => {
  const presentation = terrainPresentation(
    { ...baseReplay, game_version: '16.16.1.9' },
    { currentDdragonVersion: '16.17.1' },
    mapAsset(11),
    'error',
    true,
    false,
  );

  assert.match(presentation.label, /terrain failed to load/i);
  assert.match(presentation.label, /neutral coordinate grid/i);
  assert.match(presentation.canvasLabel, /neutral coordinate grid/i);
  assert.equal(presentation.mismatch, false);
});

test('loaded pinned terrain visibly reports match and current-catalog drift', () => {
  const presentation = terrainPresentation(
    { ...baseReplay, game_version: '16.16.1.9' },
    { currentDdragonVersion: '16.17.1' },
    mapAsset(11),
    'ready',
    true,
    false,
  );

  assert.match(presentation.label, /Data Dragon 16\.15\.1/);
  assert.match(presentation.label, /match 16\.16/);
  assert.match(presentation.label, /current game data 16\.17/);
  assert.equal(presentation.mismatch, true);
});

test('terrain listeners are registered before the image source is assigned', () => {
  const errorListener = source.indexOf("mapImage.addEventListener('error'");
  const sourceAssignment = source.indexOf('mapImage.src = assetUrl;');
  assert.ok(errorListener >= 0, 'expected an explicit terrain error listener');
  assert.ok(sourceAssignment > errorListener, 'terrain listeners must be attached before src');
});

test('an uncalibrated map with positions selects movement-only replay', () => {
  assert.equal(replayMode({
    ...baseReplay,
    map_id: 12,
    frames: [{ timestamp_ms: 0, players: [{ participant_id: 1, x: 1000, y: 1000 }] }],
    availability: { positions: true, control_estimate: false },
  }), 'movement');
});

test('positionless frames remain unavailable', () => {
  assert.equal(replayMode({
    ...baseReplay,
    frames: [{ timestamp_ms: 0, players: [] }],
    availability: { positions: false, control_estimate: false },
  }), 'unavailable');
});

test('control mode requires both positional samples and an explicit estimate flag', () => {
  const frames = [{ timestamp_ms: 0, players: [{ participant_id: 1, x: 1000, y: 1000 }] }];
  assert.equal(replayMode({ ...baseReplay, frames }), 'control');
  assert.equal(replayMode({ ...baseReplay, frames, availability: undefined }), 'movement');
});

test('mirrored teams produce a stable near-even control estimate', () => {
  const sample = computeControlSample({
    timestamp_ms: 0,
    players: [
      { participant_id: 1, x: 4500, y: 4500, level: 8, total_gold: 5000 },
      { participant_id: 2, x: 6500, y: 3500, level: 8, total_gold: 5000 },
      { participant_id: 6, x: 10500, y: 10500, level: 8, total_gold: 5000 },
      { participant_id: 7, x: 8500, y: 11500, level: 8, total_gold: 5000 },
    ],
  }, baseReplay);

  assert.ok(Number.isFinite(sample.blue));
  assert.equal(sample.blue, 50);
  assert.equal(Math.round((sample.blue + sample.red) * 1000) / 1000, 100);
  assert.equal(sample.frontier.length, 22);
});

test('a stronger blue advance moves the estimate toward blue', () => {
  const even = computeControlSample({
    players: [
      { participant_id: 1, x: 4500, y: 4500, level: 8, total_gold: 5000 },
      { participant_id: 2, x: 5500, y: 4500, level: 8, total_gold: 5000 },
      { participant_id: 6, x: 10500, y: 10500, level: 8, total_gold: 5000 },
      { participant_id: 7, x: 9500, y: 10500, level: 8, total_gold: 5000 },
    ],
  }, baseReplay);
  const blueAdvance = computeControlSample({
    players: [
      { participant_id: 1, x: 10500, y: 10500, level: 14, total_gold: 12000 },
      { participant_id: 2, x: 9500, y: 11000, level: 14, total_gold: 12000 },
      { participant_id: 6, x: 12500, y: 12500, level: 9, total_gold: 7000 },
      { participant_id: 7, x: 13000, y: 11500, level: 9, total_gold: 7000 },
    ],
  }, baseReplay);

  assert.ok(blueAdvance.blue > even.blue + 5,
    `expected blue pressure to rise: ${even.blue} -> ${blueAdvance.blue}`);
});

test('Howling Abyss uses a straight single-lane frontier near even teams', () => {
  const sample = computeControlSample({
    timestamp_ms: 60000,
    players: [
      { participant_id: 1, x: 4300, y: 4000, level: 8, total_gold: 5000 },
      { participant_id: 2, x: 5000, y: 4700, level: 8, total_gold: 5000 },
      { participant_id: 6, x: 8800, y: 8500, level: 8, total_gold: 5000 },
      { participant_id: 7, x: 8100, y: 7800, level: 8, total_gold: 5000 },
    ],
  }, aramReplay);

  assert.equal(sample.topology, 'linear_lane');
  assert.equal(sample.split, false);
  assert.equal(sample.frontier.length, 0);
  assert.equal(sample.frontier_segment.length, 2);
  assert.ok(sample.blue > 45 && sample.blue < 55, `expected near-even lane control, got ${sample.blue}`);
});

test('Howling Abyss blue advance pushes the lane frontier toward red base', () => {
  const even = computeControlSample({
    players: [
      { participant_id: 1, x: 4300, y: 4000, level: 8, total_gold: 5000 },
      { participant_id: 2, x: 5000, y: 4700, level: 8, total_gold: 5000 },
      { participant_id: 6, x: 8800, y: 8500, level: 8, total_gold: 5000 },
      { participant_id: 7, x: 8100, y: 7800, level: 8, total_gold: 5000 },
    ],
  }, aramReplay);
  const advance = computeControlSample({
    players: [
      { participant_id: 1, x: 8800, y: 8400, level: 14, total_gold: 12000 },
      { participant_id: 2, x: 9400, y: 9000, level: 14, total_gold: 12000 },
      { participant_id: 6, x: 10800, y: 10300, level: 9, total_gold: 7000 },
      { participant_id: 7, x: 11200, y: 10700, level: 9, total_gold: 7000 },
    ],
  }, aramReplay);

  assert.ok(advance.blue > even.blue + 15,
    `expected blue lane control to rise: ${even.blue} -> ${advance.blue}`);
});

test('Howling Abyss split pressure suppresses a fabricated single frontier', () => {
  const sample = computeControlSample({
    players: [
      { participant_id: 1, x: 3795, y: 3601, level: 10, total_gold: 7000 },
      { participant_id: 2, x: 9254, y: 8840, level: 10, total_gold: 7000 },
      { participant_id: 6, x: 5651, y: 5381, level: 10, total_gold: 7000 },
      { participant_id: 7, x: 7398, y: 7050, level: 10, total_gold: 7000 },
    ],
  }, aramReplay);

  assert.equal(sample.split, true);
  assert.deepEqual(sample.frontier_segment, []);
  assert.ok(Number.isFinite(sample.blue));
});

test('playback interpolates positions and gold but steps kills at event time', () => {
  const replay = {
    ...baseReplay,
    events: [{ kind: 'champion_kill', team_id: 100, timestamp_ms: 600 }],
    frames: [
      {
        timestamp_ms: 0,
        players: [{ participant_id: 1, x: 1000, y: 1000, total_gold: 500 }],
        teams: [{ team_id: 100, gold: 2500 }, { team_id: 200, gold: 2500 }],
      },
      {
        timestamp_ms: 1000,
        players: [{ participant_id: 1, x: 3000, y: 4000, total_gold: 1500 }],
        teams: [{ team_id: 100, gold: 3500 }, { team_id: 200, gold: 3000 }],
      },
    ],
  };
  const beforeKill = interpolateReplayState(replay, 500);
  const afterKill = interpolateReplayState(replay, 750);

  assert.equal(beforeKill.players[0].x, 2000);
  assert.equal(beforeKill.players[0].y, 2500);
  assert.equal(beforeKill.teams[0].gold, 3000);
  assert.equal(beforeKill.teams[0].kills, 0);
  assert.equal(afterKill.teams[0].kills, 1);
});

test('long-distance recalls and teleports snap instead of flying across terrain', () => {
  const replay = {
    ...baseReplay,
    events: [],
    frames: [
      { timestamp_ms: 0, players: [{ participant_id: 1, x: 1000, y: 1000 }], teams: [] },
      { timestamp_ms: 1000, players: [{ participant_id: 1, x: 12000, y: 12000 }], teams: [] },
    ],
  };

  assert.equal(interpolateReplayState(replay, 250).players[0].x, 1000);
  assert.equal(interpolateReplayState(replay, 750).players[0].x, 12000);
});

test('long normal movement away from base remains smoothly interpolated', () => {
  const replay = {
    ...baseReplay,
    events: [],
    frames: [
      { timestamp_ms: 0, players: [{ participant_id: 1, x: 5000, y: 5000 }], teams: [] },
      { timestamp_ms: 60000, players: [{ participant_id: 1, x: 12000, y: 12000 }], teams: [] },
    ],
  };

  const midpoint = interpolateReplayState(replay, 30000).players[0];
  assert.equal(midpoint.x, 8500);
  assert.equal(midpoint.y, 8500);
});

test('calibrated Howling Abyss base transports snap instead of crossing the bridge', () => {
  const replay = {
    ...aramReplay,
    events: [],
    frames: [
      { timestamp_ms: 0, players: [{ participant_id: 1, x: 9000, y: 8600 }], teams: [] },
      { timestamp_ms: 60000, players: [{ participant_id: 1, x: 1065, y: 981 }], teams: [] },
    ],
  };

  assert.equal(interpolateReplayState(replay, 15000).players[0].x, 9000);
  assert.equal(interpolateReplayState(replay, 45000).players[0].x, 1065);
});

test('uncalibrated maps do not apply Summoners Rift base snapping', () => {
  const replay = {
    ...baseReplay,
    map_id: 12,
    availability: { positions: true, control_estimate: false },
    events: [],
    frames: [
      { timestamp_ms: 0, players: [{ participant_id: 1, x: 1000, y: 1000 }], teams: [] },
      { timestamp_ms: 1000, players: [{ participant_id: 1, x: 12000, y: 12000 }], teams: [] },
    ],
  };

  assert.equal(interpolateReplayState(replay, 250).players[0].x, 3750);
  assert.equal(interpolateReplayState(replay, 750).players[0].x, 9250);
});

test('control samples carry forward a temporarily missing participant position', () => {
  const initialPlayers = [
    { participant_id: 1, x: 4500, y: 4500 },
    { participant_id: 2, x: 5500, y: 4500 },
    { participant_id: 6, x: 10500, y: 10500 },
    { participant_id: 7, x: 9500, y: 10500 },
  ];
  const prepared = prepareControlFrames([
    { timestamp_ms: 0, players: initialPlayers },
    { timestamp_ms: 60000, players: [{ participant_id: 1, x: 6000, y: 6000 }] },
  ], baseReplay);

  assert.equal(prepared[1].players.length, 4);
  assert.deepEqual(
    prepared[1].players.find((player) => player.participant_id === 6),
    initialPlayers[2],
  );
});
