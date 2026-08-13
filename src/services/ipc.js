export const LIVE_CLIENT_EVENTS = Object.freeze({
  gameStatus: 'game:state-changed',
  gameTick: 'live:game-tick',
  playerUpdate: 'live:player-update',
  draftUpdate: 'draft:update',
});

const GAME_STATUSES = new Set(['IN_LOBBY', 'CHAMP_SELECT', 'IN_GAME', 'ENDED']);
const COMMAND_PATTERN = /^[a-z][a-z0-9_]*$/;

const objectOrEmpty = (value) => (
  value && typeof value === 'object' && !Array.isArray(value) ? value : {}
);

const finiteNumber = (value, fallback = 0, minimum = Number.NEGATIVE_INFINITY) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.max(minimum, parsed) : fallback;
};

const finiteInteger = (value, fallback = 0, minimum = Number.NEGATIVE_INFINITY) => (
  Math.trunc(finiteNumber(value, fallback, minimum))
);

const boundedNumber = (value, fallback = 0, minimum = 0, maximum = 100) => (
  Math.min(maximum, finiteNumber(value, fallback, minimum))
);

const shortText = (value, fallback = '', maximumLength = 128) => {
  if (typeof value !== 'string') return fallback;
  const normalized = value.trim();
  return normalized ? normalized.slice(0, maximumLength) : fallback;
};

/**
 * Wrap Tauri invoke once at the application boundary. Dynamic actions such as
 * Spotify and overlay controls use the same validated, timeout-aware path as
 * literal commands instead of scattering raw invoke calls throughout the UI.
 */
export function createIpcClient(tauriInvoke) {
  if (typeof tauriInvoke !== 'function') {
    throw new TypeError('A Tauri invoke function is required');
  }

  const invoke = (command, args = {}) => {
    if (typeof command !== 'string' || !COMMAND_PATTERN.test(command)) {
      return Promise.reject(new TypeError('Invalid Tauri IPC command name'));
    }
    return Promise.resolve().then(() => tauriInvoke(command, objectOrEmpty(args)));
  };

  const invokeWithTimeout = (command, args = {}, timeoutMs = 15000) => {
    const timeout = finiteInteger(timeoutMs, 15000, 1);
    let timer;
    return Promise.race([
      invoke(command, args),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${command.replaceAll('_', ' ')} timed out after ${Math.ceil(timeout / 1000)} seconds`)),
          timeout,
        );
      }),
    ]).finally(() => clearTimeout(timer));
  };

  return Object.freeze({ invoke, invokeWithTimeout });
}

export function normalizeGameStatus(payload) {
  const candidate = typeof payload === 'string'
    ? payload
    : objectOrEmpty(payload).status;
  const normalized = shortText(candidate).toUpperCase();
  return GAME_STATUSES.has(normalized) ? normalized : null;
}

/** Normalize untrusted event payloads before they touch application state. */
export function normalizeLiveGameTick(payload) {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null;

  const activePlayer = objectOrEmpty(payload.activePlayer ?? payload.active_player);
  const kda = objectOrEmpty(activePlayer.kda);
  const objectives = objectOrEmpty(payload.objectives);
  const availability = objectOrEmpty(
    payload.metricAvailability ?? payload.metric_availability,
  );
  const sources = objectOrEmpty(payload.metricSources ?? payload.metric_sources);
  const dragonType = objectives.dragonType ?? objectives.dragon_type;
  const xpProgress = activePlayer.xpProgressPercent ?? activePlayer.xp_progress_percent;
  const observableHeldValueSource = sources.observableHeldValue
    ?? sources.observable_held_value;
  const observableValueSource = sources.observableValuePerMinute
    ?? sources.observable_value_per_minute;

  return {
    gameTime: finiteNumber(payload.gameTime ?? payload.game_time, 0, 0),
    activePlayer: {
      summonerName: shortText(
        activePlayer.summonerName ?? activePlayer.summoner_name,
        'Unknown summoner',
      ),
      championName: shortText(
        activePlayer.championName ?? activePlayer.champion_name,
        'Unknown champion',
      ),
      currentGold: finiteInteger(
        activePlayer.currentGold ?? activePlayer.current_gold,
        0,
        0,
      ),
      kda: {
        kills: finiteInteger(kda.kills, 0, 0),
        deaths: finiteInteger(kda.deaths, 0, 0),
        assists: finiteInteger(kda.assists, 0, 0),
      },
      dpm: finiteNumber(activePlayer.dpm, 0, 0),
      level: finiteInteger(activePlayer.level, 0, 0),
      creepScore: finiteInteger(
        activePlayer.creepScore ?? activePlayer.creep_score,
        0,
        0,
      ),
      creepScorePerMinute: finiteNumber(
        activePlayer.creepScorePerMinute ?? activePlayer.creep_score_per_minute,
        0,
        0,
      ),
      killParticipationPercent: boundedNumber(
        activePlayer.killParticipationPercent
          ?? activePlayer.kill_participation_percent,
      ),
      observableHeldValue: finiteNumber(
        activePlayer.observableHeldValue ?? activePlayer.observable_held_value,
        0,
        0,
      ),
      observableValuePerMinute: finiteNumber(
        activePlayer.observableValuePerMinute
          ?? activePlayer.observable_value_per_minute,
        0,
        0,
      ),
      earnedGoldPerMinute: finiteNumber(
        activePlayer.earnedGoldPerMinute
          ?? activePlayer.earned_gold_per_minute,
        0,
        0,
      ),
      xpProgressPercent: xpProgress == null
        ? null
        : boundedNumber(xpProgress),
    },
    teamGoldDelta: finiteInteger(payload.teamGoldDelta ?? payload.team_gold_delta, 0),
    objectives: {
      dragonType: dragonType == null ? null : shortText(dragonType, null, 64),
      dragonTimer: finiteNumber(
        objectives.dragonTimer ?? objectives.dragon_timer,
        0,
        0,
      ),
      baronTimer: finiteNumber(
        objectives.baronTimer ?? objectives.baron_timer,
        0,
        0,
      ),
    },
    metricAvailability: {
      currentGold: (availability.currentGold ?? availability.current_gold) === true,
      kda: availability.kda === true,
      dpm: availability.dpm === true,
      teamGoldDelta: (availability.teamGoldDelta ?? availability.team_gold_delta) === true,
      level: availability.level === true,
      creepScore: (availability.creepScore ?? availability.creep_score) === true,
      creepScorePerMinute: (
        availability.creepScorePerMinute ?? availability.creep_score_per_minute
      ) === true,
      killParticipationPercent: (
        availability.killParticipationPercent
          ?? availability.kill_participation_percent
      ) === true,
      observableHeldValue: (
        availability.observableHeldValue ?? availability.observable_held_value
      ) === true,
      observableValuePerMinute: (
        availability.observableValuePerMinute
          ?? availability.observable_value_per_minute
      ) === true,
      earnedGoldPerMinute: (
        availability.earnedGoldPerMinute
          ?? availability.earned_gold_per_minute
      ) === true,
      xpProgressPercent: (
        availability.xpProgressPercent ?? availability.xp_progress_percent
      ) === true,
    },
    metricSources: {
      observableHeldValue: observableHeldValueSource
        === 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE'
        ? observableHeldValueSource
        : null,
      observableValuePerMinute: observableValueSource
        === 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE'
        ? observableValueSource
        : null,
    },
  };
}

export function normalizePlayerStats(payload) {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null;
  const team = shortText(payload.team).toUpperCase();
  if (team !== 'ORDER' && team !== 'HARMONY') return null;

  return {
    summonerName: shortText(
      payload.summonerName ?? payload.summoner_name,
      'Unknown summoner',
    ),
    championName: shortText(
      payload.championName ?? payload.champion_name,
      'Unknown champion',
    ),
    team,
    level: finiteInteger(payload.level, 0, 0),
    creepScore: finiteInteger(payload.creepScore ?? payload.creep_score, 0, 0),
    items: (Array.isArray(payload.items) ? payload.items : [])
      .map((item) => finiteInteger(item, 0, 0))
      .filter(Boolean)
      .slice(0, 7),
  };
}

function reportListenerError(error, onError) {
  if (typeof onError === 'function') onError(error);
}

/**
 * Subscribe to the typed live-client channels and return one idempotent cleanup
 * function. A partial registration failure tears down listeners already opened.
 */
export async function subscribeLiveClientEvents(listen, handlers = {}) {
  if (typeof listen !== 'function') {
    throw new TypeError('A Tauri listen function is required');
  }

  const onError = handlers.onError;
  const unlistenHandles = [];
  let disposed = false;

  const dispose = async () => {
    if (disposed) return;
    disposed = true;
    const handles = unlistenHandles.splice(0).reverse();
    await Promise.allSettled(handles.map((unlisten) => Promise.resolve().then(unlisten)));
  };

  try {
    const statusUnlisten = await listen(LIVE_CLIENT_EVENTS.gameStatus, (event) => {
      try {
        const status = normalizeGameStatus(event?.payload);
        if (status && typeof handlers.onGameStatus === 'function') {
          handlers.onGameStatus(status, event);
        }
      } catch (error) {
        reportListenerError(error, onError);
      }
    });
    if (typeof statusUnlisten !== 'function') {
      throw new TypeError('Tauri game status listener did not return an unlisten function');
    }
    unlistenHandles.push(statusUnlisten);

    const tickUnlisten = await listen(LIVE_CLIENT_EVENTS.gameTick, (event) => {
      try {
        const tick = normalizeLiveGameTick(event?.payload);
        if (tick && typeof handlers.onGameTick === 'function') {
          handlers.onGameTick(tick, event);
        }
      } catch (error) {
        reportListenerError(error, onError);
      }
    });
    if (typeof tickUnlisten !== 'function') {
      throw new TypeError('Tauri game tick listener did not return an unlisten function');
    }
    unlistenHandles.push(tickUnlisten);

    if (typeof handlers.onPlayerUpdate === 'function') {
      const playerUnlisten = await listen(LIVE_CLIENT_EVENTS.playerUpdate, (event) => {
        try {
          const player = normalizePlayerStats(event?.payload);
          if (player) handlers.onPlayerUpdate(player, event);
        } catch (error) {
          reportListenerError(error, onError);
        }
      });
      if (typeof playerUnlisten !== 'function') {
        throw new TypeError('Tauri player update listener did not return an unlisten function');
      }
      unlistenHandles.push(playerUnlisten);
    }
  } catch (error) {
    await dispose();
    throw error;
  }

  return dispose;
}
