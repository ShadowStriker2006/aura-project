const STATUS_LABELS = Object.freeze({
  IN_LOBBY: 'In Lobby',
  CHAMP_SELECT: 'Champion Select',
  IN_GAME: 'In Game',
  ENDED: 'Game Ended',
});

const formatWholeNumber = (value) => String(Math.round(Number(value) || 0));
const formatOneDecimal = (value) => (Number(value) || 0).toFixed(1);

export function formatGameClock(seconds) {
  const safeSeconds = Math.max(0, Math.floor(Number(seconds) || 0));
  const minutes = Math.floor(safeSeconds / 60);
  const remainder = safeSeconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`;
}

export function formatObjectiveTimer(seconds) {
  const safeSeconds = Math.max(0, Number(seconds) || 0);
  return safeSeconds > 0 ? formatGameClock(safeSeconds) : 'READY';
}

export function buildLiveGameViewModel(tick, status = 'IN_GAME') {
  const kda = tick.activePlayer.kda;
  const availability = tick.metricAvailability || {};
  const kdaAvailable = availability.kda === true;
  const currentGoldAvailable = availability.currentGold === true;
  const dpmAvailable = availability.dpm === true;
  const goldDeltaAvailable = availability.teamGoldDelta === true;
  const levelAvailable = availability.level === true;
  const creepScoreAvailable = availability.creepScore === true;
  const creepScorePerMinuteAvailable = availability.creepScorePerMinute === true;
  const killParticipationAvailable = availability.killParticipationPercent === true;
  const observableHeldValueAvailable = availability.observableHeldValue === true
    && tick.metricSources?.observableHeldValue
      === 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE';
  const observableValuePerMinuteAvailable = availability.observableValuePerMinute === true
    && tick.metricSources?.observableValuePerMinute
      === 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE';
  const earnedGoldPerMinuteAvailable = availability.earnedGoldPerMinute === true;
  const xpProgressAvailable = availability.xpProgressPercent === true
    && tick.activePlayer.xpProgressPercent != null;
  const goldDelta = Number(tick.teamGoldDelta) || 0;
  const dragonTimer = formatObjectiveTimer(tick.objectives.dragonTimer);
  const dragonType = tick.objectives.dragonType;

  let goldDeltaText = 'Unavailable';
  let goldDeltaTone = 'unavailable';
  if (goldDeltaAvailable) {
    if (goldDelta > 0) {
      goldDeltaText = `+${formatWholeNumber(goldDelta)}g Blue lead`;
      goldDeltaTone = 'blue';
    } else if (goldDelta < 0) {
      goldDeltaText = `${formatWholeNumber(goldDelta)}g Red lead`;
      goldDeltaTone = 'red';
    } else {
      goldDeltaText = 'Even';
      goldDeltaTone = 'even';
    }
  }

  return {
    status: STATUS_LABELS[status] || 'Awaiting League Client',
    gameTime: formatGameClock(tick.gameTime),
    summonerName: tick.activePlayer.summonerName,
    championName: tick.activePlayer.championName,
    level: levelAvailable ? `Level ${formatWholeNumber(tick.activePlayer.level)}` : 'Level unavailable',
    levelAvailable,
    kda: kdaAvailable ? `${kda.kills} / ${kda.deaths} / ${kda.assists}` : 'Unavailable',
    kdaAvailable,
    creepScore: creepScoreAvailable
      ? formatWholeNumber(tick.activePlayer.creepScore)
      : 'Unavailable',
    creepScoreAvailable,
    creepScorePerMinute: creepScorePerMinuteAvailable
      ? formatOneDecimal(tick.activePlayer.creepScorePerMinute)
      : 'Unavailable',
    creepScorePerMinuteAvailable,
    killParticipation: killParticipationAvailable
      ? `${formatOneDecimal(tick.activePlayer.killParticipationPercent)}%`
      : 'Unavailable',
    killParticipationAvailable,
    observableHeldValue: observableHeldValueAvailable
      ? `${formatWholeNumber(tick.activePlayer.observableHeldValue)}g`
      : 'Unavailable',
    observableHeldValueAvailable,
    observableValuePerMinute: observableValuePerMinuteAvailable
      ? `${formatWholeNumber(tick.activePlayer.observableValuePerMinute)}g`
      : 'Unavailable',
    observableValuePerMinuteAvailable,
    earnedGoldPerMinute: earnedGoldPerMinuteAvailable
      ? `${formatWholeNumber(tick.activePlayer.earnedGoldPerMinute)}g`
      : 'Unavailable',
    earnedGoldPerMinuteAvailable,
    dpm: dpmAvailable ? formatWholeNumber(tick.activePlayer.dpm) : 'Unavailable',
    dpmAvailable,
    currentGold: currentGoldAvailable
      ? `${formatWholeNumber(tick.activePlayer.currentGold)}g`
      : 'Unavailable',
    currentGoldAvailable,
    goldDelta: goldDeltaText,
    goldDeltaTone,
    dragonType: dragonType || 'Dragon',
    dragonTimer,
    baronTimer: formatObjectiveTimer(tick.objectives.baronTimer),
    xpProgress: xpProgressAvailable
      ? `${formatOneDecimal(tick.activePlayer.xpProgressPercent)}%`
      : 'Unavailable from Live Client API',
    xpProgressValue: xpProgressAvailable ? tick.activePlayer.xpProgressPercent : null,
    xpProgressAvailable,
    integrityNote: observableValuePerMinuteAvailable
      ? 'Held value/min is an estimate from current cash plus current inventory listed value. Earned GPM, DPM, exact team gold, and XP progress are unavailable from Riot’s local API.'
      : 'Earned GPM, DPM, exact team gold, and XP progress are unavailable from Riot’s local API. Aura does not invent them.',
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

/** Render through textContent only; no live payload is interpreted as HTML. */
export function renderLiveGameView(elements, viewModel) {
  setText(elements.status, viewModel.status);
  setText(elements.gameTime, viewModel.gameTime);
  setText(elements.summonerName, viewModel.summonerName);
  setText(elements.championName, viewModel.championName);
  setText(elements.level, viewModel.level);
  setText(elements.kda, viewModel.kda);
  setText(elements.creepScore, viewModel.creepScore);
  setText(elements.creepScorePerMinute, viewModel.creepScorePerMinute);
  setText(elements.killParticipation, viewModel.killParticipation);
  setText(elements.observableHeldValue, viewModel.observableHeldValue);
  setText(elements.observableValuePerMinute, viewModel.observableValuePerMinute);
  setText(elements.earnedGoldPerMinute, viewModel.earnedGoldPerMinute);
  setText(elements.dpm, viewModel.dpm);
  setText(elements.currentGold, viewModel.currentGold);
  setText(elements.goldDelta, viewModel.goldDelta);
  setText(elements.dragonType, viewModel.dragonType);
  setText(elements.dragonTimer, viewModel.dragonTimer);
  setText(elements.baronTimer, viewModel.baronTimer);
  setText(elements.xpProgress, viewModel.xpProgress);
  setText(elements.integrityNote, viewModel.integrityNote);

  if (elements.xpProgressBar) {
    const nextValue = viewModel.xpProgressValue;
    if (nextValue == null) {
      if (!elements.xpProgressBar.hidden) elements.xpProgressBar.hidden = true;
      if (elements.xpProgressBar.value !== 0) elements.xpProgressBar.value = 0;
      setAttribute(elements.xpProgressBar, 'aria-label', 'XP progress unavailable');
      setAttribute(elements.xpProgressBar, 'aria-valuetext', 'Unavailable');
    } else {
      if (elements.xpProgressBar.hidden) elements.xpProgressBar.hidden = false;
      if (elements.xpProgressBar.value !== nextValue) {
        elements.xpProgressBar.value = nextValue;
      }
      setAttribute(elements.xpProgressBar, 'aria-label', 'XP progress to next level');
      setAttribute(elements.xpProgressBar, 'aria-valuetext', viewModel.xpProgress);
    }
  }

  const goldClasses = elements.goldDelta?.classList;
  if (goldClasses) {
    ['blue', 'red', 'even', 'unavailable'].forEach((tone) => {
      setClass(elements.goldDelta, `live-tone-${tone}`, tone === viewModel.goldDeltaTone);
    });
  }

  const dpmClasses = elements.dpm?.classList;
  if (dpmClasses) {
    setClass(elements.dpm, 'live-metric-unavailable', !viewModel.dpmAvailable);
  }

  [
    ['kda', viewModel.kdaAvailable],
    ['currentGold', viewModel.currentGoldAvailable],
    ['level', viewModel.levelAvailable],
    ['creepScore', viewModel.creepScoreAvailable],
    ['creepScorePerMinute', viewModel.creepScorePerMinuteAvailable],
    ['killParticipation', viewModel.killParticipationAvailable],
    ['observableHeldValue', viewModel.observableHeldValueAvailable],
    ['observableValuePerMinute', viewModel.observableValuePerMinuteAvailable],
    ['earnedGoldPerMinute', viewModel.earnedGoldPerMinuteAvailable],
    ['xpProgress', viewModel.xpProgressAvailable],
  ].forEach(([key, available]) => {
    setClass(elements[key], 'live-metric-unavailable', !available);
  });
}
