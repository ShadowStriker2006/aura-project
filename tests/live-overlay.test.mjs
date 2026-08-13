import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  buildLiveOverlayViewModel,
  emptyLiveOverlayViewModel,
  normalizeOverlayLayout,
  renderLiveOverlayView,
} from '../src/components/overlay/live-overlay.js';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const fixtureTick = {
  gameTime: 754.8,
  activePlayer: {
    summonerName: 'Aura Fixture',
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
  objectives: { dragonType: 'Infernal', dragonTimer: 28, baronTimer: 94 },
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

test('overlay view model merges CS and rate while keeping unsupported telemetry honest', () => {
  const model = buildLiveOverlayViewModel(fixtureTick, 'IN_GAME');
  assert.equal(model.status, 'In Game');
  assert.equal(model.gameTime, '12:34');
  assert.equal(model.level, 'Level 11');
  assert.equal(model.kda, '7 / 2 / 5');
  assert.equal(model.currentGold, '1475g');
  assert.equal(model.csCombined, '142 (11.3/m)');
  assert.equal(model.killParticipation, '63.2%');
  assert.equal(model.observableValuePerMinute, '413g');
  assert.equal(model.earnedGoldPerMinute, 'N/A');
  assert.equal(model.dpm, 'N/A');
  assert.equal(model.goldDelta, 'N/A');
  assert.equal(model.xpProgress, 'N/A');
  assert.equal(model.xpProgressAvailable, false);
  assert.equal(model.dragonTone, 'warning');
  assert.equal(model.baronTone, 'muted');
});

test('partial CS data is labelled rather than fabricated', () => {
  const tick = {
    ...fixtureTick,
    metricAvailability: {
      ...fixtureTick.metricAvailability,
      creepScorePerMinute: false,
    },
  };
  assert.equal(buildLiveOverlayViewModel(tick).csCombined, '142 (rate unavailable)');
});

test('empty overlay state does not retain stale match data', () => {
  const empty = emptyLiveOverlayViewModel('Game Ended');
  assert.equal(empty.status, 'Game Ended');
  assert.equal(empty.gameTime, '00:00');
  assert.equal(empty.champion, 'No active match');
  assert.equal(empty.csCombined, 'N/A');
  assert.equal(empty.goldDelta, 'N/A');
  assert.equal(empty.xpProgress, 'N/A');
  assert.equal(empty.xpProgressValue, null);
  assert.equal(empty.dragonTimer, 'READY');
  assert.equal(empty.baronTimer, 'READY');
});

test('native layout values are normalized to safe modes, opacity, and scale presets', () => {
  assert.deepEqual(normalizeOverlayLayout({
    mode: 'expanded',
    scalePercent: 90,
    opacityPercent: 25,
    locked: false,
  }), {
    mode: 'expanded',
    scalePercent: 90,
    opacityPercent: 40,
    locked: false,
  });

  assert.deepEqual(normalizeOverlayLayout({
    mode: 'invalid',
    scale_percent: 80,
    opacity_percent: 150,
  }, {
    mode: 'compact',
    scalePercent: 75,
    opacityPercent: 60,
    locked: false,
  }), {
    mode: 'compact',
    scalePercent: 75,
    opacityPercent: 100,
    locked: false,
  });
});

test('overlay renderer performs no repeated text or class writes for an unchanged tick', () => {
  let writes = 0;
  const makeElement = () => {
    let text = '';
    const classes = new Set();
    return {
      get textContent() { return text; },
      set textContent(value) { text = value; writes += 1; },
      classList: {
        contains(name) { return classes.has(name); },
        toggle(name, enabled) {
          writes += 1;
          if (enabled) classes.add(name); else classes.delete(name);
        },
      },
    };
  };
  const elements = Object.fromEntries([
    'status', 'gameTime', 'champion', 'summoner', 'level', 'kda', 'currentGold',
    'csCombined', 'killParticipation', 'observableValuePerMinute',
    'earnedGoldPerMinute', 'dpm', 'goldDelta', 'xpProgress', 'dragonType',
    'dragonTimer', 'baronTimer',
  ].map((key) => [key, makeElement()]));
  const model = buildLiveOverlayViewModel({
    ...fixtureTick,
    activePlayer: { ...fixtureTick.activePlayer, championName: '<script>unsafe()</script>' },
  });

  renderLiveOverlayView(elements, model);
  const firstRenderWrites = writes;
  assert.ok(firstRenderWrites > 0);
  assert.equal(elements.champion.textContent, '<script>unsafe()</script>');

  renderLiveOverlayView(elements, model);
  assert.equal(writes, firstRenderWrites);
});

test('overlay markup exposes compact essentials, merged telemetry, and accessible controls', async () => {
  const html = await readFile(path.join(projectRoot, 'src/overlay.html'), 'utf8');
  assert.match(html, /data-overlay-mode="standby"/);
  assert.match(html, /id="overlay-game-time"/);
  assert.match(html, /id="overlay-kda"/);
  assert.match(html, /id="overlay-team-gold"/);
  assert.match(html, /id="overlay-dragon"/);
  assert.match(html, /id="overlay-baron"/);
  assert.match(html, /id="overlay-cs-combined"/);
  assert.match(html, /id="overlay-opacity"[^>]+min="40"[^>]+max="100"/);
  assert.match(html, /data-scale="75"/);
  assert.match(html, /data-scale="90"/);
  assert.match(html, /data-scale="100"/);
  assert.match(html, /aria-controls="overlay-expanded-panel"/);
  assert.doesNotMatch(html, /id="overlay-cs"/);
  assert.doesNotMatch(html, /id="overlay-cs-minute"/);
});

test('overlay stylesheet uses one flat alpha surface without blur filters', async () => {
  const css = await readFile(path.join(projectRoot, 'src/style.css'), 'utf8');
  const overlayCss = css.slice(css.indexOf('AURA LIVE OVERLAY - COMPACT EDGE HUD'));
  assert.match(overlayCss, /--overlay-opacity:\s*\.55/);
  assert.match(overlayCss, /background:\s*rgba\(5, 8, 15, var\(--overlay-opacity\)\)/);
  assert.match(overlayCss, /\.overlay-detail-metric[\s\S]*?background:\s*rgba\(255, 255, 255, \.018\)/);
  assert.match(overlayCss, /data-overlay-mode="expanded"\][\s\S]*?\.overlay-ribbon\s*\{\s*height:\s*150px;/);
  assert.doesNotMatch(overlayCss, /backdrop-filter\s*:/);
  assert.doesNotMatch(overlayCss, /filter\s*:/);
});

test('overlay provides a local Escape-to-lock shortcut with listener cleanup', async () => {
  const source = await readFile(path.join(projectRoot, 'src/overlay.js'), 'utf8');
  assert.match(source, /event\.key !== 'Escape'/);
  assert.match(source, /window\.addEventListener\('keydown', onKeyDown\)/);
  assert.match(source, /window\.removeEventListener\('keydown', onKeyDown\)/);
  assert.match(source, /invoke\('toggle_overlay_interaction'\)/);
});
