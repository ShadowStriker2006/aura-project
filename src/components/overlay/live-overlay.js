import { buildLiveGameViewModel } from '../analytics/live-game-metrics.js';

export const OVERLAY_LAYOUT_MODES = Object.freeze(['standby', 'compact', 'expanded']);
export const OVERLAY_SCALE_PRESETS = Object.freeze([75, 90, 100]);

const timerTone = (seconds) => {
  const remaining = Math.max(0, Number(seconds) || 0);
  if (remaining === 0) return 'ready';
  return remaining <= 30 ? 'warning' : 'muted';
};

const clampInteger = (value, fallback, minimum, maximum) => {
  const parsed = Math.round(Number(value));
  return Number.isFinite(parsed)
    ? Math.min(maximum, Math.max(minimum, parsed))
    : fallback;
};

const normalizeScale = (value, fallback = 100) => {
  const parsed = clampInteger(value, fallback, 75, 100);
  return OVERLAY_SCALE_PRESETS.includes(parsed) ? parsed : fallback;
};

/** Normalize native or test-fixture layout values before applying them to CSS. */
export function normalizeOverlayLayout(value, fallback = {}) {
  const source = value && typeof value === 'object' ? value : {};
  const fallbackMode = OVERLAY_LAYOUT_MODES.includes(fallback.mode)
    ? fallback.mode
    : 'standby';
  const mode = OVERLAY_LAYOUT_MODES.includes(source.mode) ? source.mode : fallbackMode;
  return {
    mode,
    scalePercent: normalizeScale(source.scalePercent ?? source.scale_percent, fallback.scalePercent || 100),
    opacityPercent: clampInteger(
      source.opacityPercent ?? source.opacity_percent,
      fallback.opacityPercent || 55,
      40,
      100,
    ),
    locked: typeof source.locked === 'boolean' ? source.locked : fallback.locked !== false,
  };
}

const combineCreepScore = (live) => {
  if (live.creepScoreAvailable && live.creepScorePerMinuteAvailable) {
    return `${live.creepScore} (${live.creepScorePerMinute}/m)`;
  }
  if (live.creepScoreAvailable) return `${live.creepScore} (rate unavailable)`;
  if (live.creepScorePerMinuteAvailable) return `CS unavailable (${live.creepScorePerMinute}/m)`;
  return 'Unavailable';
};

export function buildLiveOverlayViewModel(tick, status = 'IN_GAME') {
  const live = buildLiveGameViewModel(tick, status);
  return {
    status: live.status,
    gameTime: live.gameTime,
    champion: live.championName,
    summoner: live.summonerName,
    level: live.level,
    levelAvailable: live.levelAvailable,
    kda: live.kdaAvailable ? live.kda : 'N/A',
    kdaAvailable: live.kdaAvailable,
    currentGold: live.currentGoldAvailable ? live.currentGold : 'N/A',
    currentGoldAvailable: live.currentGoldAvailable,
    creepScore: live.creepScore,
    creepScorePerMinute: live.creepScorePerMinute,
    csCombined: combineCreepScore(live),
    csCombinedAvailable: live.creepScoreAvailable || live.creepScorePerMinuteAvailable,
    killParticipation: live.killParticipation,
    killParticipationAvailable: live.killParticipationAvailable,
    observableValuePerMinute: live.observableValuePerMinute,
    observableValuePerMinuteAvailable: live.observableValuePerMinuteAvailable,
    earnedGoldPerMinute: live.earnedGoldPerMinuteAvailable ? live.earnedGoldPerMinute : 'N/A',
    earnedGoldPerMinuteAvailable: live.earnedGoldPerMinuteAvailable,
    dpm: live.dpmAvailable ? live.dpm : 'N/A',
    dpmAvailable: live.dpmAvailable,
    goldDelta: live.goldDeltaTone === 'unavailable' ? 'N/A' : live.goldDelta,
    goldDeltaTone: live.goldDeltaTone,
    goldDeltaAvailable: live.goldDeltaTone !== 'unavailable',
    xpProgress: live.xpProgressAvailable ? live.xpProgress : 'N/A',
    xpProgressValue: live.xpProgressValue,
    xpProgressAvailable: live.xpProgressAvailable,
    dragonType: live.dragonType,
    dragonTimer: live.dragonTimer,
    dragonTone: timerTone(tick.objectives.dragonTimer),
    baronTimer: live.baronTimer,
    baronTone: timerTone(tick.objectives.baronTimer),
  };
}

export function emptyLiveOverlayViewModel(status = 'Awaiting League Client') {
  return {
    status,
    gameTime: '00:00',
    champion: 'No active match',
    summoner: 'Aura is standing by',
    level: 'Level unavailable',
    levelAvailable: false,
    kda: 'N/A',
    kdaAvailable: false,
    currentGold: 'N/A',
    currentGoldAvailable: false,
    creepScore: 'Unavailable',
    creepScorePerMinute: 'Unavailable',
    csCombined: 'N/A',
    csCombinedAvailable: false,
    killParticipation: 'N/A',
    killParticipationAvailable: false,
    observableValuePerMinute: 'N/A',
    observableValuePerMinuteAvailable: false,
    earnedGoldPerMinute: 'N/A',
    earnedGoldPerMinuteAvailable: false,
    dpm: 'N/A',
    dpmAvailable: false,
    goldDelta: 'N/A',
    goldDeltaTone: 'unavailable',
    goldDeltaAvailable: false,
    xpProgress: 'N/A',
    xpProgressValue: null,
    xpProgressAvailable: false,
    dragonType: 'Dragon',
    dragonTimer: 'READY',
    dragonTone: 'ready',
    baronTimer: 'READY',
    baronTone: 'ready',
  };
}

const setText = (element, value) => {
  const text = String(value);
  if (element && element.textContent !== text) element.textContent = text;
};

const setAttribute = (element, name, value) => {
  const text = String(value);
  if (element?.getAttribute?.(name) !== text) element?.setAttribute?.(name, text);
};

const setClass = (element, className, enabled) => {
  if (!element?.classList) return;
  if (element.classList.contains(className) !== enabled) {
    element.classList.toggle(className, enabled);
  }
};

const setTone = (element, tone) => {
  ['ready', 'warning', 'muted'].forEach((candidate) => {
    setClass(element, `state-${candidate}`, candidate === tone);
  });
};

const setAvailability = (element, available, label) => {
  setClass(element, 'live-metric-unavailable', !available);
  if (!element) return;
  if (!available) {
    setAttribute(element, 'aria-label', `${label} unavailable`);
    setAttribute(element, 'title', 'Not available from Riot Live Client API');
    return;
  }
  if (element.getAttribute?.('aria-label') === `${label} unavailable`) {
    element.removeAttribute?.('aria-label');
  }
  if (element.getAttribute?.('title') === 'Not available from Riot Live Client API') {
    element.removeAttribute?.('title');
  }
};

const renderXpProgress = (element, model) => {
  if (!element) return;
  if (!model.xpProgressAvailable || model.xpProgressValue == null) {
    if (!element.hidden) element.hidden = true;
    if (element.value !== 0) element.value = 0;
    setAttribute(element, 'aria-label', 'XP progress unavailable');
    setAttribute(element, 'aria-valuetext', 'Unavailable');
    return;
  }
  if (element.hidden) element.hidden = false;
  if (element.value !== model.xpProgressValue) element.value = model.xpProgressValue;
  setAttribute(element, 'aria-label', 'XP progress to next level');
  setAttribute(element, 'aria-valuetext', model.xpProgress);
};

/** Render through textContent and changed-only class writes. */
export function renderLiveOverlayView(elements, model) {
  setText(elements.status, model.status);
  setText(elements.gameTime, model.gameTime);
  setText(elements.champion, model.champion);
  setText(elements.summoner, model.summoner);
  setText(elements.level, model.level);
  setText(elements.kda, model.kda);
  setText(elements.currentGold, model.currentGold);
  setText(elements.creepScore, model.creepScore);
  setText(elements.creepScorePerMinute, model.creepScorePerMinute);
  setText(elements.csCombined, model.csCombined);
  setText(elements.killParticipation, model.killParticipation);
  setText(elements.observableValuePerMinute, model.observableValuePerMinute);
  setText(elements.earnedGoldPerMinute, model.earnedGoldPerMinute);
  setText(elements.dpm, model.dpm);
  setText(elements.goldDelta, model.goldDelta);
  setText(elements.xpProgress, model.xpProgress);
  setText(elements.dragonType, model.dragonType);
  setText(elements.dragonTimer, model.dragonTimer);
  setText(elements.baronTimer, model.baronTimer);

  setTone(elements.dragonTimer, model.dragonTone);
  setTone(elements.baronTimer, model.baronTone);
  ['blue', 'red', 'even', 'unavailable'].forEach((tone) => {
    setClass(elements.goldDelta, `live-tone-${tone}`, tone === model.goldDeltaTone);
  });

  [
    ['level', model.levelAvailable, 'Level'],
    ['kda', model.kdaAvailable, 'KDA'],
    ['currentGold', model.currentGoldAvailable, 'Current gold'],
    ['csCombined', model.csCombinedAvailable, 'Creep score'],
    ['killParticipation', model.killParticipationAvailable, 'Kill participation'],
    ['observableValuePerMinute', model.observableValuePerMinuteAvailable, 'Held value per minute estimate'],
    ['earnedGoldPerMinute', model.earnedGoldPerMinuteAvailable, 'Earned gold per minute'],
    ['dpm', model.dpmAvailable, 'Damage per minute'],
    ['goldDelta', model.goldDeltaAvailable, 'Team gold lead'],
    ['xpProgress', model.xpProgressAvailable, 'XP progress'],
  ].forEach(([key, available, label]) => setAvailability(elements[key], available, label));

  renderXpProgress(elements.xpProgressBar, model);
}
