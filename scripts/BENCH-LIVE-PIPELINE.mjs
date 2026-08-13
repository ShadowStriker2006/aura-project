import { performance } from 'node:perf_hooks';

import { buildLiveGameViewModel } from '../src/components/analytics/live-game-metrics.js';
import { normalizeLiveGameTick } from '../src/services/ipc.js';

const ITERATIONS = 100_000;
const WARMUP_ITERATIONS = 5_000;

const sampleTick = Object.freeze({
  gameTime: 1_247.25,
  activePlayer: Object.freeze({
    summonerName: 'Aura Benchmark#TEST',
    championName: 'Briar',
    currentGold: 1_125,
    level: 13,
    creepScore: 176,
    creepScorePerMinute: 8.47,
    killParticipationPercent: 72.4,
    observableHeldValue: 8_925,
    observableValuePerMinute: 429.3,
    earnedGoldPerMinute: 0,
    xpProgressPercent: null,
    kda: Object.freeze({ kills: 8, deaths: 3, assists: 13 }),
    dpm: 0,
  }),
  teamGoldDelta: 0,
  objectives: Object.freeze({
    dragonType: 'Infernal',
    dragonTimer: 74,
    baronTimer: 0,
  }),
  metricAvailability: Object.freeze({
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
  }),
  metricSources: Object.freeze({
    observableHeldValue: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
    observableValuePerMinute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
  }),
});

function execute(iterations) {
  let checksum = 0;
  for (let index = 0; index < iterations; index += 1) {
    const tick = normalizeLiveGameTick(sampleTick);
    const view = buildLiveGameViewModel(tick, 'IN_GAME');
    checksum += view.gameTime.length + view.kda.length + String(view.level).length;
  }
  return checksum;
}

execute(WARMUP_ITERATIONS);
if (typeof globalThis.gc === 'function') globalThis.gc();

const heapBefore = process.memoryUsage().heapUsed;
const startedAt = performance.now();
const checksum = execute(ITERATIONS);
const elapsedMs = performance.now() - startedAt;
const heapAfter = process.memoryUsage().heapUsed;

const result = {
  benchmark: 'normalize tick plus build live HUD view model',
  node: process.version,
  iterations: ITERATIONS,
  elapsedMs: Number(elapsedMs.toFixed(3)),
  averageMicroseconds: Number(((elapsedMs * 1_000) / ITERATIONS).toFixed(3)),
  eventsPerSecond: Math.round((ITERATIONS * 1_000) / elapsedMs),
  heapDeltaBytes: heapAfter - heapBefore,
  checksum,
  scope: 'Synthetic frontend-only benchmark; not a League/WebView/native latency guarantee.',
};

console.log(JSON.stringify(result, null, 2));
