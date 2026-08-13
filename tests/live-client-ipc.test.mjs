import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createIpcClient,
  LIVE_CLIENT_EVENTS,
  normalizeGameStatus,
  normalizeLiveGameTick,
  normalizePlayerStats,
  subscribeLiveClientEvents,
} from '../src/services/ipc.js';
import {
  buildLiveGameViewModel,
  formatGameClock,
  renderLiveGameView,
} from '../src/components/analytics/live-game-metrics.js';

const completeTick = {
  gameTime: 754.8,
  activePlayer: {
    summonerName: 'ShadowStriker',
    championName: 'Briar',
    currentGold: 1475,
    kda: { kills: 7, deaths: 2, assists: 5 },
    dpm: 0,
    level: 11,
    creepScore: 142,
    creepScorePerMinute: 11.3,
    killParticipationPercent: 63.2,
    observableHeldValue: 5200,
    observableValuePerMinute: 413.4,
    earnedGoldPerMinute: 0,
    xpProgressPercent: null,
  },
  teamGoldDelta: 0,
  objectives: {
    dragonType: 'Infernal',
    dragonTimer: 83,
    baronTimer: 0,
  },
  metricAvailability: {
    currentGold: true,
    kda: true,
    dpm: false,
    teamGoldDelta: false,
    level: true,
    creepScore: true,
    creepScorePerMinute: true,
    killParticipationPercent: true,
    observableHeldValue: true,
    observableValuePerMinute: true,
    earnedGoldPerMinute: false,
    xpProgressPercent: false,
  },
  metricSources: {
    observableHeldValue: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
    observableValuePerMinute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
  },
};

test('normalizes typed game status and rejects unknown states', () => {
  assert.equal(normalizeGameStatus('IN_GAME'), 'IN_GAME');
  assert.equal(normalizeGameStatus({ status: 'champ_select' }), 'CHAMP_SELECT');
  assert.equal(normalizeGameStatus('PAUSED'), null);
  assert.equal(normalizeGameStatus(null), null);
});

test('normalizes live tick camelCase and snake_case payloads without NaN or negative counters', () => {
  assert.deepEqual(normalizeLiveGameTick(completeTick), completeTick);

  const normalized = normalizeLiveGameTick({
    game_time: Number.NaN,
    active_player: {
      summoner_name: '<img src=x onerror=alert(1)>',
      champion_name: 'Briar',
      current_gold: -20,
      kda: { kills: -2, deaths: '3', assists: 'bad' },
      dpm: Number.POSITIVE_INFINITY,
      level: -1,
      creep_score: '140',
      creep_score_per_minute: Number.NaN,
      kill_participation_percent: 180,
      observable_held_value: -10,
      observable_value_per_minute: '410.5',
      earned_gold_per_minute: Number.POSITIVE_INFINITY,
      xp_progress_percent: null,
    },
    team_gold_delta: '-55.9',
    objectives: { dragon_type: null, dragon_timer: -4, baron_timer: '15' },
    metric_availability: {
      current_gold: false,
      kda: false,
      dpm: false,
      team_gold_delta: false,
      level: false,
      creep_score: true,
      creep_score_per_minute: false,
      kill_participation_percent: true,
      observable_held_value: false,
      observable_value_per_minute: true,
      earned_gold_per_minute: false,
      xp_progress_percent: false,
    },
    metric_sources: {
      observable_held_value: 'UNTRUSTED_SOURCE',
      observable_value_per_minute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
    },
  });

  assert.equal(normalized.gameTime, 0);
  assert.equal(normalized.activePlayer.summonerName, '<img src=x onerror=alert(1)>');
  assert.equal(normalized.activePlayer.currentGold, 0);
  assert.deepEqual(normalized.activePlayer.kda, { kills: 0, deaths: 3, assists: 0 });
  assert.equal(normalized.activePlayer.dpm, 0);
  assert.equal(normalized.activePlayer.level, 0);
  assert.equal(normalized.activePlayer.creepScore, 140);
  assert.equal(normalized.activePlayer.creepScorePerMinute, 0);
  assert.equal(normalized.activePlayer.killParticipationPercent, 100);
  assert.equal(normalized.activePlayer.observableHeldValue, 0);
  assert.equal(normalized.activePlayer.observableValuePerMinute, 410.5);
  assert.equal(normalized.activePlayer.earnedGoldPerMinute, 0);
  assert.equal(normalized.activePlayer.xpProgressPercent, null);
  assert.equal(normalized.teamGoldDelta, -55);
  assert.deepEqual(normalized.objectives, { dragonType: null, dragonTimer: 0, baronTimer: 15 });
  assert.deepEqual(normalized.metricAvailability, {
    currentGold: false,
    kda: false,
    dpm: false,
    teamGoldDelta: false,
    level: false,
    creepScore: true,
    creepScorePerMinute: false,
    killParticipationPercent: true,
    observableHeldValue: false,
    observableValuePerMinute: true,
    earnedGoldPerMinute: false,
    xpProgressPercent: false,
  });
  assert.deepEqual(normalized.metricSources, {
    observableHeldValue: null,
    observableValuePerMinute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
  });
  assert.equal(normalizeLiveGameTick([]), null);
});

test('normalizes medium-frequency player updates and validates team values', () => {
  assert.deepEqual(normalizePlayerStats({
    summoner_name: 'Player',
    champion_name: 'Ahri',
    team: 'order',
    level: '11',
    creep_score: 142.9,
    items: [6655, 3020, 0, -1, '3089', Number.NaN],
  }), {
    summonerName: 'Player',
    championName: 'Ahri',
    team: 'ORDER',
    level: 11,
    creepScore: 142,
    items: [6655, 3020, 3089],
  });
  assert.equal(normalizePlayerStats({ team: 'SPECTATOR' }), null);
});

test('IPC client validates dynamic commands and converts synchronous throws to rejections', async () => {
  const calls = [];
  const client = createIpcClient((command, args) => {
    calls.push({ command, args });
    if (command === 'spotify_play') throw new Error('native failure');
    return { ok: true };
  });

  assert.deepEqual(await client.invokeWithTimeout('show_overlay', {}, 100), { ok: true });
  await assert.rejects(client.invoke('spotify_play'), /native failure/);
  await assert.rejects(client.invoke('bad-command'), /Invalid Tauri IPC command name/);
  assert.deepEqual(calls.map(({ command }) => command), ['show_overlay', 'spotify_play']);
});

test('live subscriptions route normalized payloads and clean up every listener once', async () => {
  const callbacks = new Map();
  const removed = [];
  const statuses = [];
  const ticks = [];
  const players = [];
  const listen = async (eventName, callback) => {
    callbacks.set(eventName, callback);
    return () => { removed.push(eventName); };
  };

  const dispose = await subscribeLiveClientEvents(listen, {
    onGameStatus: (status) => statuses.push(status),
    onGameTick: (tick) => ticks.push(tick),
    onPlayerUpdate: (player) => players.push(player),
  });

  callbacks.get(LIVE_CLIENT_EVENTS.gameStatus)({ payload: 'IN_GAME' });
  callbacks.get(LIVE_CLIENT_EVENTS.gameTick)({ payload: completeTick });
  callbacks.get(LIVE_CLIENT_EVENTS.playerUpdate)({
    payload: {
      summonerName: 'Player', championName: 'Ahri', team: 'HARMONY', level: 8,
      creepScore: 80, items: [1056],
    },
  });

  assert.deepEqual(statuses, ['IN_GAME']);
  assert.deepEqual(ticks, [completeTick]);
  assert.equal(players[0].team, 'HARMONY');

  await dispose();
  await dispose();
  assert.deepEqual(removed.sort(), [
    LIVE_CLIENT_EVENTS.gameStatus,
    LIVE_CLIENT_EVENTS.gameTick,
    LIVE_CLIENT_EVENTS.playerUpdate,
  ].sort());
});

test('a partial listener registration failure tears down listeners already opened', async () => {
  const removed = [];
  let calls = 0;
  const listen = async (eventName) => {
    calls += 1;
    if (calls === 2) throw new Error('registration failed');
    return () => { removed.push(eventName); };
  };

  await assert.rejects(
    subscribeLiveClientEvents(listen, {}),
    /registration failed/,
  );
  assert.deepEqual(removed, [LIVE_CLIENT_EVENTS.gameStatus]);
});

test('view model renders measured metrics and explicitly labels unavailable sentinels', () => {
  assert.equal(formatGameClock(754.8), '12:34');

  const measured = buildLiveGameViewModel(completeTick, 'IN_GAME');
  assert.equal(measured.kda, '7 / 2 / 5');
  assert.equal(measured.level, 'Level 11');
  assert.equal(measured.creepScore, '142');
  assert.equal(measured.creepScorePerMinute, '11.3');
  assert.equal(measured.killParticipation, '63.2%');
  assert.equal(measured.observableHeldValue, '5200g');
  assert.equal(measured.observableValuePerMinute, '413g');
  assert.equal(measured.earnedGoldPerMinute, 'Unavailable');
  assert.equal(measured.dpm, 'Unavailable');
  assert.equal(measured.goldDelta, 'Unavailable');
  assert.equal(measured.xpProgress, 'Unavailable from Live Client API');
  assert.match(measured.integrityNote, /Held value\/min is an estimate/);
  assert.equal(measured.dragonTimer, '01:23');
  assert.equal(measured.baronTimer, 'READY');

  const unavailable = buildLiveGameViewModel({
    ...completeTick,
    activePlayer: {
      ...completeTick.activePlayer,
      observableValuePerMinute: 0,
    },
    metricAvailability: {
      ...completeTick.metricAvailability,
      observableValuePerMinute: false,
    },
    metricSources: {
      ...completeTick.metricSources,
      observableValuePerMinute: null,
    },
  });
  assert.equal(unavailable.dpm, 'Unavailable');
  assert.equal(unavailable.goldDelta, 'Unavailable');
  assert.equal(unavailable.goldDeltaTone, 'unavailable');
  assert.equal(unavailable.observableValuePerMinute, 'Unavailable');

  const missingBasics = buildLiveGameViewModel({
    ...completeTick,
    activePlayer: {
      ...completeTick.activePlayer,
      currentGold: 0,
      kda: { kills: 0, deaths: 0, assists: 0 },
    },
    metricAvailability: {
      ...completeTick.metricAvailability,
      currentGold: false,
      kda: false,
    },
  });
  assert.equal(missingBasics.currentGold, 'Unavailable');
  assert.equal(missingBasics.kda, 'Unavailable');
});

test('DOM adapter writes live payload strings through textContent only', () => {
  const makeElement = () => {
    const classes = new Set();
    return {
      textContent: '',
      classList: {
        toggle(name, force) { if (force) classes.add(name); else classes.delete(name); },
        contains(name) { return classes.has(name); },
      },
    };
  };
  const elements = Object.fromEntries([
    'status', 'gameTime', 'summonerName', 'championName', 'level', 'kda',
    'creepScore', 'creepScorePerMinute', 'killParticipation', 'observableHeldValue',
    'observableValuePerMinute', 'earnedGoldPerMinute', 'dpm', 'currentGold',
    'goldDelta', 'dragonType', 'dragonTimer', 'baronTimer', 'xpProgress',
    'integrityNote',
  ].map((key) => [key, makeElement()]));
  const progressAttributes = new Map();
  elements.xpProgressBar = {
    hidden: false,
    value: 75,
    getAttribute(name) { return progressAttributes.get(name) ?? null; },
    setAttribute(name, value) { progressAttributes.set(name, String(value)); },
  };
  const viewModel = buildLiveGameViewModel({
    ...completeTick,
    activePlayer: { ...completeTick.activePlayer, championName: '<script>alert(1)</script>' },
  });

  renderLiveGameView(elements, viewModel);
  assert.equal(elements.championName.textContent, '<script>alert(1)</script>');
  assert.equal(elements.goldDelta.classList.contains('live-tone-unavailable'), true);
  assert.equal(elements.dpm.classList.contains('live-metric-unavailable'), true);
  assert.equal(elements.observableValuePerMinute.textContent, '413g');
  assert.equal(elements.observableHeldValue.textContent, '5200g');
  assert.equal(elements.earnedGoldPerMinute.textContent, 'Unavailable');
  assert.equal(elements.xpProgressBar.hidden, true);
  assert.equal(elements.xpProgressBar.value, 0);
  assert.equal(progressAttributes.get('aria-valuetext'), 'Unavailable');
});
